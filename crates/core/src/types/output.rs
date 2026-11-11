//! Semantic output types produced by the resolver (Phase 3).

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use super::collected::{EnumEntry, TypeName};
use super::diagnostic::{Diagnostic, DiagnosticSeverity};
use super::global::ResolveState;

pub(crate) mod interned;

/// The version of the canonical JSON shape (ADR 0017): `schemaVersion` in an [`ExtractionOutput`], an
/// [`IncrementalUpdate`](crate::pipeline::IncrementalUpdate) and a session index. Bumped whenever a reader of the
/// previous shape would misread or lose data from the new one; never by an addition it can ignore. Version 2 writes a
/// component's props as prop set ids and an inheritance layer's `omitted` as a name list id (ADR 0018). Version 5 writes
/// a tuple's elements as `{ type, label?, modifier? }` and adds the kinds the checker states (ADR 0026, slice S1).
/// Version 6 writes `bigint` as `{ "kind": "bigint" }`, where version 5 wrote `{ "kind": "named", "name": "bigint" }`.
/// Version 7 gives each component `exports`, the names its module exports it under (ADR 0027): a reader of version 6
/// has no way to tell which export a component is, and joined on the display name instead.
pub const SCHEMA_VERSION: u32 = 7;

// ─── Top-level output ─────────────────────────────────────────────────────────

/// The complete extraction output — top-level return type of the pipeline. Its JSON writes each distinct prop once
/// and refers to it by id (see `interned`).
#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionOutput {
    /// Extracted component entries, keyed by component name
    pub components: BTreeMap<String, ComponentEntry>,
    /// Enum definitions discovered during extraction
    pub enums: BTreeMap<String, Vec<EnumEntry>>,
    /// Non-fatal issues encountered during extraction
    pub diagnostics: Vec<Diagnostic>,
    /// Extraction statistics
    pub stats: ExtractionStats,
}

impl ExtractionOutput {
    /// Highest-severity diagnostic present, if any — `Error` outranks
    /// `Warning` outranks `Info`. Ranked explicitly rather than via derived
    /// `Ord` on `DiagnosticSeverity`, since that enum has no ordering today
    /// and its declaration order isn't a promise about severity ranking.
    pub fn max_severity(&self) -> Option<DiagnosticSeverity> {
        self.diagnostics.iter().map(|d| d.severity.clone()).max_by_key(severity_rank)
    }

    /// Process exit code for this output: `2` if any diagnostic is
    /// `Error`-severity, `1` if `strict` and any diagnostic is at least
    /// `Warning`-severity, `0` otherwise. This is the CLI's shared exit-code
    /// contract — `caliper-docgen check --strict`'s mapping, reused as-is
    /// by `extract`, `watch`, and `inspect`.
    pub fn exit_code(&self, strict: bool) -> i32 {
        match self.max_severity() {
            Some(DiagnosticSeverity::Error) => 2,
            Some(DiagnosticSeverity::Warning) => {
                if strict {
                    1
                } else {
                    0
                }
            }
            Some(DiagnosticSeverity::Info) => 0,
            None => 0,
        }
    }
}

/// Explicit worst-first rank for `DiagnosticSeverity` — `Error` > `Warning` > `Info`.
pub(crate) fn severity_rank(severity: &DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 2,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 0,
    }
}

/// One step in a component's resolved inheritance chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InheritedLayer {
    /// Type name as declared: "ButtonHTMLAttributes", "ButtonBaseProps", "ThemingProps"
    pub type_name: String,
    /// Absolute path of the file this came from.
    /// "node_modules/@types/react/..." for HTML attrs; project-local path otherwise.
    pub file_name: String,
    /// Props explicitly removed at this layer via Omit<Base, K>.
    pub omitted: Vec<String>,
    /// If this is an HTML element attributes type, the element name: "button", "input", etc.
    pub html_element: Option<String>,
    /// Number of props this layer contributes (for display).
    pub total_props: u32,
    /// For a collapsed source, the names of the props it declares, which the output otherwise leaves out (only the
    /// notable ones are listed as props). Empty for a layer whose props are expanded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
}

/// A single React component with its resolved props.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentEntry {
    /// Display name of the component
    pub display_name: String,
    /// Absolute path of the file containing the component
    pub file_path: Utf8PathBuf,
    /// Component-level JSDoc description
    pub description: String,
    /// Resolved props, keyed by prop name. Shared: components that inherit one interface hold the same prop, so change
    /// one through `Arc::make_mut`, which copies it first when another component holds it too.
    pub props: BTreeMap<String, Arc<ParsedProp>>,
    /// Full resolved inheritance chain, outermost first.
    pub inheritance: Vec<InheritedLayer>,
    /// Curated notable props from inherited layers (not in self.props), shared like `props`.
    pub notable_inherited: BTreeMap<String, Arc<ParsedProp>>,
    /// If props type is a discriminated union, the name of the discriminant prop (e.g. "variant").
    pub discriminant_prop: Option<String>,
    /// Type names that could not be resolved (react-docgen compat)
    pub composes: Vec<String>,
    /// JSDoc @tags on the component (e.g. @deprecated, @since)
    pub tags: BTreeMap<String, String>,
    /// Always empty for functional components; present for RDT compat
    pub methods: Vec<()>,
    /// The extractor rule that recognized this component. `typedFunctionExpression` and `wrapperTypeArgument` are
    /// inferred from a function's shape and what it returns; the others are explicit declaration forms.
    pub detected_by: crate::types::DetectionRule,
    /// Whether its module exports it, directly or as a sub-component attached to an exported component. An unexported
    /// component is an implementation detail other components render; RDT does not report those.
    pub exported: bool,
    /// How its module exports it ([`crate::types::ComponentMapping::exports`]); `exported` is whether it has any. The
    /// TypeScript checker asks for the component by these, and a reader of the canonical JSON finds the component by
    /// file and export instead of by display name.
    pub exports: Vec<crate::types::ComponentExport>,
}

impl ComponentEntry {
    /// Shortcut: the HTML element this component renders as, if any.
    /// Derived from the inheritance chain.
    pub fn html_element(&self) -> Option<&str> {
        self.inheritance.iter().find_map(|l| l.html_element.as_deref())
    }
}

/// A single resolved prop.
///
/// Built only through [`ParsedProp::new`], which makes a prop with a default value optional:
///
/// ```
/// use caliper_docgen_core::types::{ParsedProp, PropType};
///
/// let prop = ParsedProp::new("x".into(), PropType::String, true, None, String::new(), Default::default(), None, vec![]);
/// assert!(prop.required);
/// ```
///
/// A struct literal, which would skip that rule, does not compile outside this module:
///
/// ```compile_fail
/// use caliper_docgen_core::types::{ParsedProp, PropType};
///
/// let prop = ParsedProp {
///     name: "x".into(),
///     prop_type: PropType::String,
///     required: true,
///     default_value: None,
///     description: String::new(),
///     tags: Default::default(),
///     parent: None,
///     declarations: vec![],
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedProp {
    /// Prop name
    pub name: String,
    /// Resolved semantic type
    #[serde(rename = "type")]
    pub prop_type: PropType,
    /// Whether the prop is required
    pub required: bool,
    /// Default value if known (from destructured params or JSDoc)
    pub default_value: Option<DefaultValue>,
    /// JSDoc description of the prop
    pub description: String,
    /// JSDoc @tags on the prop (@deprecated, @since, @see, etc.)
    pub tags: BTreeMap<String, String>,
    /// Interface/type where this prop was originally declared
    pub parent: Option<PropParent>,
    /// All declarations of this prop name (for overloads/merging)
    pub declarations: Vec<PropParent>,
    /// Why this prop's type is partly a guess, when part of it is `opaque`: the reason kinds of its opaque members
    /// (`conditionalType`, `mappedType`, `indexedAccess`, …), each once, in the order the type holds them. Empty for a
    /// type the resolver evaluated in full. Derived from `prop_type`, so it never disagrees with it, with one addition:
    /// a prop the TypeScript checker could not read keeps its structural type, and lists after that type's own kinds
    /// the kinds of what the checker could not read, since the checker did not confirm the type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_by: Vec<String>,
    /// Private, zero-sized, and unconstructible outside this module — its
    /// only purpose is to make a bare `ParsedProp { .. }` struct literal
    /// fail to compile anywhere else, including other modules in this same
    /// crate, so `required`/`default_value` can only be set together
    /// through `ParsedProp::new`'s normalization. Skipped in both
    /// directions of serde so the wire format is unaffected.
    #[serde(skip)]
    _seal: Seal,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Seal;

impl ParsedProp {
    /// Feed `hasher` everything `==` compares, so equal props hash equal (`ResolutionContext::share`).
    pub(crate) fn structural_hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        use std::hash::Hash;
        self.name.hash(hasher);
        self.prop_type.structural_hash(hasher);
        self.required.hash(hasher);
        if let Some(default) = &self.default_value {
            default.value.hash(hasher);
            default.computed.hash(hasher);
        }
        self.description.hash(hasher);
        self.tags.hash(hasher);
        for parent in self.parent.iter().chain(&self.declarations) {
            parent.name.hash(hasher);
            parent.file_name.hash(hasher);
        }
    }

    /// This prop with `default_value` as its default, and so not required.
    pub(crate) fn with_default_value(&self, default_value: DefaultValue) -> ParsedProp {
        ParsedProp::new(
            self.name.clone(),
            self.prop_type.clone(),
            self.required,
            Some(default_value),
            self.description.clone(),
            self.tags.clone(),
            self.parent.clone(),
            self.declarations.clone(),
        )
    }

    /// Constructs a `ParsedProp`, normalizing the `required`/`default_value`
    /// relationship: RDT convention is that a supplied default value makes a
    /// prop effectively optional regardless of what `required` was computed
    /// as upstream (e.g. a destructured param with both a type annotation
    /// marking it required and a default expression).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        prop_type: PropType,
        required: bool,
        default_value: Option<DefaultValue>,
        description: String,
        tags: BTreeMap<String, String>,
        parent: Option<PropParent>,
        declarations: Vec<PropParent>,
    ) -> Self {
        let required = if default_value.is_some() { false } else { required };
        let degraded_by = prop_type.degraded_by().into_iter().map(str::to_owned).collect();
        ParsedProp {
            name,
            prop_type,
            required,
            default_value,
            description,
            tags,
            parent,
            declarations,
            degraded_by,
            _seal: Seal,
        }
    }

    /// This prop with `kinds` added to its `degraded_by`, each once and after its own: the reasons another reader of
    /// the same prop (the checker) could not read it. The type is left as it is.
    pub(crate) fn degraded_also_by(mut self, kinds: &[&'static str]) -> ParsedProp {
        for kind in kinds {
            if !self.degraded_by.iter().any(|own| own == kind) {
                self.degraded_by.push((*kind).to_string());
            }
        }
        self
    }
}

#[cfg(test)]
mod parsed_prop_tests {
    use super::*;

    #[test]
    fn new_normalizes_required_false_when_default_value_present() {
        let prop = ParsedProp::new(
            "variant".to_string(),
            PropType::String,
            true, // caller (incorrectly) says required
            Some(DefaultValue { value: "\"primary\"".to_string(), computed: false }),
            "desc".to_string(),
            Default::default(),
            None,
            vec![],
        );

        assert!(!prop.required, "a prop with a default value must not be reported as required");
        assert!(prop.default_value.is_some());
    }

    #[test]
    fn new_preserves_required_true_when_no_default_value() {
        let prop = ParsedProp::new(
            "variant".to_string(),
            PropType::String,
            true,
            None,
            "desc".to_string(),
            Default::default(),
            None,
            vec![],
        );

        assert!(prop.required);
    }
}

/// Default value for a prop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultValue {
    /// String representation of the default value
    pub value: String,
    /// Whether this is a computed value we couldn't fully evaluate
    pub computed: bool,
}

/// Where a prop was declared — enables RDT propFilter compatibility.
/// The canonical RDT pattern: `prop => !prop.parent.fileName.includes('node_modules')`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropParent {
    /// Name of the interface/type alias that declared this prop
    pub name: String,
    /// Absolute path of the file — matches RDT's `fileName` field
    pub file_name: String,
}

// ─── PropType — The Canonical Semantic Type Representation ────────────────────

/// Semantic type of a prop. Rich enough to produce any output format.
/// Use match exhaustively — no `_` fallthrough in serializers.
#[derive(Debug, Clone, PartialEq)]
pub enum PropType {
    // ── Primitives
    /// `string`.
    String,
    /// `number`.
    Number,
    /// `boolean`; a `true | false` union is collapsed into this.
    Boolean,
    /// `null`.
    Null,
    /// `undefined`.
    Undefined,
    /// `any`.
    Any,
    /// `never`.
    Never,
    /// `unknown`.
    Unknown,
    /// `void`.
    Void,
    /// `bigint`.
    BigInt,

    // ── Literals
    /// A string literal type such as `"primary"`, holding the value without its quotes.
    StringLiteral(std::string::String),
    /// A number literal type such as `3`.
    NumberLiteral(f64),
    /// `true` or `false` as a type.
    BoolLiteral(bool),
    /// A bigint literal type such as `10n` or `-1n`.
    BigIntLiteral(BigIntValue),
    /// `typeof name` where the value `name` is declared `unique symbol`: a symbol type of its own, unlike `symbol`.
    UniqueSymbol {
        /// The name of the value declared `unique symbol` (`brand` in `declare const brand: unique symbol`).
        name: TypeName,
    },
    /// One member of an enum, as a type: `Size.Small` in `enum Size { Small = "small" }`. It is that member's value
    /// and keeps the enum's identity, which a plain literal loses.
    EnumMember {
        /// The enum's name as written (`Size`).
        enum_name: TypeName,
        /// The member's name (`Small`).
        member: TypeName,
        /// The member's value.
        value: EnumMemberValue,
    },
    /// A template literal type that stays a pattern, `` `${number}px` ``: its text pieces and `${…}` holes in order.
    /// A template whose holes are all finite literal unions is expanded to its strings instead.
    TemplateLiteral(Vec<TemplatePart>),

    // ── Composites
    /// `A | B`: the members in declaration order.
    Union(Vec<PropType>),
    /// `A & B`: the members in declaration order.
    Intersection(Vec<PropType>),
    /// `T[]` or `Array<T>`: the element type.
    Array(Box<PropType>),
    /// `[A, B?, ...C[]]`: the elements in order, each with its optional, rest or variadic marker and its label. A
    /// spread of a tuple the resolver knows (`[A, ...Tail]`) is flattened into its elements, as TypeScript does.
    Tuple(Vec<TupleElement>),
    /// `readonly string[]` or `readonly [a, b]`: the array or tuple under the `readonly` operator.
    Readonly(Box<PropType>),
    /// An object type `{ a: string; b?: number }`: its members and index signatures, in declaration order.
    Object(Vec<ObjectField>),
    /// An object type with call or construct signatures that is not one plain function: overloads
    /// `{ (a: string): string; (a: number): number }`, a constructor `{ new (a: string): Date }`, or a function with
    /// properties `{ (): void; displayName?: string }`. A single call signature with nothing else is a [`Function`].
    ///
    /// [`Function`]: PropType::Function
    Callable {
        /// The call and construct signatures, in declaration order.
        signatures: Vec<CallSignature>,
        /// The properties and index signatures beside them, in declaration order.
        fields: Vec<ObjectField>,
    },

    // ── Named reference (post-resolution)
    /// A reference to a type by name that the resolver kept rather than expanding (a type from a dependency that was
    /// not loaded, or one that is better shown as a name), with its type arguments.
    Named {
        /// The name of the referenced type.
        name: TypeName,
        /// Generic args, already resolved to PropType
        args: Vec<PropType>,
    },
    /// A reference back to a type whose expansion encloses it: the nested `Json` in
    /// `type Json = string | Json[] | { [key: string]: Json }`. Not a loss: the enclosing expansion is the type.
    Recursive {
        /// The name of the enclosing type.
        name: TypeName,
        /// Its type arguments as written at the reference.
        args: Vec<PropType>,
    },
    /// A type cut short at a depth or size limit, kept as its text. Unlike [`Opaque`] it is not a construct the
    /// resolver cannot read: it chose not to expand further. It counts in `degradedBy` as `truncated`.
    ///
    /// [`Opaque`]: PropType::Opaque
    Truncated {
        /// The type's text where it was cut: its name, or how the checker prints it.
        text: std::string::String,
        /// Which limit cut it.
        limit: TruncationLimit,
    },

    // ── React-specific terminals
    /// ReactNode, ReactElement, JSX.Element — all treated the same for docgen
    ReactNode,
    /// React.CSSProperties
    CssProperties,
    /// A function type: callbacks, render props and event handlers alike (`(value: number) => ReactNode`).
    Function {
        /// The function's own type parameters: `T` in `<T>(item: T) => ReactNode`.
        type_params: Vec<std::string::String>,
        /// The parameters in order.
        params: Vec<FunctionParam>,
        /// What the function returns, as written.
        return_type: Box<PropType>,
    },
    /// `Ref<T>`, `RefObject<T>`, `ForwardedRef<T>` and the other ref types React declares.
    Ref {
        /// The React type as written, without a `React.` prefix: `RefObject`, `ForwardedRef`, `RefCallback`.
        name: std::string::String,
        /// Inner element type if known
        element: Option<std::string::String>,
    },
    /// React.ElementType — component can render as any element
    /// Used for: `as?: ElementType`, `component?: ElementType`
    ElementType,

    // ── HTML attribute inheritance
    /// All attributes of an HTML element, minus omitted keys.
    /// Produced when we see ComponentPropsWithoutRef<'button'> etc.
    HtmlAttributes {
        /// The HTML element whose attributes are inherited: `button` in `ComponentPropsWithoutRef<'button'>`.
        element: std::string::String,
        /// Attribute names left out of the element's set.
        omitted: Vec<std::string::String>,
    },

    // ── Variant systems (statically resolved)
    /// Result of CvaVariantPropsHandler — the wrapper is fully dissolved.
    /// Each member is a string literal value.
    LiteralUnion {
        /// The string literal values, in declaration order.
        members: Vec<std::string::String>,
        /// true if this prop has a defaultVariant
        has_default: bool,
    },

    // ── Known opaque patterns (cannot/should not be expanded)
    /// MUI SxProps, SystemStyleObject etc. — complex, not user-facing props
    SxProps,

    // ── Unresolvable — graceful degradation
    /// A type the resolver gave up on, kept as written with the reason. Every one comes with a diagnostic that
    /// explains why (see [`OpaqueDetail`]).
    Opaque(OpaqueDetail),
}

/// The private payload of `PropType::Opaque`. Fields are unreachable from
/// outside this module — the only ways to build one are `give_up` (pushes
/// the diagnostic that explains the degradation, then builds the value) and
/// `new` (for the one documented exception: `known.rs`, which has no
/// diagnostics channel of its own and pushes its own diagnostic separately
/// via `known::push_known_opaque_diagnostic` — see that function's doc
/// comment). Read the payload back through `raw()`/`reason()`.
#[derive(Debug, Clone, PartialEq)]
pub struct OpaqueDetail {
    raw: std::string::String,
    reason: OpaqueReason,
}

impl OpaqueDetail {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(raw: impl Into<std::string::String>, reason: OpaqueReason) -> PropType {
        PropType::Opaque(OpaqueDetail { raw: raw.into(), reason })
    }

    /// The sanctioned "give up and record why" constructor — pushes
    /// `diagnostic` onto `state` before building the value, so a resolver
    /// call site that reaches for this can't forget to explain the
    /// degradation the way the old bare `PropType::Opaque { .. }` literal let
    /// call sites do.
    pub(crate) fn give_up(
        state: &mut ResolveState,
        raw: impl Into<std::string::String>,
        reason: OpaqueReason,
        diagnostic: Diagnostic,
    ) -> PropType {
        state.diagnostics.push(diagnostic);
        Self::new(raw, reason)
    }

    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    pub(crate) fn reason(&self) -> &OpaqueReason {
        &self.reason
    }
}

impl PropType {
    /// Feed `hasher` everything that tells two types apart, so equal types hash equal. The resolver shares identical
    /// props by this hash (`ResolutionContext::share`).
    pub(crate) fn structural_hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        use std::hash::Hash;
        std::mem::discriminant(self).hash(hasher);
        let all = |hasher: &mut H, members: &[PropType]| {
            members.len().hash(hasher);
            members.iter().for_each(|member| member.structural_hash(hasher));
        };
        match self {
            PropType::String
            | PropType::Number
            | PropType::Boolean
            | PropType::Null
            | PropType::Undefined
            | PropType::Any
            | PropType::Never
            | PropType::Unknown
            | PropType::Void
            | PropType::BigInt
            | PropType::ReactNode
            | PropType::CssProperties
            | PropType::ElementType
            | PropType::SxProps => {}
            PropType::StringLiteral(s) => s.hash(hasher),
            PropType::NumberLiteral(n) => n.to_bits().hash(hasher),
            PropType::BoolLiteral(b) => b.hash(hasher),
            PropType::BigIntLiteral(value) => value.as_str().hash(hasher),
            PropType::UniqueSymbol { name } => name.hash(hasher),
            PropType::EnumMember { enum_name, member, value } => {
                enum_name.hash(hasher);
                member.hash(hasher);
                match value {
                    EnumMemberValue::String(s) => s.hash(hasher),
                    EnumMemberValue::Number(n) => n.to_bits().hash(hasher),
                }
            }
            PropType::TemplateLiteral(parts) => {
                parts.len().hash(hasher);
                for part in parts {
                    match part {
                        TemplatePart::Text(text) => text.hash(hasher),
                        TemplatePart::Type(ty) => ty.structural_hash(hasher),
                    }
                }
            }
            PropType::Union(members) | PropType::Intersection(members) => all(hasher, members),
            PropType::Tuple(elements) => {
                elements.len().hash(hasher);
                for element in elements {
                    element.kind.hash(hasher);
                    element.label.hash(hasher);
                    element.element_type.structural_hash(hasher);
                }
            }
            PropType::Array(inner) | PropType::Readonly(inner) => inner.structural_hash(hasher),
            PropType::Object(fields) => hash_fields(fields, hasher),
            PropType::Callable { signatures, fields } => {
                signatures.len().hash(hasher);
                for signature in signatures {
                    signature.kind.hash(hasher);
                    hash_signature(&signature.type_params, &signature.params, &signature.return_type, hasher);
                }
                hash_fields(fields, hasher);
            }
            PropType::Named { name, args } | PropType::Recursive { name, args } => {
                name.hash(hasher);
                all(hasher, args);
            }
            PropType::Truncated { text, limit } => {
                text.hash(hasher);
                limit.hash(hasher);
            }
            PropType::Function { type_params, params, return_type } => {
                hash_signature(type_params, params, return_type, hasher)
            }
            PropType::Ref { name, element } => {
                name.hash(hasher);
                element.hash(hasher);
            }
            PropType::HtmlAttributes { element, omitted } => {
                element.hash(hasher);
                omitted.hash(hasher);
            }
            PropType::LiteralUnion { members, has_default } => {
                members.hash(hasher);
                has_default.hash(hasher);
            }
            // Rare, and its reason has no cheap hash of its own.
            PropType::Opaque(detail) => {
                detail.raw.hash(hasher);
                format!("{:?}", detail.reason).hash(hasher);
            }
        }
    }

    /// Whether a conditional, mapped or indexed access type the static resolver can't evaluate appears anywhere in this
    /// type, so its text would include that type's source.
    pub(crate) fn holds_unevaluated_type(&self) -> bool {
        self.holds_opaque_where(&|detail| {
            matches!(
                detail.reason(),
                OpaqueReason::ConditionalType | OpaqueReason::MappedType | OpaqueReason::IndexedAccess { .. }
            )
        })
    }

    /// Whether any opaque or truncated type, whatever the reason, appears anywhere in this type: whether the checker
    /// could give a better answer for it.
    pub(crate) fn holds_opaque(&self) -> bool {
        let mut found = false;
        self.visit(&mut |ty| found |= matches!(ty, PropType::Opaque(_) | PropType::Truncated { .. }));
        found
    }

    fn holds_opaque_where(&self, test: &dyn Fn(&OpaqueDetail) -> bool) -> bool {
        let mut found = false;
        self.visit(&mut |ty| found |= matches!(ty, PropType::Opaque(detail) if test(detail)));
        found
    }

    /// Call `f` on this type, then on every type inside it, depth first and in the order the type holds them (an
    /// index signature's key before its value).
    fn visit(&self, f: &mut dyn FnMut(&PropType)) {
        f(self);
        match self {
            PropType::Union(members) | PropType::Intersection(members) => members.iter().for_each(|m| m.visit(f)),
            PropType::Tuple(elements) => elements.iter().for_each(|e| e.element_type.visit(f)),
            PropType::Array(inner) | PropType::Readonly(inner) => inner.visit(f),
            PropType::Object(fields) => visit_fields(fields, f),
            PropType::Callable { signatures, fields } => {
                for signature in signatures {
                    signature.params.iter().for_each(|p| p.param_type.visit(f));
                    signature.return_type.visit(f);
                }
                visit_fields(fields, f);
            }
            PropType::TemplateLiteral(parts) => parts.iter().for_each(|part| match part {
                TemplatePart::Text(_) => {}
                TemplatePart::Type(ty) => ty.visit(f),
            }),
            PropType::Named { args, .. } | PropType::Recursive { args, .. } => args.iter().for_each(|a| a.visit(f)),
            PropType::Function { params, return_type, .. } => {
                params.iter().for_each(|p| p.param_type.visit(f));
                return_type.visit(f);
            }
            PropType::String
            | PropType::Number
            | PropType::Boolean
            | PropType::Null
            | PropType::Undefined
            | PropType::Any
            | PropType::Never
            | PropType::Unknown
            | PropType::Void
            | PropType::BigInt
            | PropType::StringLiteral(_)
            | PropType::NumberLiteral(_)
            | PropType::BoolLiteral(_)
            | PropType::BigIntLiteral(_)
            | PropType::UniqueSymbol { .. }
            | PropType::EnumMember { .. }
            | PropType::Truncated { .. }
            | PropType::ReactNode
            | PropType::CssProperties
            | PropType::Ref { .. }
            | PropType::ElementType
            | PropType::HtmlAttributes { .. }
            | PropType::LiteralUnion { .. }
            | PropType::SxProps
            | PropType::Opaque(_) => {}
        }
    }

    /// The primitive this type is one literal value of, or `None` for a type that is not a single literal. An enum
    /// member is a literal of its value's primitive. Every projection that decides whether a type is a value to offer
    /// (RDT's `enum`, TOON's uncut unions) asks this, so they agree on what a literal is.
    pub(crate) fn literal_kind(&self) -> Option<LiteralKind> {
        match self {
            PropType::StringLiteral(_) | PropType::EnumMember { value: EnumMemberValue::String(_), .. } => {
                Some(LiteralKind::String)
            }
            PropType::NumberLiteral(_) | PropType::EnumMember { value: EnumMemberValue::Number(_), .. } => {
                Some(LiteralKind::Number)
            }
            PropType::BoolLiteral(_) => Some(LiteralKind::Boolean),
            PropType::BigIntLiteral(_) => Some(LiteralKind::BigInt),
            PropType::String
            | PropType::Number
            | PropType::Boolean
            | PropType::Null
            | PropType::Undefined
            | PropType::Any
            | PropType::Never
            | PropType::Unknown
            | PropType::Void
            | PropType::BigInt
            | PropType::UniqueSymbol { .. }
            | PropType::TemplateLiteral(_)
            | PropType::Union(_)
            | PropType::Intersection(_)
            | PropType::Array(_)
            | PropType::Tuple(_)
            | PropType::Readonly(_)
            | PropType::Object(_)
            | PropType::Callable { .. }
            | PropType::Named { .. }
            | PropType::Recursive { .. }
            | PropType::Truncated { .. }
            | PropType::ReactNode
            | PropType::CssProperties
            | PropType::Function { .. }
            | PropType::Ref { .. }
            | PropType::ElementType
            | PropType::HtmlAttributes { .. }
            | PropType::LiteralUnion { .. }
            | PropType::SxProps
            | PropType::Opaque(_) => None,
        }
    }

    /// The source text of this type as one option of a control: the literal's own text, and for an enum member its
    /// value's (`"small"` for `Size.Small`), which is what the control passes at runtime. Every other type is its
    /// `raw_string`.
    pub fn option_text(&self) -> std::string::String {
        let PropType::EnumMember { value, .. } = self else { return self.raw_string() };
        match value {
            EnumMemberValue::String(s) => PropType::StringLiteral(s.clone()).raw_string(),
            EnumMemberValue::Number(n) => PropType::NumberLiteral(*n).raw_string(),
        }
    }

    /// True if this type offers string or number options, which RDT output writes as an `enum` so Storybook shows the
    /// options as a radio: a union of string and number literals (enum members included), or a single one. A union
    /// that also has a boolean member is an enum only when Storybook cannot read it from its type name (see
    /// `reads_as_json_options`), because the enum shape turns `true` into the string "true". A union with no members,
    /// or only booleans, has nothing to offer; a bigint literal is not an option Storybook can pass.
    pub fn is_literal_union(&self) -> bool {
        let is_option = |m: &PropType| matches!(m.literal_kind(), Some(LiteralKind::String | LiteralKind::Number));
        match self {
            PropType::Union(members) => {
                let is_literal = |m: &PropType| is_option(m) || m.literal_kind() == Some(LiteralKind::Boolean);
                members.iter().any(is_option) && members.iter().all(is_literal) && !self.reads_as_json_options()
            }
            PropType::LiteralUnion { members, .. } => !members.is_empty(),
            single => is_option(single),
        }
    }

    /// True for a union with a boolean member whose type name Storybook reads whole: it splits the name at each `|` and
    /// parses every part as JSON, which gives `true` as a real boolean. That holds only while every member's source
    /// text is one JSON value that parses back to the member and holds no `|`. A string with a `|`, a `"` or a `\`
    /// does not, and the union is then an `enum` so that no option is lost. An enum member's name is not JSON either.
    fn reads_as_json_options(&self) -> bool {
        let PropType::Union(members) = self else { return false };
        members.iter().any(|m| m.literal_kind() == Some(LiteralKind::Boolean))
            && members.iter().all(|m| {
                // An enum member's text is its name, not a value.
                let named = matches!(m, PropType::EnumMember { .. });
                match m.literal_kind() {
                    Some(LiteralKind::String) => {
                        let raw = m.raw_string();
                        !named
                            && !raw.contains('|')
                            && serde_json::from_str::<std::string::String>(&raw)
                                .is_ok_and(|parsed| m == &PropType::StringLiteral(parsed))
                    }
                    Some(LiteralKind::Number | LiteralKind::Boolean) => !named,
                    Some(LiteralKind::BigInt) | None => false,
                }
            })
    }

    /// Raw type string for display (e.g. in RDT PropItemType.raw)
    pub fn raw_string(&self) -> std::string::String {
        match self {
            PropType::String => "string".into(),
            PropType::Number => "number".into(),
            PropType::Boolean => "boolean".into(),
            PropType::Null => "null".into(),
            PropType::Undefined => "undefined".into(),
            PropType::Any => "any".into(),
            PropType::Never => "never".into(),
            PropType::Unknown => "unknown".into(),
            PropType::Void => "void".into(),
            PropType::BigInt => "bigint".into(),
            PropType::StringLiteral(s) => format!("\"{}\"", s),
            PropType::NumberLiteral(n) => n.to_string(),
            PropType::BoolLiteral(b) => b.to_string(),
            PropType::BigIntLiteral(value) => format!("{}n", value.as_str()),
            PropType::UniqueSymbol { name } => format!("typeof {name}"),
            PropType::EnumMember { enum_name, member, .. } => format!("{enum_name}.{member}"),
            PropType::TemplateLiteral(parts) => {
                let body: std::string::String = parts
                    .iter()
                    .map(|part| match part {
                        TemplatePart::Text(text) => text.clone(),
                        TemplatePart::Type(ty) => format!("${{{}}}", ty.raw_string()),
                    })
                    .collect();
                format!("`{body}`")
            }
            PropType::Union(members) => {
                members.iter().map(|m| m.operand_string(Precedence::Union)).collect::<Vec<_>>().join(" | ")
            }
            PropType::Intersection(members) => {
                members.iter().map(|m| m.operand_string(Precedence::Intersection)).collect::<Vec<_>>().join(" & ")
            }
            PropType::Array(inner) => format!("{}[]", inner.operand_string(Precedence::Array)),
            PropType::Readonly(inner) => format!("readonly {}", inner.operand_string(Precedence::Array)),
            PropType::Tuple(elements) => {
                format!("[{}]", elements.iter().map(TupleElement::source).collect::<Vec<_>>().join(", "))
            }
            PropType::Object(fields) => {
                format!("{{ {} }}", fields.iter().map(ObjectField::source).collect::<Vec<_>>().join("; "))
            }
            PropType::Callable { signatures, fields } => {
                let members =
                    signatures.iter().map(CallSignature::source).chain(fields.iter().map(ObjectField::source));
                format!("{{ {} }}", members.collect::<Vec<_>>().join("; "))
            }
            PropType::LiteralUnion { members, .. } => {
                members.iter().map(|m| format!(r#""{}""#, m)).collect::<Vec<_>>().join(" | ")
            }
            PropType::Named { name, args } | PropType::Recursive { name, args } if args.is_empty() => name.to_string(),
            PropType::Named { name, args } | PropType::Recursive { name, args } => {
                let args_str = args.iter().map(|a| a.raw_string()).collect::<Vec<_>>().join(", ");
                format!("{}<{}>", name, args_str)
            }
            PropType::Truncated { text, .. } => text.clone(),
            PropType::ReactNode => "ReactNode".into(),
            PropType::CssProperties => "CSSProperties".into(),
            PropType::Function { type_params, params, return_type } => {
                format!("{} => {}", signature_head(type_params, params), return_type.raw_string())
            }
            PropType::Ref { name, element: Some(e) } => format!("{name}<{e}>"),
            PropType::Ref { name, element: None } => format!("{name}<unknown>"),
            PropType::ElementType => "ElementType".into(),
            PropType::HtmlAttributes { element, .. } => {
                format!("{}HTMLAttributes", element_to_type_name(element))
            }
            PropType::SxProps => "SxProps".into(),
            PropType::Opaque(detail) => detail.raw.clone(),
        }
    }

    /// `raw_string`, parenthesized where TypeScript needs it as an operand of `context`: a function inside a union,
    /// a union inside an intersection, any of them before `[]`.
    fn operand_string(&self, context: Precedence) -> std::string::String {
        let own = match self {
            PropType::Function { .. } => Precedence::Function,
            PropType::Union(_) | PropType::LiteralUnion { .. } => Precedence::Union,
            PropType::Intersection(_) => Precedence::Intersection,
            PropType::String
            | PropType::Number
            | PropType::Boolean
            | PropType::Null
            | PropType::Undefined
            | PropType::Any
            | PropType::Never
            | PropType::Unknown
            | PropType::Void
            | PropType::BigInt
            | PropType::StringLiteral(_)
            | PropType::NumberLiteral(_)
            | PropType::BoolLiteral(_)
            | PropType::BigIntLiteral(_)
            | PropType::UniqueSymbol { .. }
            | PropType::EnumMember { .. }
            | PropType::TemplateLiteral(_)
            | PropType::Array(_)
            | PropType::Tuple(_)
            | PropType::Readonly(_)
            | PropType::Object(_)
            | PropType::Callable { .. }
            | PropType::Named { .. }
            | PropType::Recursive { .. }
            | PropType::Truncated { .. }
            | PropType::ReactNode
            | PropType::CssProperties
            | PropType::Ref { .. }
            | PropType::ElementType
            | PropType::HtmlAttributes { .. }
            | PropType::SxProps
            | PropType::Opaque(_) => Precedence::Array,
        };
        if own < context { format!("({})", self.raw_string()) } else { self.raw_string() }
    }
}

/// How tightly a type operator binds, loosest first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Function,
    Union,
    Intersection,
    Array,
}

/// Streams each variant as a map with `kind` first, then its fields in a fixed order. Building a `serde_json::Value`
/// per node first made serialization the largest cost of a run at scale, and left the key order to whether a
/// dependency happened to enable serde_json's `preserve_order`.
impl serde::Serialize for PropType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;
        match self {
            PropType::String => map.serialize_entry("kind", "string")?,
            PropType::Number => map.serialize_entry("kind", "number")?,
            PropType::Boolean => map.serialize_entry("kind", "boolean")?,
            PropType::Null => map.serialize_entry("kind", "null")?,
            PropType::Undefined => map.serialize_entry("kind", "undefined")?,
            PropType::Any => map.serialize_entry("kind", "any")?,
            PropType::Never => map.serialize_entry("kind", "never")?,
            PropType::Unknown => map.serialize_entry("kind", "unknown")?,
            PropType::Void => map.serialize_entry("kind", "void")?,
            PropType::BigInt => map.serialize_entry("kind", "bigint")?,
            PropType::ReactNode => map.serialize_entry("kind", "reactNode")?,
            PropType::CssProperties => map.serialize_entry("kind", "cssProperties")?,
            PropType::ElementType => map.serialize_entry("kind", "elementType")?,
            PropType::SxProps => map.serialize_entry("kind", "sxProps")?,
            // Newtype/tuple variants get a real field name instead of a positional "0" key.
            PropType::StringLiteral(value) => {
                map.serialize_entry("kind", "stringLiteral")?;
                map.serialize_entry("value", value)?;
            }
            PropType::NumberLiteral(n) => {
                map.serialize_entry("kind", "numberLiteral")?;
                map.serialize_entry("value", &WireNumber(*n))?;
            }
            PropType::BoolLiteral(value) => {
                map.serialize_entry("kind", "boolLiteral")?;
                map.serialize_entry("value", value)?;
            }
            PropType::BigIntLiteral(value) => {
                map.serialize_entry("kind", "bigintLiteral")?;
                map.serialize_entry("value", value.as_str())?;
            }
            PropType::UniqueSymbol { name } => {
                map.serialize_entry("kind", "uniqueSymbol")?;
                map.serialize_entry("name", name.as_str())?;
            }
            PropType::EnumMember { enum_name, member, value } => {
                map.serialize_entry("kind", "enumMember")?;
                map.serialize_entry("enum", enum_name.as_str())?;
                map.serialize_entry("member", member.as_str())?;
                // The value as the literal type it is, so a reader reuses its literal reading.
                let literal = match value {
                    EnumMemberValue::String(s) => PropType::StringLiteral(s.clone()),
                    EnumMemberValue::Number(n) => PropType::NumberLiteral(*n),
                };
                map.serialize_entry("value", &literal)?;
            }
            PropType::TemplateLiteral(parts) => {
                map.serialize_entry("kind", "templateLiteral")?;
                map.serialize_entry("parts", &WireTemplate(parts))?;
            }
            PropType::Union(members) => {
                map.serialize_entry("kind", "union")?;
                map.serialize_entry("members", members)?;
            }
            PropType::Intersection(members) => {
                map.serialize_entry("kind", "intersection")?;
                map.serialize_entry("members", members)?;
            }
            PropType::Array(inner) => {
                map.serialize_entry("kind", "array")?;
                map.serialize_entry("element", inner)?;
            }
            PropType::Tuple(elements) => {
                map.serialize_entry("kind", "tuple")?;
                map.serialize_entry("elements", &WireTuple(elements))?;
            }
            PropType::Readonly(inner) => {
                map.serialize_entry("kind", "readonly")?;
                map.serialize_entry("element", inner)?;
            }
            PropType::Object(fields) => {
                map.serialize_entry("kind", "object")?;
                map.serialize_entry("fields", &WireFields(fields))?;
            }
            PropType::Callable { signatures, fields } => {
                map.serialize_entry("kind", "callable")?;
                map.serialize_entry("signatures", &WireSignatures(signatures))?;
                map.serialize_entry("fields", &WireFields(fields))?;
            }
            // Struct variants: fields merge directly with kind.
            PropType::Named { name, args } => {
                map.serialize_entry("kind", "named")?;
                map.serialize_entry("name", name.as_str())?;
                map.serialize_entry("args", args)?;
            }
            PropType::Recursive { name, args } => {
                map.serialize_entry("kind", "recursive")?;
                map.serialize_entry("name", name.as_str())?;
                map.serialize_entry("args", args)?;
            }
            PropType::Truncated { text, limit } => {
                map.serialize_entry("kind", "truncated")?;
                map.serialize_entry("text", text)?;
                map.serialize_entry("limit", &WireLimit(limit))?;
            }
            PropType::Function { type_params, params, return_type } => {
                map.serialize_entry("kind", "function")?;
                map.serialize_entry("typeParams", type_params)?;
                map.serialize_entry("params", params)?;
                map.serialize_entry("returnType", return_type)?;
            }
            PropType::Ref { name, element } => {
                map.serialize_entry("kind", "ref")?;
                map.serialize_entry("name", name)?;
                map.serialize_entry("element", element)?;
            }
            PropType::HtmlAttributes { element, omitted } => {
                map.serialize_entry("kind", "htmlAttributes")?;
                map.serialize_entry("element", element)?;
                map.serialize_entry("omitted", omitted)?;
            }
            PropType::LiteralUnion { members, has_default } => {
                map.serialize_entry("kind", "literalUnion")?;
                map.serialize_entry("members", members)?;
                map.serialize_entry("hasDefault", has_default)?;
            }
            PropType::Opaque(OpaqueDetail { raw, reason }) => {
                map.serialize_entry("kind", "opaque")?;
                map.serialize_entry("raw", raw)?;
                map.serialize_entry("reason", &WireReason(reason))?;
            }
        }
        map.end()
    }
}

/// A number, with NaN and the infinities as the strings `"NaN"`, `"Infinity"` and `"-Infinity"`: JSON numbers cannot
/// hold them (serde_json would write `null`, which reads back as 0). `read_number` reads it back.
struct WireNumber(f64);

impl serde::Serialize for WireNumber {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let n = self.0;
        if n.is_finite() {
            serializer.serialize_f64(n)
        } else if n.is_nan() {
            serializer.serialize_str("NaN")
        } else if n > 0.0 {
            serializer.serialize_str("Infinity")
        } else {
            serializer.serialize_str("-Infinity")
        }
    }
}

/// A tuple's elements as `{ type }`, plus `label` when it has one and `modifier` (`optional`, `rest`, `variadic`) when
/// it is not a plain required element.
struct WireTuple<'a>(&'a [TupleElement]);

impl serde::Serialize for WireTuple<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};

        struct Element<'a>(&'a TupleElement);
        impl serde::Serialize for Element<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("type", &self.0.element_type)?;
                if let Some(label) = &self.0.label {
                    map.serialize_entry("label", label.as_str())?;
                }
                if let Some(modifier) = self.0.kind.modifier() {
                    map.serialize_entry("modifier", modifier)?;
                }
                map.end()
            }
        }

        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for element in self.0 {
            seq.serialize_element(&Element(element))?;
        }
        seq.end()
    }
}

/// A template literal's parts, each `{ text }` or `{ type }`.
struct WireTemplate<'a>(&'a [TemplatePart]);

impl serde::Serialize for WireTemplate<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};

        struct Part<'a>(&'a TemplatePart);
        impl serde::Serialize for Part<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(Some(1))?;
                match self.0 {
                    TemplatePart::Text(text) => map.serialize_entry("text", text)?,
                    TemplatePart::Type(ty) => map.serialize_entry("type", ty)?,
                }
                map.end()
            }
        }

        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for part in self.0 {
            seq.serialize_element(&Part(part))?;
        }
        seq.end()
    }
}

/// Call and construct signatures as `{ typeParams, params, returnType }`, plus `construct: true` for a construct
/// signature.
struct WireSignatures<'a>(&'a [CallSignature]);

impl serde::Serialize for WireSignatures<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};

        struct Signature<'a>(&'a CallSignature);
        impl serde::Serialize for Signature<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(None)?;
                match self.0.kind {
                    SignatureKind::Call => {}
                    SignatureKind::Construct => map.serialize_entry("construct", &true)?,
                }
                map.serialize_entry("typeParams", &self.0.type_params)?;
                map.serialize_entry("params", &self.0.params)?;
                map.serialize_entry("returnType", &self.0.return_type)?;
                map.end()
            }
        }

        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for signature in self.0 {
            seq.serialize_element(&Signature(signature))?;
        }
        seq.end()
    }
}

/// A truncation's limit as `{ type: "depth" }` or `{ type: "members", total }`.
struct WireLimit<'a>(&'a TruncationLimit);

impl serde::Serialize for WireLimit<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            TruncationLimit::Depth => map.serialize_entry("type", "depth")?,
            TruncationLimit::Members { total } => {
                map.serialize_entry("type", "members")?;
                map.serialize_entry("total", total)?;
            }
        }
        map.end()
    }
}

/// An object type's fields as `{ name, propType, required, description }`, plus `readonly` and `indexKey` when set.
struct WireFields<'a>(&'a [ObjectField]);

impl serde::Serialize for WireFields<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};

        struct Field<'a>(&'a ObjectField);
        impl serde::Serialize for Field<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("name", &self.0.name)?;
                map.serialize_entry("propType", &self.0.prop_type)?;
                map.serialize_entry("required", &self.0.required)?;
                map.serialize_entry("description", &self.0.description)?;
                if self.0.readonly {
                    map.serialize_entry("readonly", &true)?;
                }
                if let Some(key) = &self.0.index_key {
                    map.serialize_entry("indexKey", key)?;
                }
                map.end()
            }
        }

        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for field in self.0 {
            seq.serialize_element(&Field(field))?;
        }
        seq.end()
    }
}

/// An `OpaqueReason` as `{ type, ...fields }`.
struct WireReason<'a>(&'a OpaqueReason);

impl serde::Serialize for WireReason<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            OpaqueReason::ConditionalType => map.serialize_entry("type", "conditionalType")?,
            OpaqueReason::MappedType => map.serialize_entry("type", "mappedType")?,
            OpaqueReason::ModuleAugmentation => map.serialize_entry("type", "moduleAugmentation")?,
            OpaqueReason::RuntimeDependent { function_name } => {
                map.serialize_entry("type", "runtimeDependent")?;
                map.serialize_entry("functionName", function_name)?;
            }
            OpaqueReason::UnresolvableImport { specifier } => {
                map.serialize_entry("type", "unresolvableImport")?;
                map.serialize_entry("specifier", specifier)?;
            }
            OpaqueReason::PandaCodegenMissing => map.serialize_entry("type", "pandaCodegenMissing")?,
            OpaqueReason::DepthExceeded => map.serialize_entry("type", "depthExceeded")?,
            OpaqueReason::IndexedAccess { expression } => {
                map.serialize_entry("type", "indexedAccess")?;
                map.serialize_entry("expression", expression)?;
            }
            OpaqueReason::UnsupportedExpression => map.serialize_entry("type", "unsupportedExpression")?,
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for PropType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = serde_json::Value::deserialize(deserializer)?;
        Self::from_tagged_value(&v).map_err(D::Error::custom)
    }
}

impl PropType {
    fn from_tagged_value(v: &serde_json::Value) -> Result<Self, std::string::String> {
        let kind = v
            .get("kind")
            .and_then(|k| k.as_str())
            .ok_or_else(|| "missing 'kind' field in PropType JSON".to_string())?;
        match kind {
            "string" => Ok(PropType::String),
            "number" => Ok(PropType::Number),
            "boolean" => Ok(PropType::Boolean),
            "null" => Ok(PropType::Null),
            "undefined" => Ok(PropType::Undefined),
            "any" => Ok(PropType::Any),
            "never" => Ok(PropType::Never),
            "unknown" => Ok(PropType::Unknown),
            "void" => Ok(PropType::Void),
            "bigint" => Ok(PropType::BigInt),
            "reactNode" | "react_node" => Ok(PropType::ReactNode),
            "cssProperties" | "css_properties" => Ok(PropType::CssProperties),
            "elementType" | "element_type" => Ok(PropType::ElementType),
            "sxProps" | "sx_props" => Ok(PropType::SxProps),
            "stringLiteral" | "string_literal" => {
                let s = v["value"].as_str().unwrap_or("").to_owned();
                Ok(PropType::StringLiteral(s))
            }
            "numberLiteral" | "number_literal" => Ok(PropType::NumberLiteral(read_number(v.get("value")))),
            "boolLiteral" | "bool_literal" => {
                let b = v["value"].as_bool().unwrap_or(false);
                Ok(PropType::BoolLiteral(b))
            }
            "bigintLiteral" => {
                let text = v["value"].as_str().unwrap_or("");
                let value =
                    BigIntValue::parse(text).ok_or_else(|| format!("'{text}' is not a bigint literal's value"))?;
                Ok(PropType::BigIntLiteral(value))
            }
            "uniqueSymbol" => Ok(PropType::UniqueSymbol { name: v["name"].as_str().unwrap_or("").into() }),
            "enumMember" => {
                let value = match Self::from_tagged_value(&v["value"])? {
                    PropType::StringLiteral(s) => EnumMemberValue::String(s),
                    PropType::NumberLiteral(n) => EnumMemberValue::Number(n),
                    other => {
                        return Err(format!("an enum member's value is a string or number literal, not {other:?}"));
                    }
                };
                Ok(PropType::EnumMember {
                    enum_name: v["enum"].as_str().unwrap_or("").into(),
                    member: v["member"].as_str().unwrap_or("").into(),
                    value,
                })
            }
            "templateLiteral" => {
                let parts = v["parts"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|part| match (part.get("text").and_then(|t| t.as_str()), part.get("type")) {
                                (Some(text), _) => Ok(TemplatePart::Text(text.to_owned())),
                                (None, Some(ty)) => Ok(TemplatePart::Type(Self::from_tagged_value(ty)?)),
                                (None, None) => Err("a template literal part has neither 'text' nor 'type'".to_owned()),
                            })
                            .collect::<Result<Vec<_>, std::string::String>>()
                    })
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::TemplateLiteral(parts))
            }
            "callable" => {
                let signatures = v["signatures"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|s| {
                                Ok(CallSignature {
                                    kind: if s["construct"].as_bool().unwrap_or(false) {
                                        SignatureKind::Construct
                                    } else {
                                        SignatureKind::Call
                                    },
                                    type_params: read_type_params(&s["typeParams"]),
                                    params: Self::read_params(&s["params"])?,
                                    return_type: Self::from_tagged_value(&s["returnType"])?,
                                })
                            })
                            .collect::<Result<Vec<_>, std::string::String>>()
                    })
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Callable { signatures, fields: Self::read_fields(&v["fields"])? })
            }
            "recursive" => Ok(PropType::Recursive {
                name: v["name"].as_str().unwrap_or("").into(),
                args: Self::read_list(&v["args"])?,
            }),
            "truncated" => {
                let limit = match v["limit"]["type"].as_str() {
                    Some("members") => TruncationLimit::Members {
                        total: v["limit"]["total"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
                    },
                    Some("depth") => TruncationLimit::Depth,
                    other => return Err(format!("unknown truncation limit {other:?}")),
                };
                Ok(PropType::Truncated { text: v["text"].as_str().unwrap_or("").to_owned(), limit })
            }
            "union" => {
                let members = v["members"]
                    .as_array()
                    .map(|a| a.iter().map(Self::from_tagged_value).collect::<Result<Vec<_>, _>>())
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Union(members))
            }
            "intersection" => {
                let members = v["members"]
                    .as_array()
                    .map(|a| a.iter().map(Self::from_tagged_value).collect::<Result<Vec<_>, _>>())
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Intersection(members))
            }
            "array" => {
                let inner = Self::from_tagged_value(&v["element"])?;
                Ok(PropType::Array(Box::new(inner)))
            }
            "tuple" => {
                let elements = v["elements"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|e| {
                                let modifier = e["modifier"].as_str();
                                let kind = TupleElementKind::from_modifier(modifier)
                                    .ok_or_else(|| format!("unknown tuple element modifier {modifier:?}"))?;
                                let label = match e["label"].as_str() {
                                    Some(label) => Some(
                                        TupleLabel::new(label)
                                            .ok_or_else(|| format!("'{label}' is not a tuple element label"))?,
                                    ),
                                    None => None,
                                };
                                Ok(TupleElement { element_type: Self::from_tagged_value(&e["type"])?, kind, label })
                            })
                            .collect::<Result<Vec<_>, std::string::String>>()
                    })
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Tuple(elements))
            }
            "readonly" => {
                let inner = Self::from_tagged_value(&v["element"])?;
                Ok(PropType::Readonly(Box::new(inner)))
            }
            "object" => Ok(PropType::Object(Self::read_fields(&v["fields"])?)),
            "named" => {
                let name = v["name"].as_str().unwrap_or("").into();
                Ok(PropType::Named { name, args: Self::read_list(&v["args"])? })
            }
            "function" => {
                let type_params = read_type_params(&v["typeParams"]);
                let params = Self::read_params(&v["params"])?;
                let return_type = Box::new(Self::from_tagged_value(&v["returnType"])?);
                Ok(PropType::Function { type_params, params, return_type })
            }
            "ref" => {
                let name = v["name"].as_str().unwrap_or("Ref").to_owned();
                let element = v["element"].as_str().map(|s| s.to_owned());
                Ok(PropType::Ref { name, element })
            }
            "htmlAttributes" | "html_attributes" => {
                let element = v["element"].as_str().unwrap_or("div").to_owned();
                let omitted = v["omitted"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.to_owned()).collect())
                    .unwrap_or_default();
                Ok(PropType::HtmlAttributes { element, omitted })
            }
            "literalUnion" | "literal_union" => {
                let members = v["members"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.to_owned()).collect())
                    .unwrap_or_default();
                let has_default = v["hasDefault"].as_bool().or_else(|| v["has_default"].as_bool()).unwrap_or(false);
                Ok(PropType::LiteralUnion { members, has_default })
            }
            "opaque" => {
                let raw = v["raw"].as_str().unwrap_or("").to_owned();
                let reason = match v["reason"]["type"].as_str().unwrap_or("depthExceeded") {
                    "conditionalType" => OpaqueReason::ConditionalType,
                    "mappedType" => OpaqueReason::MappedType,
                    "moduleAugmentation" => OpaqueReason::ModuleAugmentation,
                    "runtimeDependent" => OpaqueReason::RuntimeDependent {
                        function_name: v["reason"]["functionName"].as_str().unwrap_or("").to_owned(),
                    },
                    "unresolvableImport" => OpaqueReason::UnresolvableImport {
                        specifier: v["reason"]["specifier"].as_str().unwrap_or("").to_owned(),
                    },
                    "pandaCodegenMissing" => OpaqueReason::PandaCodegenMissing,
                    "indexedAccess" => OpaqueReason::IndexedAccess {
                        expression: v["reason"]["expression"].as_str().unwrap_or("").to_owned(),
                    },
                    "unsupportedExpression" => OpaqueReason::UnsupportedExpression,
                    _ => OpaqueReason::DepthExceeded,
                };
                Ok(OpaqueDetail::new(raw, reason))
            }
            other => Ok(OpaqueDetail::new(format!("unknown PropType kind: {}", other), OpaqueReason::DepthExceeded)),
        }
    }

    /// A JSON array of types; a missing one is empty.
    fn read_list(v: &serde_json::Value) -> Result<Vec<PropType>, std::string::String> {
        v.as_array().map(|a| a.iter().map(Self::from_tagged_value).collect()).unwrap_or(Ok(vec![]))
    }

    /// An object type's fields, as `WireFields` writes them.
    fn read_fields(v: &serde_json::Value) -> Result<Vec<ObjectField>, std::string::String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .map(|f| {
                        Ok(ObjectField {
                            name: f["name"].as_str().unwrap_or("").to_owned(),
                            prop_type: Self::from_tagged_value(&f["propType"])?,
                            required: f["required"].as_bool().unwrap_or(false),
                            description: f["description"].as_str().unwrap_or("").to_owned(),
                            readonly: f["readonly"].as_bool().unwrap_or(false),
                            index_key: match f.get("indexKey") {
                                Some(key) => Some(Box::new(Self::from_tagged_value(key)?)),
                                None => None,
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or(Ok(vec![]))
    }

    /// A signature's parameters.
    fn read_params(v: &serde_json::Value) -> Result<Vec<FunctionParam>, std::string::String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .map(|p| {
                        Ok(FunctionParam {
                            name: p["name"].as_str().map(str::to_owned),
                            param_type: Self::from_tagged_value(&p["type"])?,
                            optional: p["optional"].as_bool().unwrap_or(false),
                            rest: p["rest"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or(Ok(vec![]))
    }
}

/// A signature's type parameter names.
fn read_type_params(v: &serde_json::Value) -> Vec<std::string::String> {
    v.as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// A number as `WireNumber` writes it; a missing or unreadable one is 0.
fn read_number(v: Option<&serde_json::Value>) -> f64 {
    match v {
        Some(val) if val.is_string() => match val.as_str().unwrap_or("") {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => 0.0,
        },
        Some(val) => val.as_f64().unwrap_or(0.0),
        None => 0.0,
    }
}

/// Feed `hasher` an object type's fields.
fn hash_fields<H: std::hash::Hasher>(fields: &[ObjectField], hasher: &mut H) {
    use std::hash::Hash;
    fields.len().hash(hasher);
    for field in fields {
        field.name.hash(hasher);
        field.required.hash(hasher);
        field.readonly.hash(hasher);
        if let Some(key) = &field.index_key {
            key.structural_hash(hasher);
        }
        field.description.hash(hasher);
        field.prop_type.structural_hash(hasher);
    }
}

/// Feed `hasher` a signature: its type parameters, parameters and return type.
fn hash_signature<H: std::hash::Hasher>(
    type_params: &[std::string::String],
    params: &[FunctionParam],
    return_type: &PropType,
    hasher: &mut H,
) {
    use std::hash::Hash;
    type_params.hash(hasher);
    params.len().hash(hasher);
    for param in params {
        param.name.hash(hasher);
        param.optional.hash(hasher);
        param.rest.hash(hasher);
        param.param_type.structural_hash(hasher);
    }
    return_type.structural_hash(hasher);
}

/// `visit` over an object type's fields: an index signature's key, then each field's type.
fn visit_fields(fields: &[ObjectField], f: &mut dyn FnMut(&PropType)) {
    for field in fields {
        if let Some(key) = &field.index_key {
            key.visit(f);
        }
        field.prop_type.visit(f);
    }
}

/// A signature's `<T>(a: A, b?: B)`, as TypeScript prints it before `=>` or `:`.
fn signature_head(type_params: &[std::string::String], params: &[FunctionParam]) -> std::string::String {
    let type_params =
        if type_params.is_empty() { std::string::String::new() } else { format!("<{}>", type_params.join(", ")) };
    let params = params.iter().enumerate().map(|(i, p)| p.signature(i)).collect::<Vec<_>>().join(", ");
    format!("{type_params}({params})")
}

fn element_to_type_name(element: &str) -> &'static str {
    match element {
        "button" => "Button",
        "input" => "Input",
        "textarea" => "Textarea",
        "select" => "Select",
        "a" => "Anchor",
        "form" => "Form",
        "label" => "Label",
        "img" => "Img",
        _ => "HTML",
    }
}

/// Whether `name` can appear bare as an object-type field key in TypeScript
/// source (`{ name: T }`) or must be quoted (`{ 'name': T }`) — e.g. CSS custom
/// property names (`--accent`) and dashed HTML attributes (`data-testid`).
fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    (first.is_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Why a type could not be resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpaqueReason {
    /// TypeScript conditional type — needs type checker
    ConditionalType,
    /// TypeScript mapped type — needs type checker
    MappedType,
    /// Module augmentation result — needs type checker
    ModuleAugmentation,
    /// Runtime-dependent: cva(), tv(), etc. where args not statically visible
    RuntimeDependent {
        /// The function whose call builds the type: `cva`, `tv`.
        function_name: std::string::String,
    },
    /// Import could not be resolved to a file
    UnresolvableImport {
        /// The module specifier as written in the import: `@acme/ui`, `./types`.
        specifier: std::string::String,
    },
    /// PandaCSS styled-system not generated yet
    PandaCodegenMissing,
    /// Maximum resolution depth exceeded (circular or too deep)
    DepthExceeded,
    /// Indexed access type (Type["key"]) the static resolver could not follow.
    IndexedAccess {
        /// The indexed access type as written: `Props["variant"]`.
        expression: std::string::String,
    },
    /// A raw type expression the extractor couldn't parse into any recognized
    /// structural shape (not a depth or circularity problem — see
    /// DepthExceeded for that).
    UnsupportedExpression,
}

impl OpaqueReason {
    /// Every reason's name in the canonical JSON, in declaration order, so a consumer such as the `schema` command can
    /// list them without a copy of its own. A test keeps it equal to [`OpaqueReason::kind`] over every variant.
    pub const KINDS: [&'static str; 9] = [
        "conditionalType",
        "mappedType",
        "moduleAugmentation",
        "runtimeDependent",
        "unresolvableImport",
        "pandaCodegenMissing",
        "depthExceeded",
        "indexedAccess",
        "unsupportedExpression",
    ];

    /// The reason's name in the canonical JSON (`"mappedType"`), without its payload.
    pub fn kind(&self) -> &'static str {
        match self {
            OpaqueReason::ConditionalType => "conditionalType",
            OpaqueReason::MappedType => "mappedType",
            OpaqueReason::ModuleAugmentation => "moduleAugmentation",
            OpaqueReason::RuntimeDependent { .. } => "runtimeDependent",
            OpaqueReason::UnresolvableImport { .. } => "unresolvableImport",
            OpaqueReason::PandaCodegenMissing => "pandaCodegenMissing",
            OpaqueReason::DepthExceeded => "depthExceeded",
            OpaqueReason::IndexedAccess { .. } => "indexedAccess",
            OpaqueReason::UnsupportedExpression => "unsupportedExpression",
        }
    }
}

impl PropType {
    /// The kinds of reason behind the `opaque` members of this type, and `truncated` for a member cut at a limit, each
    /// once, in the order they occur (an index signature's key before its value): what the resolver could not
    /// evaluate or did not expand. Empty when nothing in the type is `opaque` or `truncated`. A name left as an
    /// unexpanded reference is not listed, nor is a `recursive` reference; a name's diagnostic says so.
    pub fn degraded_by(&self) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        self.visit(&mut |ty| {
            let kind = if let PropType::Opaque(detail) = ty {
                detail.reason().kind()
            } else if let PropType::Truncated { .. } = ty {
                TRUNCATED_KIND
            } else {
                return;
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        });
        kinds
    }
}

/// The `degradedBy` entry of a [`PropType::Truncated`] member.
pub(crate) const TRUNCATED_KIND: &str = "truncated";

/// One parameter of a [`PropType::Function`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionParam {
    /// `None` for a destructured parameter, which has no single name.
    pub name: Option<std::string::String>,
    /// The parameter's type; serialized as `type`.
    #[serde(rename = "type")]
    pub param_type: PropType,
    /// The parameter is written `name?: T`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// The parameter is the rest parameter `...name: T[]`; its `type` is the array type as written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rest: bool,
}

impl FunctionParam {
    /// A required parameter.
    pub fn new(name: Option<std::string::String>, param_type: PropType) -> Self {
        Self { name, param_type, optional: false, rest: false }
    }

    /// The parameter as TypeScript prints it, with its `...` and `?`: `...rest: number[]`, `a?: string`.
    fn signature(&self, index: usize) -> std::string::String {
        let (prefix, marker) = (if self.rest { "..." } else { "" }, if self.optional { "?" } else { "" });
        match &self.name {
            Some(name) => format!("{prefix}{name}{marker}: {}", self.param_type.raw_string()),
            // A destructured parameter has no name; TypeScript prints it as `__0`, `__1`, ...
            None => format!("{prefix}__{index}{marker}: {}", self.param_type.raw_string()),
        }
    }
}

/// The primitive a literal type is a value of (see [`PropType::literal_kind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralKind {
    String,
    Number,
    Boolean,
    BigInt,
}

/// One element of a [`PropType::Tuple`].
#[derive(Debug, Clone, PartialEq)]
pub struct TupleElement {
    /// The element's type. For a rest element, the type of each element it collects: `number` in `...number[]`, as
    /// TypeScript's checker states it; for a variadic one, the spread type as written (`T` in `...T`).
    pub element_type: PropType,
    /// Whether it is required, optional, a rest element or a variadic spread.
    pub kind: TupleElementKind,
    /// The element's label, `head` in `[head: string]`.
    pub label: Option<TupleLabel>,
}

impl TupleElement {
    /// A required, unlabeled element.
    pub fn required(element_type: PropType) -> Self {
        Self { element_type, kind: TupleElementKind::Required, label: None }
    }

    /// The element as TypeScript prints it: `string`, `string?`, `...number[]`, `...T`, `head: string`,
    /// `tail?: number`, `...rest: number[]`.
    fn source(&self) -> std::string::String {
        // Without a label, `?` and `...` bind to the type itself: `(string | number)?`, `...(A | B)`.
        let operand = || match self.label {
            Some(_) => self.element_type.raw_string(),
            None => self.element_type.operand_string(Precedence::Array),
        };
        let ty = match self.kind {
            TupleElementKind::Rest => format!("{}[]", self.element_type.operand_string(Precedence::Array)),
            TupleElementKind::Optional | TupleElementKind::Variadic => operand(),
            TupleElementKind::Required => self.element_type.raw_string(),
        };
        let spread = match self.kind {
            TupleElementKind::Rest | TupleElementKind::Variadic => "...",
            TupleElementKind::Required | TupleElementKind::Optional => "",
        };
        let optional = match self.kind {
            TupleElementKind::Optional => "?",
            TupleElementKind::Required | TupleElementKind::Rest | TupleElementKind::Variadic => "",
        };
        match &self.label {
            Some(label) => format!("{spread}{}{optional}: {ty}", label.as_str()),
            None => format!("{spread}{ty}{optional}"),
        }
    }
}

/// How a tuple element is declared, TypeScript's element flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TupleElementKind {
    /// `T`: always present.
    #[default]
    Required,
    /// `T?`: may be left out.
    Optional,
    /// `...T[]`: any number of elements of type `T`.
    Rest,
    /// `...T` where `T` is not a known array or tuple (a type parameter, an unresolved name): its elements are spread
    /// in, whatever they turn out to be.
    Variadic,
}

impl TupleElementKind {
    /// The `modifier` the canonical JSON writes for it; `None` for a required element, which writes none.
    fn modifier(self) -> Option<&'static str> {
        match self {
            Self::Required => None,
            Self::Optional => Some("optional"),
            Self::Rest => Some("rest"),
            Self::Variadic => Some("variadic"),
        }
    }

    /// The kind a `modifier` names; `None` for an unknown one.
    fn from_modifier(modifier: Option<&str>) -> Option<Self> {
        match modifier {
            None => Some(Self::Required),
            Some("optional") => Some(Self::Optional),
            Some("rest") => Some(Self::Rest),
            Some("variadic") => Some(Self::Variadic),
            Some(_) => None,
        }
    }
}

/// A tuple element's label: an identifier, as TypeScript requires (`[head: string]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TupleLabel(TypeName);

impl TupleLabel {
    /// `label` as a tuple label, or `None` when it is not an identifier.
    pub fn new(label: &str) -> Option<Self> {
        is_valid_identifier(label).then(|| Self(label.into()))
    }

    /// The label's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A bigint literal's value in base 10, with a leading `-` when negative and no `n` suffix: `10`, `-1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BigIntValue(std::string::String);

impl BigIntValue {
    /// `text` as a bigint value: decimal digits, optionally after a `-`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('-').unwrap_or(text);
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| Self(text.to_owned()))
    }

    /// The value's decimal text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The value of a [`PropType::EnumMember`].
#[derive(Debug, Clone, PartialEq)]
pub enum EnumMemberValue {
    /// A string member's value.
    String(std::string::String),
    /// A numeric member's value.
    Number(f64),
}

/// One piece of a [`PropType::TemplateLiteral`].
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    /// Text between holes, as written in the source (escapes included).
    Text(std::string::String),
    /// The type in a `${…}` hole.
    Type(PropType),
}

/// One call or construct signature of a [`PropType::Callable`].
#[derive(Debug, Clone, PartialEq)]
pub struct CallSignature {
    /// Whether it is a call signature `(a: A): R` or a construct signature `new (a: A): R`.
    pub kind: SignatureKind,
    /// The signature's own type parameters.
    pub type_params: Vec<std::string::String>,
    /// The parameters in order.
    pub params: Vec<FunctionParam>,
    /// What a call returns, or what `new` creates.
    pub return_type: PropType,
}

impl CallSignature {
    /// The signature as an object type member: `(value: string): string`, `new (value: string): Date`.
    fn source(&self) -> std::string::String {
        let new = match self.kind {
            SignatureKind::Call => "",
            SignatureKind::Construct => "new ",
        };
        format!("{new}{}: {}", signature_head(&self.type_params, &self.params), self.return_type.raw_string())
    }
}

/// Which signature a [`CallSignature`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureKind {
    /// `(a: A): R`: called as a function.
    Call,
    /// `new (a: A): R`: called with `new`.
    Construct,
}

/// The limit a [`PropType::Truncated`] type was cut at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruncationLimit {
    /// The type is nested deeper than the converter expands.
    Depth,
    /// The type has more members than the converter lists.
    Members {
        /// How many members it has.
        total: u32,
    },
}

/// A field in an object type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectField {
    /// The field's name; for an index signature, the key parameter's (`key` in `[key: string]: T`).
    pub name: std::string::String,
    /// The field's type; for an index signature, the value type.
    pub prop_type: PropType,
    /// The field is not written `name?: T`.
    pub required: bool,
    /// The field's JSDoc description, empty when it has none.
    pub description: std::string::String,
    /// The field is written `readonly name: T`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub readonly: bool,
    /// Set for an index signature `[name: K]: T`: the key type `K`. The field's `prop_type` is the value type `T`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_key: Option<Box<PropType>>,
}

impl ObjectField {
    /// A named member, as opposed to an index signature.
    pub(crate) fn is_member(&self) -> bool {
        self.index_key.is_none()
    }

    /// The field as an object type member: `readonly name?: T`, `[key: K]: T`.
    fn source(&self) -> std::string::String {
        let readonly = if self.readonly { "readonly " } else { "" };
        if let Some(key) = &self.index_key {
            return format!("{readonly}[{}: {}]: {}", self.name, key.raw_string(), self.prop_type.raw_string());
        }
        let name = if is_valid_identifier(&self.name) { self.name.clone() } else { format!("'{}'", self.name) };
        let optional = if self.required { "" } else { "?" };
        format!("{readonly}{name}{optional}: {}", self.prop_type.raw_string())
    }

    /// A named field that isn't `readonly`.
    pub fn new(
        name: impl Into<std::string::String>,
        prop_type: PropType,
        required: bool,
        description: impl Into<std::string::String>,
    ) -> Self {
        Self {
            name: name.into(),
            prop_type,
            required,
            description: description.into(),
            readonly: false,
            index_key: None,
        }
    }
}

// ─── Statistics ────────────────────────────────────────────────────────────────

/// Counters for one extraction run, written alongside the components.
///
/// Four of them (`components_skipped`, `tier1_count`, `tier3_count`, `opaque_count`) are part of the output shape but
/// the pipeline does not set them: they are always `0` in a real run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionStats {
    /// How many components the output holds.
    pub components_extracted: u32,
    /// Not set by the pipeline: always `0`.
    pub components_skipped: u32,
    /// How many source files were parsed.
    pub files_parsed: u32,
    /// How many `.d.ts` files went through the parse cache, hits and misses together; a miss is parsed.
    pub dts_files_parsed: u32,
    /// How many of those the cache answered without parsing.
    pub dts_cache_hits: u32,
    /// How long the run took, in milliseconds. It differs between two runs over the same input, so compare outputs
    /// without it.
    pub duration_ms: u64,
    /// Not set by the pipeline: always `0`.
    pub tier1_count: u32,
    /// Not set by the pipeline: always `0`.
    pub tier3_count: u32,
    /// Not set by the pipeline: always `0`; the number of opaque types is the number of [`PropType::Opaque`] values.
    pub opaque_count: u32,
}

#[cfg(test)]
mod tests;
