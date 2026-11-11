//! A checker type converted to [`PropType`] from the checker's structured answers: type flags, a union's members, a
//! reference's type arguments, a tuple's element flags, an object's properties, index and call signatures, an alias's
//! symbol and arguments. The printed type (`typeToString`) names things for display and for the two facts the
//! structure does not state (see [`crate::printed`]); no shape is read from it.
//!
//! The conversion of a type is a function of the type, how deep it sits and which instantiations enclose it, never of
//! the order the checker was asked in, so the memo can be keyed by the checker's type handle (valid inside one
//! snapshot) without making the output depend on query order. A handle is also what tells one instantiation from
//! another while a type is being expanded (`Box<Box<string>>` is two instantiations of one declaration): only its
//! equality is read, inside one conversion, so the answer does not depend on the numbers. The recursion marker that
//! results is named by the declaration (the alias or interface) and holds no handle. The memo is keyed by the enclosing
//! instantiations a conversion actually asked about, not by all of them (see [`State::memo`]).
//!
//! Every descent into the parts of a type (members, properties, elements, holes) passes [`Converter::enter`], which
//! decides before anything is fetched or converted whether the type is an enclosing instantiation (a back-reference),
//! sits too deep, or is too wide.
//!
//! What stays a name instead of an expansion, the named boundary: an object type declared in a dependency
//! (`node_modules`) or an interface or class of the TypeScript lib (`Date`, `HTMLElement`, `Promise<T>`), with its type
//! arguments; a function type is always expanded, since its parameters are what a reader wants. The lib's type-level
//! utilities (`Partial`, `Pick`, `Omit`, `Record`) are evaluated, not named: their result is the type, unless it is
//! wider than the budget and what it applies to has a name (see [`Converter::over_budget`]). A dependency's union of
//! literals is expanded too, since its members are the values a control offers. What the caps cut is a typed
//! [`PropType::Truncated`], never the checker's text standing in as a name. The width of an object counts only the
//! members the project did not declare: its own are listed whole.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use caliper_docgen_core::react_types::{html_element_for, html_element_from_type_arg, is_intrinsic_tag};
use caliper_docgen_core::types::semantic::{degraded_type, unresolved_type};
use caliper_docgen_core::types::{
    BigIntValue, CallSignature, EnumMemberValue, FunctionParam, ObjectField, OpaqueReason, PropType,
    SignatureKind as Kind, TemplatePart, TruncationLimit, TupleElement, TupleElementKind, TupleLabel,
};
use rustc_hash::FxHashMap;

use crate::declared::Declared;
use crate::printed::{Marker, SignatureMarkers, signature_markers, tuple_labels};
use crate::rpc::{
    CallError, CompactSymbol, ObjectMembers, Rpc, SignatureInfo, SignatureKind, SymbolInfo, TypeInfo, decode, flag,
};
use serde_json::value::RawValue;

/// Nesting past which an object, union or intersection is cut with [`TruncationLimit::Depth`].
const MAX_DEPTH: u8 = 5;
/// The most properties an object type lists at the top (a prop's own type) and below it that the project did not declare
/// itself (see [`is_own`]): what it declared is never cut.
const MAX_FIELDS: [usize; 2] = [100, 40];
/// The most members a union or intersection lists at the top and below it.
const MAX_MEMBERS: [usize; 2] = [500, 120];
/// How many of React's tag names (see [`is_tag`]) a union has to list to be React's `ElementType` (`JSX.IntrinsicElements`
/// has over 170): the tag names are the one part of it that no declaration identifies.
const MIN_INTRINSIC_TAGS: usize = 100;
/// The lib's utility aliases that keep the properties of the type they are applied to, and so can stand for it.
const UTILITIES: [&str; 5] = ["Omit", "Pick", "Partial", "Required", "Readonly"];

/// What a descent found at [`Converter::enter`].
enum Gate<T> {
    /// The type is not expanded: it is a back-reference, or a cap cut it.
    Stop(PropType),
    /// The parts of the type, and the context to convert them in.
    Open { parts: T, inner: Cx },
}

type Converted<'a> = Pin<Box<dyn Future<Output = Result<PropType, CallError>> + Send + 'a>>;

/// Where a type sits: how deep, and which named instantiations (an alias or interface with its type arguments, as the
/// checker's type handle) enclose it, outermost first.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cx {
    depth: u8,
    enclosing: Arc<Vec<u64>>,
    /// The conversion this context belongs to, which records which enclosing instantiations it asks about. `None` at
    /// the roots.
    frame: Option<Arc<Frame>>,
}

/// What one conversion has asked of the instantiations enclosing it, kept to build its memo entry (see
/// [`State::memo`]).
#[derive(Debug)]
struct Frame {
    /// How many instantiations enclosed the type when its conversion began: those below this are the outer context, the
    /// rest were opened by the conversion itself.
    base: usize,
    asked: Mutex<Asked>,
}

#[derive(Debug, Default)]
struct Asked {
    /// The handles this conversion's own gate compared against its enclosing instantiations.
    ids: Vec<u64>,
    /// The sets asked by the conversions of its parts, each with the instantiations this conversion had opened by then:
    /// those were answered inside it and do not depend on its outer context.
    parts: Vec<(Arc<[u64]>, Vec<u64>)>,
}

impl Frame {
    fn new(base: usize) -> Self {
        Frame { base, asked: Mutex::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Asked> {
        self.asked.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn ask(&self, id: u64) {
        self.lock().ids.push(id);
    }

    /// A part of this conversion was converted (or recalled) under `enclosing`, having asked about `asked`.
    fn absorb(&self, asked: &Arc<[u64]>, enclosing: &[u64]) {
        if !asked.is_empty() {
            self.lock().parts.push((asked.clone(), enclosing.get(self.base..).unwrap_or_default().to_vec()));
        }
    }

    /// Every handle the conversion asked about whose answer came from outside it: sorted, once each.
    fn finish(&self) -> Arc<[u64]> {
        let asked = self.lock();
        let mut ids = asked.ids.clone();
        for (part, opened) in &asked.parts {
            ids.extend(part.iter().copied().filter(|id| !opened.contains(id)));
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into()
    }
}

impl Cx {
    /// The context of what a name carries, its type arguments: one level down. Each argument is converted on its own
    /// and passes [`Converter::enter`] if it has parts.
    fn argument(&self) -> Cx {
        Cx { depth: self.depth.saturating_add(1), enclosing: self.enclosing.clone(), frame: self.frame.clone() }
    }

    /// The context of the parts of a type that [`Converter::enter`] has admitted. Only a named instance is kept, since
    /// only a name can be pointed back to.
    fn below(&self, instance: Option<u64>) -> Cx {
        let mut enclosing = self.enclosing.clone();
        if let Some(id) = instance
            && !enclosing.contains(&id)
        {
            let mut grown = (*enclosing).clone();
            grown.push(id);
            enclosing = Arc::new(grown);
        }
        Cx { depth: self.depth.saturating_add(1), enclosing, frame: self.frame.clone() }
    }

    /// Whether `id` is an instantiation being expanded around the type. The conversion in progress is recorded to have
    /// asked, whichever the answer.
    fn encloses(&self, id: u64) -> bool {
        if let Some(frame) = &self.frame {
            frame.ask(id);
        }
        self.enclosing.contains(&id)
    }

    fn limit<const N: usize>(&self, limits: [usize; N]) -> usize {
        limits[usize::from(self.depth > 0).min(N - 1)]
    }
}

/// What declares a type: the alias it was written through, else its interface or class.
#[derive(Debug, Clone)]
struct Origin {
    name: String,
    /// The declaring file as the checker keys it (lower-cased path).
    file: String,
    alias: bool,
}

impl Origin {
    fn of(symbol: &SymbolInfo, alias: bool) -> Self {
        let declaration = symbol.declarations.first().map(String::as_str);
        Origin {
            name: symbol.name.clone(),
            file: declaration.map(|declaration| declared_in(declaration).to_string()).unwrap_or_default(),
            alias,
        }
    }

    fn in_dependency(&self) -> bool {
        file_in_dependency(&self.file)
    }

    fn in_lib(&self) -> bool {
        file_in_lib(&self.file)
    }

    fn in_react(&self) -> bool {
        self.file.contains("/@types/react/")
    }

    /// Whether the name stands for the type: declared in a dependency, or an interface or class of the lib. An alias
    /// of the lib is a type-level function (`Partial<T>`), and its result is the type.
    fn is_boundary(&self) -> bool {
        // The lib may itself sit under `node_modules` (`typescript/lib`, `@tsrs/<platform>`).
        if self.in_lib() { !self.alias } else { self.in_dependency() }
    }

    /// The React type that PropType holds as a terminal, found by the declaration and not by how it prints.
    fn react_terminal(&self) -> Option<PropType> {
        if !self.in_react() {
            return None;
        }
        match (self.name.as_str(), self.alias) {
            ("ReactNode", true) | ("ReactElement" | "Element", false) => Some(PropType::ReactNode),
            ("ElementType", true) => Some(PropType::ElementType),
            ("CSSProperties", false) => Some(PropType::CssProperties),
            _ => None,
        }
    }
}

/// Whether a union member is a tag of React's `JSX.IntrinsicElements`: a string literal (not an enum member) whose value
/// that table lists. A literal the checker gave no value for is not one, so it is kept beside the terminal.
fn is_tag(member: &TypeInfo) -> bool {
    member.flags & flag::STRING_LITERAL != 0
        && member.flags & flag::ENUM_LITERAL == 0
        && member.value.as_ref().and_then(|value| value.as_str()).is_some_and(is_intrinsic_tag)
}

/// Whether the checker states the base types of this type in its own terms: the declaration of a class or interface
/// does. An instantiation of a generic one (`Box<string>`, only a reference) does not state them in terms of its
/// arguments, so a traversal that keeps the arguments cannot list what it inherits; one that wants only names reads the
/// generic type instead ([`Converter::named_bases`]).
fn states_bases(info: &TypeInfo) -> bool {
    info.object_flags & (flag::OBJECT_CLASS | flag::OBJECT_INTERFACE) != 0
}

/// Where the base types of a type are to be asked, from what the checker says of the type itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BasesOf {
    /// A class or interface declaration: the checker states them ([`states_bases`]).
    Itself,
    /// An instantiation of a generic type: its generic target states them.
    Generic,
    /// An object type no class or interface declares (a type literal, a mapped type): it has none.
    None,
    /// A union, a type parameter or any other type the checker was not asked about: whether it has bases is not known.
    Unknown,
}

impl BasesOf {
    fn of(info: &TypeInfo) -> Self {
        if states_bases(info) {
            Self::Itself
        } else if info.object_flags & flag::OBJECT_REFERENCE != 0 {
            Self::Generic
        } else if info.flags & flag::OBJECT != 0 {
            Self::None
        } else {
            Self::Unknown
        }
    }
}

fn file_in_dependency(file: &str) -> bool {
    file.contains("/node_modules/")
}

fn file_in_lib(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or("");
    name.starts_with("lib.") && name.ends_with(".d.ts")
}

/// Whether a property is the project's own: its first declaration is in a file of the project, not a dependency or the
/// lib. A property with no declaration (a mapped type's synthetic one) is not, so a wide mapped type is still cut.
fn is_own(property: &SymbolInfo) -> bool {
    property.declarations.first().is_some_and(|declaration| {
        let file = declared_in(declaration);
        !file_in_dependency(file) && !file_in_lib(file)
    })
}

/// `declarations` entries read `"<node index>.<kind>.<path>"`: the path is lower-cased, as the checker keys files.
pub(crate) fn declared_in(declaration: &str) -> &str {
    declaration.splitn(3, '.').nth(2).unwrap_or("")
}

/// A converted type, with the enclosing instantiations its conversion depended on.
#[derive(Debug)]
struct Entry {
    /// Every handle the conversion compared against the instantiations enclosing it, sorted.
    asked: Arc<[u64]>,
    /// Those of `asked` that did enclose it.
    enclosed_by: Vec<u64>,
    converted: PropType,
}

impl Entry {
    /// Whether the conversion would go the same way inside `enclosing`: among the handles it asked about, the ones
    /// that enclose the type there are exactly `enclosed_by`.
    fn holds_in(&self, enclosing: &[u64]) -> bool {
        self.enclosed_by.iter().all(|id| enclosing.contains(id))
            && enclosing.iter().all(|id| self.enclosed_by.contains(id) || self.asked.binary_search(id).is_err())
    }
}

type Memo = Mutex<FxHashMap<(u64, u8), Vec<Arc<Entry>>>>;

/// The parts of each type asked about so far, by (type handle, which parts), as the text the checker answered: read
/// again on each use, which costs a fraction of the round trip, and is several times smaller than the read lists.
type PartsSeen = Mutex<FxHashMap<(u64, Parts), Arc<RawValue>>>;

/// Which parts of a type [`Converter::parts`] asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Parts {
    /// A union's, intersection's or template literal's members (`getTypesOfType`).
    Members,
    /// A reference's type arguments (`getTypeArguments`), the `this` type the checker appends included.
    Arguments,
}

/// The type arguments a reference names, without the `this` type the checker appends to a class's or interface's.
fn without_this(arguments: &[TypeInfo]) -> &[TypeInfo] {
    let named = arguments.iter().rposition(|argument| !argument.is_this_type).map_or(0, |last| last + 1);
    arguments.get(..named).unwrap_or_default()
}

#[derive(Default)]
struct State {
    /// Conversions by (type handle, depth), each entry with the enclosing instantiations it depended on. The handles
    /// are valid inside this snapshot only.
    ///
    /// Why an entry may be reused where [`Entry::holds_in`] says. A conversion reads its context in two ways only: the
    /// depth, which is in the key, and [`Cx::encloses`], the question whether a handle is an instantiation being
    /// expanded around the type. Everything else it reads is the checker's answer about the type, which does not change
    /// inside a snapshot. So a conversion is a deterministic walk whose every step is fixed by the type, the depth and
    /// the answers to those questions, in the order they are asked. Run it again where each handle it asked about has
    /// the same answer, and it asks the same questions in the same order and returns the same type. A handle that the
    /// conversion opened itself is settled inside it (it was found not to enclose the type at its own gate, which is
    /// recorded), so only the handles asked of the outer context are kept. A type in no cycle with its context asks
    /// nothing that encloses it and holds everywhere; one in a cycle holds where the same part of the cycle encloses
    /// it. Entries that hold in a context all agree, so which of them is found first, and so the order in which
    /// types were first reached, cannot show in the output.
    memo: Memo,
    origins: Mutex<FxHashMap<(bool, CompactSymbol), Option<Origin>>>,
    /// By (type handle, whether printed expanded: see [`Rpc::type_to_string_expanded`]).
    printed: Mutex<FxHashMap<(u64, bool), String>>,
    /// The parts of a type by its handle (see [`Parts`]). They are the checker's answer about the type, fixed inside the
    /// snapshot, while the memo converts a type again at another depth or under other instantiations: kept, that
    /// conversion asks for none of them again. Kept for the whole snapshot, since components of one library share
    /// most of their types: kept per component, a program of every Chakra component answered 16 at a time held 16
    /// copies of the same style props' members.
    parts: PartsSeen,
    /// Where props are declared ([`crate::declared`]), kept for the snapshot like the rest.
    declared: crate::declared::Memo,
    hits: AtomicU64,
    misses: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct Converter {
    rpc: Arc<Rpc>,
    state: Arc<State>,
    /// Ask the checker about the members of a type last to first. The answer is the same, and a test holds it to that.
    reverse: bool,
}

impl Converter {
    pub(crate) fn new(rpc: Arc<Rpc>, reverse: bool) -> Self {
        Self { rpc, state: Arc::new(State::default()), reverse }
    }

    /// Every type converted, in the order of `infos`, though when `reverse` is set the checker is asked last to first.
    async fn convert_all(&self, infos: &[TypeInfo], cx: &Cx) -> Result<Vec<PropType>, CallError> {
        let mut slots: Vec<Option<PropType>> = vec![None; infos.len()];
        let order: Box<dyn Iterator<Item = usize> + Send> =
            if self.reverse { Box::new((0..infos.len()).rev()) } else { Box::new(0..infos.len()) };
        for index in order {
            if let (Some(info), Some(slot)) = (infos.get(index), slots.get_mut(index)) {
                *slot = Some(self.convert(info, cx).await?);
            }
        }
        Ok(slots.into_iter().flatten().collect())
    }

    /// Memo hits and misses so far, for the debug line.
    pub(crate) fn memo_stats(&self) -> (u64, u64) {
        (self.state.hits.load(Ordering::Relaxed), self.state.misses.load(Ordering::Relaxed))
    }

    /// The own types of a component's props, from the top.
    pub(crate) async fn convert_roots(&self, infos: &[TypeInfo]) -> Result<Vec<PropType>, CallError> {
        self.convert_all(infos, &Cx::default()).await
    }

    /// Where each prop of `asked` is declared and what its JSDoc says ([`crate::declared`]).
    pub(crate) async fn declared(&self, asked: &[crate::declared::Asked<'_>]) -> Result<Vec<Declared>, CallError> {
        crate::declared::declared(&self.rpc, &self.state.declared, asked).await
    }

    /// The `@types/react` types a props type is made of, by name, in the order it names them: its bases and the members
    /// of an intersection, seen through the project's and dependencies' own types and through a lib utility (`Omit`,
    /// `Pick`) to what it applies to. A React type is found by where it is declared, as [`Origin::react_terminal`]
    /// finds React's terminals, and is not looked into. `None` when a type on the way has bases the checker cannot be
    /// asked for ([`Self::named_bases`]): the list would not be the answer, and an empty one would say there are none.
    pub(crate) async fn composes(&self, props_type: &TypeInfo) -> Result<Option<Vec<String>>, CallError> {
        let mut names: Vec<String> = Vec::new();
        let mut seen: Vec<u64> = Vec::new();
        let mut pending = vec![props_type.clone()];
        while let Some(info) = pending.pop() {
            if seen.contains(&info.id) {
                continue;
            }
            seen.push(info.id);
            let origin = self.origin(&info).await?;
            if let Some(origin) = &origin {
                if origin.in_react() {
                    if !names.contains(&origin.name) {
                        names.push(origin.name.clone());
                    }
                    continue;
                }
                if origin.alias && origin.in_lib() && UTILITIES.contains(&origin.name.as_str()) {
                    pending.extend(self.rpc.alias_type_arguments(info.id).await?.into_iter().take(1));
                    continue;
                }
            }
            let parts = if info.flags & flag::INTERSECTION != 0 {
                self.parts(info.id, Parts::Members).await?
            } else {
                let Some(bases) = self.named_bases(&info).await? else { return Ok(None) };
                bases
            };
            // Last first onto the stack, so they are taken in the order the type names them.
            pending.extend(parts.into_iter().rev());
        }
        Ok(Some(names))
    }

    /// The base types of `info` for a traversal that wants their names and not their arguments. The checker states the
    /// bases of the declaration of a class or interface; an instantiation of a generic one (`Props<string>`, only a
    /// reference) has none of its own, so they are read from the generic type it instantiates, which names the same
    /// types over its own type parameters. `None` when the checker cannot be asked ([`BasesOf::Unknown`]). A traversal
    /// that keeps the arguments of what it finds ([`Self::base_layers`]) cannot read them this way, since
    /// `HTMLAttributes<T>` is not `HTMLAttributes<HTMLDivElement>`, and tests [`states_bases`] of the type itself.
    async fn named_bases(&self, info: &TypeInfo) -> Result<Option<Vec<TypeInfo>>, CallError> {
        match BasesOf::of(info) {
            BasesOf::Itself => Ok(Some(self.rpc.base_types(info.id).await?)),
            BasesOf::Generic => {
                let target = self.rpc.target_of_type(info.id).await?;
                match BasesOf::of(&target) {
                    BasesOf::Itself => Ok(Some(self.rpc.base_types(target.id).await?)),
                    _ => Ok(info.is_tuple_type.then(Vec::new)),
                }
            }
            BasesOf::None => Ok(Some(Vec::new())),
            BasesOf::Unknown => Ok(None),
        }
    }

    fn convert<'a>(&'a self, info: &'a TypeInfo, cx: &'a Cx) -> Converted<'a> {
        Box::pin(async move {
            if let Some(entry) = self.recall(info.id, cx) {
                self.state.hits.fetch_add(1, Ordering::Relaxed);
                if let Some(frame) = &cx.frame {
                    frame.absorb(&entry.asked, &cx.enclosing);
                }
                return Ok(entry.converted.clone());
            }
            self.state.misses.fetch_add(1, Ordering::Relaxed);
            let frame = Arc::new(Frame::new(cx.enclosing.len()));
            let own = Cx { depth: cx.depth, enclosing: cx.enclosing.clone(), frame: Some(frame.clone()) };
            let converted = self.uncached(info, &own).await?;
            let asked = frame.finish();
            if let Some(parent) = &cx.frame {
                parent.absorb(&asked, &cx.enclosing);
            }
            let enclosed_by = cx.enclosing.iter().copied().filter(|id| asked.binary_search(id).is_ok()).collect();
            let entry = Arc::new(Entry { asked, enclosed_by, converted: converted.clone() });
            if let Ok(mut memo) = self.state.memo.lock() {
                memo.entry((info.id, cx.depth)).or_default().push(entry);
            }
            Ok(converted)
        })
    }

    /// The entry for the type at this depth that holds inside the instantiations enclosing it, if there is one.
    fn recall(&self, id: u64, cx: &Cx) -> Option<Arc<Entry>> {
        let memo = self.state.memo.lock().ok()?;
        memo.get(&(id, cx.depth))?.iter().find(|entry| entry.holds_in(&cx.enclosing)).cloned()
    }

    async fn uncached(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        if info.is_error() {
            return Ok(unresolved_type(self.display(info.id).await?));
        }
        if let Some(simple) = simple(info) {
            return Ok(simple);
        }
        let flags = info.flags;
        if flags & flag::TYPE_PARAMETER != 0 {
            return self.type_parameter(info).await;
        }
        if flags & (flag::STRING_LITERAL | flag::NUMBER_LITERAL | flag::BIG_INT_LITERAL) != 0 {
            return self.literal(info).await;
        }
        if flags & flag::BOOLEAN_LITERAL != 0 {
            return Ok(PropType::BoolLiteral(self.boolean_value(info).await?));
        }
        if flags & flag::UNIQUE_ES_SYMBOL != 0 {
            let name = self.rpc.symbol_of_type(info.id).await?.map(|symbol| symbol.name);
            return Ok(match name {
                Some(name) => PropType::UniqueSymbol { name: name.into() },
                None => named(self.display(info.id).await?, Vec::new()),
            });
        }
        if flags & flag::TEMPLATE_LITERAL != 0 {
            return self.template(info, cx).await;
        }
        // `Uppercase<string>` and its siblings: a name, since there is no shape to expand.
        if flags & flag::STRING_MAPPING != 0 {
            return Ok(named(self.display(info.id).await?, Vec::new()));
        }
        // Types the checker could not resolve because a type parameter is still open.
        if flags & (flag::CONDITIONAL | flag::INDEXED_ACCESS | flag::INDEX | flag::SUBSTITUTION) != 0 {
            let text = self.display(info.id).await?;
            let reason = if flags & flag::CONDITIONAL != 0 {
                OpaqueReason::ConditionalType
            } else if flags & flag::INDEXED_ACCESS != 0 {
                OpaqueReason::IndexedAccess { expression: text.clone() }
            } else {
                OpaqueReason::UnsupportedExpression
            };
            return Ok(degraded_type(text, reason));
        }
        if flags & flag::UNION != 0 {
            return self.union(info, cx).await;
        }
        if flags & flag::INTERSECTION != 0 {
            return self.intersection(info, cx).await;
        }
        if flags & flag::OBJECT != 0 {
            return self.object(info, cx).await;
        }
        Ok(named(self.display(info.id).await?, Vec::new()))
    }

    /// How the checker prints the type; only ever a name for display, a truncated type's text, or the source of the two
    /// facts in [`crate::printed`].
    async fn display(&self, id: u64) -> Result<String, CallError> {
        self.printed(id, false).await
    }

    /// How the checker prints the type with an alias at the top expanded and nothing cut short: the text a signature's
    /// parameter markers are read from, since the alias's name states no parameter list.
    async fn display_expanded(&self, id: u64) -> Result<String, CallError> {
        self.printed(id, true).await
    }

    async fn printed(&self, id: u64, expanded: bool) -> Result<String, CallError> {
        let key = (id, expanded);
        if let Some(text) = self.state.printed.lock().ok().and_then(|printed| printed.get(&key).cloned()) {
            return Ok(text);
        }
        let text =
            if expanded { self.rpc.type_to_string_expanded(id).await? } else { self.rpc.type_to_string(id).await? };
        if let Ok(mut printed) = self.state.printed.lock() {
            printed.insert(key, text.clone());
        }
        Ok(text)
    }

    /// The members or type arguments of a type, asked once per type however many conversions read them.
    async fn parts(&self, id: u64, kind: Parts) -> Result<Vec<TypeInfo>, CallError> {
        let key = (id, kind);
        let known = self.state.parts.lock().ok().and_then(|parts| parts.get(&key).cloned());
        let text = match known {
            Some(text) => text,
            None => {
                let fetched: Arc<RawValue> = match kind {
                    Parts::Members => self.rpc.members_text(id).await?,
                    Parts::Arguments => self.rpc.type_arguments_text(id).await?,
                }
                .into();
                if let Ok(mut parts) = self.state.parts.lock() {
                    parts.insert(key, fetched.clone());
                }
                fetched
            }
        };
        decode(&text)
    }

    /// The alias a type was written through, when it has one.
    async fn alias_origin(&self, info: &TypeInfo) -> Result<Option<Origin>, CallError> {
        match &info.alias_symbol {
            Some(alias) => self.origin_of(alias, true, info.id).await,
            None => Ok(None),
        }
    }

    /// What declares the type, when it has a name of its own: an alias, or an interface, class or instantiation of one.
    async fn origin(&self, info: &TypeInfo) -> Result<Option<Origin>, CallError> {
        if info.alias_symbol.is_some() {
            return self.alias_origin(info).await;
        }
        if info.object_flags & (flag::OBJECT_CLASS | flag::OBJECT_INTERFACE | flag::OBJECT_REFERENCE) != 0 {
            return self.symbol_origin(info).await;
        }
        Ok(None)
    }

    /// The interface or class a type is, whatever alias it was written through.
    async fn symbol_origin(&self, info: &TypeInfo) -> Result<Option<Origin>, CallError> {
        match &info.symbol {
            Some(symbol) => self.origin_of(symbol, false, info.id).await,
            None => Ok(None),
        }
    }

    /// The declaration behind a symbol, asked once per symbol however many types refer to it.
    async fn origin_of(&self, symbol: &CompactSymbol, alias: bool, type_id: u64) -> Result<Option<Origin>, CallError> {
        let key = (alias, symbol.clone());
        if let Some(known) = self.state.origins.lock().ok().and_then(|origins| origins.get(&key).cloned()) {
            return Ok(known);
        }
        let found =
            if alias { self.rpc.alias_symbol(type_id).await? } else { self.rpc.symbol_of_type(type_id).await? };
        // `__type` and `__object` are the checker's names for a type that has none.
        let found = found.map(|symbol| Origin::of(&symbol, alias)).filter(|origin| !origin.name.starts_with("__"));
        if let Ok(mut origins) = self.state.origins.lock() {
            origins.insert(key, found.clone());
        }
        Ok(found)
    }

    async fn type_parameter(&self, info: &TypeInfo) -> Result<PropType, CallError> {
        if info.is_this_type {
            return Ok(named("this".to_string(), Vec::new()));
        }
        let name = match self.rpc.symbol_of_type(info.id).await? {
            Some(symbol) => symbol.name,
            None => self.display(info.id).await?,
        };
        Ok(named(name, Vec::new()))
    }

    async fn literal(&self, info: &TypeInfo) -> Result<PropType, CallError> {
        let flags = info.flags;
        let value = info.value.as_ref();
        let known = if flags & flag::BIG_INT_LITERAL != 0 {
            value.and_then(|value| value.as_str()).and_then(BigIntValue::parse).map(PropType::BigIntLiteral)
        } else if flags & flag::STRING_LITERAL != 0 {
            value.and_then(|value| value.as_str()).map(|text| PropType::StringLiteral(text.to_string()))
        } else {
            value.and_then(|value| value.as_f64()).map(PropType::NumberLiteral)
        };
        let Some(literal) = known else { return Ok(named(self.display(info.id).await?, Vec::new())) };
        if flags & flag::ENUM_LITERAL == 0 {
            return Ok(literal);
        }
        // A member of an enum is its value and keeps the enum's name.
        let Some(member) = self.rpc.symbol_of_type(info.id).await? else { return Ok(literal) };
        let Some(parent) = self.rpc.parent_of_symbol(&member.reference).await? else { return Ok(literal) };
        let value = match literal {
            PropType::StringLiteral(text) => EnumMemberValue::String(text),
            PropType::NumberLiteral(number) => EnumMemberValue::Number(number),
            other => return Ok(other),
        };
        Ok(PropType::EnumMember { enum_name: parent.name.into(), member: member.name.into(), value })
    }

    /// The beta's responses carry neither `value` nor, in some builds, `intrinsicName` for `true` and `false`.
    async fn boolean_value(&self, info: &TypeInfo) -> Result<bool, CallError> {
        if let Some(name) = info.intrinsic_name.as_deref() {
            return Ok(name == "true");
        }
        if let Some(value) = info.value.as_ref().and_then(|value| value.as_bool()) {
            return Ok(value);
        }
        Ok(self.display(info.id).await? == "true")
    }

    async fn template(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let origin = self.alias_origin(info).await?;
        let gate = self.enter(info, origin.as_ref(), cx, MAX_MEMBERS, || self.members_of(info)).await?;
        let (holes, inner) = match gate {
            Gate::Stop(stopped) => return Ok(stopped),
            Gate::Open { parts, inner } => (parts, inner),
        };
        let mut types = self.convert_all(&holes, &inner).await?.into_iter();
        let mut parts = Vec::new();
        for text in &info.texts {
            if !text.is_empty() {
                parts.push(TemplatePart::Text(text.clone()));
            }
            if let Some(hole) = types.next() {
                parts.push(TemplatePart::Type(hole));
            }
        }
        Ok(PropType::TemplateLiteral(parts))
    }

    /// The cut type for a cap: its declaration's name, else how the checker prints it.
    async fn truncated(
        &self,
        info: &TypeInfo,
        origin: Option<&Origin>,
        limit: TruncationLimit,
    ) -> Result<PropType, CallError> {
        let text = match origin {
            Some(origin) => origin.name.clone(),
            None => self.display(info.id).await?,
        };
        Ok(PropType::Truncated { text, limit })
    }

    /// The reference back to a declaration that is being expanded around this type.
    async fn recursive(&self, info: &TypeInfo, origin: &Origin, cx: &Cx) -> Result<PropType, CallError> {
        let args = self.type_arguments_of(info, origin, cx).await?;
        Ok(PropType::Recursive { name: origin.name.clone().into(), args })
    }

    /// The type arguments a name carries: an alias's own, or the instantiation's (without the `this` type the checker
    /// appends to a class or interface).
    async fn type_arguments_of(&self, info: &TypeInfo, origin: &Origin, cx: &Cx) -> Result<Vec<PropType>, CallError> {
        if origin.alias {
            if info.alias_type_arguments.is_empty() {
                return Ok(Vec::new());
            }
            let arguments = self.rpc.alias_type_arguments(info.id).await?;
            return self.convert_all(&arguments, &cx.argument()).await;
        }
        if info.object_flags & flag::OBJECT_REFERENCE == 0 {
            return Ok(Vec::new());
        }
        let arguments = self.parts(info.id, Parts::Arguments).await?;
        self.convert_all(without_this(&arguments), &cx.argument()).await
    }

    async fn named_type(&self, info: &TypeInfo, origin: &Origin, cx: &Cx) -> Result<PropType, CallError> {
        let args = self.type_arguments_of(info, origin, cx).await?;
        Ok(PropType::Named { name: origin.name.clone().into(), args })
    }

    /// The one gate every descent into the parts of a type passes, before the parts are converted. It stops, in this
    /// order, a type that is an instantiation already being expanded around it (a back-reference to its declaration,
    /// whose arguments are the instantiation's), a type past [`MAX_DEPTH`], and a type whose `measure`d parts exceed the
    /// `limits`. `measure` fetches the parts and says how many there are; it is not called for a type stopped before.
    /// Otherwise the parts come back with the context to convert them in.
    async fn enter<T, F, Fut>(
        &self,
        info: &TypeInfo,
        origin: Option<&Origin>,
        cx: &Cx,
        limits: [usize; 2],
        measure: F,
    ) -> Result<Gate<T>, CallError>
    where
        T: Send,
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<(T, usize), CallError>> + Send,
    {
        // The handle is the instantiation: `Box<string>` inside `Box<Box<string>>` is another type from the one around it.
        if let Some(origin) = origin
            && cx.encloses(info.id)
        {
            return Ok(Gate::Stop(self.recursive(info, origin, cx).await?));
        }
        if cx.depth >= MAX_DEPTH {
            return Ok(Gate::Stop(self.truncated(info, origin, TruncationLimit::Depth).await?));
        }
        let (parts, total) = measure().await?;
        if total > cx.limit(limits) {
            let limit = TruncationLimit::Members { total: to_u32(total) };
            return Ok(Gate::Stop(self.truncated(info, origin, limit).await?));
        }
        Ok(Gate::Open { parts, inner: cx.below(origin.map(|_| info.id)) })
    }

    async fn union(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let origin = self.origin(info).await?;
        if let Some(terminal) = origin.as_ref().and_then(Origin::react_terminal) {
            return Ok(terminal);
        }
        // The width is checked below, after React's aliases flattened into the union are recognized: they are wide.
        let gate = self.enter(info, origin.as_ref(), cx, [usize::MAX; 2], || self.members_of(info)).await?;
        let (members, inner) = match gate {
            Gate::Stop(stopped) => return Ok(stopped),
            Gate::Open { parts, inner } => (parts, inner),
        };
        // `ReactNode | (() => void)` arrives with the alias flattened into its members: they are the terminal again.
        let over = members.len() > cx.limit(MAX_MEMBERS);
        let Some((terminal, absorbed)) = self.react_alias_members(&members, over).await? else {
            if over {
                let limit = TruncationLimit::Members { total: to_u32(members.len()) };
                return self.truncated(info, origin.as_ref(), limit).await;
            }
            return Ok(union_of(self.convert_all(&members, &inner).await?));
        };
        let kept: Vec<TypeInfo> = members
            .iter()
            .zip(&absorbed)
            .filter(|(_, absorbed)| !**absorbed)
            .map(|(member, _)| member.clone())
            .collect();
        if kept.len() + 1 > cx.limit(MAX_MEMBERS) {
            let limit = TruncationLimit::Members { total: to_u32(kept.len() + 1) };
            return self.truncated(info, origin.as_ref(), limit).await;
        }
        let mut converted = self.convert_all(&kept, &inner).await?;
        let first = absorbed.iter().position(|absorbed| *absorbed).unwrap_or_default();
        let before = absorbed.iter().take(first).filter(|absorbed| !**absorbed).count();
        converted.insert(before, terminal);
        Ok(union_of(converted))
    }

    /// The members of a union that are a React alias the checker flattened into it, with the terminal they stand for,
    /// when the union holds all of that alias's members. `ReactNode | Fn` and `ElementType | undefined` arrive without
    /// the alias, so what is left to recognize it by is React's own declarations, never the printed text. `ElementType`
    /// is recognized only where the union is wider than the budget: below it the members are listed, which keeps the
    /// props of a constrained `ElementType<P>` in its component types' arguments, as it does at the top of a prop.
    async fn react_alias_members(
        &self,
        members: &[TypeInfo],
        over: bool,
    ) -> Result<Option<(PropType, Vec<bool>)>, CallError> {
        if let Some(absorbed) = self.react_node_members(members).await? {
            return Ok(Some((PropType::ReactNode, absorbed)));
        }
        if !over {
            return Ok(None);
        }
        Ok(self.element_type_members(members).await?.map(|absorbed| (PropType::ElementType, absorbed)))
    }

    /// Which members of a union are those of React's `ElementType`: the tag names of `JSX.IntrinsicElements` as string
    /// literals and `ComponentType`'s two arms, `ComponentClass` and `FunctionComponent`. The tag names are many
    /// ([`MIN_INTRINSIC_TAGS`]), which keeps a union of a few tags and a component type the author's own. Only a literal
    /// that [`is_tag`] is taken: any other string literal is an alternative the author wrote beside `ElementType`, and
    /// the terminal does not stand for it.
    async fn element_type_members(&self, members: &[TypeInfo]) -> Result<Option<Vec<bool>>, CallError> {
        let tags = members.iter().filter(|member| is_tag(member)).count();
        if tags < MIN_INTRINSIC_TAGS {
            return Ok(None);
        }
        let (mut class, mut function) = (false, false);
        let mut absorbed = Vec::with_capacity(members.len());
        for member in members {
            let mut takes = is_tag(member);
            if member.flags & flag::OBJECT != 0
                && member.object_flags & (flag::OBJECT_CLASS | flag::OBJECT_INTERFACE | flag::OBJECT_REFERENCE) != 0
                && let Some(origin) = self.symbol_origin(member).await?
                && origin.in_react()
            {
                match origin.name.as_str() {
                    "ComponentClass" => (class, takes) = (true, true),
                    "FunctionComponent" => (function, takes) = (true, true),
                    _ => {}
                }
            }
            absorbed.push(takes);
        }
        Ok((class && function).then_some(absorbed))
    }

    /// Which members of a union are the members of React's `ReactNode`, when the union holds all of them. The union has
    /// `ReactElement`, `ReactPortal` and an `Iterable` of `ReactNode` (found by its argument's alias) and the primitives
    /// `ReactNode` lists. It then also takes the other members `ReactNode` has (`bigint`, `Promise<AwaitedReactNode>`),
    /// and leaves the rest. A union that has only some of them is the author's own and is expanded.
    async fn react_node_members(&self, members: &[TypeInfo]) -> Result<Option<Vec<bool>>, CallError> {
        let primitive = |flags: u64| flags & (flag::STRING | flag::NUMBER | flag::BOOLEAN | flag::BOOLEAN_LITERAL) != 0;
        let has = |flags: u64| members.iter().any(|member| member.flags & flags != 0);
        let listed = has(flag::STRING)
            && has(flag::NUMBER)
            && has(flag::BOOLEAN | flag::BOOLEAN_LITERAL)
            && has(flag::NULL)
            && has(flag::UNDEFINED);
        if !listed {
            return Ok(None);
        }
        let (mut element, mut portal, mut iterable) = (false, false, false);
        let mut absorbed = Vec::with_capacity(members.len());
        for member in members {
            let flags = member.flags;
            let takes = if primitive(flags) || flags & (flag::BIG_INT | flag::NULL | flag::UNDEFINED) != 0 {
                true
            } else if flags & flag::OBJECT != 0
                && member.object_flags & (flag::OBJECT_CLASS | flag::OBJECT_INTERFACE | flag::OBJECT_REFERENCE) != 0
                && let Some(origin) = self.symbol_origin(member).await?
            {
                let react = origin.in_react() && !origin.alias;
                let lib = origin.in_lib() && !origin.alias;
                if react && origin.name == "ReactElement" {
                    element = true;
                    true
                } else if react && origin.name == "ReactPortal" {
                    portal = true;
                    true
                } else if lib && origin.name == "Iterable" && self.argument_is_alias(member, "ReactNode").await? {
                    iterable = true;
                    true
                } else {
                    lib && origin.name == "Promise" && self.argument_is_alias(member, "AwaitedReactNode").await?
                }
            } else {
                false
            };
            absorbed.push(takes);
        }
        Ok((element && portal && iterable).then_some(absorbed))
    }

    /// Whether the first type argument of an instantiation is React's alias `name`.
    async fn argument_is_alias(&self, info: &TypeInfo, name: &str) -> Result<bool, CallError> {
        let arguments = self.parts(info.id, Parts::Arguments).await?;
        let Some(first) = arguments.first() else { return Ok(false) };
        Ok(self
            .alias_origin(first)
            .await?
            .is_some_and(|origin| origin.in_react() && origin.alias && origin.name == name))
    }

    /// The members of a union, intersection or template literal, and how many there are.
    async fn members_of(&self, info: &TypeInfo) -> Result<(Vec<TypeInfo>, usize), CallError> {
        let members = self.parts(info.id, Parts::Members).await?;
        let total = members.len();
        Ok((members, total))
    }

    async fn intersection(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let origin = self.origin(info).await?;
        // A dependency's `Nested<Style>`-like alias is a name, as its interface would be.
        if let Some(origin) = &origin
            && origin.is_boundary()
        {
            return self.named_type(info, origin, cx).await;
        }
        let gate = self.enter(info, origin.as_ref(), cx, MAX_MEMBERS, || self.members_of(info)).await?;
        let (members, inner) = match gate {
            Gate::Stop(stopped) => return Ok(stopped),
            Gate::Open { parts, inner } => (parts, inner),
        };
        let mut converted: Vec<PropType> = Vec::with_capacity(members.len());
        for member in self.convert_all(&members, &inner).await? {
            if !converted.contains(&member) {
                converted.push(member);
            }
        }
        Ok(if converted.len() == 1 { converted.swap_remove(0) } else { PropType::Intersection(converted) })
    }

    async fn object(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        if info.is_tuple_type {
            return self.tuple(info, cx).await;
        }
        if info.object_flags & flag::OBJECT_REFERENCE != 0
            && let Some(own) = self.symbol_origin(info).await?
            && own.in_lib()
            && !own.alias
            && matches!(own.name.as_str(), "Array" | "ReadonlyArray")
        {
            return self.array(info, own.name == "ReadonlyArray", cx).await;
        }
        let origin = self.origin(info).await?;
        if let Some(terminal) = origin.as_ref().and_then(Origin::react_terminal) {
            return Ok(terminal);
        }
        let boundary = origin.as_ref().filter(|origin| origin.is_boundary());
        // A dependency's interface is a name unless it is callable; its properties are not asked for.
        if let Some(origin) = boundary
            && self.rpc.signatures(info.id, SignatureKind::Call).await?.is_empty()
        {
            return self.named_type(info, origin, cx).await;
        }
        // The width is not counted at the gate: only the members that are not the project's own are, below.
        let gate = self
            .enter(info, origin.as_ref(), cx, [usize::MAX; 2], || async {
                Ok((self.rpc.object_members(info.id).await?, 0))
            })
            .await?;
        let (members, inner) = match gate {
            Gate::Stop(stopped) => return Ok(stopped),
            Gate::Open { parts, inner } => (parts, inner),
        };
        if let Some(origin) = boundary
            && !members.properties.is_empty()
        {
            return self.named_type(info, origin, cx).await;
        }
        let outside = members.properties.iter().filter(|property| !is_own(property)).count();
        if outside > cx.limit(MAX_FIELDS) {
            return self.over_budget(info, origin.as_ref(), members, cx, &inner).await;
        }
        let ObjectMembers { calls, constructs, properties, indexes } = members;

        // Which of a signature's parameters are optional is read from the printed type here, and nowhere else.
        let Some(markers) = self.printed_markers(info.id, &calls, &constructs).await? else {
            return self.unreadable_signatures(info.id).await;
        };
        let mut signatures = Vec::with_capacity(calls.len() + constructs.len());
        for (kind, group, printed) in
            [(Kind::Call, &calls, &markers.calls), (Kind::Construct, &constructs, &markers.constructs)]
        {
            for (index, signature) in group.iter().enumerate() {
                let printed = printed.get(index).map(Vec::as_slice);
                let Some((type_params, params, return_type)) = self.signature_parts(signature, printed, &inner).await?
                else {
                    return self.unreadable_signatures(info.id).await;
                };
                signatures.push(CallSignature { kind, type_params, params, return_type });
            }
        }
        if signatures.len() == 1 && calls.len() == 1 && properties.is_empty() && indexes.is_empty() {
            let CallSignature { type_params, params, return_type, .. } = signatures.swap_remove(0);
            return Ok(PropType::Function { type_params, params, return_type: Box::new(return_type) });
        }
        let fields = self.fields(&properties, &indexes, &inner).await?;
        if signatures.is_empty() {
            return Ok(PropType::Object(fields));
        }
        Ok(PropType::Callable { signatures, fields })
    }

    /// An object with more members from outside the project (inherited from a dependency or the lib) than the budget
    /// lists. What the project declared itself is never cut. The rest stands as what it is named through: the utility
    /// wrapper that built the type (`Omit<HTMLAttributes<HTMLDivElement>, "color">`), or the base types of the interface
    /// or class that inherits it. When neither can be named, the rest is a [`PropType::Truncated`] with the real count.
    async fn over_budget(
        &self,
        info: &TypeInfo,
        origin: Option<&Origin>,
        members: ObjectMembers,
        cx: &Cx,
        inner: &Cx,
    ) -> Result<PropType, CallError> {
        let ObjectMembers { calls, constructs, properties, indexes } = members;
        let cut =
            TruncationLimit::Members { total: to_u32(properties.iter().filter(|property| !is_own(property)).count()) };
        // A type with signatures is a function with properties: its shape is not a set of fields to split.
        if !calls.is_empty() || !constructs.is_empty() {
            return self.truncated(info, origin, cut).await;
        }
        if let Some(origin) = origin
            && origin.alias
            && let Some(wrapper) = self.wrapper(info, origin, cx).await?
        {
            return Ok(wrapper);
        }
        // The declaration of a class or interface has base types; an instantiation of a generic one (`Box<string>`, only a
        // reference) does not state them in terms of its arguments, so it is cut.
        let layers = if origin.is_some_and(|origin| !origin.alias) && states_bases(info) {
            self.base_layers(info, cx).await?
        } else {
            None
        };
        let own: Vec<SymbolInfo> = properties.into_iter().filter(is_own).collect();
        let mut parts = Vec::new();
        if !own.is_empty() || !indexes.is_empty() {
            parts.push(PropType::Object(self.fields(&own, &indexes, inner).await?));
        }
        match layers {
            Some(layers) => parts.extend(layers),
            None => parts.push(self.truncated(info, origin, cut).await?),
        }
        Ok(if parts.len() == 1 { parts.swap_remove(0) } else { PropType::Intersection(parts) })
    }

    /// What a type declared as `interface A extends B, C` stands on: each base type named (see [`Self::layer`]), a base
    /// the project declares seen through to its own bases (its members are the project's own and are listed). `None`
    /// when a base is neither.
    async fn base_layers(&self, info: &TypeInfo, cx: &Cx) -> Result<Option<Vec<PropType>>, CallError> {
        let mut layers: Vec<PropType> = Vec::new();
        let mut pending: Vec<TypeInfo> = self.rpc.base_types(info.id).await?;
        pending.reverse();
        while let Some(base) = pending.pop() {
            if let Some(layer) = self.layer(&base, cx).await? {
                if !layers.contains(&layer) {
                    layers.push(layer);
                }
                continue;
            }
            // A base declared in the project: its members are already among the type's own, and what it inherits is
            // seen through to its own bases. An instantiation of a generic one does not state them, so what it
            // inherits would be missing from the answer: the type is cut instead, with its count.
            let declared_here = match self.origin(&base).await? {
                Some(origin) => !origin.alias && !origin.in_dependency() && !origin.in_lib(),
                None => false,
            };
            if !declared_here || !states_bases(&base) {
                return Ok(None);
            }
            let mut inherited = self.rpc.base_types(base.id).await?;
            inherited.reverse();
            pending.extend(inherited);
        }
        // Members came from somewhere: bases that account for none of them are not an answer.
        Ok((!layers.is_empty()).then_some(layers))
    }

    /// The lib's utility alias (`Omit`, `Pick`, `Partial`, `Required`, `Readonly`) a type was written through, kept as a
    /// name with its arguments when what it applies to has a name of its own (see [`Self::layer`]).
    async fn wrapper(&self, info: &TypeInfo, origin: &Origin, cx: &Cx) -> Result<Option<PropType>, CallError> {
        if !origin.in_lib() || !UTILITIES.contains(&origin.name.as_str()) || info.alias_type_arguments.is_empty() {
            return Ok(None);
        }
        let arguments = self.rpc.alias_type_arguments(info.id).await?;
        let Some((first, keys)) = arguments.split_first() else { return Ok(None) };
        let Some(applied) = Box::pin(self.layer(first, cx)).await? else { return Ok(None) };
        let mut args = vec![applied];
        args.extend(self.convert_all(keys, &cx.argument()).await?);
        Ok(Some(PropType::Named { name: origin.name.clone().into(), args }))
    }

    /// A type that is named instead of expanded because the name says more than its members would: a React DOM
    /// attribute type as [`PropType::HtmlAttributes`] (recognized by the declaring file and the interface's name, and the
    /// element read from its type argument, never from the printed text), a utility wrapper over one (see
    /// [`Self::wrapper`]), or an object a dependency or the lib declares, with its arguments. `None` for any other type.
    async fn layer(&self, info: &TypeInfo, cx: &Cx) -> Result<Option<PropType>, CallError> {
        let Some(origin) = self.origin(info).await? else { return Ok(None) };
        if let Some(family) = self.html_family(info, &origin).await? {
            return Ok(Some(family));
        }
        if origin.alias
            && let Some(wrapper) = self.wrapper(info, &origin, cx).await?
        {
            return Ok(Some(wrapper));
        }
        if origin.is_boundary() {
            return Ok(Some(self.named_type(info, &origin, cx).await?));
        }
        Ok(None)
    }

    /// The attributes of a DOM element, when `info` is one of React's attribute types for it: `HTMLAttributes<T>` and
    /// the element-specific `*HTMLAttributes<T>`, `HTMLProps<T>`, `SVGAttributes<T>`, and `DetailedHTMLProps<A, T>`
    /// through its first argument. The element comes from the interface's name where it names one (`AnchorHTMLAttributes`
    /// is `a`), else from its type argument (`HTMLAttributes<HTMLSpanElement>` is `span`); a type argument that names no
    /// known element leaves the type unrecognized, since `div` would be a guess.
    async fn html_family(&self, info: &TypeInfo, origin: &Origin) -> Result<Option<PropType>, CallError> {
        if !origin.in_react() {
            return Ok(None);
        }
        if origin.alias {
            if origin.name != "DetailedHTMLProps" || info.alias_type_arguments.is_empty() {
                return Ok(None);
            }
            let arguments = self.rpc.alias_type_arguments(info.id).await?;
            let Some(attributes) = arguments.first() else { return Ok(None) };
            return match self.origin(attributes).await? {
                Some(inner) if !inner.alias => Box::pin(self.html_family(attributes, &inner)).await,
                _ => Ok(None),
            };
        }
        let element = match origin.name.as_str() {
            "HTMLAttributes" | "DOMAttributes" | "HTMLProps" | "AllHTMLAttributes" | "SVGAttributes" | "SVGProps" => {
                let arguments = self.parts(info.id, Parts::Arguments).await?;
                let Some(element) = without_this(&arguments).first() else { return Ok(None) };
                let Some(declared) = self.symbol_origin(element).await? else { return Ok(None) };
                html_element_from_type_arg(&declared.name)
            }
            name => html_element_for(name),
        };
        Ok(element.map(|element| PropType::HtmlAttributes { element: element.to_string(), omitted: Vec::new() }))
    }

    async fn array(&self, info: &TypeInfo, readonly: bool, cx: &Cx) -> Result<PropType, CallError> {
        // Only an alias can name an array that contains itself (`type Loop = Loop[]`); `Array` is the lib's.
        let origin = self.alias_origin(info).await?;
        let gate = self
            .enter(info, origin.as_ref(), cx, MAX_MEMBERS, || async {
                let arguments = self.parts(info.id, Parts::Arguments).await?;
                let total = arguments.len();
                Ok((arguments, total))
            })
            .await?;
        let (arguments, inner) = match gate {
            Gate::Stop(stopped) => return Ok(stopped),
            Gate::Open { parts, inner } => (parts, inner),
        };
        let Some(element) = arguments.first() else {
            return Ok(named(self.display(info.id).await?, Vec::new()));
        };
        let element = PropType::Array(Box::new(self.convert(element, &inner).await?));
        // `readonly T[]` is `ReadonlyArray<T>`: the same reference through another declaration.
        Ok(if readonly { PropType::Readonly(Box::new(element)) } else { element })
    }

    async fn tuple(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let origin = self.alias_origin(info).await?;
        let gate = self
            .enter(info, origin.as_ref(), cx, MAX_MEMBERS, || async {
                // The reference carries the arguments; the tuple type it instantiates carries how each element is declared.
                let arguments = self.parts(info.id, Parts::Arguments).await?;
                let target = self.rpc.target_of_type(info.id).await?;
                let total = arguments.len();
                Ok(((arguments, target), total))
            })
            .await?;
        let ((arguments, target), inner) = match gate {
            Gate::Stop(stopped) => return Ok(stopped),
            Gate::Open { parts, inner } => (parts, inner),
        };
        let labels = if target.labeled_element_declarations.is_empty() {
            None
        } else {
            tuple_labels(&self.display(info.id).await?, arguments.len())
        };
        let element_types = self.convert_all(&arguments, &inner).await?;
        let mut elements = Vec::with_capacity(arguments.len());
        for (index, element_type) in element_types.into_iter().enumerate() {
            let element_flags = target.element_flags.get(index).copied().unwrap_or(1);
            let kind = if element_flags & flag::ELEMENT_VARIADIC != 0 {
                TupleElementKind::Variadic
            } else if element_flags & flag::ELEMENT_REST != 0 {
                TupleElementKind::Rest
            } else if element_flags & flag::ELEMENT_OPTIONAL != 0 {
                TupleElementKind::Optional
            } else {
                TupleElementKind::Required
            };
            let label = labels.as_ref().and_then(|labels| labels.get(index)).and_then(|label| TupleLabel::new(label));
            // An optional element's type argument includes the `undefined` of leaving it out.
            let element_type = without_undefined(element_type, kind == TupleElementKind::Optional);
            elements.push(TupleElement { element_type, kind, label });
        }
        let tuple = PropType::Tuple(elements);
        Ok(if target.readonly { PropType::Readonly(Box::new(tuple)) } else { tuple })
    }

    async fn fields(
        &self,
        properties: &[SymbolInfo],
        indexes: &[crate::rpc::IndexInfo],
        cx: &Cx,
    ) -> Result<Vec<ObjectField>, CallError> {
        let references: Vec<_> = properties.iter().map(|property| &property.reference).collect();
        let (types, readonly) = if references.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            (self.rpc.types_of_symbols(&references).await?, self.rpc.readonly_symbols(&references).await?)
        };
        let converted = self.convert_all(&types, cx).await?;
        let mut fields = Vec::with_capacity(properties.len() + indexes.len());
        for ((property, prop_type), readonly) in properties.iter().zip(converted).zip(readonly) {
            let required = property.flags & flag::SYMBOL_OPTIONAL == 0;
            let prop_type = without_undefined(prop_type, !required);
            let mut field = ObjectField::new(property_name(&property.name), prop_type, required, String::new());
            field.readonly = readonly;
            fields.push(field);
        }
        let sides: Vec<TypeInfo> =
            indexes.iter().flat_map(|index| [index.key_type.clone(), index.value_type.clone()]).collect();
        let mut sides = self.convert_all(&sides, cx).await?.into_iter();
        for index in indexes {
            let (Some(key), Some(value)) = (sides.next(), sides.next()) else { break };
            let mut field = ObjectField::new("key", value, true, String::new());
            field.readonly = index.is_readonly;
            field.index_key = Some(Box::new(key));
            fields.push(field);
        }
        Ok(fields)
    }

    /// The parameter markers of the signatures of an object type, as the checker prints the type (see
    /// [`crate::printed`]): the only source of which parameters are optional, since the API's parameter symbols do not
    /// say. `None` when some signature has a parameter and the printed type does not state one entry per signature.
    async fn printed_markers(
        &self,
        id: u64,
        calls: &[SignatureInfo],
        constructs: &[SignatureInfo],
    ) -> Result<Option<SignatureMarkers>, CallError> {
        if !calls.iter().chain(constructs).any(|signature| !signature.parameters.is_empty()) {
            let none = |count: usize| vec![Vec::new(); count];
            return Ok(Some(SignatureMarkers { calls: none(calls.len()), constructs: none(constructs.len()) }));
        }
        let stated = signature_markers(&self.display_expanded(id).await?);
        Ok(stated.filter(|stated| stated.calls.len() == calls.len() && stated.constructs.len() == constructs.len()))
    }

    /// A type whose signatures state parameters that cannot be told optional from required is not given a shape that
    /// would assert one of them: it is opaque, with how the checker prints it.
    async fn unreadable_signatures(&self, id: u64) -> Result<PropType, CallError> {
        Ok(degraded_type(self.display_expanded(id).await?, OpaqueReason::UnsupportedExpression))
    }

    /// One signature's type parameters, parameters and return type; `None` when a parameter that is not the rest
    /// parameter has no marker, since whether it is optional is then unknown and is not guessed.
    async fn signature_parts(
        &self,
        signature: &SignatureInfo,
        printed: Option<&[Marker]>,
        cx: &Cx,
    ) -> Result<Option<(Vec<String>, Vec<FunctionParam>, PropType)>, CallError> {
        let parameters = self.rpc.parameters(signature.id).await?;
        let has_rest = signature.flags & flag::SIGNATURE_HAS_REST_PARAMETER != 0;
        // A printed signature that does not give one entry per parameter states none of them.
        let markers = printed.filter(|markers| markers.len() == parameters.len());
        let known_without_markers = usize::from(has_rest);
        if markers.is_none() && parameters.len() > known_without_markers {
            return Ok(None);
        }
        let references: Vec<_> = parameters.iter().map(|parameter| &parameter.reference).collect();
        let types = if references.is_empty() { Vec::new() } else { self.rpc.types_of_symbols(&references).await? };
        let converted = self.convert_all(&types, cx).await?;
        let mut params = Vec::with_capacity(parameters.len());
        for (index, (parameter, param_type)) in parameters.iter().zip(converted).enumerate() {
            let marker = markers.and_then(|markers| markers.get(index)).copied().unwrap_or_default();
            let rest = marker.rest || (has_rest && index + 1 == parameters.len());
            let optional = marker.optional && !rest;
            let param_type = without_undefined(param_type, optional);
            // A destructured parameter has no name of its own; the checker calls it `__0`.
            let name = (!is_destructured(&parameter.name)).then(|| parameter.name.clone());
            params.push(FunctionParam { name, param_type, optional, rest });
        }
        let return_info = self.rpc.return_type(signature.id).await?;
        let return_type = self.convert(&return_info, cx).await?;
        let mut type_params = Vec::new();
        if !signature.type_parameters.is_empty() {
            for parameter in self.rpc.type_parameters(signature.id).await? {
                match self.rpc.symbol_of_type(parameter.id).await? {
                    Some(symbol) => type_params.push(symbol.name),
                    None => type_params.push(self.display(parameter.id).await?),
                }
            }
        }
        Ok(Some((type_params, params, return_type)))
    }
}

/// The checker keys a property named by a symbol `__@<name>@<symbol id>`; the source writes `[Symbol.iterator]` or
/// `[brand]`. The id is the checker's and does not belong in an answer.
fn property_name(name: &str) -> String {
    const WELL_KNOWN: [&str; 15] = [
        "iterator",
        "asyncIterator",
        "hasInstance",
        "isConcatSpreadable",
        "match",
        "matchAll",
        "replace",
        "search",
        "species",
        "split",
        "toPrimitive",
        "toStringTag",
        "unscopables",
        "dispose",
        "asyncDispose",
    ];
    let Some((symbol, _id)) = name.strip_prefix("__@").and_then(|rest| rest.rsplit_once('@')) else {
        return name.to_string();
    };
    if WELL_KNOWN.contains(&symbol) { format!("[Symbol.{symbol}]") } else { format!("[{symbol}]") }
}

fn is_destructured(name: &str) -> bool {
    name.strip_prefix("__").is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

fn to_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn named(name: String, args: Vec<PropType>) -> PropType {
    PropType::Named { name: name.into(), args }
}

/// The types whose conversion needs nothing but their flags.
fn simple(info: &TypeInfo) -> Option<PropType> {
    let flags = info.flags;
    // `boolean` is the union `false | true` and carries both flags.
    if flags & flag::BOOLEAN != 0 {
        return Some(PropType::Boolean);
    }
    let known = [
        (flag::ANY, PropType::Any),
        (flag::UNKNOWN, PropType::Unknown),
        (flag::UNDEFINED, PropType::Undefined),
        (flag::NULL, PropType::Null),
        (flag::VOID, PropType::Void),
        (flag::NEVER, PropType::Never),
        (flag::STRING, PropType::String),
        (flag::NUMBER, PropType::Number),
        (flag::BIG_INT, PropType::BigInt),
        (flag::ES_SYMBOL, named("symbol".to_string(), Vec::new())),
        (flag::NON_PRIMITIVE, named("object".to_string(), Vec::new())),
    ];
    known.into_iter().find(|(bit, _)| flags & bit != 0).map(|(_, converted)| converted)
}

/// A union of converted members: `false | true` is `boolean`, a union inside a union is flattened, a repeated member
/// is kept once, and a union of one is that member.
fn union_of(converted: Vec<PropType>) -> PropType {
    let mut flat = Vec::with_capacity(converted.len());
    for member in converted {
        match member {
            PropType::Union(inner) => flat.extend(inner),
            other => flat.push(other),
        }
    }
    let both_booleans = flat.contains(&PropType::BoolLiteral(true)) && flat.contains(&PropType::BoolLiteral(false));
    let mut members: Vec<PropType> = Vec::with_capacity(flat.len());
    for member in flat {
        let member = match member {
            PropType::BoolLiteral(_) if both_booleans => PropType::Boolean,
            other => other,
        };
        if !members.contains(&member) {
            members.push(member);
        }
    }
    if members.len() == 1 { members.swap_remove(0) } else { PropType::Union(members) }
}

/// An optional prop's type includes `undefined`; react-docgen-typescript drops it, as does the structural result.
pub(crate) fn without_undefined(prop_type: PropType, optional: bool) -> PropType {
    match prop_type {
        PropType::Union(members) if optional => {
            let mut kept: Vec<PropType> = members.into_iter().filter(|m| *m != PropType::Undefined).collect();
            if kept.len() == 1 { kept.swap_remove(0) } else { PropType::Union(kept) }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_of(flags: u64, object_flags: u64) -> TypeInfo {
        serde_json::from_value(serde_json::json!({ "id": 1, "flags": flags, "objectFlags": object_flags })).unwrap()
    }

    #[test]
    fn the_bases_of_a_declaration_are_its_own_and_those_of_an_instantiation_are_its_generic_targets() {
        assert_eq!(BasesOf::of(&type_of(flag::OBJECT, flag::OBJECT_INTERFACE)), BasesOf::Itself);
        assert_eq!(BasesOf::of(&type_of(flag::OBJECT, flag::OBJECT_CLASS)), BasesOf::Itself);
        assert_eq!(BasesOf::of(&type_of(flag::OBJECT, flag::OBJECT_REFERENCE)), BasesOf::Generic);
    }

    #[test]
    fn a_type_literal_has_no_bases_and_a_type_that_is_not_an_object_is_not_known_to() {
        assert_eq!(BasesOf::of(&type_of(flag::OBJECT, 16)), BasesOf::None);
        for flags in [flag::UNION, flag::TYPE_PARAMETER, flag::CONDITIONAL, flag::INDEXED_ACCESS, flag::STRING] {
            assert_eq!(BasesOf::of(&type_of(flags, 0)), BasesOf::Unknown, "flags {flags}");
        }
    }
}
