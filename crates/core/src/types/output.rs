//! Semantic output types produced by the resolver (Phase 3).

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::collected::{EnumEntry, TypeName};
use super::diagnostic::{Diagnostic, DiagnosticSeverity};
use super::global::ResolveState;

// ─── Top-level output ─────────────────────────────────────────────────────────

/// The complete extraction output — top-level return type of the pipeline.
#[must_use]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
fn severity_rank(severity: &DiagnosticSeverity) -> u8 {
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
}

impl ComponentEntry {
    /// Shortcut: the HTML element this component renders as, if any.
    /// Derived from the inheritance chain.
    pub fn html_element(&self) -> Option<&str> {
        self.inheritance.iter().find_map(|l| l.html_element.as_deref())
    }
}

/// A single resolved prop.
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
    /// Event handler: (e: MouseEvent) => void etc.
    EventHandler {
        /// "MouseEvent", "`ChangeEvent<HTMLInputElement>`", etc.
        event_type: std::string::String,
        /// The source parameter's name (e.g. "open" in `(open: boolean) => void`),
        /// if the underlying function type had a simple identifier binding.
        param_name: Option<std::string::String>,
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
            PropType::Union(members) => members.iter().map(|m| m.raw_string()).collect::<Vec<_>>().join(" | "),
            PropType::Intersection(members) => members.iter().map(|m| m.raw_string()).collect::<Vec<_>>().join(" & "),
            PropType::Array(inner) => format!("{}[]", inner.raw_string()),
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
            PropType::EventHandler { event_type, param_name } => {
                format!("({}: {}) => void", param_name.as_deref().unwrap_or("e"), event_type)
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
}

impl serde::Serialize for PropType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_tagged_value().serialize(serializer)
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
    fn to_tagged_value(&self) -> serde_json::Value {
        match self {
            // Unit variants (primitives)
            PropType::String => serde_json::json!({"kind": "string"}),
            PropType::Number => serde_json::json!({"kind": "number"}),
            PropType::Boolean => serde_json::json!({"kind": "boolean"}),
            PropType::Null => serde_json::json!({"kind": "null"}),
            PropType::Undefined => serde_json::json!({"kind": "undefined"}),
            PropType::Any => serde_json::json!({"kind": "any"}),
            PropType::Never => serde_json::json!({"kind": "never"}),
            PropType::Unknown => serde_json::json!({"kind": "unknown"}),
            PropType::Void => serde_json::json!({"kind": "void"}),
            PropType::ReactNode => serde_json::json!({"kind": "reactNode"}),
            PropType::CssProperties => serde_json::json!({"kind": "cssProperties"}),
            PropType::ElementType => serde_json::json!({"kind": "elementType"}),
            PropType::SxProps => serde_json::json!({"kind": "sxProps"}),
            // Newtype/tuple variants — give each a real field name instead of a
            // positional "0" key, matching the struct-style variants below.
            PropType::StringLiteral(s) => serde_json::json!({"kind": "stringLiteral", "value": s}),
            PropType::NumberLiteral(n) => {
                // `serde_json::Number` cannot represent NaN/Infinity (they'd
                // silently become JSON `null`, then round-trip back as `0.0`
                // — see `from_tagged_value` below). Tag non-finite values as
                // strings instead so the read side can tell them apart from a
                // genuinely-absent value.
                let value = if n.is_finite() {
                    serde_json::json!(n)
                } else if n.is_nan() {
                    serde_json::json!("NaN")
                } else if *n > 0.0 {
                    serde_json::json!("Infinity")
                } else {
                    serde_json::json!("-Infinity")
                };
                serde_json::json!({"kind": "numberLiteral", "value": value})
            }
            PropType::BoolLiteral(b) => serde_json::json!({"kind": "boolLiteral", "value": b}),
            PropType::Union(members) => serde_json::json!({
                "kind": "union",
                "members": members.iter().map(|m| m.to_tagged_value()).collect::<Vec<_>>()
            }),
            PropType::Intersection(members) => serde_json::json!({
                "kind": "intersection",
                "members": members.iter().map(|m| m.to_tagged_value()).collect::<Vec<_>>()
            }),
            PropType::Array(inner) => serde_json::json!({
                "kind": "array",
                "element": inner.to_tagged_value()
            }),
            PropType::Tuple(members) => serde_json::json!({
                "kind": "tuple",
                "elements": members.iter().map(|m| m.to_tagged_value()).collect::<Vec<_>>()
            }),
            PropType::Object(fields) => serde_json::json!({
                "kind": "object",
                "fields": fields.iter().map(|f| serde_json::json!({
                    "name": f.name,
                    "propType": f.prop_type.to_tagged_value(),
                    "required": f.required,
                    "description": f.description,
                })).collect::<Vec<_>>()
            }),
            // Struct variants — fields merge directly with kind
            PropType::Named { name, args } => serde_json::json!({
                "kind": "named",
                "name": name.as_str(),
                "args": args.iter().map(|a| a.to_tagged_value()).collect::<Vec<_>>()
            }),
            PropType::EventHandler { event_type, param_name } => serde_json::json!({
                "kind": "eventHandler",
                "eventType": event_type,
                "paramName": param_name
            }),
            PropType::Ref { element } => serde_json::json!({
                "kind": "ref",
                "element": element
            }),
            PropType::HtmlAttributes { element, omitted } => serde_json::json!({
                "kind": "htmlAttributes",
                "element": element,
                "omitted": omitted
            }),
            PropType::LiteralUnion { members, has_default } => serde_json::json!({
                "kind": "literalUnion",
                "members": members,
                "hasDefault": has_default
            }),
            PropType::Opaque(OpaqueDetail { raw, reason }) => {
                let reason_val = match reason {
                    OpaqueReason::ConditionalType => serde_json::json!({"type": "conditionalType"}),
                    OpaqueReason::MappedType => serde_json::json!({"type": "mappedType"}),
                    OpaqueReason::ModuleAugmentation => {
                        serde_json::json!({"type": "moduleAugmentation"})
                    }
                    OpaqueReason::RuntimeDependent { function_name } => {
                        serde_json::json!({"type": "runtimeDependent", "functionName": function_name})
                    }
                    OpaqueReason::UnresolvableImport { specifier } => {
                        serde_json::json!({"type": "unresolvableImport", "specifier": specifier})
                    }
                    OpaqueReason::PandaCodegenMissing => {
                        serde_json::json!({"type": "pandaCodegenMissing"})
                    }
                    OpaqueReason::DepthExceeded => serde_json::json!({"type": "depthExceeded"}),
                    OpaqueReason::IndexedAccess { expression } => {
                        serde_json::json!({"type": "indexedAccess", "expression": expression})
                    }
                    OpaqueReason::TemplateLiteral { expression } => {
                        serde_json::json!({"type": "templateLiteral", "expression": expression})
                    }
                    OpaqueReason::MultiParamFunction => serde_json::json!({"type": "multiParamFunction"}),
                    OpaqueReason::UnsupportedExpression => {
                        serde_json::json!({"type": "unsupportedExpression"})
                    }
                };
                serde_json::json!({"kind": "opaque", "raw": raw, "reason": reason_val})
            }
        }
    }

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
            "eventHandler" | "event_handler" => {
                let event_type = v["eventType"].as_str().or_else(|| v["event_type"].as_str()).unwrap_or("").to_owned();
                let param_name = v["paramName"].as_str().or_else(|| v["param_name"].as_str()).map(|s| s.to_owned());
                Ok(PropType::EventHandler { event_type, param_name })
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
                    "multiParamFunction" => OpaqueReason::MultiParamFunction,
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
    /// Indexed access type (Type["key"]) — enable typescript-go to resolve.
    IndexedAccess { expression: std::string::String },
    /// Template literal type — partially or fully unresolvable.
    TemplateLiteral { expression: std::string::String },
    /// A function type with more than one parameter — describing it fully
    /// needs a real type signature, not just an EventHandler-shaped summary.
    MultiParamFunction,
    /// A raw type expression the extractor couldn't parse into any recognized
    /// structural shape (not a depth or circularity problem — see
    /// DepthExceeded for that).
    UnsupportedExpression,
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
mod tests {
    use super::*;

    #[test]
    fn tuple_raw_string_renders_real_elements_not_a_placeholder() {
        let ty = PropType::Tuple(vec![PropType::Number, PropType::Number]);
        assert_eq!(ty.raw_string(), "[number, number]");
    }

    #[test]
    fn object_raw_string_renders_real_fields_not_a_placeholder() {
        // Matches the real rdt-compat/types.tsx `tokenStyle` fixture shape:
        // `CSSProperties & { '--accent': string }` — the object side must render
        // its real field, not the literal placeholder "object".
        let ty = PropType::Object(vec![ObjectField {
            name: "--accent".into(),
            prop_type: PropType::String,
            required: true,
            description: String::new(),
        }]);
        assert_eq!(ty.raw_string(), "{ '--accent': string }");
    }

    #[test]
    fn object_raw_string_quotes_non_identifier_keys_only() {
        let ty = PropType::Object(vec![
            ObjectField {
                name: "label".into(),
                prop_type: PropType::String,
                required: true,
                description: String::new(),
            },
            ObjectField {
                name: "data-testid".into(),
                prop_type: PropType::String,
                required: false,
                description: String::new(),
            },
        ]);
        assert_eq!(ty.raw_string(), "{ label: string; 'data-testid'?: string }");
    }

    #[test]
    fn exit_code_is_zero_with_no_diagnostics() {
        let output = ExtractionOutput {
            components: BTreeMap::new(),
            enums: BTreeMap::new(),
            diagnostics: vec![],
            stats: ExtractionStats::default(),
        };
        assert_eq!(output.exit_code(false), 0);
        assert_eq!(output.exit_code(true), 0);
    }

    #[test]
    fn exit_code_is_two_when_any_diagnostic_is_error_severity() {
        let output = ExtractionOutput {
            components: BTreeMap::new(),
            enums: BTreeMap::new(),
            diagnostics: vec![crate::types::diagnostic::Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: "boom".into(),
                file: None,
                line: None,
                column: None,
                help: None,
                code: crate::types::diagnostic::DiagnosticCode::Unknown,
            }],
            stats: ExtractionStats::default(),
        };
        assert_eq!(output.exit_code(false), 2);
        assert_eq!(output.exit_code(true), 2);
    }

    #[test]
    fn exit_code_is_one_only_when_strict_and_a_warning_is_present() {
        let output = ExtractionOutput {
            components: BTreeMap::new(),
            enums: BTreeMap::new(),
            diagnostics: vec![crate::types::diagnostic::Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "heads up".into(),
                file: None,
                line: None,
                column: None,
                help: None,
                code: crate::types::diagnostic::DiagnosticCode::Unknown,
            }],
            stats: ExtractionStats::default(),
        };
        assert_eq!(output.exit_code(false), 0, "non-strict must not fail on warnings");
        assert_eq!(output.exit_code(true), 1, "strict must fail on warnings");
    }
}

#[cfg(test)]
mod opaque_detail_tests {
    use super::*;
    use crate::types::diagnostic::DiagnosticCode;
    use crate::types::diagnostic::DiagnosticSeverity;
    use crate::types::global::ResolveState;

    #[test]
    fn give_up_pushes_the_diagnostic_and_builds_the_opaque_payload() {
        let mut state = ResolveState::default();
        let diagnostic = Diagnostic {
            severity: DiagnosticSeverity::Info,
            message: "gave up".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::OpaqueType,
        };

        let pt = OpaqueDetail::give_up(&mut state, "SomeType", OpaqueReason::DepthExceeded, diagnostic.clone());

        assert_eq!(state.diagnostics.len(), 1);
        // Full equality, not just .message: the criterion's own stated test
        // contract is "the pushed diagnostic equals what was passed in" —
        // Diagnostic derives PartialEq+Clone, so this costs nothing extra and
        // catches give_up silently mutating severity/code/file/line/column/help
        // before pushing, which a single-field check couldn't.
        assert_eq!(state.diagnostics[0], diagnostic);
        let PropType::Opaque(detail) = &pt else { panic!("expected PropType::Opaque, got {:?}", pt) };
        assert_eq!(detail.raw(), "SomeType");
        assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
    }

    #[test]
    fn opaque_round_trips_through_the_tagged_json_wire_format() {
        let pt = OpaqueDetail::new("A<B> | C", OpaqueReason::UnsupportedExpression);
        let json = pt.to_tagged_value();
        let restored = PropType::from_tagged_value(&json).expect("should deserialize");
        assert_eq!(pt, restored);
    }
}

#[cfg(test)]
mod number_literal_roundtrip_tests {
    use super::*;

    #[test]
    fn nan_number_literal_round_trips_as_nan_not_zero() {
        let original = PropType::NumberLiteral(f64::NAN);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert!(n.is_nan(), "expected NaN to survive the round-trip, got {n}"),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    // ── SPEC-TYPES-001 AC-004: a NaN payload reachable NESTED inside Union,
    // Intersection, Array, Tuple, Object (via ObjectField.prop_type), or
    // Named's args must also survive the round-trip as NaN — AC-006 only
    // proves the top-level case; this covers every position the criterion
    // explicitly enumerates in one fixture rather than leaving them unasserted.

    #[test]
    fn nan_number_literal_survives_the_round_trip_when_nested() {
        let original = PropType::Union(vec![
            PropType::Intersection(vec![PropType::NumberLiteral(f64::NAN), PropType::String]),
            PropType::Array(Box::new(PropType::NumberLiteral(f64::NAN))),
            PropType::Tuple(vec![PropType::NumberLiteral(f64::NAN), PropType::String]),
            PropType::Object(vec![ObjectField {
                name: "n".into(),
                prop_type: PropType::NumberLiteral(f64::NAN),
                required: true,
                description: String::new(),
            }]),
            PropType::Named { name: "Foo".into(), args: vec![PropType::NumberLiteral(f64::NAN)] },
        ]);

        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        let PropType::Union(members) = &restored else { panic!("expected Union, got {restored:?}") };

        let PropType::Intersection(items) = &members[0] else { panic!("expected Intersection, got {:?}", members[0]) };
        assert!(
            matches!(items[0], PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Intersection did not survive, got {:?}",
            items[0]
        );

        let PropType::Array(inner) = &members[1] else { panic!("expected Array, got {:?}", members[1]) };
        assert!(
            matches!(**inner, PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Array did not survive, got {inner:?}"
        );

        let PropType::Tuple(items) = &members[2] else { panic!("expected Tuple, got {:?}", members[2]) };
        assert!(
            matches!(items[0], PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Tuple did not survive, got {:?}",
            items[0]
        );

        let PropType::Object(fields) = &members[3] else { panic!("expected Object, got {:?}", members[3]) };
        assert!(
            matches!(fields[0].prop_type, PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Object's ObjectField.prop_type did not survive, got {:?}",
            fields[0].prop_type
        );

        let PropType::Named { args, .. } = &members[4] else { panic!("expected Named, got {:?}", members[4]) };
        assert!(
            matches!(args[0], PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Named's args did not survive, got {:?}",
            args[0]
        );
    }

    #[test]
    fn infinity_number_literal_round_trips_as_infinity() {
        let original = PropType::NumberLiteral(f64::INFINITY);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert_eq!(n, f64::INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn finite_number_literal_still_round_trips_normally() {
        let original = PropType::NumberLiteral(42.5);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        assert_eq!(restored, PropType::NumberLiteral(42.5));
    }

    // ── SPEC-TYPES-001 AC-007: PropType::NumberLiteral(NEG_INFINITY) must also
    // round-trip exactly — only +INFINITY was previously tested, an asymmetry
    // against CollectedType's own test for the negative case.

    #[test]
    fn neg_infinity_number_literal_round_trips_as_neg_infinity() {
        let original = PropType::NumberLiteral(f64::NEG_INFINITY);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert_eq!(n, f64::NEG_INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod prop_type_composite_roundtrip_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-004: manual serde impls round-trip nested/composite
    // PropType shapes, not just the leaf Opaque/NumberLiteral cases.

    #[test]
    fn union_of_composite_members_round_trips_exactly() {
        let original = PropType::Union(vec![
            PropType::StringLiteral("a".into()),
            PropType::Array(Box::new(PropType::Number)),
            PropType::Named { name: "Foo".into(), args: vec![PropType::String] },
        ]);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    #[test]
    fn object_with_fields_round_trips_exactly() {
        let original = PropType::Object(vec![
            ObjectField { name: "a".into(), prop_type: PropType::String, required: true, description: "desc".into() },
            ObjectField { name: "b".into(), prop_type: PropType::Boolean, required: false, description: String::new() },
        ]);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    #[test]
    fn tuple_and_named_with_args_round_trip_exactly() {
        let original = PropType::Tuple(vec![PropType::String, PropType::Named { name: "Bar".into(), args: vec![] }]);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    // ── SPEC-TYPES-001 AC-004C: from_tagged_value returns Err when "kind" is
    // absent or present-but-not-a-string.

    #[test]
    fn from_tagged_value_errs_on_missing_kind() {
        let v = serde_json::json!({"notKind": "string"});
        assert!(PropType::from_tagged_value(&v).is_err());
    }

    #[test]
    fn from_tagged_value_errs_on_non_string_kind() {
        let v = serde_json::json!({"kind": 42});
        assert!(PropType::from_tagged_value(&v).is_err());
    }

    // ── SPEC-TYPES-001 AC-004C2: an unrecognized "kind" degrades to
    // Opaque(DepthExceeded) with the raw string naming the bad tag, not an Err.

    #[test]
    fn from_tagged_value_unrecognized_kind_degrades_to_opaque() {
        let v = serde_json::json!({"kind": "bogus"});
        let pt = PropType::from_tagged_value(&v).expect("unrecognized kind should degrade, not error");
        match pt {
            PropType::Opaque(detail) => {
                assert_eq!(detail.raw(), "unknown PropType kind: bogus");
                assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
            }
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

    // ── SPEC-TYPES-001 AC-005: OpaqueReason's OWN derived serialize output
    // differs from the hand-built {"type": ...} shape PropType::to_tagged_value
    // produces for the same reason — proving to_tagged_value intercepts the
    // reason before serde's derived impl ever runs on a bare OpaqueReason.

    #[test]
    fn opaque_reason_bare_derived_serialization_differs_from_the_intercepted_wire_form() {
        let reason = OpaqueReason::RuntimeDependent { function_name: "getVariant".into() };

        // The bare, derived serde output (serde's default externally-tagged form).
        let bare = serde_json::to_value(&reason).expect("OpaqueReason derives Serialize directly");

        // The wire form actually used when the reason is embedded in an Opaque PropType.
        // OpaqueDetail::new returns PropType::Opaque(..) directly, not a bare OpaqueDetail.
        let pt = OpaqueDetail::new("raw", reason);
        let wire = pt.to_tagged_value();
        let wire_reason = wire.get("reason").expect("expected a 'reason' field in the tagged Opaque JSON");

        assert_ne!(
            &bare, wire_reason,
            "OpaqueReason's bare derived form must differ from to_tagged_value's hand-built {{\"type\": ...}} \
             shape — if these ever match, to_tagged_value's manual construction has become redundant \
             or, worse, a bare OpaqueReason is leaking through unintercepted"
        );
        assert_eq!(wire_reason["type"], "runtimeDependent");
        assert_eq!(wire_reason["functionName"], "getVariant");
    }
}

#[cfg(test)]
mod serde_key_casing_tests {
    use super::*;
    use crate::types::diagnostic::DiagnosticCode;

    // ── SPEC-TYPES-001 AC-012B: Diagnostic's `severity` renders camelCase,
    // `code` renders SCREAMING_SNAKE_CASE — only the `code` half was
    // previously asserted anywhere in this crate.

    #[test]
    fn diagnostic_severity_serializes_as_camel_case() {
        let d = Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "boom".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::Unknown,
        };
        let v = serde_json::to_value(&d).expect("serialize");
        assert_eq!(v["severity"], "error", "expected camelCase 'error', got {:?}", v["severity"]);
    }

    // ── SPEC-TYPES-001 AC-012C: ComponentEntry/ParsedProp field renames —
    // display_name->displayName, file_path->filePath, default_value->defaultValue,
    // and prop_type->"type" specifically (not "propType", which rename_all would
    // otherwise have produced).

    #[test]
    fn component_entry_and_parsed_prop_serialize_with_the_documented_key_renames() {
        let prop = ParsedProp::new(
            "label".to_string(),
            PropType::String,
            true,
            None,
            String::new(),
            Default::default(),
            None,
            vec![],
        );
        let mut props = BTreeMap::new();
        props.insert("label".to_string(), prop);

        let entry = ComponentEntry {
            display_name: "Widget".to_string(),
            file_path: "Widget.tsx".into(),
            description: String::new(),
            props,
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
        };

        let v = serde_json::to_value(&entry).expect("serialize");
        assert!(v.get("displayName").is_some(), "expected 'displayName' key, got {v}");
        assert!(v.get("display_name").is_none(), "unexpected snake_case 'display_name' key, got {v}");
        assert!(v.get("filePath").is_some(), "expected 'filePath' key, got {v}");

        let prop_json = &v["props"]["label"];
        assert!(prop_json.get("type").is_some(), "expected the prop_type field under key 'type', got {prop_json}");
        assert!(prop_json.get("propType").is_none(), "prop_type must not serialize as 'propType', got {prop_json}");
    }

    #[test]
    fn parsed_prop_default_value_serializes_as_camel_case() {
        let prop = ParsedProp::new(
            "size".to_string(),
            PropType::String,
            false,
            Some(DefaultValue { value: "md".to_string(), computed: false }),
            String::new(),
            Default::default(),
            None,
            vec![],
        );
        let v = serde_json::to_value(&prop).expect("serialize");
        assert!(v.get("defaultValue").is_some(), "expected 'defaultValue' key, got {v}");
    }
}

#[cfg(test)]
mod component_entry_roundtrip_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-014: ComponentEntry derives both serde traits
    // directly (no manual impl) — there was previously NO round-trip test for
    // ComponentEntry at all.

    #[test]
    fn component_entry_round_trips_through_json() {
        let prop = ParsedProp::new(
            "label".to_string(),
            PropType::String,
            true,
            None,
            "the label".to_string(),
            Default::default(),
            None,
            vec![],
        );
        let mut props = BTreeMap::new();
        props.insert("label".to_string(), prop);

        let original = ComponentEntry {
            display_name: "Widget".to_string(),
            file_path: "Widget.tsx".into(),
            description: "A widget".to_string(),
            props,
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: Some("variant".to_string()),
            composes: vec!["Base".to_string()],
            tags: Default::default(),
            methods: vec![],
        };

        let json = serde_json::to_value(&original).expect("serialize");
        let restored: ComponentEntry = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }
}

#[cfg(test)]
mod parsed_prop_mutation_and_deserialize_escape_hatch_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-014B: ParsedProp::new normalizes required/default_value
    // at construction time only — direct field mutation after construction can
    // still produce the contradictory state. This is the documented, decided
    // exception (see non_goals), pinned here as expected behavior, not a gap.

    #[test]
    fn direct_field_mutation_can_reintroduce_the_contradictory_state() {
        let mut prop = ParsedProp::new(
            "p".to_string(),
            PropType::String,
            true,
            Some(DefaultValue { value: "a".to_string(), computed: false }),
            String::new(),
            Default::default(),
            None,
            vec![],
        );
        assert!(!prop.required, "sanity check: new() should have normalized required to false");

        prop.required = true;

        assert!(prop.required);
        assert!(prop.default_value.is_some());
    }

    // ── SPEC-TYPES-001 AC-014C: the derived Deserialize impl (with _seal
    // skipped) performs no normalization — deserializing a hand-crafted JSON
    // object with required:true and a non-null defaultValue produces the
    // contradictory state directly.

    #[test]
    fn derived_deserialize_does_not_normalize_required_and_default_value() {
        let json = serde_json::json!({
            "name": "p",
            "type": {"kind": "string"},
            "required": true,
            "defaultValue": {"value": "a", "computed": false},
            "description": "",
            "tags": {},
            "parent": null,
            "declarations": []
        });

        let prop: ParsedProp = serde_json::from_value(json).expect("deserialize");
        assert!(prop.required);
        assert!(prop.default_value.is_some());
    }
}

#[cfg(test)]
mod is_literal_union_tests {
    use super::*;

    #[test]
    fn empty_literal_union_is_not_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec![], has_default: false };
        assert!(!pt.is_literal_union());
    }

    #[test]
    fn single_member_literal_union_is_not_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec!["only".to_string()], has_default: false };
        assert!(!pt.is_literal_union());
    }

    #[test]
    fn two_member_literal_union_is_still_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec!["a".to_string(), "b".to_string()], has_default: false };
        assert!(pt.is_literal_union());
    }
}
