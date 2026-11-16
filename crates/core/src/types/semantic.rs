//! What the TypeScript checker backend (on by default, ADR 0026) hands back, and how it merges over the structural result.
//!
//! The backend (`caliper-docgen-semantic`) owns the process, the protocol and the checker walk; this module owns only
//! the result shape and the merge rules of `docs/type-checker-integration.md`, so `crates/core` stays free of I/O.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use camino::Utf8PathBuf;
use rustc_hash::{FxHashMap, FxHasher};

mod order;
mod stored;
mod syntax;

use crate::extractor::written::WrittenMember;
use crate::types::collected::ComponentExport;
use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, Superseded};
use crate::types::options::CollapseContext;
use crate::types::output::{
    ComponentEntry, DefaultValue, ExtractionOutput, InheritedLayer, OpaqueDetail, OpaqueReason, ParsedProp, PropParent,
    PropType, TRUNCATED_KIND,
};
use order::Placed;
use syntax::SourceSyntax;

/// The type of a name the checker could not resolve (an import that does not resolve, a name declared nowhere), which
/// the checker answers with its error type: an `any` that is not a written `any`. It is `opaque` with the reason
/// `unresolvableImport`, so the prop carries `degradedBy`, and `written` is the name as the source writes it, which is
/// also what the reason names (the checker does not say which module specifier was behind it).
pub fn unresolved_type(written: impl Into<String>) -> PropType {
    let written = written.into();
    #[allow(
        clippy::disallowed_methods,
        reason = "the checker's error type is an unresolvable name; the caller reports it"
    )]
    OpaqueDetail::new(written.clone(), OpaqueReason::UnresolvableImport { specifier: written })
}

/// A type the checker holds that PropType has no shape for: a conditional or indexed access type it could not resolve
/// (its parameters are still open), `keyof T`. It is `opaque` with `reason`, so the prop carries `degradedBy`, and
/// `raw` is how the checker prints it.
pub fn degraded_type(raw: impl Into<String>, reason: OpaqueReason) -> PropType {
    #[allow(clippy::disallowed_methods, reason = "the checker names the reason; the caller reports it")]
    OpaqueDetail::new(raw, reason)
}

/// One prop as the checker reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticProp {
    /// The prop's name as written in the props type.
    pub name: String,
    /// The type the checker computed for it, in the same shape the structural resolver produces.
    pub prop_type: PropType,
    /// Whether the checker found the prop non-optional.
    pub required: bool,
    /// The prop's JSDoc description, empty when it has none; the merge then keeps the structural description.
    pub description: String,
    /// The prop's JSDoc tags (`@deprecated`, `@since`, `@default`), each by name with the text the checker renders, empty
    /// when it has none; the merge then keeps the structural tags, as for `description`. `@types/react`'s props are not
    /// asked for theirs.
    pub tags: BTreeMap<String, String>,
    /// The interface that declares the prop, as react-docgen-typescript states it: the parent of its first declaration
    /// when that is an interface, with the file it is in. `None` for a prop declared in a type literal or with no
    /// declaration. It replaces the structural `parent`, `None` included.
    pub parent: Option<PropParent>,
    /// The interface of each declaration that is in one, in the checker's order: two when an intersection or a merged
    /// interface declares the prop twice.
    pub declarations: Vec<PropParent>,
    /// Declared by `@types/react` (`children`, `aria-*`, handlers, `className`): not the component's own prop.
    pub react_declared: bool,
}

impl SemanticProp {
    fn structural_hash(&self) -> u64 {
        let mut hasher = FxHasher::default();
        self.name.hash(&mut hasher);
        self.required.hash(&mut hasher);
        self.react_declared.hash(&mut hasher);
        self.description.hash(&mut hasher);
        self.tags.hash(&mut hasher);
        for parent in self.parent.iter().chain(&self.declarations) {
            parent.name.hash(&mut hasher);
            parent.file_name.hash(&mut hasher);
        }
        self.prop_type.structural_hash(&mut hasher);
        hasher.finish()
    }
}

/// Shares identical props between components, as the structural resolver does: a design system's components repeat
/// the same few hundred props, and Chakra's 600 components would otherwise hold 470,000 copies.
#[derive(Debug, Default)]
pub struct PropInterner {
    by_hash: Mutex<FxHashMap<u64, Vec<Arc<SemanticProp>>>>,
}

impl PropInterner {
    /// The one shared copy of `prop`: an equal prop interned earlier, or `prop` itself the first time. If the internal
    /// lock is poisoned it returns `prop` unshared rather than failing.
    pub fn share(&self, prop: SemanticProp) -> Arc<SemanticProp> {
        self.share_arc(Arc::new(prop))
    }

    /// [`Self::share`] for a prop already behind an `Arc`, such as one another interner shared: the one copy equal to
    /// it, or `prop` itself the first time.
    pub fn share_arc(&self, prop: Arc<SemanticProp>) -> Arc<SemanticProp> {
        let hash = prop.structural_hash();
        let Ok(mut by_hash) = self.by_hash.lock() else { return prop };
        let bucket = by_hash.entry(hash).or_default();
        if let Some(existing) = bucket.iter().find(|existing| ***existing == *prop) {
            return existing.clone();
        }
        bucket.push(prop.clone());
        prop
    }
}

/// One component the checker answered for.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticComponent {
    /// The output key of the [`SemanticTarget`] this answers: the entry to merge into.
    pub key: String,
    /// Declared props in declaration order, with the `@types/react` ones the request asked for.
    pub props: Vec<Arc<SemanticProp>>,
    /// The `@types/react` types the props type is made of (`RefAttributes`, `HTMLAttributes` under an `Omit`), by
    /// name, in the order the props type names them: what the checker leaves to the inheritance layers. `Some` replaces
    /// the structural `composes`, which also lists the types the structural pass could not follow and the checker did,
    /// and `Some` of an empty list says the props type is made of none. `None` says the checker could not tell (a type
    /// it cannot read the bases of), and the structural `composes` stays.
    pub composes: Option<Vec<String>>,
    /// The interfaces the checker proved the props type reaches whole, each as a prop's `parent` names it: every member
    /// of one can reach the component, so nothing was selected away. Found as `composes` finds its types, through only
    /// the edges that keep every member: intersections, bases, and the lib's `Partial`, `Required`, `Readonly` and
    /// `Omit`. One under a `Pick`, a mapped type, a project alias of either, a conditional or a union is not here. A
    /// source the structural answer does not read collapses only when it is here (`layer_checker_props`): not proven,
    /// it is listed.
    pub whole: Vec<PropParent>,
}

/// A component the backend left to the structural result, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticFallback {
    /// The output key of the [`SemanticTarget`] this answers.
    pub key: String,
    /// Why the checker did not answer, for example that the component is not exported by its module. The merge groups
    /// components by this text into one warning.
    pub reason: String,
}

/// The backend's answer for a run. A backend that could not start returns no components and one diagnostic, never an
/// error: the structural result is always a complete answer on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SemanticOutcome {
    /// The components the checker answered for; each replaces the props of the structural entry it names.
    pub components: Vec<SemanticComponent>,
    /// The requested components the checker left to the structural result, each with its reason.
    pub fallbacks: Vec<SemanticFallback>,
    /// What the backend itself reports (it could not start, it timed out); merged into the output's diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// A component the checker may be asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTarget {
    /// The component's output key. Its answer, or its fallback, carries it back, and the merge finds the entry by it:
    /// never by file and display name, which two components can share.
    pub key: String,
    /// The file that declares the component.
    pub file_path: Utf8PathBuf,
    /// The component's display name, for messages; the checker does not look the component up by it.
    pub display_name: String,
    /// How the module exports the component ([`ComponentEntry::exports`]): the checker asks for it by exactly these.
    /// It answers only for exported components and reports the rest as fallbacks.
    pub exports: Vec<ComponentExport>,
    /// Props the structural pass already has a description for. The checker is not asked for those: it is 65% of
    /// the requests on Mantine, and the merge keeps the structural description when the checker's is empty. Empty
    /// under [`ResolverFacts::NoFallback`] ([`ResolverFacts::ready`]), whose merge keeps no structural description.
    pub described: rustc_hash::FxHashSet<String>,
    /// Which props `@types/react` declares the checker reports: `None` for all of them, else only these names. The
    /// session decides from `htmlAttributes` (`react_types::react_props_wanted`); the checker never filters by itself.
    pub react_wanted: Option<rustc_hash::FxHashSet<String>>,
    /// What the merge of the answer decides its collapsed layers from, when the caller merges it that way: the checker
    /// then converts only the props the merge keeps, and reports each other one by name and `parent` with
    /// [`unconverted_type`] ([`Layering::kept`]). `None` converts every prop.
    pub layering: Option<Layering>,
}

/// A checker that answers for components as they are asked about, for a long-lived session.
///
/// `request` never blocks: it queues the targets and returns the receiver that yields the outcome once the checker
/// has it, so a caller can wait a bounded time and take the structural answer when the checker is slower.
pub trait SemanticProvider: Send + Sync {
    /// Queues `targets` for the checker and returns at once with the receiver that will yield their
    /// [`SemanticOutcome`]. Someone is waiting for it: the request is [`Priority::Interactive`].
    fn request(&self, targets: Vec<SemanticTarget>) -> std::sync::mpsc::Receiver<SemanticOutcome>;

    /// Queues `targets` at `priority` and returns at once; `delivery` receives the outcome. A checker that answers one
    /// request at a time answers by [`Priority`] class, and reads `priority` again when it picks the next, since a
    /// caller that joins a request may raise it, and drops one that was cancelled before it started
    /// ([`SharedPriority::cancel`]). The default ignores the priority, so it never drops a request, and waits for
    /// [`Self::request`]'s receiver on a thread of its own.
    fn request_at(&self, targets: Vec<SemanticTarget>, priority: SharedPriority, delivery: Delivery) {
        drop(priority);
        let receiver = self.request(targets);
        let delivery = Arc::new(Mutex::new(Some(delivery)));
        let take =
            |slot: &Mutex<Option<Delivery>>| slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let waiter = delivery.clone();
        let waiting = std::thread::Builder::new().name("caliper-checker-answer".to_string()).spawn(move || {
            let outcome = receiver.recv().ok();
            if let Some(delivery) = take(&waiter) {
                delivery.deliver(outcome);
            }
        });
        if let Err(error) = waiting
            && let Some(delivery) = take(&delivery)
        {
            delivery.deliver(Some(SemanticOutcome::unavailable(format!(
                "could not start a thread to wait for it: {error}"
            ))));
        }
    }

    /// Tells the checker that files changed on disk, before any request that may read them is queued. A checker that
    /// keeps what it read (a warm `tsc` holding a snapshot, ADR 0026) reads them again; one that reads every file
    /// afresh for each request needs nothing, which is the default.
    fn changed(&self, _change: SourceChange) {}
}

/// Who waits for a request to the checker, which decides what a busy checker answers first. The classes are ordered
/// as a checker answers them: [`Priority::Interactive`] first, then [`Priority::Neighbour`], then
/// [`Priority::Background`], so the lesser of two priorities is the one answered first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    /// A component someone is looking at now: a story that was opened. A caller that names no priority asks at this
    /// one ([`SemanticProvider::request`]).
    Interactive,
    /// A component of a story next to the one shown, which the client may open next: a prefetch.
    Neighbour,
    /// Work nobody waits for yet: a warm-up, a fill.
    Background,
}

/// The bit of a [`SharedPriority`] set once its request is cancelled; the bits below it hold the [`Priority`].
const CANCELLED: u8 = 0b100;

/// The bit set once the checker took the request to convert it; it is never cancelled after that.
const STARTED: u8 = 0b1000;

/// The priority of one queued request, shared between the caller and the checker so that a later caller can raise it
/// ([`SharedPriority::raise`]) while the request waits, or cancel it ([`SharedPriority::cancel`]) when nobody will
/// read its answer. Clones share one priority.
#[derive(Debug, Clone)]
pub struct SharedPriority(Arc<std::sync::atomic::AtomicU8>);

impl SharedPriority {
    /// A priority that starts at `priority`, not cancelled.
    pub fn new(priority: Priority) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU8::new(Self::bits(priority))))
    }

    /// The priority now.
    pub fn get(&self) -> Priority {
        Self::class(self.0.load(std::sync::atomic::Ordering::Acquire))
    }

    fn bits(priority: Priority) -> u8 {
        match priority {
            Priority::Interactive => 0,
            Priority::Neighbour => 1,
            Priority::Background => 2,
        }
    }

    fn class(bits: u8) -> Priority {
        match bits & !(CANCELLED | STARTED) {
            0 => Priority::Interactive,
            1 => Priority::Neighbour,
            _ => Priority::Background,
        }
    }

    /// Makes the request interactive: someone is now waiting for it.
    pub fn raise(&self) {
        self.raise_to(Priority::Interactive);
    }

    /// Raises the request to `priority`, or leaves it where it is when it already goes before `priority`. A caller
    /// asked for it again, so it is no longer cancelled.
    pub fn raise_to(&self, priority: Priority) {
        let raise = |bits: u8| Some(bits & STARTED | Self::bits(Self::class(bits).min(priority)));
        self.0.try_update(std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire, raise).ok();
    }

    /// Cancels the request: nobody will read its answer (the client no longer shows its component, or an edit
    /// superseded the revision it was asked at), so a checker that has not started it drops it, and its delivery
    /// receives `None`. A checker already converting it ([`SharedPriority::start`]) finishes, and the request is not
    /// cancelled. An interactive request is waited for and is never cancelled; returns whether this one was.
    pub fn cancel(&self) -> bool {
        let cancel = |bits: u8| match (bits & STARTED != 0, Self::class(bits)) {
            (true, _) | (false, Priority::Interactive) => None,
            (false, Priority::Neighbour | Priority::Background) => Some(bits | CANCELLED),
        };
        self.0.try_update(std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire, cancel).is_ok()
    }

    /// Marks the request as being converted, so it can no longer be cancelled; returns false, marking nothing, when it
    /// was cancelled first and is to be dropped instead.
    pub fn start(&self) -> bool {
        let start = |bits: u8| (bits & CANCELLED == 0).then_some(bits | STARTED);
        self.0.try_update(std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire, start).is_ok()
    }

    /// Whether the request was cancelled and not raised since.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire) & CANCELLED != 0
    }
}

/// Where the outcome of a request made with [`SemanticProvider::request_at`] goes. It is used once: with the outcome,
/// or with `None` when the checker stopped without answering. Dropping it unused counts as `None`, so a caller is
/// told even when the request is lost (a worker that stopped, a thread that could not start).
pub struct Delivery(Option<Box<dyn FnOnce(Option<SemanticOutcome>) + Send>>);

impl Delivery {
    /// A delivery that hands the outcome to `receive`.
    pub fn new(receive: impl FnOnce(Option<SemanticOutcome>) + Send + 'static) -> Self {
        Self(Some(Box::new(receive)))
    }

    /// Hands over the outcome, or `None` for a checker that stopped without answering.
    pub fn deliver(mut self, outcome: Option<SemanticOutcome>) {
        if let Some(receive) = self.0.take() {
            receive(outcome);
        }
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        if let Some(receive) = self.0.take() {
            receive(None);
        }
    }
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delivery").field("delivered", &self.0.is_none()).finish()
    }
}

/// What changed on disk, as a session tells its checker ([`SemanticProvider::changed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChange {
    /// These files were edited, created or deleted.
    Files(Vec<Utf8PathBuf>),
    /// Anything may have changed: the session extracted everything again (a resolution config changed, or it started).
    Everything,
}

impl SemanticOutcome {
    /// An outcome for a backend that did nothing: the structural result stands, for this reason.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let diagnostic = Diagnostic::new(
            DiagnosticSeverity::Warning,
            DiagnosticCode::SemanticBackend,
            format!("TypeScript checker backend unavailable: {}", reason.into()),
        )
        .with_help("The structural result is used instead.");
        Self { diagnostics: vec![diagnostic], ..Self::default() }
    }
}

/// Whether a merge may take a fact the checker does not supply from the structural resolver's answer for the same prop.
///
/// `Off` is the test-only mode of ADR 0026's S7 gates (`extract --no-resolver-facts`, which the construct suite's
/// checker lane runs): the merged facts come from the checker and oxc's syntax alone, as they will once the resolver is
/// deleted (S7e). S7a2 covers `defaultValue` and union member order; S7b extends it to the other facts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResolverFacts {
    /// Where oxc's syntax does not tell, the structural answer does.
    #[default]
    Used,
    /// The structural answer is not read for these facts.
    Off,
    /// `Off`, and no structural fallback at all: the test-only mode of ADR 0026's S7b gate (`extract
    /// --no-structural-fallback`), the merge as it will be once the resolver is gone. A prop whose checker type holds an
    /// unread part keeps the checker's `opaque` type and its `degradedBy`, not the structural type; a description and
    /// tags come from the checker alone; and every prop of a component the checker did not answer for is an `opaque`
    /// `unknown` (`unsupportedExpression`), with one diagnostic naming the components.
    NoFallback,
}

impl ResolverFacts {
    /// Whether the merge may fall back to the structural answer where the checker has none.
    fn falls_back(self) -> bool {
        self != Self::NoFallback
    }

    /// Readies `targets` for a request whose outcome this mode merges. With [`ResolverFacts::NoFallback`] the checker
    /// is asked for every prop's description, [`SemanticTarget::described`] included: the merge keeps no structural
    /// description, so a prop the checker was not asked about would lose its own.
    pub fn ready(self, targets: &mut [SemanticTarget]) {
        if !self.falls_back() {
            for target in targets {
                target.described.clear();
            }
        }
    }
}

/// Replaces `entry`'s props and `composes` with the checker's, keeping a description or tags only the structural pass
/// has, for the whole-project merge. Facts are read as [`merge_semantic_props`] reads them, from `syntax` and as
/// `resolver` allows. The checker's props are laid out in the layers `cx` decides ([`layer_checker_props`]).
#[allow(clippy::too_many_arguments)]
fn apply_props(
    entry: &mut ComponentEntry,
    semantic: &SemanticComponent,
    shared: &mut FxHashMap<u64, Vec<Arc<ParsedProp>>>,
    memo: &mut MergeMemo,
    syntax: &SourceSyntax,
    resolver: ResolverFacts,
    cx: CollapseContext,
) -> MergedProps {
    // The whole-project merge replaces the props. Of `@types/react`'s it takes those the structural answer lists, as
    // the session's merge does (`checker_prop_used`): relevance decided them there (ADR 0028), and the rest are names
    // in the inheritance layers.
    let own: Vec<&Arc<SemanticProp>> = semantic
        .props
        .iter()
        .filter(|prop| crate::react_types::react_prop_listed(&prop.name, prop.react_declared, &entry.props))
        .collect();
    let (own, notable) = layer_checker_props(entry, own, &semantic.whole, cx);
    let merge = Merge { composes: semantic.composes.as_deref(), unanswered: Unanswered::Drop, syntax, resolver };
    let merged = replace_props(entry, own.into_iter(), &merge, shared, memo);
    move_notable(entry, &notable);
    listed_props_leave_layers(entry, true);
    crate::relevance::narrow(entry);
    merged
}

/// Each prop shown once: a collapsed layer never holds a name `entry` lists (the listed prop wins), and a name only
/// the first collapsed layer holding it keeps. The collapse decision stands, only the names go: a layer left with none
/// is dropped. A notable member copied to `notableInherited` stays its layer's (the layer contract). Run wherever a
/// component's layers are final: the structural answer and each merge of the checker's.
///
/// In the structural answer (`merged` false) a listed prop whose `parent` is `@types/react`'s takes no name from a
/// layer: the merge drops a prop React alone declares, and the name must then still be the layer's.
pub(crate) fn listed_props_leave_layers(entry: &mut ComponentEntry, merged: bool) {
    let mut held: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    let listed = |name: &String| {
        entry.props.get(name).is_some_and(|prop| {
            merged || !prop.parent.as_ref().is_some_and(|parent| parent.file_name.contains("/@types/react/"))
        })
    };
    entry.inheritance.retain_mut(|layer| {
        if layer.members.is_empty() {
            return true;
        }
        let before = layer.members.len();
        layer.members.retain(|name| !listed(name) && held.insert(name.clone()));
        let removed = u32::try_from(before - layer.members.len()).unwrap_or(u32::MAX);
        layer.total_props = layer.total_props.saturating_sub(removed);
        !layer.members.is_empty()
    });
}

/// The checker's `props` for `entry` without those an inherited source collapsed into a layer holds, and the notable
/// ones to move to `notableInherited` once merged ([`move_notable`]).
///
/// The collapse decision has one owner, [`CollapseInheritedProps::collapses`](crate::types::options::CollapseInheritedProps::collapses),
/// asked with the members of a source that reach the component. The structural resolver decides the sources it reads,
/// which are its layers here; a source the structural answer did not read (no structural prop and no layer has it as
/// its source) is decided by `cx` ([`CollapseContext::collapses`]). [`Layering::kept`] makes the same decision from a
/// [`LayerFacts`] of `entry`, before the answer's types are converted.
///
/// 1. A prop a collapsed layer holds, by (source, name): its `parent` is the layer's source and the layer names it
///    (`InheritedLayer::members`), is left to it, unless the structural answer lists the prop itself (the component
///    declares it again), or it is one of the layer's notable members: that one is merged as any prop is (the
///    component's default, the checker's type and JSDoc) and then moved to `notableInherited`, replacing the structural
///    entry, as for a layer made here. A prop of the same name another source declares is not the layer's: it is kept.
/// 2. A source only the checker read becomes a collapsed layer only when the checker proved the props type reaches it
///    whole (`whole`, [`SemanticComponent::whole`]) and the owner collapses it, asked with the members that reach the
///    component: the props of the answer whose `parent` is the source, each name once (the structural resolver counts
///    the same; no declared count is asked of the checker). Never hide on silence: a source reached through a `Pick`, a
///    mapped type, a project alias of either, a conditional or a union, or one the checker said nothing of, is listed,
///    whatever its size. `members` are those that reach and `totalProps` their number, what the layer holds. Its notable
///    props move to `notableInherited`, as the structural resolver keeps a collapsed source's notable members.
pub(crate) fn layer_checker_props<'a>(
    entry: &mut ComponentEntry,
    props: Vec<&'a Arc<SemanticProp>>,
    whole: &[PropParent],
    cx: CollapseContext,
) -> (Vec<&'a Arc<SemanticProp>>, Vec<(String, PropParent)>) {
    // The notable members of the layers the entry has (rule 1), then those of the layers made here (rule 2).
    let of_entry = layer_notable(entry);
    let (layers, made) = checker_sources(
        &*entry,
        props.iter().map(|prop| (prop.name.as_str(), prop.parent.as_ref(), prop.react_declared)),
        whole,
        cx,
    );
    entry.inheritance.extend(layers);
    let layered: Held<'_> = entry
        .inheritance
        .iter()
        .flat_map(|layer| {
            layer.members.iter().map(|name| (name.as_str(), layer.type_name.as_str(), layer.file_name.as_str()))
        })
        .collect();
    let unconverted = unconverted_type();
    let kept: Vec<&'a Arc<SemanticProp>> = props
        .into_iter()
        .filter(|prop| {
            layered.is_empty()
                || keeps(
                    &prop.name,
                    prop.parent.as_ref(),
                    (&of_entry, &made),
                    entry.props.contains_key(&prop.name),
                    &layered,
                )
        })
        .collect();
    // A kept notable member of a layer the entry has is merged over its structural entry (of that same source), which
    // keeps a description or tags only the structural pass has; `move_notable` moves the merged prop back.
    for (name, source) in &of_entry {
        if kept.iter().any(|prop| prop.name == *name && prop.parent.as_ref() == Some(source))
            && !entry.props.contains_key(name)
            && entry.notable_inherited.get(name).is_some_and(|structural| structural.parent.as_ref() == Some(source))
            && let Some(structural) = entry.notable_inherited.remove(name)
        {
            entry.props.insert(name.clone(), structural);
        }
    }
    let mut notable = of_entry;
    notable.extend(made);
    // A prop the checker left unconverted is one this decision leaves to a layer ([`Layering::kept`]). Keeping one means
    // the target's layering was made from another structural answer or context than this merge's: it is never merged
    // as a type, so the structural prop stays.
    debug_assert!(
        kept.iter().all(|prop| prop.prop_type != unconverted),
        "the merge keeps a prop the checker left to a layer: the target's layering is not this merge's"
    );
    let kept = kept.into_iter().filter(|prop| prop.prop_type != unconverted).collect();
    (kept, notable)
}

/// What the structural answer says of a component that decides which of the checker's props a collapsed layer takes
/// (the merge's `layer_checker_props`): the props it lists, the source each names (`parent`), and its inheritance layers, their
/// sources and members. A [`SemanticTarget`] carries it in a [`Layering`], so the checker can leave unconverted what the
/// merge leaves to a layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct LayerFacts {
    /// The names of the props the structural answer lists.
    listed: std::collections::BTreeSet<String>,
    /// The `parent` of each prop it lists, each once.
    parents: Vec<PropParent>,
    /// Each inheritance layer's source, by type name and file.
    sources: Vec<(String, String)>,
    /// The members of every inheritance layer, each by (name, the layer's type name, its file) ([`Held`]).
    members: std::collections::BTreeSet<(String, String, String)>,
    /// The notable members of its collapsed layers it does not list, with their source ([`layer_notable`]).
    notable: Vec<(String, PropParent)>,
}

/// The notable members (`className`, `style`, …) of `entry`'s collapsed layers that it does not list itself, each with
/// its layer's source: the merge merges the checker's answer for each and moves it back to `notableInherited`.
fn layer_notable(entry: &ComponentEntry) -> Vec<(String, PropParent)> {
    let notable_names = crate::react_types::notable_html_attrs("");
    entry
        .inheritance
        .iter()
        .filter(|layer| layer.html_element.is_none())
        .flat_map(|layer| {
            layer
                .members
                .iter()
                .filter(|name| notable_names.contains(&name.as_str()) && !entry.props.contains_key(*name))
                .map(|name| {
                    (name.clone(), PropParent { name: layer.type_name.clone(), file_name: layer.file_name.clone() })
                })
        })
        .collect()
}

impl LayerFacts {
    /// What `entry`, a structural answer, says.
    pub fn of(entry: &ComponentEntry) -> Self {
        let mut parents: Vec<PropParent> = Vec::new();
        for parent in entry.props.values().filter_map(|prop| prop.parent.as_ref()) {
            if !parents.contains(parent) {
                parents.push(parent.clone());
            }
        }
        Self {
            listed: entry.props.keys().cloned().collect(),
            parents,
            sources: entry.inheritance.iter().map(|layer| (layer.type_name.clone(), layer.file_name.clone())).collect(),
            members: entry
                .inheritance
                .iter()
                .flat_map(|layer| {
                    layer.members.iter().map(|name| (name.clone(), layer.type_name.clone(), layer.file_name.clone()))
                })
                .collect(),
            notable: layer_notable(entry),
        }
    }
}

/// What the merge of the checker's answer into a component's structural answer is decided from besides the answer:
/// that structural answer's [`LayerFacts`] and the run's [`CollapseContext`]. A checker given it converts only the props
/// the merge keeps ([`Layering::kept`]), so the merge must be asked with the same structural answer and the same context:
/// otherwise it may keep a prop the checker did not convert ([`unconverted_type`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Layering {
    /// The structural answer's facts.
    pub facts: LayerFacts,
    /// The run's collapse choices.
    pub cx: CollapseContext,
}

impl Layering {
    /// The layering of `entry`, a structural answer, under `cx`.
    pub fn of(entry: &ComponentEntry, cx: CollapseContext) -> Self {
        Self { facts: LayerFacts::of(entry), cx }
    }

    /// Whether the merge can leave any prop to a layer: the structural answer has a layer, or the run collapses.
    pub fn may_layer(&self) -> bool {
        !self.facts.members.is_empty() || self.cx.mode != crate::types::options::CollapseInheritedProps::None
    }

    /// Whether the structural answer lists a prop named `name`: the merge keeps such a prop whatever its source
    /// ([`Layering::kept`]), so only a prop it does not list can be left to a layer.
    pub fn lists(&self, name: &str) -> bool {
        self.facts.listed.contains(name)
    }

    /// For each prop of a checker's answer, by name, `parent` and whether `@types/react` declares it, in the answer's
    /// order: whether the merge (`layer_checker_props`) keeps it. The same decision from the same inputs, read from the
    /// [`LayerFacts`] instead of the entry. An `@types/react` prop is always reported kept: whether the merge sees it at
    /// all is decided by its own filter first. `whole` is the answer's [`SemanticComponent::whole`]: only those sources
    /// can be left to a layer the answer adds, as in the merge.
    pub fn kept(&self, props: &[(&str, Option<&PropParent>, bool)], whole: &[PropParent]) -> Vec<bool> {
        let (layers, made) = checker_sources(&self.facts, props.iter().copied(), whole, self.cx);
        let layered: Held<'_> = self
            .facts
            .members
            .iter()
            .map(|(name, type_name, file_name)| (name.as_str(), type_name.as_str(), file_name.as_str()))
            .chain(layers.iter().flat_map(|layer| {
                layer.members.iter().map(|name| (name.as_str(), layer.type_name.as_str(), layer.file_name.as_str()))
            }))
            .collect();
        props
            .iter()
            .map(|(name, parent, react_declared)| {
                *react_declared
                    || keeps(name, *parent, (&self.facts.notable, &made), self.facts.listed.contains(*name), &layered)
            })
            .collect()
    }
}

/// The type a checker's prop has when the checker did not convert it, because the merge leaves it to a collapsed layer
/// ([`Layering::kept`]): its name and `parent` are all a layer reads. It is never merged as a prop's type.
pub fn unconverted_type() -> PropType {
    degraded_type("unconverted: left to a collapsed layer", OpaqueReason::UnsupportedExpression)
}

/// What [`checker_sources`] reads of a structural answer.
trait Structural {
    /// Whether it read `source`: a prop it lists names it as `parent`, or one of its layers is it.
    fn reads(&self, source: &PropParent) -> bool;
}

impl Structural for ComponentEntry {
    fn reads(&self, source: &PropParent) -> bool {
        self.props.values().any(|prop| prop.parent.as_ref() == Some(source))
            || self
                .inheritance
                .iter()
                .any(|layer| layer.type_name == source.name && layer.file_name == source.file_name)
    }
}

impl Structural for LayerFacts {
    fn reads(&self, source: &PropParent) -> bool {
        self.parents.contains(source)
            || self
                .sources
                .iter()
                .any(|(type_name, file_name)| *type_name == source.name && *file_name == source.file_name)
    }
}

/// The layers a checker's answer adds to `structural`'s, and the notable props they hold, to move to
/// `notableInherited` ([`layer_checker_props`]'s second rule). `props` are the answer's, by name, `parent` and whether
/// `@types/react` declares it, in its order. Only a source in `whole` can become a layer (the second rule).
fn checker_sources<'p>(
    structural: &impl Structural,
    props: impl Iterator<Item = (&'p str, Option<&'p PropParent>, bool)>,
    whole: &[PropParent],
    cx: CollapseContext,
) -> (Vec<InheritedLayer>, Vec<(String, PropParent)>) {
    // Each source the answer names (a prop's `parent` is an interface), in the order it first names it, with the
    // members of it that reach the component, each once.
    let mut sources: Vec<(&'p PropParent, Vec<String>)> = Vec::new();
    for (name, parent, react_declared) in props {
        let Some(parent) = parent.filter(|_| !react_declared) else { continue };
        match sources.iter_mut().find(|(source, _)| *source == parent) {
            Some((_, names)) if !names.iter().any(|member| member == name) => names.push(name.to_string()),
            Some(_) => {}
            None => sources.push((parent, vec![name.to_string()])),
        }
    }
    let notable_names = crate::react_types::notable_html_attrs("");
    let mut notable = Vec::new();
    let mut layers = Vec::new();
    for (source, members) in sources {
        // The one rule: a source the structural answer did not read collapses only when the checker proved the props
        // type reaches it whole. Not proven (a selection on the way, or nothing said), it is listed.
        if structural.reads(source)
            || !whole.contains(source)
            || !cx.collapses(&source.file_name, &source.name, members.len())
        {
            continue;
        }
        notable.extend(
            members
                .iter()
                .filter(|name| notable_names.contains(&name.as_str()))
                .map(|name| (name.clone(), source.clone())),
        );
        layers.push(InheritedLayer {
            type_name: source.name.clone(),
            file_name: source.file_name.clone(),
            omitted: Vec::new(),
            html_element: None,
            total_props: u32::try_from(members.len()).unwrap_or(u32::MAX),
            members,
            used: Vec::new(),
        });
    }
    (layers, notable)
}

/// A notable prop's name and the collapsed source it belongs to.
type Notable = (String, PropParent);

/// The members of collapsed layers, each by (name, the layer's type name, its file): a layer holds a prop only when the
/// prop's `parent` is the layer's source, never by its name alone.
type Held<'a> = rustc_hash::FxHashSet<(&'a str, &'a str, &'a str)>;

/// Whether the merge keeps a checker prop named `name` declared in `parent`: a notable member of a layer the entry has
/// or the answer adds whose source is `parent` (`notable.0`, [`layer_notable`]; `notable.1`), a prop the structural
/// answer lists itself (`listed`), or one no layer holds as a member of its own source (`layered`). A prop with no
/// `parent` is no source's member, so no layer holds it.
fn keeps(
    name: &str,
    parent: Option<&PropParent>,
    notable: (&[Notable], &[Notable]),
    listed: bool,
    layered: &Held<'_>,
) -> bool {
    let (of_entry, made) = notable;
    let of_source = |(notable, source): &Notable| notable == name && parent == Some(source);
    made.iter().any(of_source)
        || of_entry.iter().any(of_source)
        || listed
        || !parent.is_some_and(|parent| layered.contains(&(name, parent.name.as_str(), parent.file_name.as_str())))
}

/// Moves each merged prop `notable` names from `entry`'s props to its `notableInherited`, when it is the collapsed
/// source's member (its `parent` is that source); a prop of that name the component declares itself stays a prop.
pub(crate) fn move_notable(entry: &mut ComponentEntry, notable: &[(String, PropParent)]) {
    for (name, source) in notable {
        if entry.props.get(name).is_some_and(|prop| prop.parent.as_ref() == Some(source))
            && let Some(prop) = entry.props.remove(name)
        {
            entry.notable_inherited.insert(name.clone(), prop);
        }
    }
}

/// What one merged prop was made from, by identity: the checker's prop, the structural prop of the same name, the
/// union a prop without a `parent` writes in the entry's file (a prop with one finds it by itself) and the code
/// default, each by address (`0` for none). The first two are shared copies
/// (the backend's and the resolver's interners), so the components of a design system that share a prop share its key.
type MergeKey = (usize, usize, (usize, usize), usize);

/// The props one merge has made, by what they were made from ([`MergeKey`]). A merged prop is a function of those
/// inputs and the merge's [`ResolverFacts`] alone, so a prop made from the same ones is the one made before: it skips
/// the copy, the reordering and the interning of a type that is often hundreds of members wide. Each entry holds the
/// two props its key points at, so neither is freed and its address taken by another prop while the merge runs.
#[derive(Default)]
struct MergeMemo {
    made: FxHashMap<MergeKey, MemoEntry>,
}

struct MemoEntry {
    _semantic: Arc<SemanticProp>,
    _structural: Option<Arc<ParsedProp>>,
    merged: Arc<ParsedProp>,
    unread: bool,
}

/// How a merge went for one component.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergedProps {
    /// Structural props the checker did not return, kept as the structural pass resolved them.
    pub kept: usize,
    /// Props the checker returned with a type it could not read (an `opaque` part: an import that does not resolve, a
    /// conditional type it left open), where the structural pass read the type in full. They keep the structural type,
    /// and their `degradedBy` also lists what the checker could not read.
    pub unread: usize,
    /// A source file the merge could not read for the defaults and union order of the answer.
    pub diagnostics: Vec<Diagnostic>,
}

/// Merges the checker's `props` into `entry` by name: the checker's type, optionality, description, tags, `parent` and
/// `declarations` win where both have a prop, and every structural prop the checker did not return stays as the
/// structural pass resolved it. `composes` replaces the entry's when the checker decided it, and an undecided one leaves
/// the structural `composes` ([`SemanticComponent::composes`]). A prop whose checker
/// type holds an `opaque` part, where the structural type holds none, keeps the structural type and adds the checker's
/// reasons to its `degradedBy`: the checker's answer there is a placeholder, and the structural one is not confirmed,
/// so it is marked. A type the checker only cut at a limit (`truncated`) is still its answer.
///
/// Two facts the checker does not have come from oxc's syntax, read from the files the answer names: a prop's default
/// (the component's code: a destructured parameter's initializer, `defaultProps`; else the checker's `@default` tag)
/// and the order of a union's members (the order its declaration writes them in). The structural answer gives them
/// where the syntax does not tell ([`ResolverFacts::Used`]).
pub fn merge_semantic_props<'a>(
    entry: &mut ComponentEntry,
    props: impl Iterator<Item = &'a Arc<SemanticProp>>,
    composes: Option<&[String]>,
    shared: &mut FxHashMap<u64, Vec<Arc<ParsedProp>>>,
) -> MergedProps {
    let props: Vec<Arc<SemanticProp>> = props.cloned().collect();
    let syntax = SourceSyntax::read(std::iter::once((&*entry, props.as_slice())));
    let merge = Merge { composes, unanswered: Unanswered::Keep, syntax: &syntax, resolver: ResolverFacts::Used };
    let mut merged = replace_props(entry, props.iter(), &merge, shared, &mut MergeMemo::default());
    merged.diagnostics = syntax.diagnostics;
    merged
}

/// The kinds of what the checker could not read in `checker`, when the structural type `structural` read everything:
/// then the structural type is kept and marked with them. Empty when the checker's type is its answer.
fn unread_by_checker(checker: &PropType, structural: &PropType) -> Vec<&'static str> {
    let unread: Vec<&'static str> = checker.degraded_by().into_iter().filter(|kind| *kind != TRUNCATED_KIND).collect();
    if unread.is_empty() || structural.degraded_by().iter().any(|kind| *kind != TRUNCATED_KIND) {
        return Vec::new();
    }
    unread
}

/// The one diagnostic for the props of `affects` that kept their structural type because the checker could not read it
/// ([`MergedProps::unread`]); each such prop says why in its `degradedBy`.
pub(crate) fn unread_diagnostic(props: usize, affects: Vec<String>) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        DiagnosticSeverity::Info,
        DiagnosticCode::SemanticBackend,
        format!(
            "{props} prop(s) of {} component(s) keep their structural type: the TypeScript checker could not read it, \
             and their degradedBy says what it could not read",
            affects.len()
        ),
    );
    diagnostic.affects = affects;
    diagnostic
}

/// [`ResolverFacts::NoFallback`]'s diagnostic for the props of `affects` whose checker type holds a part the checker
/// could not read, where the structural type read it all: they keep the checker's `opaque` type.
fn unread_without_fallback(props: usize, affects: Vec<String>) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        DiagnosticSeverity::Warning,
        DiagnosticCode::SemanticBackend,
        format!(
            "{props} prop(s) of {} component(s) are opaque: the TypeScript checker could not read their type, the \
             structural fallback is off, and their degradedBy says what it could not read",
            affects.len()
        ),
    );
    diagnostic.affects = affects;
    diagnostic
}

/// [`ResolverFacts::NoFallback`]: every component of `output` the checker did not answer for (`answered`, sorted) keeps
/// its props by name and whether they are required, each typed an `opaque` `unknown` (`unsupportedExpression`) with no
/// default, description or tags, and one warning names the components.
fn degrade_unanswered(output: &mut ExtractionOutput, answered: &[String]) {
    let mut affects = Vec::new();
    for (key, entry) in &mut output.components {
        if answered.binary_search(key).is_ok() {
            continue;
        }
        affects.push(key.clone());
        for prop in entry.props.values_mut() {
            let unknown = degraded_type("unknown", OpaqueReason::UnsupportedExpression);
            *prop = Arc::new(ParsedProp::new(
                prop.name.clone(),
                unknown,
                prop.required,
                None,
                String::new(),
                BTreeMap::new(),
                None,
                Vec::new(),
            ));
        }
    }
    if affects.is_empty() {
        return;
    }
    affects.sort();
    let mut diagnostic = Diagnostic::new(
        DiagnosticSeverity::Warning,
        DiagnosticCode::SemanticBackend,
        format!(
            "{} component(s) have no TypeScript checker answer and the structural fallback is off: their props are \
             opaque",
            affects.len()
        ),
    );
    diagnostic.affects = affects;
    output.diagnostics.push(diagnostic);
}

/// A component the checker answered for, as [`drop_superseded`] reads it.
pub(crate) struct Superseding<'a> {
    key: &'a str,
    /// A structural prop the checker did not return still holds a part the structural pass could not read, which a
    /// structural diagnostic naming the component may be about.
    structural_left: bool,
    /// A prop of the merged answer holds a name the checker could not resolve either.
    unresolved: bool,
}

impl<'a> Superseding<'a> {
    /// `key`'s component as `merged` holds it after the checker's props were merged in. `structural_left` says whether a
    /// structural prop the checker did not return still holds a part the structural pass could not read.
    pub(crate) fn new(key: &'a str, merged: &ComponentEntry, structural_left: bool) -> Self {
        let unresolved_kind = OpaqueReason::UnresolvableImport { specifier: String::new() }.kind();
        let unresolved = merged.props.values().any(|prop| prop.degraded_by.iter().any(|kind| kind == unresolved_kind));
        Self { key, structural_left, unresolved }
    }

    fn supersedes(&self, superseded: Superseded) -> bool {
        match superseded {
            Superseded::No => false,
            Superseded::Yes => !self.structural_left,
            Superseded::UnlessUnresolvedByChecker => !self.structural_left && !self.unresolved,
        }
    }
}

/// Takes the components the checker answered for out of the `affects` of each structural diagnostic its answer
/// supersedes ([`DiagnosticCode::superseded_by_checker`]), and drops a diagnostic left affecting no component. A
/// diagnostic that affected none (a file's, the run's) is about no component's props and stays. Both merges call it
/// before the checker's own diagnostics join `diagnostics`, so it reads only the structural pass's.
pub(crate) fn drop_superseded(diagnostics: &mut Vec<Diagnostic>, answered: &[Superseding<'_>]) {
    if answered.is_empty() {
        return;
    }
    let by_key: FxHashMap<&str, &Superseding<'_>> = answered.iter().map(|answer| (answer.key, answer)).collect();
    diagnostics.retain_mut(|diagnostic| {
        if diagnostic.affects.is_empty() {
            return true;
        }
        let superseded = diagnostic.code.superseded_by_checker();
        diagnostic.affects.retain(|key| by_key.get(key.as_str()).is_none_or(|answer| !answer.supersedes(superseded)));
        !diagnostic.affects.is_empty()
    });
}

/// What a merge does with a structural prop the checker did not return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unanswered {
    /// The checker decides which props exist, as in the whole-project merge: the structural prop is dropped.
    Drop,
    /// The checker's props are merged by name, as in a session's merge: the structural prop stays as resolved.
    Keep,
}

/// How one merge reads what the checker does not answer.
struct Merge<'a> {
    composes: Option<&'a [String]>,
    unanswered: Unanswered,
    syntax: &'a SourceSyntax,
    resolver: ResolverFacts,
}

/// A prop's default: the component's code default for it, else the checker's `@default` (or `@defaultValue`) tag, else
/// with [`ResolverFacts::Used`] the structural default. The code wins over the tag, as in react-docgen-typescript.
fn default_of(
    code: Option<&DefaultValue>,
    checker_tags: &BTreeMap<String, String>,
    structural: Option<&ParsedProp>,
    resolver: ResolverFacts,
) -> Option<DefaultValue> {
    if let Some(code) = code {
        return Some(code.clone());
    }
    let tag = checker_tags.get("default").or_else(|| checker_tags.get("defaultValue"));
    if let Some(tag) = tag {
        return Some(DefaultValue { value: tag.trim().to_owned(), computed: false });
    }
    match resolver {
        ResolverFacts::Used => structural.and_then(|structural| structural.default_value.clone()),
        ResolverFacts::Off | ResolverFacts::NoFallback => None,
    }
}

/// The checker's type with its union members in declared order: the order the declaration writes them in where it
/// states every member, else the structural union's order where that placed any member (with
/// [`ResolverFacts::Used`]), else the written order of the members the declaration does state. Neither leaves the
/// checker's order. A structural order that placed members is kept even where it is the checker's own: the partial
/// written order would put the members it does not state last, where they may not have been written.
fn declared_order(
    checker: PropType,
    written: Option<&[WrittenMember]>,
    structural: Option<&ParsedProp>,
    resolver: ResolverFacts,
) -> PropType {
    let structural = match resolver {
        ResolverFacts::Used => structural,
        ResolverFacts::Off | ResolverFacts::NoFallback => None,
    };
    // The structural order also reaches the unions nested in members, which the written one does not.
    let (ordered, placed) = match structural {
        Some(structural) => {
            let placed = order::places_any(&checker, &structural.prop_type);
            (order::in_declared_order(checker, &structural.prop_type), placed)
        }
        None => (checker, false),
    };
    match written.and_then(|written| order::in_written_order(&ordered, written)) {
        Some((in_written, Placed::All)) => in_written,
        Some((in_written, Placed::Some)) if !placed => in_written,
        Some(_) | None => ordered,
    }
}

fn replace_props<'a>(
    entry: &mut ComponentEntry,
    props_in: impl Iterator<Item = &'a Arc<SemanticProp>>,
    merge: &Merge<'_>,
    shared: &mut FxHashMap<u64, Vec<Arc<ParsedProp>>>,
    memo: &mut MergeMemo,
) -> MergedProps {
    let Merge { composes, unanswered, syntax, resolver } = *merge;
    let code_defaults = syntax.code_defaults(entry);
    // Collected, then built into the map in one pass: a design system's components place thousands of props each, and
    // one insert per prop searched the map every time. A name given twice keeps the later prop, as an insert did.
    let mut placed: Vec<(String, Arc<ParsedProp>)> = Vec::with_capacity(props_in.size_hint().0);
    let mut unread = 0;
    for prop in props_in {
        let structural = entry.props.get(&prop.name);
        let code_default = code_defaults.and_then(|defaults| defaults.get(&prop.name));
        // The union a prop with a `parent` writes is found by the prop alone, so it needs no place in the key; one with
        // none is looked for in the entry's own file.
        let written = prop.parent.is_none().then(|| syntax.written(entry, prop));
        let key: MergeKey = (
            Arc::as_ptr(prop) as usize,
            structural.map_or(0, |s| Arc::as_ptr(s) as usize),
            written.flatten().map_or((0, 0), |w| (w.as_ptr() as usize, w.len())),
            code_default.map_or(0, |d| std::ptr::from_ref(d) as usize),
        );
        if let Some(made) = memo.made.get(&key) {
            unread += usize::from(made.unread);
            placed.push((prop.name.clone(), made.merged.clone()));
            continue;
        }
        let written = written.unwrap_or_else(|| syntax.written(entry, prop));
        // With no fallback, the structural prop gives nothing: the checker's description, tags and type stand.
        let fallback = structural.filter(|_| resolver.falls_back());
        let description = if prop.description.is_empty() {
            fallback.map(|s| s.description.clone()).unwrap_or_default()
        } else {
            prop.description.clone()
        };
        let tags =
            if prop.tags.is_empty() { fallback.map(|s| s.tags.clone()).unwrap_or_default() } else { prop.tags.clone() };
        let unread_kinds = structural.map(|s| unread_by_checker(&prop.prop_type, &s.prop_type)).unwrap_or_default();
        let was_unread = !unread_kinds.is_empty();
        unread += usize::from(was_unread);
        // Without the fallback the checker's type keeps its own `opaque` part, which already carries the kinds.
        let unread_kinds = if resolver.falls_back() { unread_kinds } else { Vec::new() };
        let prop_type = match structural {
            Some(structural) if !unread_kinds.is_empty() => structural.prop_type.clone(),
            Some(_) | None => declared_order(prop.prop_type.clone(), written, structural.map(|s| &**s), resolver),
        };
        let merged = ParsedProp::new(
            prop.name.clone(),
            prop_type,
            prop.required,
            default_of(code_default, &prop.tags, structural.map(|s| &**s), resolver),
            description,
            tags,
            prop.parent.clone(),
            prop.declarations.clone(),
        )
        .degraded_also_by(&unread_kinds);
        let mut hasher = FxHasher::default();
        merged.structural_hash(&mut hasher);
        let bucket = shared.entry(hasher.finish()).or_default();
        let merged = match bucket.iter().find(|existing| ***existing == merged) {
            Some(existing) => existing.clone(),
            None => {
                let merged = Arc::new(merged);
                bucket.push(merged.clone());
                merged
            }
        };
        memo.made.insert(
            key,
            MemoEntry {
                _semantic: prop.clone(),
                _structural: structural.cloned(),
                merged: merged.clone(),
                unread: was_unread,
            },
        );
        placed.push((prop.name.clone(), merged));
    }
    let mut props: BTreeMap<String, Arc<ParsedProp>> = placed.into_iter().collect();
    let mut kept = 0;
    if unanswered == Unanswered::Keep {
        for (name, structural) in &entry.props {
            if !props.contains_key(name) {
                props.insert(name.clone(), structural.clone());
                kept += 1;
            }
        }
    }
    entry.props = props;
    if let Some(composes) = composes {
        entry.composes = composes.to_vec();
    }
    MergedProps { kept, unread, diagnostics: Vec::new() }
}

/// Merge the checker's props over `output`, component by component.
///
/// Per `docs/type-checker-integration.md`: the checker decides which props exist, their types, whether they are
/// required, `parent`, `declarations` and the component's `composes`; a description and tags come from the checker,
/// else the structural pass; a default value and a union's member order come from oxc's syntax, else the structural
/// pass ([`merge_semantic_props`]). Inheritance layers, curated inherited props and `discriminantProp` stay structural.
/// Components the checker did not answer for keep their structural result untouched. A structural diagnostic stops
/// naming a component the checker answered for when the answer supersedes it
/// ([`DiagnosticCode::superseded_by_checker`]), and goes once it names none.
///
/// Records provenance as `Info` diagnostics whose `affects` lists the output keys, so the canonical JSON gains no
/// field: one for the components the checker resolved, one per fallback reason.
pub fn merge_semantic(output: &mut ExtractionOutput, outcome: SemanticOutcome) {
    merge_semantic_with(output, outcome, ResolverFacts::Used);
}

/// [`merge_semantic`], taking the facts the checker does not supply from the structural answer only as `resolver`
/// allows.
pub fn merge_semantic_with(output: &mut ExtractionOutput, outcome: SemanticOutcome, resolver: ResolverFacts) {
    merge_semantic_in(output, outcome, resolver, CollapseContext::OFF);
}

/// [`merge_semantic_with`] for a run with the choices `cx`: a source only the checker read collapses as the structural
/// pass would collapse it (`layer_checker_props`). The other merges follow the structural layers only.
pub fn merge_semantic_in(
    output: &mut ExtractionOutput,
    outcome: SemanticOutcome,
    resolver: ResolverFacts,
    cx: CollapseContext,
) {
    let SemanticOutcome { components, fallbacks, diagnostics } = outcome;
    let syntax = SourceSyntax::read(
        components
            .iter()
            .filter_map(|semantic| Some((output.components.get(&semantic.key)?, semantic.props.as_slice()))),
    );
    let mut merged_keys = Vec::new();
    let (mut unread_keys, mut unread_props) = (Vec::new(), 0);
    let mut shared: FxHashMap<u64, Vec<Arc<ParsedProp>>> = FxHashMap::default();
    let mut memo = MergeMemo::default();
    for semantic in components {
        let Some(entry) = output.components.get_mut(&semantic.key) else { continue };
        let unread = apply_props(entry, &semantic, &mut shared, &mut memo, &syntax, resolver, cx).unread;
        if unread > 0 {
            unread_props += unread;
            unread_keys.push(semantic.key.clone());
        }
        merged_keys.push(semantic.key);
    }
    // The whole-project merge drops every structural prop the checker did not return, so none is left over.
    let answered: Vec<Superseding<'_>> =
        merged_keys.iter().filter_map(|key| Some(Superseding::new(key, output.components.get(key)?, false))).collect();
    drop_superseded(&mut output.diagnostics, &answered);
    output.diagnostics.extend(diagnostics);
    output.diagnostics.extend(syntax.diagnostics);

    merged_keys.sort();
    if !resolver.falls_back() {
        degrade_unanswered(output, &merged_keys);
    }
    if !merged_keys.is_empty() {
        let mut provenance = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            format!("{} component(s) resolved by the TypeScript checker", merged_keys.len()),
        );
        provenance.affects = merged_keys;
        output.diagnostics.push(provenance);
    }
    if !unread_keys.is_empty() {
        unread_keys.sort();
        output.diagnostics.push(match resolver.falls_back() {
            true => unread_diagnostic(unread_props, unread_keys),
            false => unread_without_fallback(unread_props, unread_keys),
        });
    }

    let mut by_reason: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for SemanticFallback { key, reason } in fallbacks {
        if output.components.contains_key(&key) {
            by_reason.entry(reason).or_default().push(key);
        }
    }
    for (reason, mut keys) in by_reason {
        keys.sort();
        let mut diagnostic = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            format!("{} component(s) kept their structural result: {reason}", keys.len()),
        );
        diagnostic.affects = keys;
        output.diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests;
