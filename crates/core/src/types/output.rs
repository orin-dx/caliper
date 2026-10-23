//! Semantic output types produced by the resolver (Phase 3).

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::collected::{EnumEntry, TypeName};
use super::diagnostic::{Diagnostic, DiagnosticSeverity};
use super::global::ResolveState;

pub(crate) mod interned;

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
    /// Resolved props, keyed by prop name
    pub props: BTreeMap<String, ParsedProp>,
    /// Full resolved inheritance chain, outermost first.
    pub inheritance: Vec<InheritedLayer>,
    /// Curated notable props from inherited layers (not in self.props).
    pub notable_inherited: BTreeMap<String, ParsedProp>,
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
        ParsedProp { name, prop_type, required, default_value, description, tags, parent, declarations, _seal: Seal }
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
    String,
    Number,
    Boolean,
    Null,
    Undefined,
    Any,
    Never,
    Unknown,
    Void,

    // ── Literals
    StringLiteral(std::string::String),
    NumberLiteral(f64),
    BoolLiteral(bool),

    // ── Composites
    Union(Vec<PropType>),
    Intersection(Vec<PropType>),
    Array(Box<PropType>),
    Tuple(Vec<PropType>),
    Object(Vec<ObjectField>),

    // ── Named reference (post-resolution)
    Named {
        name: TypeName,
        /// Generic args, already resolved to PropType
        args: Vec<PropType>,
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
        params: Vec<FunctionParam>,
        return_type: Box<PropType>,
    },
    /// `Ref<T>` / `RefObject<T>` / `ForwardedRef<T>`
    Ref {
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
        element: std::string::String,
        omitted: Vec<std::string::String>,
    },

    // ── Variant systems (statically resolved)
    /// Result of CvaVariantPropsHandler — the wrapper is fully dissolved.
    /// Each member is a string literal value.
    LiteralUnion {
        members: Vec<std::string::String>,
        /// true if this prop has a defaultVariant
        has_default: bool,
    },

    // ── Known opaque patterns (cannot/should not be expanded)
    /// MUI SxProps, SystemStyleObject etc. — complex, not user-facing props
    SxProps,

    // ── Unresolvable — graceful degradation
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
    /// Whether a conditional or mapped type the static resolver can't evaluate appears anywhere in this type, so its
    /// text would include that type's source.
    pub(crate) fn holds_unevaluated_type(&self) -> bool {
        match self {
            PropType::Opaque(detail) => {
                matches!(detail.reason(), OpaqueReason::ConditionalType | OpaqueReason::MappedType)
            }
            PropType::Union(members) | PropType::Intersection(members) | PropType::Tuple(members) => {
                members.iter().any(PropType::holds_unevaluated_type)
            }
            PropType::Array(inner) => inner.holds_unevaluated_type(),
            PropType::Object(fields) => fields.iter().any(|f| f.prop_type.holds_unevaluated_type()),
            PropType::Named { args, .. } => args.iter().any(PropType::holds_unevaluated_type),
            PropType::Function { params, return_type, .. } => {
                params.iter().any(|p| p.param_type.holds_unevaluated_type()) || return_type.holds_unevaluated_type()
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
            | PropType::StringLiteral(_)
            | PropType::NumberLiteral(_)
            | PropType::BoolLiteral(_)
            | PropType::ReactNode
            | PropType::CssProperties
            | PropType::Ref { .. }
            | PropType::ElementType
            | PropType::HtmlAttributes { .. }
            | PropType::LiteralUnion { .. }
            | PropType::SxProps => false,
        }
    }

    /// True if this type is a pure literal union (all members are literals).
    /// Used by serializers to choose between "enum" and "union" in RDT output.
    /// Requires at least 2 members — a 0- or 1-member "union" isn't a
    /// meaningful `<select>` shape, so RDT output falls back to plain
    /// `raw_string()` for those instead of an empty/single-option enum.
    pub fn is_literal_union(&self) -> bool {
        match self {
            PropType::Union(members) => {
                members.len() >= 2
                    && members.iter().all(|m| {
                        matches!(m, PropType::StringLiteral(_) | PropType::NumberLiteral(_) | PropType::BoolLiteral(_))
                    })
            }
            PropType::LiteralUnion { members, .. } => members.len() >= 2,
            _ => false,
        }
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
            PropType::StringLiteral(s) => format!("\"{}\"", s),
            PropType::NumberLiteral(n) => n.to_string(),
            PropType::BoolLiteral(b) => b.to_string(),
            PropType::Union(members) => {
                members.iter().map(|m| m.operand_string(Precedence::Union)).collect::<Vec<_>>().join(" | ")
            }
            PropType::Intersection(members) => {
                members.iter().map(|m| m.operand_string(Precedence::Intersection)).collect::<Vec<_>>().join(" & ")
            }
            PropType::Array(inner) => format!("{}[]", inner.operand_string(Precedence::Array)),
            PropType::Tuple(members) => {
                format!("[{}]", members.iter().map(|m| m.raw_string()).collect::<Vec<_>>().join(", "))
            }
            PropType::Object(fields) => {
                let fields_str = fields
                    .iter()
                    .map(|f| {
                        let name = if is_valid_identifier(&f.name) { f.name.clone() } else { format!("'{}'", f.name) };
                        let optional = if f.required { "" } else { "?" };
                        format!("{}{}: {}", name, optional, f.prop_type.raw_string())
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("{{ {} }}", fields_str)
            }
            PropType::LiteralUnion { members, .. } => {
                members.iter().map(|m| format!(r#""{}""#, m)).collect::<Vec<_>>().join(" | ")
            }
            PropType::Named { name, args } if args.is_empty() => name.to_string(),
            PropType::Named { name, args } => {
                let args_str = args.iter().map(|a| a.raw_string()).collect::<Vec<_>>().join(", ");
                format!("{}<{}>", name, args_str)
            }
            PropType::ReactNode => "ReactNode".into(),
            PropType::CssProperties => "CSSProperties".into(),
            PropType::Function { type_params, params, return_type } => {
                let type_params = if type_params.is_empty() {
                    std::string::String::new()
                } else {
                    format!("<{}>", type_params.join(", "))
                };
                // A destructured parameter has no name; TypeScript prints it as `__0`, `__1`, ...
                let params = params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| match &p.name {
                        Some(name) => format!("{name}: {}", p.param_type.raw_string()),
                        None => format!("__{i}: {}", p.param_type.raw_string()),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{type_params}({params}) => {}", return_type.raw_string())
            }
            PropType::Ref { element: Some(e) } => format!("Ref<{}>", e),
            PropType::Ref { element: None } => "Ref<unknown>".into(),
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
            _ => Precedence::Array,
        };
        if own < context {
            format!("({})", self.raw_string())
        } else {
            self.raw_string()
        }
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
        let unit_kind: Option<&str> = match self {
            PropType::String => Some("string"),
            PropType::Number => Some("number"),
            PropType::Boolean => Some("boolean"),
            PropType::Null => Some("null"),
            PropType::Undefined => Some("undefined"),
            PropType::Any => Some("any"),
            PropType::Never => Some("never"),
            PropType::Unknown => Some("unknown"),
            PropType::Void => Some("void"),
            PropType::ReactNode => Some("reactNode"),
            PropType::CssProperties => Some("cssProperties"),
            PropType::ElementType => Some("elementType"),
            PropType::SxProps => Some("sxProps"),
            _ => None,
        };
        if let Some(kind) = unit_kind {
            map.serialize_entry("kind", kind)?;
            return map.end();
        }
        match self {
            // Newtype/tuple variants get a real field name instead of a positional "0" key.
            PropType::StringLiteral(value) => {
                map.serialize_entry("kind", "stringLiteral")?;
                map.serialize_entry("value", value)?;
            }
            PropType::NumberLiteral(n) => {
                map.serialize_entry("kind", "numberLiteral")?;
                // JSON numbers cannot hold NaN or Infinity (serde_json would write `null`, which reads back as 0), so
                // those are tagged as strings; see `from_tagged_value`.
                if n.is_finite() {
                    map.serialize_entry("value", n)?;
                } else if n.is_nan() {
                    map.serialize_entry("value", "NaN")?;
                } else if *n > 0.0 {
                    map.serialize_entry("value", "Infinity")?;
                } else {
                    map.serialize_entry("value", "-Infinity")?;
                }
            }
            PropType::BoolLiteral(value) => {
                map.serialize_entry("kind", "boolLiteral")?;
                map.serialize_entry("value", value)?;
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
            PropType::Tuple(members) => {
                map.serialize_entry("kind", "tuple")?;
                map.serialize_entry("elements", members)?;
            }
            PropType::Object(fields) => {
                map.serialize_entry("kind", "object")?;
                map.serialize_entry("fields", &WireFields(fields))?;
            }
            // Struct variants: fields merge directly with kind.
            PropType::Named { name, args } => {
                map.serialize_entry("kind", "named")?;
                map.serialize_entry("name", name.as_str())?;
                map.serialize_entry("args", args)?;
            }
            PropType::Function { type_params, params, return_type } => {
                map.serialize_entry("kind", "function")?;
                map.serialize_entry("typeParams", type_params)?;
                map.serialize_entry("params", params)?;
                map.serialize_entry("returnType", return_type)?;
            }
            PropType::Ref { element } => {
                map.serialize_entry("kind", "ref")?;
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
            _ => {}
        }
        map.end()
    }
}

/// An object type's fields as `{ name, propType, required, description }`.
struct WireFields<'a>(&'a [ObjectField]);

impl serde::Serialize for WireFields<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};

        struct Field<'a>(&'a ObjectField);
        impl serde::Serialize for Field<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("name", &self.0.name)?;
                map.serialize_entry("propType", &self.0.prop_type)?;
                map.serialize_entry("required", &self.0.required)?;
                map.serialize_entry("description", &self.0.description)?;
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
            OpaqueReason::TemplateLiteral { expression } => {
                map.serialize_entry("type", "templateLiteral")?;
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
            "reactNode" | "react_node" => Ok(PropType::ReactNode),
            "cssProperties" | "css_properties" => Ok(PropType::CssProperties),
            "elementType" | "element_type" => Ok(PropType::ElementType),
            "sxProps" | "sx_props" => Ok(PropType::SxProps),
            "stringLiteral" | "string_literal" => {
                let s = v["value"].as_str().unwrap_or("").to_owned();
                Ok(PropType::StringLiteral(s))
            }
            "numberLiteral" | "number_literal" => {
                let n = match v.get("value") {
                    Some(val) if val.is_string() => match val.as_str().unwrap_or("") {
                        "NaN" => f64::NAN,
                        "Infinity" => f64::INFINITY,
                        "-Infinity" => f64::NEG_INFINITY,
                        _ => 0.0,
                    },
                    Some(val) => val.as_f64().unwrap_or(0.0),
                    None => 0.0,
                };
                Ok(PropType::NumberLiteral(n))
            }
            "boolLiteral" | "bool_literal" => {
                let b = v["value"].as_bool().unwrap_or(false);
                Ok(PropType::BoolLiteral(b))
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
                let members = v["elements"]
                    .as_array()
                    .map(|a| a.iter().map(Self::from_tagged_value).collect::<Result<Vec<_>, _>>())
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Tuple(members))
            }
            "object" => {
                let fields = v["fields"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|f| {
                                Ok(ObjectField {
                                    name: f["name"].as_str().unwrap_or("").to_owned(),
                                    prop_type: Self::from_tagged_value(&f["propType"])?,
                                    required: f["required"].as_bool().unwrap_or(false),
                                    description: f["description"].as_str().unwrap_or("").to_owned(),
                                })
                            })
                            .collect::<Result<Vec<_>, std::string::String>>()
                    })
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Object(fields))
            }
            "named" => {
                let name = v["name"].as_str().unwrap_or("").into();
                let args = v["args"]
                    .as_array()
                    .map(|a| a.iter().map(Self::from_tagged_value).collect::<Result<Vec<_>, _>>())
                    .unwrap_or(Ok(vec![]))?;
                Ok(PropType::Named { name, args })
            }
            "function" => {
                let type_params = v["typeParams"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                let params = v["params"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|p| {
                                Ok(FunctionParam {
                                    name: p["name"].as_str().map(str::to_owned),
                                    param_type: Self::from_tagged_value(&p["type"])?,
                                })
                            })
                            .collect::<Result<Vec<_>, std::string::String>>()
                    })
                    .unwrap_or(Ok(vec![]))?;
                let return_type = Box::new(Self::from_tagged_value(&v["returnType"])?);
                Ok(PropType::Function { type_params, params, return_type })
            }
            "ref" => {
                let element = v["element"].as_str().map(|s| s.to_owned());
                Ok(PropType::Ref { element })
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
                    "templateLiteral" => OpaqueReason::TemplateLiteral {
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
        "div" | "span" | "p" | "h1" | "h2" | "h3" => "HTML",
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
    RuntimeDependent { function_name: std::string::String },
    /// Import could not be resolved to a file
    UnresolvableImport { specifier: std::string::String },
    /// PandaCSS styled-system not generated yet
    PandaCodegenMissing,
    /// Maximum resolution depth exceeded (circular or too deep)
    DepthExceeded,
    /// Indexed access type (Type["key"]) the static resolver could not follow.
    IndexedAccess { expression: std::string::String },
    /// Template literal type — partially or fully unresolvable.
    TemplateLiteral { expression: std::string::String },
    /// A raw type expression the extractor couldn't parse into any recognized
    /// structural shape (not a depth or circularity problem — see
    /// DepthExceeded for that).
    UnsupportedExpression,
}

/// One parameter of a [`PropType::Function`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionParam {
    /// `None` for a destructured parameter, which has no single name.
    pub name: Option<std::string::String>,
    #[serde(rename = "type")]
    pub param_type: PropType,
}

/// A field in an object type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectField {
    pub name: std::string::String,
    pub prop_type: PropType,
    pub required: bool,
    pub description: std::string::String,
}

// ─── Statistics ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionStats {
    pub components_extracted: u32,
    pub components_skipped: u32,
    pub files_parsed: u32,
    pub dts_files_parsed: u32,
    pub dts_cache_hits: u32,
    pub duration_ms: u64,
    pub tier1_count: u32,
    pub tier3_count: u32,
    pub opaque_count: u32,
}

#[cfg(test)]
mod tests;
