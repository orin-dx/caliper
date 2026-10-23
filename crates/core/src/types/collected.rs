//! Raw AST-level types produced by the extractor (Phase 0-2).
//! These are the extractor's output and the resolver's input.

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::diagnostic::Diagnostic;

// ─── Type Aliases ─────────────────────────────────────────────────────────────

/// Compact string used for names, type strings — avoids heap alloc under 24 bytes.
pub type TypeName = CompactString;

// ─── CollectedType ────────────────────────────────────────────────────────────

/// A structured representation of a TypeScript type as collected from the AST.
/// This is the extractor's output — NOT yet resolved to a semantic PropType.
/// The resolver pattern-matches on this to produce PropType.
#[derive(Debug, Clone)]
pub enum CollectedType {
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
    BigInt,
    Symbol,

    // ── Literals
    StringLiteral(CompactString),
    NumberLiteral(f64),
    BoolLiteral(bool),

    // ── Structural
    Union(Vec<CollectedType>),
    Intersection(Vec<CollectedType>),
    Array(Box<CollectedType>),
    Tuple(Vec<CollectedType>),
    Object(Vec<CollectedObjectField>),

    // ── Named type reference (possibly generic, possibly not yet resolved)
    Named {
        name: CompactString,
        args: Vec<CollectedType>,
    },

    // ── `typeof X` — reference to the type of a value
    TypeOf(CompactString),

    // ── `keyof X` — the union of X's own key names. Fully resolved only when
    // consumed by `Omit`'s key argument (`CollectedTypeAlias::Omit::keys`
    // resolves X as a props chain and reads its field names); standalone usage
    // degrades to `PropType::Opaque` since we don't (yet) have a general
    // type-to-key-names resolver outside the chain machinery.
    KeyOf(Box<CollectedType>),

    // ── Resolver-internal only (the extractor never produces this): pins `inner`
    // to the file it was written in. TypeScript resolves a generic alias's type
    // *arguments* in the caller's scope, not the callee's — e.g. in
    // `type SelectRootProps<T> = Assign<HTMLProps<'div'>, SelectRootBaseProps<T>>`,
    // `SelectRootBaseProps` must resolve relative to the file declaring
    // `SelectRootProps`, even though `Assign`'s own body (`Omit<T, keyof U> & U`)
    // lives in a different file. Every `CollectedType` the resolver's generic-alias
    // substitution (resolver/substitute.rs) splices into a callee's body gets
    // wrapped in this so later name lookups switch back to the correct file.
    AtFile {
        file: Utf8PathBuf,
        inner: Box<CollectedType>,
    },

    // ── Indexed access: Type["key"] or Type[K]
    IndexedAccess {
        obj: Box<CollectedType>,
        key: Box<CollectedType>,
    },

    // ── Template literal: `compact-${Size}` → parts = [Str("compact-"), Named("Size", [])]
    TemplateLiteral(Vec<CollectedType>),

    // ── Function type: (props: P) => ReactNode
    Function {
        /// The function's own type parameters (`<T>(item: T) => ReactNode`), in scope in its parameters and return type.
        type_params: Vec<CompactString>,
        params: Vec<CollectedType>,
        /// Source parameter names, parallel to `params` (same length). `None` for
        /// a given index when that parameter has no simple identifier binding
        /// (e.g. destructured) or the caller didn't capture one.
        param_names: Vec<Option<CompactString>>,
        return_type: Box<CollectedType>,
    },

    // ── Conditional: T extends U ? X : Y — cannot resolve statically
    Conditional {
        check: Box<CollectedType>,
        extends_type: Box<CollectedType>,
        true_type: Box<CollectedType>,
        false_type: Box<CollectedType>,
    },

    // ── Mapped: { [K in Keys]: Value } — cannot resolve statically
    Mapped {
        key_type: Box<CollectedType>,
        value_type: Box<CollectedType>,
    },

    // ── Fallback for syntax we don't recognise
    Raw(std::string::String),
}

impl CollectedType {
    /// True if this type requires the TypeScript type checker to resolve.
    pub fn needs_type_checker(&self) -> bool {
        matches!(self, CollectedType::Conditional { .. } | CollectedType::Mapped { .. })
    }

    /// Extract literal string keys from a type shaped like `'a' | 'b'` or a bare
    /// `'a'`. Used for `Pick`/`Omit`'s key argument. Returns an empty vec for
    /// anything else — notably `keyof T`, which can't be read as a string list
    /// (see `CollectedTypeAlias::Omit::keys`, which resolves the
    /// operand as a props chain instead).
    pub fn as_string_union_keys(&self) -> Vec<std::string::String> {
        match self {
            CollectedType::StringLiteral(s) => vec![s.to_string()],
            CollectedType::Union(members) => members.iter().flat_map(CollectedType::as_string_union_keys).collect(),
            _ => vec![],
        }
    }

    /// Produce a raw string representation for diagnostics and fallback display.
    pub fn to_raw_string(&self) -> std::string::String {
        match self {
            CollectedType::String => "string".into(),
            CollectedType::Number => "number".into(),
            CollectedType::Boolean => "boolean".into(),
            CollectedType::Null => "null".into(),
            CollectedType::Undefined => "undefined".into(),
            CollectedType::Any => "any".into(),
            CollectedType::Never => "never".into(),
            CollectedType::Unknown => "unknown".into(),
            CollectedType::Void => "void".into(),
            CollectedType::BigInt => "bigint".into(),
            CollectedType::Symbol => "symbol".into(),
            CollectedType::StringLiteral(s) => format!("\"{}\"", s),
            CollectedType::NumberLiteral(n) => n.to_string(),
            CollectedType::BoolLiteral(b) => b.to_string(),
            CollectedType::Union(members) => members.iter().map(|m| m.to_raw_string()).collect::<Vec<_>>().join(" | "),
            CollectedType::Intersection(members) => {
                members.iter().map(|m| m.to_raw_string()).collect::<Vec<_>>().join(" & ")
            }
            CollectedType::Array(inner) => format!("{}[]", inner.to_raw_string()),
            CollectedType::Named { name, args } if args.is_empty() => name.to_string(),
            CollectedType::Named { name, args } => {
                format!("{}<{}>", name, args.iter().map(|a| a.to_raw_string()).collect::<Vec<_>>().join(", "))
            }
            CollectedType::TypeOf(name) => format!("typeof {}", name),
            CollectedType::KeyOf(inner) => format!("keyof {}", inner.to_raw_string()),
            CollectedType::AtFile { inner, .. } => inner.to_raw_string(),
            CollectedType::IndexedAccess { obj, key } => {
                format!("{}[{}]", obj.to_raw_string(), key.to_raw_string())
            }
            CollectedType::TemplateLiteral(parts) => format!(
                "`{}`",
                parts
                    .iter()
                    .map(|p| match p {
                        CollectedType::StringLiteral(s) => s.to_string(),
                        other => format!("${{{}}}", other.to_raw_string()),
                    })
                    .collect::<Vec<_>>()
                    .join("")
            ),
            CollectedType::Function { type_params, params, param_names, return_type } => format!(
                "{}({}) => {}",
                if type_params.is_empty() { String::new() } else { format!("<{}>", type_params.join(", ")) },
                params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| match param_names.get(i).and_then(|n| n.as_ref()) {
                        Some(name) => format!("{name}: {}", p.to_raw_string()),
                        None => p.to_raw_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                return_type.to_raw_string()
            ),
            CollectedType::Conditional { check, extends_type, true_type, false_type } => format!(
                "{} extends {} ? {} : {}",
                check.to_raw_string(),
                extends_type.to_raw_string(),
                true_type.to_raw_string(),
                false_type.to_raw_string()
            ),
            CollectedType::Mapped { key_type, value_type } => {
                format!("{{ [K in {}]: {} }}", key_type.to_raw_string(), value_type.to_raw_string())
            }
            CollectedType::Tuple(members) => {
                format!("[{}]", members.iter().map(|m| m.to_raw_string()).collect::<Vec<_>>().join(", "))
            }
            CollectedType::Object(_) => "{ ... }".into(),
            CollectedType::Raw(s) => s.clone(),
        }
    }

    fn to_json_value(&self) -> serde_json::Value {
        match self {
            // Primitives: serialize as short string tags
            CollectedType::String => serde_json::json!("str"),
            CollectedType::Number => serde_json::json!("num"),
            CollectedType::Boolean => serde_json::json!("bool"),
            CollectedType::Null => serde_json::json!("null"),
            CollectedType::Undefined => serde_json::json!("undef"),
            CollectedType::Any => serde_json::json!("any"),
            CollectedType::Never => serde_json::json!("never"),
            CollectedType::Unknown => serde_json::json!("unknown"),
            CollectedType::Void => serde_json::json!("void"),
            CollectedType::BigInt => serde_json::json!("bigint"),
            CollectedType::Symbol => serde_json::json!("symbol"),
            // Literals
            CollectedType::StringLiteral(s) => serde_json::json!({"sl": s.as_str()}),
            CollectedType::NumberLiteral(n) => {
                // `serde_json::Number` cannot represent NaN/Infinity (they'd
                // silently become JSON `null`, then fail every `from_json_value`
                // match arm and fall through to `Raw`). Tag non-finite values as
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
                serde_json::json!({"nl": value})
            }
            CollectedType::BoolLiteral(b) => serde_json::json!({"bl": b}),
            // Named: {"n": name, "a": [args...]}
            CollectedType::Named { name, args } => serde_json::json!({
                "n": name.as_str(),
                "a": args.iter().map(|a| a.to_json_value()).collect::<Vec<_>>()
            }),
            // Union: {"u": [members...]}
            CollectedType::Union(members) => serde_json::json!({
                "u": members.iter().map(|m| m.to_json_value()).collect::<Vec<_>>()
            }),
            // Intersection: {"i": [members...]}
            CollectedType::Intersection(members) => serde_json::json!({
                "i": members.iter().map(|m| m.to_json_value()).collect::<Vec<_>>()
            }),
            // Array: {"arr": inner}
            CollectedType::Array(inner) => serde_json::json!({"arr": inner.to_json_value()}),
            // Tuple: {"tup": [members...]}
            CollectedType::Tuple(members) => serde_json::json!({
                "tup": members.iter().map(|m| m.to_json_value()).collect::<Vec<_>>()
            }),
            // Object: {"obj": [{name, t, req, desc}...]}
            CollectedType::Object(fields) => serde_json::json!({
                "obj": fields.iter().map(|f| serde_json::json!({
                    "name": f.name,
                    "t": f.collected_type.to_json_value(),
                    "req": f.required,
                    "desc": f.description,
                })).collect::<Vec<_>>()
            }),
            // TypeOf: {"to": name}
            CollectedType::TypeOf(name) => serde_json::json!({"to": name.as_str()}),
            // KeyOf: {"keyof": inner}
            CollectedType::KeyOf(inner) => serde_json::json!({"keyof": inner.to_json_value()}),
            // AtFile is resolver-internal only (see the variant doc comment) — the
            // extractor's `SourceData` (the only thing ever cached/serialized) never
            // contains one, so there's nothing meaningful to round-trip. Serialize
            // transparently as `inner` so this stays total rather than panicking.
            CollectedType::AtFile { inner, .. } => inner.to_json_value(),
            // IndexedAccess: {"ia": {o, k}}
            CollectedType::IndexedAccess { obj, key } => serde_json::json!({
                "ia": {"o": obj.to_json_value(), "k": key.to_json_value()}
            }),
            // TemplateLiteral: {"tl": [parts...]}
            CollectedType::TemplateLiteral(parts) => serde_json::json!({
                "tl": parts.iter().map(|p| p.to_json_value()).collect::<Vec<_>>()
            }),
            // Function: {"fn": {tp: [type_params], p: [params], names: [param_names], r: return_type}}
            CollectedType::Function { type_params, params, param_names, return_type } => serde_json::json!({
                "fn": {
                    "tp": type_params.iter().map(CompactString::as_str).collect::<Vec<_>>(),
                    "p": params.iter().map(|p| p.to_json_value()).collect::<Vec<_>>(),
                    "names": param_names.iter().map(|n| n.as_ref().map(|s| s.as_str())).collect::<Vec<_>>(),
                    "r": return_type.to_json_value()
                }
            }),
            // Conditional: {"cond": {c, e, t, f}}
            CollectedType::Conditional { check, extends_type, true_type, false_type } => {
                serde_json::json!({
                    "cond": {
                        "c": check.to_json_value(),
                        "e": extends_type.to_json_value(),
                        "t": true_type.to_json_value(),
                        "f": false_type.to_json_value(),
                    }
                })
            }
            // Mapped: {"mapped": {k, v}}
            CollectedType::Mapped { key_type, value_type } => serde_json::json!({
                "mapped": {"k": key_type.to_json_value(), "v": value_type.to_json_value()}
            }),
            // Raw fallback: {"raw": s}
            CollectedType::Raw(s) => serde_json::json!({"raw": s}),
        }
    }

    fn from_json_value(v: &serde_json::Value) -> Result<Self, std::string::String> {
        match v {
            serde_json::Value::String(s) => Ok(match s.as_str() {
                "str" => CollectedType::String,
                "num" => CollectedType::Number,
                "bool" => CollectedType::Boolean,
                "null" => CollectedType::Null,
                "undef" => CollectedType::Undefined,
                "any" => CollectedType::Any,
                "never" => CollectedType::Never,
                "unknown" => CollectedType::Unknown,
                "void" => CollectedType::Void,
                "bigint" => CollectedType::BigInt,
                "symbol" => CollectedType::Symbol,
                other => CollectedType::Raw(other.to_string()),
            }),
            serde_json::Value::Object(map) => {
                if let Some(s) = map.get("sl").and_then(|v| v.as_str()) {
                    return Ok(CollectedType::StringLiteral(s.into()));
                }
                if let Some(val) = map.get("nl") {
                    if let Some(s) = val.as_str() {
                        let n = match s {
                            "NaN" => f64::NAN,
                            "Infinity" => f64::INFINITY,
                            "-Infinity" => f64::NEG_INFINITY,
                            _ => 0.0,
                        };
                        return Ok(CollectedType::NumberLiteral(n));
                    }
                    if let Some(n) = val.as_f64() {
                        return Ok(CollectedType::NumberLiteral(n));
                    }
                }
                if let Some(b) = map.get("bl").and_then(|v| v.as_bool()) {
                    return Ok(CollectedType::BoolLiteral(b));
                }
                if let Some(name) = map.get("n").and_then(|v| v.as_str()) {
                    let args = map
                        .get("a")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().map(Self::from_json_value).collect::<Result<Vec<_>, _>>())
                        .unwrap_or(Ok(vec![]))?;
                    return Ok(CollectedType::Named { name: name.into(), args });
                }
                if let Some(arr) = map.get("u").and_then(|v| v.as_array()) {
                    return Ok(CollectedType::Union(arr.iter().map(Self::from_json_value).collect::<Result<_, _>>()?));
                }
                if let Some(arr) = map.get("i").and_then(|v| v.as_array()) {
                    return Ok(CollectedType::Intersection(
                        arr.iter().map(Self::from_json_value).collect::<Result<_, _>>()?,
                    ));
                }
                if let Some(inner) = map.get("arr") {
                    return Ok(CollectedType::Array(Box::new(Self::from_json_value(inner)?)));
                }
                if let Some(arr) = map.get("tup").and_then(|v| v.as_array()) {
                    return Ok(CollectedType::Tuple(arr.iter().map(Self::from_json_value).collect::<Result<_, _>>()?));
                }
                if let Some(arr) = map.get("obj").and_then(|v| v.as_array()) {
                    let fields: Vec<CollectedObjectField> = arr
                        .iter()
                        .map(|f| {
                            let o = f.as_object().ok_or_else(|| "expected object for field".to_string())?;
                            // A missing/wrong-type "name" would otherwise silently
                            // default to "" — indistinguishable from a real (if
                            // unusual) empty-string prop name, unlike every other
                            // field in this struct, which either has a visible
                            // Raw("null") escape hatch (`t`, via the recursive
                            // from_json_value call) or a low-stakes default
                            // (`req`/`desc`). Erroring here surfaces malformed
                            // cache data as a deserialize failure — cache.rs's
                            // `rmp_serde::from_slice(&bytes).ok()?` already treats
                            // any such error as a full cache miss, falling back to
                            // re-parsing from source, rather than serving a
                            // corrupted-but-plausible-looking prop.
                            let name = o
                                .get("name")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| "object field missing a 'name' string".to_string())?
                                .to_owned();
                            Ok(CollectedObjectField {
                                name,
                                collected_type: Self::from_json_value(o.get("t").unwrap_or(&serde_json::Value::Null))?,
                                required: o.get("req").and_then(|v| v.as_bool()).unwrap_or(false),
                                description: o.get("desc").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                            })
                        })
                        .collect::<Result<_, std::string::String>>()?;
                    return Ok(CollectedType::Object(fields));
                }
                if let Some(name) = map.get("to").and_then(|v| v.as_str()) {
                    return Ok(CollectedType::TypeOf(name.into()));
                }
                if let Some(inner) = map.get("keyof") {
                    return Ok(CollectedType::KeyOf(Box::new(Self::from_json_value(inner)?)));
                }
                if let Some(inner) = map.get("ia") {
                    let obj = Self::from_json_value(inner.get("o").unwrap_or(&serde_json::Value::Null))?;
                    let key = Self::from_json_value(inner.get("k").unwrap_or(&serde_json::Value::Null))?;
                    return Ok(CollectedType::IndexedAccess { obj: Box::new(obj), key: Box::new(key) });
                }
                if let Some(arr) = map.get("tl").and_then(|v| v.as_array()) {
                    return Ok(CollectedType::TemplateLiteral(
                        arr.iter().map(Self::from_json_value).collect::<Result<_, _>>()?,
                    ));
                }
                if let Some(inner) = map.get("fn") {
                    let params = inner
                        .get("p")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().map(Self::from_json_value).collect::<Result<Vec<_>, _>>())
                        .unwrap_or(Ok(vec![]))?;
                    let param_names = inner
                        .get("names")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().map(|v| v.as_str().map(CompactString::from)).collect::<Vec<_>>())
                        .unwrap_or_default();
                    let type_params = inner
                        .get("tp")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(CompactString::from)).collect())
                        .unwrap_or_default();
                    let rt = Self::from_json_value(inner.get("r").unwrap_or(&serde_json::Value::Null))?;
                    return Ok(CollectedType::Function { type_params, params, param_names, return_type: Box::new(rt) });
                }
                if let Some(inner) = map.get("cond") {
                    let check = Self::from_json_value(inner.get("c").unwrap_or(&serde_json::Value::Null))?;
                    let ext = Self::from_json_value(inner.get("e").unwrap_or(&serde_json::Value::Null))?;
                    let tt = Self::from_json_value(inner.get("t").unwrap_or(&serde_json::Value::Null))?;
                    let ft = Self::from_json_value(inner.get("f").unwrap_or(&serde_json::Value::Null))?;
                    return Ok(CollectedType::Conditional {
                        check: Box::new(check),
                        extends_type: Box::new(ext),
                        true_type: Box::new(tt),
                        false_type: Box::new(ft),
                    });
                }
                if let Some(inner) = map.get("mapped") {
                    let k = Self::from_json_value(inner.get("k").unwrap_or(&serde_json::Value::Null))?;
                    let vt = Self::from_json_value(inner.get("v").unwrap_or(&serde_json::Value::Null))?;
                    return Ok(CollectedType::Mapped { key_type: Box::new(k), value_type: Box::new(vt) });
                }
                if let Some(s) = map.get("raw").and_then(|v| v.as_str()) {
                    return Ok(CollectedType::Raw(s.to_owned()));
                }
                // Unknown shape — fall back to raw
                Ok(CollectedType::Raw(v.to_string()))
            }
            _ => Ok(CollectedType::Raw(v.to_string())),
        }
    }
}

impl serde::Serialize for CollectedType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let v = self.to_json_value();
        v.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for CollectedType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let v = serde_json::Value::deserialize(deserializer)?;
        Self::from_json_value(&v).map_err(D::Error::custom)
    }
}

/// An object field as collected from the AST (not yet resolved).
///
/// `CollectedType`'s own Serialize/Deserialize builds this struct's fields
/// manually (see `to_json_value`/`from_json_value`) and never calls this
/// derive — it only matters if something serializes a `CollectedObjectField`
/// directly, outside a `CollectedType::Object`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedObjectField {
    pub name: std::string::String,
    pub collected_type: CollectedType,
    pub required: bool,
    pub description: std::string::String,
}

// ─── Enum Types (used in both SourceData and ExtractionOutput) ────────────────

/// An enum-like constant discovered during extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumEntry {
    pub name: std::string::String,
    pub value: EnumValue,
    pub description: std::string::String,
}

/// The value of an enum entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum EnumValue {
    String(std::string::String),
    Number(f64),
    Bool(bool),
}

impl EnumValue {
    /// Render as the string a `LiteralUnion` member expects — every variant
    /// display-formatted, regardless of its original JS type.
    pub fn to_display_string(&self) -> std::string::String {
        match self {
            EnumValue::String(s) => s.clone(),
            EnumValue::Number(n) => n.to_string(),
            EnumValue::Bool(b) => b.to_string(),
        }
    }
}

// ─── Source Data (collected during Phase 0-2) ─────────────────────────────────

/// Raw data collected from parsing a single file.
/// Owned data only — no AST references. Safe for rayon + arc.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceData {
    /// Interfaces found in this file.
    /// Key: "${absolute_file_path}:${interface_name}"
    pub interfaces: FxHashMap<std::string::String, CollectedInterface>,

    /// Type aliases found in this file.
    /// Key: "${absolute_file_path}:${type_name}"
    pub type_aliases: FxHashMap<std::string::String, CollectedTypeAlias>,

    /// Declared type parameter names for generic type alias declarations, e.g.
    /// `type Assign<T, U> = ...` records `["T", "U"]` here. Keyed identically to
    /// `type_aliases`. Absent (no entry) for the common non-generic alias — the
    /// resolver only attempts call-site substitution when an entry exists.
    pub type_alias_params: FxHashMap<std::string::String, Vec<TypeName>>,

    /// Declared type parameter names for generic interface declarations, e.g.
    /// `interface ColumnDef<TData, TValue> { ... }` records `["TData", "TValue"]`
    /// here. Keyed identically to `interfaces`. Used by the resolver to recognize
    /// a bare `TData` reference inside the interface's own body as an expected
    /// generic placeholder rather than an unresolvable type — see
    /// `resolver::chain::resolve_interface_chain`.
    pub interface_type_params: FxHashMap<std::string::String, Vec<TypeName>>,

    /// Enum-like values found in this file.
    /// Key: "${absolute_file_path}:${name}"
    pub enums: FxHashMap<std::string::String, Vec<EnumEntry>>,

    /// Flat `const X = [...] as const` array literals found in this file —
    /// e.g. `const _ButtonTypes = ['default', 'primary'] as const`, referenced
    /// via `(typeof _ButtonTypes)[number]` to build a literal union without an
    /// explicit `enum`. Deliberately separate from `enums`: unlike an enum or
    /// a cva/tv variant group, a plain array has no per-entry name to group
    /// by, and `enums` is surfaced directly in the public `ExtractionOutput`
    /// (see `pipeline::collect_public_enums`) — these arrays are resolver-
    /// internal only, never part of that output.
    /// Key: "${absolute_file_path}:${name}"
    pub const_arrays: FxHashMap<std::string::String, Vec<EnumValue>>,

    /// Component → prop type mappings found in this file.
    /// Only populated for .tsx files.
    pub component_mappings: Vec<ComponentMapping>,

    /// Import bindings in this file — local name → source
    pub imports: Vec<ImportBinding>,

    /// Re-exports from this file
    pub exports: Vec<LexedExport>,

    /// Non-fatal issues discovered while parsing this file (e.g. excessive nesting,
    /// TypeScript syntax errors). Drained into the pipeline's top-level diagnostics
    /// during `GlobalSourceData` merge — never dropped silently.
    pub diagnostics: Vec<Diagnostic>,
}

/// A collected interface declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectedInterface {
    /// Scoped key: "${file_path}:${name}"
    pub scoped_key: std::string::String,
    pub name: TypeName,
    pub file_path: Utf8PathBuf,
    pub props: Vec<RawProp>,
    pub extends: Vec<ExtendsRef>,
    pub description: std::string::String,
    pub tags: BTreeMap<std::string::String, std::string::String>,
}

/// A raw prop — types not yet resolved to PropType.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawProp {
    pub name: std::string::String,
    /// Structured type collected from the AST — replaces raw string for reliable resolver access.
    pub collected_type: CollectedType,
    pub required: bool,
    pub description: std::string::String,
    pub tags: BTreeMap<std::string::String, std::string::String>,
    /// Byte span in source file — used for miette diagnostics
    pub span_start: u32,
    pub span_end: u32,
}

/// An extends reference in an interface declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtendsRef {
    /// interface Foo extends Bar — Bar defined in SAME file
    SameFile { name: TypeName, type_args: Vec<CollectedType> },
    /// interface Foo extends Bar — Bar came from an import
    Imported {
        local_name: TypeName,
        type_args: Vec<CollectedType>,
        /// The import specifier this name came from, if determinable
        source_specifier: Option<std::string::String>,
    },
    /// `interface Foo extends ButtonHTMLAttributes<HTMLButtonElement>`
    /// Recognized as baked-in React/DOM type — no file lookup needed
    Builtin {
        name: TypeName,
        /// Resolved HTML element: "button", "input", etc.
        element: Option<std::string::String>,
        type_args: Vec<CollectedType>,
    },
}

impl ExtendsRef {
    /// This reference with each type argument rewritten by `f`.
    #[must_use]
    pub fn map_type_args(&self, f: impl Fn(&CollectedType) -> CollectedType) -> ExtendsRef {
        let map = |args: &[CollectedType]| args.iter().map(&f).collect();
        match self {
            ExtendsRef::SameFile { name, type_args } => {
                ExtendsRef::SameFile { name: name.clone(), type_args: map(type_args) }
            }
            ExtendsRef::Imported { local_name, type_args, source_specifier } => ExtendsRef::Imported {
                local_name: local_name.clone(),
                type_args: map(type_args),
                source_specifier: source_specifier.clone(),
            },
            ExtendsRef::Builtin { name, element, type_args } => {
                ExtendsRef::Builtin { name: name.clone(), element: element.clone(), type_args: map(type_args) }
            }
        }
    }
}

/// A collected type alias.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CollectedTypeAlias {
    /// `Omit<Base, Keys>`. `keys` is the key expression as written (`"a" | "b"`, `keyof Other`, a `Keys` alias,
    /// `Exclude<…>`); the resolver turns it into a key set, so an alias or a substituted parameter works too.
    Omit {
        base: CollectedType,
        keys: CollectedType,
        file_path: Utf8PathBuf,
    },
    /// `Pick<Base, Keys>`, with `keys` resolved the same way as `Omit`'s.
    Pick {
        base: CollectedType,
        keys: CollectedType,
        file_path: Utf8PathBuf,
    },
    Partial {
        base: CollectedType,
        file_path: Utf8PathBuf,
    },
    Required {
        base: CollectedType,
        file_path: Utf8PathBuf,
    },
    Union {
        members: Vec<CollectedType>,
        file_path: Utf8PathBuf,
    },
    Intersection {
        members: Vec<CollectedType>,
        file_path: Utf8PathBuf,
    },
    LiteralUnion {
        members: Vec<std::string::String>,
        file_path: Utf8PathBuf,
    },
    /// e.g. type Size = SomeOtherType — transparent alias
    Passthrough {
        target: CollectedType,
        file_path: Utf8PathBuf,
    },
}

impl CollectedTypeAlias {
    /// The file this alias was declared in — members referenced in its own body
    /// (e.g. union/intersection operands) must resolve relative to this, not
    /// whichever file happens to be consuming the alias.
    pub(crate) fn file_path(&self) -> &Utf8Path {
        match self {
            CollectedTypeAlias::Omit { file_path, .. }
            | CollectedTypeAlias::Pick { file_path, .. }
            | CollectedTypeAlias::Partial { file_path, .. }
            | CollectedTypeAlias::Required { file_path, .. }
            | CollectedTypeAlias::Union { file_path, .. }
            | CollectedTypeAlias::Intersection { file_path, .. }
            | CollectedTypeAlias::LiteralUnion { file_path, .. }
            | CollectedTypeAlias::Passthrough { file_path, .. } => file_path,
        }
    }
}

/// The source of a default value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DefaultSource {
    /// Extracted from parameter destructuring: `({ size = 'md' }: Props) => ...`
    Destructuring,
    /// Extracted from `Component.defaultProps = { size: 'md' }` assignment.
    DefaultProps,
    /// Extracted from `/** @default 'md' */` JSDoc/TSDoc annotation.
    JsDoc,
}

/// A default value as collected by the extractor (before resolver processing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawDefault {
    /// String representation of the default value (literal or source expression).
    pub value: std::string::String,
    /// True if `value` is a runtime expression we couldn't statically evaluate.
    pub computed: bool,
    /// Where this default came from.
    pub source: DefaultSource,
}

/// The extractor rule that recognized a component. Carried so a mapping can be explained and, later, so rules can be
/// ranked by how likely they are to be wrong. The cache encodes variants by their serialized name, so renaming one
/// changes the cache's bytes and needs a `CACHE_SCHEMA_VERSION` bump.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectionRule {
    /// `const Button: FC<Props> = ...`
    FcAnnotation,
    /// `forwardRef<El, Props>(...)` or a render function typed through a `ForwardRefComponent<Props>` annotation.
    ForwardRef,
    /// `declare const Button: ForwardRefExoticComponent<Props & RefAttributes<El>>`
    ForwardRefExoticDecl,
    /// `memo(...)`, `observer(...)` or any call whose first argument is a function with a typed props parameter.
    HocWrapper,
    /// `class Button extends Component<Props>`
    ClassComponent,
    /// `function Button(props: Props) {}`
    FunctionDeclaration,
    /// `export default function (props: Props) {}`, named after the file.
    AnonymousDefaultExport,
    /// `Object.assign(Root, { Item })` or a static property attached to a component.
    AttachedSubComponent,
    /// `const Button = (props: Props) => ...` or `const Button = function (props: Props) {}`: a PascalCase binding
    /// whose initializer is a synchronous function with a props-typed first parameter.
    TypedFunctionExpression,
    /// `const Button = wrapper<ButtonFactory>(...)` or `wrapper<El, ButtonProps>(...)`: a generic call whose type
    /// arguments carry the props type, chosen by shape from declarations in the same file.
    WrapperTypeArgument,
}

/// A component → prop type mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentMapping {
    pub component_name: std::string::String,
    pub props_type_name: TypeName,
    pub props_type_args: Vec<CollectedType>,
    pub file_path: Utf8PathBuf,
    pub description: std::string::String,
    pub tags: BTreeMap<std::string::String, std::string::String>,
    /// Byte span for diagnostics
    pub span_start: u32,
    pub span_end: u32,
    /// Default values extracted from parameter destructuring or defaultProps.
    /// Key = prop name. Populated only for implementation files (.tsx/.ts), not .d.ts.
    pub param_defaults: FxHashMap<std::string::String, RawDefault>,
    /// Which extractor rule matched.
    pub rule: DetectionRule,
    /// Whether its module exports it, directly or as a sub-component attached to an exported component.
    pub exported: bool,
}

impl ComponentMapping {
    /// A mapping with no type arguments, documentation or parameter defaults; add them with the `with_*` methods.
    pub fn new(
        component_name: std::string::String,
        props_type_name: TypeName,
        file_path: Utf8PathBuf,
        span_start: u32,
        span_end: u32,
        rule: DetectionRule,
    ) -> Self {
        Self {
            component_name,
            props_type_name,
            props_type_args: Vec::new(),
            file_path,
            description: std::string::String::new(),
            tags: BTreeMap::new(),
            span_start,
            span_end,
            param_defaults: FxHashMap::default(),
            rule,
            exported: false,
        }
    }

    #[must_use]
    pub fn with_props_type_args(mut self, props_type_args: Vec<CollectedType>) -> Self {
        self.props_type_args = props_type_args;
        self
    }

    #[must_use]
    pub fn with_doc(
        mut self,
        description: std::string::String,
        tags: BTreeMap<std::string::String, std::string::String>,
    ) -> Self {
        self.description = description;
        self.tags = tags;
        self
    }

    #[must_use]
    pub fn with_param_defaults(mut self, param_defaults: FxHashMap<std::string::String, RawDefault>) -> Self {
        self.param_defaults = param_defaults;
        self
    }
}

// ─── Import/Export Types ───────────────────────────────────────────────────────

/// An import binding found in a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBinding {
    /// Local name as used in this file (after `as` rename)
    pub local_name: TypeName,
    /// Original exported name
    pub exported_name: TypeName,
    /// Import specifier: "@radix-ui/react-button", "./types", etc.
    pub specifier: std::string::String,
    /// true for `import type { ... }`
    pub is_type_only: bool,
}

/// An export found in a source file — classified for re-export tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LexedExport {
    /// `export { Foo }` or `export { Foo as Bar }` from "./types"
    ReExportNamed {
        local_name: std::string::String,
        source_name: std::string::String,
        source_specifier: std::string::String,
        is_type_only: bool,
    },
    /// `export * from "./types"`
    ReExportAll { source_specifier: std::string::String, is_type_only: bool },
    /// `export * as Ns from "./types"`
    ReExportNamespace { namespace: std::string::String, source_specifier: std::string::String },
    /// `export interface Foo { }` / `export type Bar = ...` / `export const X` / `export { Base as Foo }`
    LocalDeclaration {
        name: std::string::String,
        is_type_only: bool,
        /// The declaration's own name when it is exported under another (`Base` in `export { Base as Foo }`).
        declared_as: Option<std::string::String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    // CollectedType's own Serialize/Deserialize never actually calls into
    // CollectedObjectField's impl (to_json_value/from_json_value build fields
    // manually), so this exercises CollectedObjectField's own impl directly —
    // the only way anything would ever actually invoke it. rmp_serde's default
    // (non-named) encoding writes structs POSITIONALLY (as a MessagePack array,
    // not a map of field names), which requires a Deserialize impl to handle
    // `visit_seq`, not just `visit_map`.
    #[test]
    fn collected_object_field_round_trips_through_rmp_serde_positional_encoding() {
        let field = CollectedObjectField {
            name: "label".to_owned(),
            collected_type: CollectedType::String,
            required: true,
            description: "the label".to_owned(),
        };

        let bytes = rmp_serde::to_vec(&field).expect("serialization should succeed");
        let round_tripped: CollectedObjectField =
            rmp_serde::from_slice(&bytes).expect("deserialization should succeed for rmp_serde's positional encoding");

        assert_eq!(round_tripped.name, "label");
        assert!(round_tripped.required);
        assert_eq!(round_tripped.description, "the label");
    }

    // ── SPEC-TYPES-001 AC-004B: CollectedType's manual serde impls
    // (to_json_value/from_json_value) round-trip a composite shape, not just
    // leaf variants.

    #[test]
    fn union_of_composite_members_round_trips_exactly() {
        // CollectedType has no PartialEq — compare via re-serialization instead.
        let original = CollectedType::Union(vec![
            CollectedType::StringLiteral("a".into()),
            CollectedType::Array(Box::new(CollectedType::Number)),
            CollectedType::Object(vec![CollectedObjectField {
                name: "x".to_owned(),
                collected_type: CollectedType::Boolean,
                required: true,
                description: String::new(),
            }]),
        ]);
        let json = original.to_json_value();
        let restored = CollectedType::from_json_value(&json).expect("deserialize");
        assert_eq!(restored.to_json_value(), json, "round-tripped value must re-serialize identically");
    }

    // ── SPEC-TYPES-001 AC-004D: from_json_value on a number/bool/null, and on
    // an object matching no recognized shape key, returns Ok(Raw(v.to_string())).

    #[test]
    fn from_json_value_on_number_bool_null_and_unrecognized_object_returns_raw() {
        for v in [serde_json::json!(42), serde_json::json!(true), serde_json::Value::Null] {
            let result = CollectedType::from_json_value(&v).expect("should not error");
            match result {
                CollectedType::Raw(s) => assert_eq!(s, v.to_string(), "input was {v}"),
                other => panic!("expected Raw for input {v}, got {other:?}"),
            }
        }

        let unrecognized = serde_json::json!({"notAKnownShapeKey": 1});
        let result = CollectedType::from_json_value(&unrecognized).expect("should not error");
        match result {
            CollectedType::Raw(s) => assert_eq!(s, unrecognized.to_string()),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    // ── SPEC-TYPES-001 AC-004D2: from_json_value on each of the eleven
    // recognized strings returns the correspondingly-named unit variant; any
    // other string returns Ok(Raw(that string's own contents, unquoted)).

    #[test]
    fn from_json_value_recognized_strings_map_to_unit_variants() {
        // All 11 tags AC-004D2 enumerates — a prior version of this test only
        // covered 5, and the loop structure made it LOOK exhaustive while
        // silently skipping "null"/"undef"/"any"/"never"/"unknown"/"void".
        let cases: &[(&str, &str)] = &[
            ("str", "String"),
            ("num", "Number"),
            ("bool", "Boolean"),
            ("null", "Null"),
            ("undef", "Undefined"),
            ("any", "Any"),
            ("never", "Never"),
            ("unknown", "Unknown"),
            ("void", "Void"),
            ("bigint", "BigInt"),
            ("symbol", "Symbol"),
        ];
        for (tag, expected) in cases {
            let v = serde_json::json!(tag);
            let result = CollectedType::from_json_value(&v).expect("should not error");
            let debug = format!("{result:?}");
            assert_eq!(debug, *expected, "tag was {tag}");
        }
    }

    #[test]
    fn from_json_value_unrecognized_string_returns_raw_unquoted_contents() {
        let v = serde_json::json!("foo");
        let result = CollectedType::from_json_value(&v).expect("should not error");
        match result {
            CollectedType::Raw(s) => assert_eq!(s, "foo", "expected unquoted contents, not v.to_string()"),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    // ── SPEC-TYPES-001 AC-004E: a non-object element inside the "obj" array
    // is an Err — each element must be an object.

    #[test]
    fn from_json_value_obj_array_with_a_non_object_element_errs() {
        let v = serde_json::json!({"obj": [1]});
        assert!(CollectedType::from_json_value(&v).is_err());
    }

    // ── A malformed cache entry (corrupted "obj" field missing its own
    // "name") must surface as a deserialize error, not silently become a
    // plausible-looking empty-string prop name — see the fix's doc comment
    // at the "obj" match arm.

    #[test]
    fn from_json_value_obj_field_missing_name_errs() {
        let v = serde_json::json!({"obj": [{"t": "str", "req": true, "desc": "d"}]});
        assert!(
            CollectedType::from_json_value(&v).is_err(),
            "an object field with no 'name' key must be rejected, not silently default to an empty name"
        );
    }

    #[test]
    fn from_json_value_obj_field_with_a_non_string_name_errs() {
        let v = serde_json::json!({"obj": [{"name": 42, "t": "str", "req": true, "desc": "d"}]});
        assert!(CollectedType::from_json_value(&v).is_err(), "a non-string 'name' must be rejected");
    }

    #[test]
    fn from_json_value_obj_field_with_a_valid_name_still_round_trips() {
        // Regression guard: the fix above must not break the normal path.
        let v = serde_json::json!({"obj": [{"name": "label", "t": "str", "req": true, "desc": "d"}]});
        let restored = CollectedType::from_json_value(&v).expect("a well-formed object field must still parse");
        match restored {
            CollectedType::Object(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "label");
                assert!(fields[0].required);
            }
            other => panic!("expected Object, got {other:?}"),
        }
    }

    #[test]
    fn nan_number_literal_round_trips_as_nan_not_raw() {
        let original = CollectedType::NumberLiteral(f64::NAN);
        let json = original.to_json_value();
        let restored = CollectedType::from_json_value(&json).expect("deserialize");

        match restored {
            CollectedType::NumberLiteral(n) => assert!(n.is_nan(), "expected NaN to survive the round-trip, got {n}"),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn infinity_number_literal_round_trips_as_infinity() {
        let original = CollectedType::NumberLiteral(f64::INFINITY);
        let json = original.to_json_value();
        let restored = CollectedType::from_json_value(&json).expect("deserialize");

        match restored {
            CollectedType::NumberLiteral(n) => assert_eq!(n, f64::INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn negative_infinity_number_literal_round_trips_as_negative_infinity() {
        let original = CollectedType::NumberLiteral(f64::NEG_INFINITY);
        let json = original.to_json_value();
        let restored = CollectedType::from_json_value(&json).expect("deserialize");

        match restored {
            CollectedType::NumberLiteral(n) => assert_eq!(n, f64::NEG_INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn finite_number_literal_still_round_trips_normally() {
        let original = CollectedType::NumberLiteral(42.5);
        let json = original.to_json_value();
        let restored = CollectedType::from_json_value(&json).expect("deserialize");

        match restored {
            CollectedType::NumberLiteral(n) => assert_eq!(n, 42.5),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }
}
