//! A checker type converted to [`PropType`] from the checker's structured answers: type flags, a union's members, a
//! reference's type arguments, a tuple's element flags, an object's properties, index and call signatures, an alias's
//! symbol and arguments. The printed type (`typeToString`) names things for display and for the two facts the
//! structure does not state (see [`crate::printed`]); no shape is read from it.
//!
//! The conversion of a type is a function of the type, how deep it sits and which declarations enclose it, never of
//! the order the checker was asked in, so the memo can be keyed by the checker's type handle (valid inside one
//! snapshot) without making the output depend on query order. Nothing here keys on a type id except that memo: a
//! recursion marker is keyed by the declaration (the alias or interface), not the id.
//!
//! What stays a name instead of an expansion, the named boundary: an object type declared in a dependency
//! (`node_modules`) or an interface or class of the TypeScript lib (`Date`, `HTMLElement`, `Promise<T>`), with its type
//! arguments; a function type is always expanded, since its parameters are what a reader wants. The lib's type-level
//! utilities (`Partial`, `Pick`, `Omit`, `Record`) are evaluated, not named: their result is the type. A dependency's
//! union of literals is expanded too, since its members are the values a control offers. What the caps cut is a typed
//! [`PropType::Truncated`], never the checker's text standing in as a name.

use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use caliper_docgen_core::types::semantic::{degraded_type, unresolved_type};
use caliper_docgen_core::types::{
    BigIntValue, CallSignature, EnumMemberValue, FunctionParam, ObjectField, OpaqueReason, PropType,
    SignatureKind as Kind, TemplatePart, TruncationLimit, TupleElement, TupleElementKind, TupleLabel,
};
use rustc_hash::{FxHashMap, FxHasher};

use crate::printed::{Marker, parameter_markers, tuple_labels};
use crate::rpc::{
    CallError, CompactSymbol, ObjectMembers, Rpc, SignatureInfo, SignatureKind, SymbolInfo, TypeInfo, flag,
};

/// Nesting past which an object, union or intersection is cut with [`TruncationLimit::Depth`].
const MAX_DEPTH: u8 = 5;
/// The most properties an object type lists at the top (a prop's own type) and below it.
const MAX_FIELDS: [usize; 2] = [100, 40];
/// The most members a union or intersection lists at the top and below it.
const MAX_MEMBERS: [usize; 2] = [500, 120];

type Converted<'a> = Pin<Box<dyn Future<Output = Result<PropType, CallError>> + Send + 'a>>;

/// Where a type sits: how deep, and which declarations (aliases, interfaces) enclose it, outermost first.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cx {
    depth: u8,
    enclosing: Arc<Vec<String>>,
}

impl Cx {
    fn deeper(&self) -> Cx {
        Cx { depth: self.depth.saturating_add(1), enclosing: self.enclosing.clone() }
    }

    fn inside(&self, identity: &str) -> Cx {
        let mut enclosing = (*self.enclosing).clone();
        enclosing.push(identity.to_string());
        Cx { depth: self.depth, enclosing: Arc::new(enclosing) }
    }

    fn limit<const N: usize>(&self, limits: [usize; N]) -> usize {
        limits[usize::from(self.depth > 0).min(N - 1)]
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = FxHasher::default();
        self.enclosing.hash(&mut hasher);
        hasher.finish()
    }
}

/// What declares a type: the alias it was written through, else its interface or class.
#[derive(Debug, Clone)]
struct Origin {
    name: String,
    /// The first declaration's handle: stable for the same declaration whatever order the checker is asked in.
    identity: String,
    /// The declaring file as the checker keys it (lower-cased path).
    file: String,
    alias: bool,
}

impl Origin {
    fn of(symbol: &SymbolInfo, alias: bool) -> Self {
        let declaration = symbol.declarations.first().map(String::as_str);
        Origin {
            name: symbol.name.clone(),
            identity: declaration.map_or_else(|| format!("{}:{alias}", symbol.name), str::to_string),
            file: declaration.map(|declaration| declared_in(declaration).to_string()).unwrap_or_default(),
            alias,
        }
    }

    fn in_dependency(&self) -> bool {
        self.file.contains("/node_modules/")
    }

    fn in_lib(&self) -> bool {
        let name = self.file.rsplit('/').next().unwrap_or("");
        name.starts_with("lib.") && name.ends_with(".d.ts")
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

/// `declarations` entries read `"<node index>.<kind>.<path>"`: the path is lower-cased, as the checker keys files.
pub(crate) fn declared_in(declaration: &str) -> &str {
    declaration.splitn(3, '.').nth(2).unwrap_or("")
}

type Memo = Mutex<FxHashMap<(u64, u8, u64), PropType>>;

#[derive(Default)]
struct State {
    /// Conversions by (type handle, depth, enclosing declarations). The handle is valid inside this snapshot only.
    memo: Memo,
    origins: Mutex<FxHashMap<(bool, CompactSymbol), Option<Origin>>>,
    printed: Mutex<FxHashMap<u64, String>>,
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

    fn convert<'a>(&'a self, info: &'a TypeInfo, cx: &'a Cx) -> Converted<'a> {
        Box::pin(async move {
            let key = (info.id, cx.depth, cx.fingerprint());
            if let Some(hit) = self.state.memo.lock().ok().and_then(|memo| memo.get(&key).cloned()) {
                self.state.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(hit);
            }
            self.state.misses.fetch_add(1, Ordering::Relaxed);
            let converted = self.uncached(info, cx).await?;
            if let Ok(mut memo) = self.state.memo.lock() {
                memo.insert(key, converted.clone());
            }
            Ok(converted)
        })
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
        // Types the checker could not resolve because a type parameter is still open, and `Uppercase<…>`.
        if flags & flag::STRING_MAPPING != 0 {
            return Ok(named(self.display(info.id).await?, Vec::new()));
        }
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
        if let Some(text) = self.state.printed.lock().ok().and_then(|printed| printed.get(&id).cloned()) {
            return Ok(text);
        }
        let text = self.rpc.type_to_string(id).await?;
        if let Ok(mut printed) = self.state.printed.lock() {
            printed.insert(id, text.clone());
        }
        Ok(text)
    }

    /// What declares the type, when it has a name of its own: an alias, or an interface, class or instantiation of one.
    async fn origin(&self, info: &TypeInfo) -> Result<Option<Origin>, CallError> {
        if let Some(alias) = &info.alias_symbol {
            return self.origin_of(alias, true, info.id).await;
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
        let holes = self.rpc.members(info.id).await?;
        let mut types = self.convert_all(&holes, &cx.deeper()).await?.into_iter();
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
        let arguments = if origin.alias {
            if info.alias_type_arguments.is_empty() {
                Vec::new()
            } else {
                self.rpc.alias_type_arguments(info.id).await?
            }
        } else if info.object_flags & flag::OBJECT_REFERENCE != 0 {
            let mut arguments = self.rpc.type_arguments(info.id).await?;
            while arguments.last().is_some_and(|argument| argument.is_this_type) {
                arguments.pop();
            }
            arguments
        } else {
            Vec::new()
        };
        self.convert_all(&arguments, &cx.deeper()).await
    }

    async fn named_type(&self, info: &TypeInfo, origin: &Origin, cx: &Cx) -> Result<PropType, CallError> {
        let args = self.type_arguments_of(info, origin, cx).await?;
        Ok(PropType::Named { name: origin.name.clone().into(), args })
    }

    async fn union(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let origin = self.origin(info).await?;
        if let Some(terminal) = origin.as_ref().and_then(Origin::react_terminal) {
            return Ok(terminal);
        }
        if let Some(origin) = &origin
            && cx.enclosing.contains(&origin.identity)
        {
            return self.recursive(info, origin, cx).await;
        }
        let members = self.rpc.members(info.id).await?;
        let total = members.len();
        if total > cx.limit(MAX_MEMBERS) {
            return self.truncated(info, origin.as_ref(), TruncationLimit::Members { total: to_u32(total) }).await;
        }
        if cx.depth >= MAX_DEPTH {
            return self.truncated(info, origin.as_ref(), TruncationLimit::Depth).await;
        }
        let inner = origin.as_ref().map_or_else(|| cx.deeper(), |origin| cx.inside(&origin.identity).deeper());
        Ok(union_of(self.convert_all(&members, &inner).await?))
    }

    async fn intersection(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let origin = self.origin(info).await?;
        if let Some(origin) = &origin {
            if cx.enclosing.contains(&origin.identity) {
                return self.recursive(info, origin, cx).await;
            }
            // A dependency's `Nested<Style>`-like alias is a name, as its interface would be.
            if origin.is_boundary() {
                return self.named_type(info, origin, cx).await;
            }
        }
        let members = self.rpc.members(info.id).await?;
        let total = members.len();
        if total > cx.limit(MAX_MEMBERS) {
            return self.truncated(info, origin.as_ref(), TruncationLimit::Members { total: to_u32(total) }).await;
        }
        if cx.depth >= MAX_DEPTH {
            return self.truncated(info, origin.as_ref(), TruncationLimit::Depth).await;
        }
        let inner = origin.as_ref().map_or_else(|| cx.deeper(), |origin| cx.inside(&origin.identity).deeper());
        let mut converted: Vec<PropType> = Vec::with_capacity(total);
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
        if let Some(origin) = &origin
            && cx.enclosing.contains(&origin.identity)
        {
            return self.recursive(info, origin, cx).await;
        }
        if cx.depth >= MAX_DEPTH {
            return self.truncated(info, origin.as_ref(), TruncationLimit::Depth).await;
        }

        let ObjectMembers { calls, constructs, properties, indexes } = self.rpc.object_members(info.id).await?;
        // A dependency's function with properties of its own (a component, a hook) is a name.
        if let Some(origin) = boundary
            && !properties.is_empty()
        {
            return self.named_type(info, origin, cx).await;
        }
        let total = properties.len() + indexes.len();
        if total > cx.limit(MAX_FIELDS) {
            return self.truncated(info, origin.as_ref(), TruncationLimit::Members { total: to_u32(total) }).await;
        }

        let inner = origin.as_ref().map_or_else(|| cx.deeper(), |origin| cx.inside(&origin.identity).deeper());
        if let [call] = calls.as_slice()
            && constructs.is_empty()
            && properties.is_empty()
            && indexes.is_empty()
        {
            return self.function(call, Some(info.id), &inner).await;
        }
        let fields = self.fields(&properties, &indexes, &inner).await?;
        if calls.is_empty() && constructs.is_empty() {
            return Ok(PropType::Object(fields));
        }
        let mut signatures = Vec::with_capacity(calls.len() + constructs.len());
        for (kind, group) in [(Kind::Call, &calls), (Kind::Construct, &constructs)] {
            for signature in group {
                signatures.push(self.call_signature(kind, signature, &inner).await?);
            }
        }
        Ok(PropType::Callable { signatures, fields })
    }

    async fn array(&self, info: &TypeInfo, readonly: bool, cx: &Cx) -> Result<PropType, CallError> {
        let arguments = self.rpc.type_arguments(info.id).await?;
        let Some(element) = arguments.first() else {
            return Ok(named(self.display(info.id).await?, Vec::new()));
        };
        let element = PropType::Array(Box::new(self.convert(element, &cx.deeper()).await?));
        // `readonly T[]` is `ReadonlyArray<T>`: the same reference through another declaration.
        Ok(if readonly { PropType::Readonly(Box::new(element)) } else { element })
    }

    async fn tuple(&self, info: &TypeInfo, cx: &Cx) -> Result<PropType, CallError> {
        let arguments = self.rpc.type_arguments(info.id).await?;
        // The reference carries the arguments; the tuple type it instantiates carries how each element is declared.
        let target = self.rpc.target_of_type(info.id).await?;
        let labels = if target.labeled_element_declarations.is_empty() {
            None
        } else {
            tuple_labels(&self.display(info.id).await?, arguments.len())
        };
        let element_types = self.convert_all(&arguments, &cx.deeper()).await?;
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

    async fn function(&self, signature: &SignatureInfo, printed: Option<u64>, cx: &Cx) -> Result<PropType, CallError> {
        let (type_params, params, return_type) = self.signature_parts(signature, printed, cx).await?;
        Ok(PropType::Function { type_params, params, return_type: Box::new(return_type) })
    }

    async fn call_signature(&self, kind: Kind, signature: &SignatureInfo, cx: &Cx) -> Result<CallSignature, CallError> {
        // The printed type of an overload set is not one parameter list: which parameters are optional is not stated.
        let (type_params, params, return_type) = self.signature_parts(signature, None, cx).await?;
        Ok(CallSignature { kind, type_params, params, return_type })
    }

    async fn signature_parts(
        &self,
        signature: &SignatureInfo,
        printed: Option<u64>,
        cx: &Cx,
    ) -> Result<(Vec<String>, Vec<FunctionParam>, PropType), CallError> {
        let parameters = self.rpc.parameters(signature.id).await?;
        let references: Vec<_> = parameters.iter().map(|parameter| &parameter.reference).collect();
        let types = if references.is_empty() { Vec::new() } else { self.rpc.types_of_symbols(&references).await? };
        // The printed type of the function states which parameters are optional; it is only asked for when there is one.
        let markers: Option<Vec<Marker>> = match printed {
            Some(id) if !parameters.is_empty() => parameter_markers(&self.display(id).await?),
            _ => None,
        };
        // A printed signature that does not parse into one entry per parameter leaves the markers off.
        let markers = markers.filter(|markers| markers.len() == parameters.len());
        let has_rest = signature.flags & flag::SIGNATURE_HAS_REST_PARAMETER != 0;
        let converted = self.convert_all(&types, cx).await?;
        let mut params = Vec::with_capacity(parameters.len());
        for (index, (parameter, param_type)) in parameters.iter().zip(converted).enumerate() {
            let marker = markers.as_ref().and_then(|markers| markers.get(index)).copied().unwrap_or_default();
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
        Ok((type_params, params, return_type))
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
        (flag::BIG_INT, named("bigint".to_string(), Vec::new())),
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
