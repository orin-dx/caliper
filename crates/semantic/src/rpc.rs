//! Typed calls over one session of the linked checker ([`crate::checker`]). Wire shapes are the ones the official 7.1
//! client sends (traced against `7.1.0-beta`); a method this module does not name is not used.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::checker::{Client, RequestError};

/// Why a call failed, split by whether the rest of the run can go on.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The session is gone or stuck: every later call would fail the same way.
    Fatal(String),
    /// The checker rejected this one call; other components may still resolve.
    Rejected(String),
    /// The run's heap is past its budget ([`crate::heap`]): the call was not sent, and no later one will be.
    OverBudget,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Fatal(message) | CallError::Rejected(message) => f.write_str(message),
            CallError::OverBudget => f.write_str("the run's heap is past its budget"),
        }
    }
}

fn classify(error: RequestError) -> CallError {
    match error {
        RequestError::Rejected(message) => CallError::Rejected(message),
        other => CallError::Fatal(other.to_string()),
    }
}

/// The reference the checker wants back to name a symbol, kept as the JSON text it arrived as: it is opaque to us, and
/// is only ever sent back, written into a request as it is.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct SymbolRef(Box<RawValue>);

impl SymbolRef {
    /// The reference's JSON text, which names the symbol within its snapshot.
    pub(crate) fn text(&self) -> &str {
        self.0.get()
    }
}

#[cfg(test)]
impl From<Value> for SymbolRef {
    fn from(value: Value) -> Self {
        SymbolRef(serde_json::value::to_raw_value(&value).unwrap_or_else(|_| RawValue::NULL.to_owned()))
    }
}

/// A request [`Rpc::call`] sends: its parameters, and the method that reads them. Each type names its method once, so a
/// method cannot be sent with another's parameters; `tests::every_key_sent_is_one_tsrs_reads` checks the keys of each.
trait Request: Serialize {
    fn method(&self) -> &'static str;

    /// Whether it gives memory back, so is sent past the heap budget too.
    fn frees_memory(&self) -> bool {
        false
    }
}

/// Writes [`Ask`] from one list, so that each checker method and the parameter keys it is sent with are written once.
macro_rules! asks {
    ($($(#[$doc:meta])* $method:literal => $variant:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? })*) => {
        /// A question about a program, one variant per checker method, holding exactly the parameters that method is
        /// sent besides the snapshot and project [`Scoped`] adds; a symbol's reference goes as the text it arrived as.
        #[derive(Debug, Serialize)]
        #[serde(untagged)]
        pub(crate) enum Ask<'a> {
            $($(#[$doc])* $variant { $($(#[$attr])* $field: $ty),* },)*
        }

        impl Ask<'_> {
            pub(crate) fn method(&self) -> &'static str {
                match self {
                    $(Ask::$variant { .. } => $method,)*
                }
            }
        }

        /// Every method [`Ask`] sends.
        #[cfg(test)]
        const ASKED: &[&str] = &[$($method),*];
    };
}

asks! {
    "getSourceFileNames" => SourceFileNames {}
    "getSymbolOfSourceFile" => SymbolOfSourceFile { file: &'a str }
    "getExportsOfModule" => ExportsOfModule { symbol: &'a SymbolRef }
    /// Only for symbols flagged `Alias`: the beta panics on a plain `default` export.
    "getAliasedSymbol" => AliasedSymbol { symbol: &'a SymbolRef }
    "getTypeOfSymbol" => TypeOfSymbol { symbol: &'a SymbolRef }
    "getTypesOfSymbols" => TypesOfSymbols { symbols: &'a [&'a SymbolRef] }
    "getDeclaredTypeOfSymbol" => DeclaredTypeOfSymbol { symbol: &'a SymbolRef }
    "getParentOfSymbolForChecker" => ParentOfSymbol { symbol: &'a SymbolRef }
    "isReadonlySymbol" => ReadonlySymbol { symbol: &'a SymbolRef }
    "getDocumentationComment" => DocumentationComment { symbol: &'a SymbolRef }
    "getJsDocTags" => JsDocTags { symbol: &'a SymbolRef }
    /// A declaration, by its node handle.
    "getSymbolOfDeclarationForChecker" => SymbolOfDeclaration { location: &'a str }
    /// Diagnostics take `files`; the singular `file` is ignored and checks the whole program.
    "getSemanticDiagnostics" => SemanticDiagnostics { files: &'a [&'a str] }
    "getSignaturesOfType" => SignaturesOfType { #[serde(rename = "type")] type_id: u64, kind: u8 }
    "getBaseTypes" => BaseTypes { #[serde(rename = "type")] type_id: u64 }
    "getPropertiesOfType" => PropertiesOfType { #[serde(rename = "type")] type_id: u64 }
    "getIndexInfosOfType" => IndexInfosOfType { #[serde(rename = "type")] type_id: u64 }
    "getTypeArguments" => TypeArguments { #[serde(rename = "type")] type_id: u64 }
    /// Printed with `TypeFormatFlags`, or without them as the checker prints by default.
    "typeToString" => TypeToString {
        #[serde(rename = "type")] type_id: u64,
        #[serde(skip_serializing_if = "Option::is_none")] flags: Option<u64>,
    }
    // A type or signature, by the handle the checker gave it.
    "getSymbolOfType" => SymbolOfType { #[serde(rename = "objectId")] object_id: u64 }
    "getTargetOfType" => TargetOfType { #[serde(rename = "objectId")] object_id: u64 }
    "getTypesOfType" => TypesOfType { #[serde(rename = "objectId")] object_id: u64 }
    "getAliasSymbolOfType" => AliasSymbolOfType { #[serde(rename = "objectId")] object_id: u64 }
    "getAliasTypeArgumentsOfType" => AliasTypeArgumentsOfType { #[serde(rename = "objectId")] object_id: u64 }
    "getDefaultFromTypeParameter" => DefaultFromTypeParameter { #[serde(rename = "objectId")] object_id: u64 }
    "getTypeParametersOfSignature" => TypeParametersOfSignature { #[serde(rename = "objectId")] object_id: u64 }
    "getParametersOfSignature" => ParametersOfSignature { #[serde(rename = "objectId")] object_id: u64 }
    "getReturnTypeOfSignature" => ReturnTypeOfSignature { #[serde(rename = "objectId")] object_id: u64 }
}

/// A question's parameters: the snapshot and project, then the fields of what it asks.
#[derive(Serialize)]
struct Scoped<'a> {
    project: &'a str,
    snapshot: u64,
    #[serde(flatten)]
    ask: Ask<'a>,
}

impl Request for Scoped<'_> {
    fn method(&self) -> &'static str {
        self.ask.method()
    }
}

/// One request of a batch.
#[derive(Serialize)]
struct Batched<'a> {
    method: &'static str,
    params: Scoped<'a>,
}

impl<'a> Batched<'a> {
    fn of(params: Scoped<'a>) -> Self {
        Batched { method: params.method(), params }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchRequests<'a, 'b> {
    requests: &'b [Batched<'a>],
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation_token: Option<&'b str>,
}

impl Request for BatchRequests<'_, '_> {
    fn method(&self) -> &'static str {
        "batchRequests"
    }
}

/// Starts the session; it takes no parameters, written `null`.
#[derive(Serialize)]
struct Initialize;

impl Request for Initialize {
    fn method(&self) -> &'static str {
        "initialize"
    }
}

#[derive(Serialize)]
struct ParseConfigFile<'a> {
    file: &'a str,
}

impl Request for ParseConfigFile<'_> {
    fn method(&self) -> &'static str {
        "parseConfigFile"
    }
}

#[derive(Serialize)]
struct Release {
    snapshot: u64,
}

impl Request for Release {
    fn method(&self) -> &'static str {
        "release"
    }

    fn frees_memory(&self) -> bool {
        true
    }
}

/// What a new snapshot changes from the one it is made from, or, sent to `createSnapshot`, from nothing.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Changes<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    file_system: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_notifications: Option<FileNotifications<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    create_programs: Option<[NewProgram<'a>; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reconfigure_programs: Option<[Reconfigured<'a>; 1]>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    ensure_programs: bool,
}

#[derive(Serialize)]
struct FileNotifications<'a> {
    changed: &'a [String],
    deleted: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewProgram<'a> {
    root_files: &'a [String],
    compiler_options: &'a Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reconfigured<'a> {
    id: &'a str,
    root_files: &'a [String],
    compiler_options: &'a Value,
}

#[derive(Serialize)]
#[serde(transparent)]
struct CreateSnapshot<'a>(Changes<'a>);

impl Request for CreateSnapshot<'_> {
    fn method(&self) -> &'static str {
        "createSnapshot"
    }
}

#[derive(Serialize)]
struct UpdateSnapshot<'a> {
    snapshot: u64,
    changes: Changes<'a>,
}

impl Request for UpdateSnapshot<'_> {
    fn method(&self) -> &'static str {
        "updateSnapshot"
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(from = "SymbolWire")]
pub(crate) struct SymbolInfo {
    /// The full reference the checker wants back to name this symbol; opaque to us.
    pub reference: SymbolRef,
    /// The name the source writes: the checker's escaped name with the escaping removed (see
    /// [`unescape_leading_underscores`]).
    pub name: String,
    /// What the source writes for a member the checker keys by an internal name (`[Symbol.iterator]`, `#type`); None
    /// for every other. Read off the escaped name: once unescaped, the quoted member `"__#8@#type"` (`___#8@#type`)
    /// looks like the private member `#type` (`__#8@#type`).
    pub member: Option<String>,
    pub flags: u64,
    pub declarations: Vec<String>,
    /// The symbol it is a member of: the interface or type literal that declares a property. Absent for a property a
    /// mapped type or an intersection made (`Omit<Base, "x">`, `A & B`), whose declarations still name the original.
    pub parent: Option<CompactSymbol>,
}

impl SymbolInfo {
    /// The name of this member as the source writes it.
    pub(crate) fn member_name(&self) -> &str {
        self.member.as_deref().unwrap_or(&self.name)
    }
}

/// A symbol as the checker answers it, its name still escaped.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolWire {
    reference: SymbolRef,
    name: String,
    flags: u64,
    #[serde(default)]
    declarations: Vec<String>,
    #[serde(default)]
    parent: Option<CompactSymbol>,
}

impl From<SymbolWire> for SymbolInfo {
    fn from(wire: SymbolWire) -> Self {
        let member = internal_member_name(&wire.name);
        let name = unescape_leading_underscores(&wire.name).to_string();
        Self {
            reference: wire.reference,
            name,
            member,
            flags: wire.flags,
            declarations: wire.declarations,
            parent: wire.parent,
        }
    }
}

/// The checker keys a property named by a symbol `__@<name>@<symbol id>`; the source writes `[Symbol.iterator]` or
/// `[brand]`. An ES-private member is `__#<symbol id>@#<name>`; the source writes `#<name>`. The id is the checker's
/// and does not belong in an answer. Takes the escaped name: a name the source writes starting with two underscores
/// has a third, so it never matches.
fn internal_member_name(escaped: &str) -> Option<String> {
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
    if let Some((id, private)) = escaped.strip_prefix("__#").and_then(|rest| rest.split_once("@#"))
        && !id.is_empty()
        && id.chars().all(|c| c.is_ascii_digit())
    {
        return Some(format!("#{private}"));
    }
    let (symbol, _id) = escaped.strip_prefix("__@").and_then(|rest| rest.rsplit_once('@'))?;
    Some(if WELL_KNOWN.contains(&symbol) { format!("[Symbol.{symbol}]") } else { format!("[{symbol}]") })
}

/// A JSDoc tag as `getJsDocTags` renders it: `@since 2.0` is `since` and `"2.0"`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct JsDocTag {
    pub name: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// A symbol as another response embeds it: the id, and the node id of the file that owns it. The full symbol is one
/// call away (`getSymbolOfType`, `getAliasSymbolOfType`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub(crate) struct CompactSymbol {
    pub id: u64,
    #[serde(default)]
    pub file: Option<String>,
}

/// The checker keeps a symbol's name in an escaped form: a name that starts with two underscores gets a third
/// (`escapeLeadingUnderscores`), so that it cannot collide with its own internal names such as `__call`. The source
/// writes `__size`, the checker answers `___size`. A name with fewer than three leading underscores is unchanged, and
/// so are the internal names, which start with exactly two.
pub(crate) fn unescape_leading_underscores(name: &str) -> &str {
    if name.starts_with("___") { name.strip_prefix('_').unwrap_or(name) } else { name }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TypeInfo {
    pub id: u64,
    pub flags: u64,
    #[serde(default)]
    pub object_flags: u64,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub is_tuple_type: bool,
    /// The checker's name for an intrinsic type (`"any"`, `"string"`, `"error"`). It alone tells a written `any` from
    /// the error type of an unresolved name: both carry the `Any` flag, and the error type has an id per site.
    #[serde(default)]
    pub intrinsic_name: Option<String>,
    /// `TupleElementFlags` of a tuple type's elements: required 1, optional 2, rest 4, variadic 8.
    #[serde(default)]
    pub element_flags: Vec<u64>,
    /// A tuple type written `readonly [...]`.
    #[serde(default)]
    pub readonly: bool,
    /// One declaration handle per element of a tuple whose elements are labeled, `[head: string]`; empty otherwise.
    #[serde(default)]
    pub labeled_element_declarations: Vec<String>,
    /// The text pieces of a template literal type, around its holes.
    #[serde(default)]
    pub texts: Vec<String>,
    /// The `this` type of a class or interface, which the checker also appends to a reference's type arguments.
    #[serde(default)]
    pub is_this_type: bool,
    /// The symbol that declares an object type; absent for primitives and unions.
    #[serde(default)]
    pub symbol: Option<CompactSymbol>,
    /// The alias this type was written through, `Size` in `type Size = "sm" | "lg"`.
    #[serde(default)]
    pub alias_symbol: Option<CompactSymbol>,
    /// Ids of the alias's type arguments; `Rpc::alias_type_arguments` fetches them.
    #[serde(default)]
    pub alias_type_arguments: Vec<u64>,
    /// The keys a mapped type ranges over (`keyof Conditions` in `{ [K in keyof Conditions]?: V }`), as the checker's
    /// type handle; absent for a type that is not a mapped type.
    #[serde(default)]
    pub constraint_type: Option<u64>,
    /// The `as` clause of a mapped type, which renames its keys; absent when it has none.
    #[serde(default)]
    pub name_type: Option<u64>,
}

impl TypeInfo {
    /// The checker's error type: what a name that does not resolve is typed as.
    pub(crate) fn is_error(&self) -> bool {
        self.flags & flag::ANY != 0 && self.intrinsic_name.as_deref() == Some("error")
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignatureInfo {
    pub id: u64,
    /// `SignatureFlags`; see [`flag::SIGNATURE_HAS_REST_PARAMETER`].
    #[serde(default)]
    pub flags: u64,
    /// The signature's own type parameters (ids; `Rpc::type_parameters` fetches them).
    #[serde(default)]
    pub type_parameters: Vec<u64>,
    /// One entry per declared parameter, `this` excluded; `Rpc::parameters` fetches them.
    #[serde(default)]
    pub parameters: Vec<serde::de::IgnoredAny>,
}

/// What an object type holds, as one batch answers it.
pub(crate) struct ObjectMembers {
    pub calls: Vec<SignatureInfo>,
    pub constructs: Vec<SignatureInfo>,
    pub properties: Vec<SymbolInfo>,
    pub indexes: Vec<IndexInfo>,
}

/// One index signature of an object type, `[key: string]: T`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexInfo {
    pub key_type: TypeInfo,
    pub value_type: TypeInfo,
    #[serde(default)]
    pub is_readonly: bool,
}

/// `SignatureKind` as the checker's `getSignaturesOfType` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SignatureKind {
    Call = 0,
    Construct = 1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchPage {
    responses: Vec<BatchAnswer>,
    #[serde(default)]
    continuation_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BatchAnswer {
    /// The answer's text, parsed by the caller into the shape it expects; absent reads as `null`.
    #[serde(default)]
    result: Option<Box<RawValue>>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnapshotResponse {
    snapshot: u64,
    projects: Vec<ProjectResponse>,
}

#[derive(Debug, Deserialize)]
struct ProjectResponse {
    id: String,
}

pub(crate) mod flag {
    pub(crate) const ANY: u64 = 1;
    pub(crate) const UNKNOWN: u64 = 2;
    pub(crate) const UNDEFINED: u64 = 4;
    pub(crate) const NULL: u64 = 8;
    pub(crate) const VOID: u64 = 16;
    pub(crate) const STRING: u64 = 32;
    pub(crate) const NUMBER: u64 = 64;
    pub(crate) const BIG_INT: u64 = 128;
    pub(crate) const BOOLEAN: u64 = 256;
    pub(crate) const ES_SYMBOL: u64 = 512;
    pub(crate) const STRING_LITERAL: u64 = 1024;
    pub(crate) const NUMBER_LITERAL: u64 = 2048;
    pub(crate) const BIG_INT_LITERAL: u64 = 4096;
    pub(crate) const BOOLEAN_LITERAL: u64 = 8192;
    pub(crate) const UNIQUE_ES_SYMBOL: u64 = 16384;
    pub(crate) const ENUM_LITERAL: u64 = 32768;
    pub(crate) const NON_PRIMITIVE: u64 = 131072;
    pub(crate) const NEVER: u64 = 262144;
    pub(crate) const TYPE_PARAMETER: u64 = 524288;
    pub(crate) const OBJECT: u64 = 1048576;
    pub(crate) const INDEX: u64 = 2097152;
    pub(crate) const TEMPLATE_LITERAL: u64 = 4194304;
    pub(crate) const STRING_MAPPING: u64 = 8388608;
    pub(crate) const SUBSTITUTION: u64 = 16777216;
    pub(crate) const INDEXED_ACCESS: u64 = 33554432;
    pub(crate) const CONDITIONAL: u64 = 67108864;
    pub(crate) const UNION: u64 = 134217728;
    pub(crate) const INTERSECTION: u64 = 268435456;
    pub(crate) const SYMBOL_INTERFACE: u64 = 64;
    pub(crate) const SYMBOL_ALIAS: u64 = 2097152;
    pub(crate) const SYMBOL_OPTIONAL: u64 = 16777216;
    /// `SignatureFlags.HasRestParameter`: the signature's last parameter is a rest parameter. The beta sets it for
    /// `(...args: T[]) => R` and for `(a?: A, ...rest: B[]) => R`, and leaves every other signature at 0.
    pub(crate) const SIGNATURE_HAS_REST_PARAMETER: u64 = 1;
    /// `ObjectFlags`: a class or interface type, an instantiation of one (a reference), a tuple, a mapped type.
    pub(crate) const OBJECT_CLASS: u64 = 1;
    pub(crate) const OBJECT_INTERFACE: u64 = 2;
    pub(crate) const OBJECT_REFERENCE: u64 = 4;
    pub(crate) const OBJECT_MAPPED: u64 = 32;
    /// `TypeFormatFlags` for `typeToString`: `NoTruncation`, `UseAliasDefinedOutsideCurrentScope`,
    /// `AllowUniqueESSymbolType` (the last two are what the checker uses when no flags are given) and `InTypeAlias`.
    pub(crate) const PRINT_EXPANDED: u64 = 1 | 16384 | 1_048_576 | 8_388_608;
    /// `TupleElementFlags`.
    pub(crate) const ELEMENT_OPTIONAL: u64 = 2;
    pub(crate) const ELEMENT_REST: u64 = 4;
    pub(crate) const ELEMENT_VARIADIC: u64 = 8;
}

tokio::task_local! {
    /// The requests the component being answered has sent, so a debug line can say what each one cost.
    pub(crate) static REQUESTS: std::sync::Arc<std::sync::atomic::AtomicU64>;
}

/// One checker session bound to a snapshot and project.
pub(crate) struct Rpc {
    client: Client,
    deadline: Duration,
    snapshot: u64,
    project: String,
    /// The compiler options the program was built with, for a snapshot that adds files to it ([`Rpc::layered`]).
    options: Value,
    /// Every call and its answer, when the caller asked to record a replay fixture; shared with a layered snapshot.
    recording: Option<Arc<Mutex<Vec<Value>>>>,
    calls: Arc<Mutex<BTreeMap<&'static str, u64>>>,
    /// The snapshots of the other lanes [`Rpc::open_programs`] opened beside this one, which [`Rpc::release`] on this
    /// one releases too, unless [`Rpc::release_lanes`] released them already.
    others: Mutex<Vec<u64>>,
}

/// A diagnostic the checker reports for a file: where it starts and what it says.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDiagnostic {
    /// The file the error is in, which is not always the file asked about.
    pub file_name: String,
    pub start_position: Position,
    /// `DiagnosticCategory`: warning 0, error 1, suggestion 2, message 3.
    pub category: u8,
    /// The message; a chain of messages arrives as an object, of which only the first is shown.
    pub text: Value,
}

impl FileDiagnostic {
    pub(crate) fn is_error(&self) -> bool {
        self.category == 1
    }

    pub(crate) fn message(&self) -> String {
        match &self.text {
            Value::String(text) => text.clone(),
            other => other.get("messageText").and_then(Value::as_str).unwrap_or("a type error").to_string(),
        }
    }
}

/// A position in a file, zero-based.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct Position {
    pub line: usize,
}

impl Rpc {
    /// Initializes the `tsc` session and reads the compiler options, once per process. The beta panics on
    /// string-valued enum options such as `target: "es2022"`, so options come from `parseConfigFile`, which yields
    /// numeric ones.
    pub(crate) async fn prepare(
        client: &Client,
        deadline: Duration,
        tsconfig: Option<&str>,
    ) -> Result<Value, CallError> {
        let rpc = Rpc {
            client: client.clone(),
            deadline,
            snapshot: 0,
            project: String::new(),
            options: Value::Null,
            recording: None,
            calls: Arc::default(),
            others: Mutex::default(),
        };
        rpc.call(&Initialize).await?;
        Ok(match tsconfig {
            Some(file) => {
                let parsed: Value = decode(&rpc.call(&ParseConfigFile { file }).await?)?;
                parsed.get("options").cloned().unwrap_or_else(default_options)
            }
            None => default_options(),
        })
    }

    /// One snapshot of a program over `root_files` for each of `count` lanes (one when `count` is at most one), in a
    /// session it initializes first ([`Rpc::prepare`]), with one session bound to each, the `n`th asking down the
    /// checker's `n`th lane ([`Client::lane`]): a checker that answers requests about different snapshots at once
    /// ([`crate::checker`]) answers the sessions in parallel. Each lane has a snapshot of its own because the checker
    /// registers the symbols, types and signatures it hands out in one table per snapshot, behind one lock: two programs
    /// of one snapshot wait for each other on it. The first lane's snapshot is built from the disk; each other one is
    /// derived from it ([`Rpc::beside`]), so every program reads the same text of every file. They share the request
    /// counts and the recording; [`Rpc::release`] on the first releases every snapshot.
    pub(crate) async fn open_programs(
        client: Client,
        deadline: Duration,
        tsconfig: Option<&str>,
        root_files: &[String],
        record: bool,
        count: usize,
    ) -> Result<Vec<Rpc>, CallError> {
        let options = Rpc::prepare(&client, deadline, tsconfig).await?;
        let first = Rpc::open_with(client.lane(0), deadline, &options, root_files, record).await?;
        let mut lanes: Vec<Rpc> = Vec::with_capacity(count.max(1));
        for lane in 1..count {
            match first.beside(client.lane(lane), root_files).await {
                Ok(rpc) => lanes.push(rpc),
                Err(error) => {
                    // Nothing will hold the snapshots made so far to release them later.
                    first.release().await;
                    for opened in &lanes {
                        opened.release().await;
                    }
                    return Err(error);
                }
            }
        }
        let others = lanes.iter().map(|rpc| rpc.snapshot).collect();
        lanes.insert(0, Rpc { others: Mutex::new(others), ..first });
        Ok(lanes)
    }

    /// A snapshot of its own for a program over `root_files` beside this one's, asked through `client`: derived from
    /// this snapshot (`updateSnapshot` with `createPrograms`), it reads every file as this one holds it, from the files
    /// this snapshot has read and parsed, and does not go to the disk again. It shares the request counts and the
    /// recording; it is released on its own.
    async fn beside(&self, client: Client, root_files: &[String]) -> Result<Rpc, CallError> {
        let changes = Changes {
            create_programs: Some([NewProgram { root_files, compiler_options: &self.options }]),
            ensure_programs: true,
            ..Changes::default()
        };
        let response: SnapshotResponse =
            decode(&self.call(&UpdateSnapshot { snapshot: self.snapshot, changes }).await?)?;
        let rpc = Rpc {
            client,
            deadline: self.deadline,
            snapshot: response.snapshot,
            project: String::new(),
            options: self.options.clone(),
            recording: self.recording.clone(),
            calls: self.calls.clone(),
            others: Mutex::default(),
        };
        // The response lists the programs the update made or changed: the one created, not this one's.
        let Some(project) = response.projects.into_iter().find(|project| project.id != self.project) else {
            // Nothing will hold this snapshot to release it later.
            rpc.release().await;
            return Err(CallError::Rejected("updateSnapshot created no program".to_string()));
        };
        Ok(Rpc { project: project.id, ..rpc })
    }

    /// A snapshot over `root_files` in a session [`Rpc::prepare`] already initialized.
    pub(crate) async fn open_with(
        client: Client,
        deadline: Duration,
        options: &Value,
        root_files: &[String],
        record: bool,
    ) -> Result<Rpc, CallError> {
        let rpc = Rpc {
            client,
            deadline,
            snapshot: 0,
            project: String::new(),
            options: options.clone(),
            recording: record.then(Arc::default),
            calls: Arc::default(),
            others: Mutex::default(),
        };
        let program = NewProgram { root_files, compiler_options: options };
        rpc.created(Changes { create_programs: Some([program]), ..Changes::default() }).await
    }

    /// A snapshot of a program rooted at `files` alone, held in the checker's memory and never written to disk (see
    /// [`Rpc::layered`]), in a session [`Rpc::prepare`] already initialized.
    pub(crate) async fn open_in_memory(
        client: Client,
        deadline: Duration,
        options: &Value,
        files: &[(String, String)],
    ) -> Result<Rpc, CallError> {
        let session = Rpc {
            client,
            deadline,
            snapshot: 0,
            project: String::new(),
            options: options.clone(),
            recording: None,
            calls: Arc::default(),
            others: Mutex::default(),
        };
        let roots: Vec<String> = files.iter().map(|(path, _)| path.clone()).collect();
        let program = NewProgram { root_files: &roots, compiler_options: options };
        let changes = Changes {
            file_system: Some(in_memory_layer(files)),
            create_programs: Some([program]),
            ..Changes::default()
        };
        session.created(changes).await
    }

    /// The snapshot after this one with `files` added, held in the checker's memory and never written to disk (each is
    /// a root file, keyed by its absolute path), and this one's program rooted at `root_files` and them instead, built at
    /// once (`updateSnapshot` with `reconfigurePrograms`). It is derived from this snapshot, so it reads every other file
    /// as this one holds it, from the files this one has read and parsed, and holds one program: a fresh `createSnapshot`
    /// read and parsed every file again into a second whole program while this one's was still alive. The calls on it
    /// count, and are recorded, with this one's; it is released on its own, and this one stays alive until it is.
    pub(crate) async fn layered(&self, root_files: &[String], files: &[(String, String)]) -> Result<Rpc, CallError> {
        let mut roots = root_files.to_vec();
        roots.extend(files.iter().map(|(path, _)| path.clone()));
        let program = Reconfigured { id: &self.project, root_files: &roots, compiler_options: &self.options };
        let changes = Changes {
            file_system: Some(in_memory_layer(files)),
            reconfigure_programs: Some([program]),
            ensure_programs: true,
            ..Changes::default()
        };
        let response: SnapshotResponse =
            decode(&self.call(&UpdateSnapshot { snapshot: self.snapshot, changes }).await?)?;
        // The response lists the projects the update replaced: the reconfigured one, which may keep its id.
        let mut projects = response.projects.into_iter().map(|project| project.id);
        let project = match projects.len() {
            0 => self.project.clone(),
            1 => projects.next().unwrap_or_default(),
            _ => match projects.find(|id| *id == self.project) {
                Some(id) => id,
                None => {
                    // Nothing will hold this snapshot to release it later.
                    self.call(&Release { snapshot: response.snapshot }).await.ok();
                    return Err(CallError::Rejected(
                        "updateSnapshot changed programs other than the one reconfigured".to_string(),
                    ));
                }
            },
        };
        Ok(Rpc {
            client: self.client.clone(),
            deadline: self.deadline,
            snapshot: response.snapshot,
            project,
            options: self.options.clone(),
            recording: self.recording.clone(),
            calls: self.calls.clone(),
            others: Mutex::default(),
        })
    }

    /// The snapshot after this one with `changed` and `deleted` read again from the disk and, given `roots`, its program
    /// rooted there instead, built at once (`updateSnapshot` with `ensurePrograms`). Files the program has not changed
    /// are kept as parsed: the checker re-reads only what it is told about, and a synthetic program sees even that
    /// only when it is rebuilt. The calls on it count with this one's; this snapshot stays alive until it is released.
    pub(crate) async fn updated(
        &self,
        changed: &[String],
        deleted: &[String],
        roots: Option<&[String]>,
    ) -> Result<Rpc, CallError> {
        let changes = Changes {
            file_notifications: (!changed.is_empty() || !deleted.is_empty())
                .then_some(FileNotifications { changed, deleted }),
            reconfigure_programs: roots
                .map(|root_files| [Reconfigured { id: &self.project, root_files, compiler_options: &self.options }]),
            ensure_programs: true,
            ..Changes::default()
        };
        let response: SnapshotResponse =
            decode(&self.call(&UpdateSnapshot { snapshot: self.snapshot, changes }).await?)?;
        // The response lists only the projects it replaced; the synthetic program keeps its id.
        let project = response.projects.into_iter().next().map_or_else(|| self.project.clone(), |project| project.id);
        Ok(Rpc {
            client: self.client.clone(),
            deadline: self.deadline,
            snapshot: response.snapshot,
            project,
            options: self.options.clone(),
            recording: self.recording.clone(),
            calls: self.calls.clone(),
            others: Mutex::default(),
        })
    }

    /// Runs `work` on the checker thread this session asks ([`Client::run_on_thread`]), given this session as that
    /// thread asks it, in place; each request is bounded by this session's deadline.
    pub(crate) async fn run_on_thread<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(Arc<Rpc>) -> T + Send + 'static,
    ) -> Result<T, CallError> {
        let this = Arc::clone(self);
        let run = self.client.run_on_thread(self.deadline, move |client| work(Arc::new(this.through(client))));
        run.await.map_err(classify)
    }

    /// This session asked through `client`: the same snapshot and project, counts and recording.
    fn through(&self, client: Client) -> Rpc {
        Rpc {
            client,
            deadline: self.deadline,
            snapshot: self.snapshot,
            project: self.project.clone(),
            options: self.options.clone(),
            recording: self.recording.clone(),
            calls: self.calls.clone(),
            others: Mutex::default(),
        }
    }

    /// The snapshot this session asks about.
    #[cfg(test)]
    pub(crate) fn snapshot_id(&self) -> u64 {
        self.snapshot
    }

    /// Every file the snapshot's program read, as the checker names them.
    pub(crate) async fn source_file_names(&self) -> Result<Vec<String>, CallError> {
        self.scoped(Ask::SourceFileNames {}).await
    }

    /// This session bound to the snapshot `createSnapshot` makes from `params`, and to its one project.
    async fn created(mut self, changes: Changes<'_>) -> Result<Rpc, CallError> {
        let response: SnapshotResponse = decode(&self.call(&CreateSnapshot(changes)).await?)?;
        self.snapshot = response.snapshot;
        let Some(project) = response.projects.into_iter().next() else {
            // Nothing will hold this snapshot to release it later.
            self.release().await;
            return Err(CallError::Rejected("createSnapshot returned no project".to_string()));
        };
        self.project = project.id;
        Ok(self)
    }

    /// Releases the snapshot, and those of the lanes opened beside it ([`Rpc::open_programs`]).
    pub(crate) async fn release(&self) {
        let lanes = self.take_lanes();
        for snapshot in std::iter::once(self.snapshot).chain(lanes) {
            self.call(&Release { snapshot }).await.ok();
        }
    }

    /// Releases the snapshots of the lanes opened beside this one ([`Rpc::open_programs`]) and keeps this one's: what
    /// they hold (each lane's program and its checker's caches) is freed before the work that follows the lanes. Each is
    /// released once, here or by [`Rpc::release`], never by both. Nothing may ask a lane after.
    pub(crate) async fn release_lanes(&self) {
        for snapshot in self.take_lanes() {
            self.call(&Release { snapshot }).await.ok();
        }
    }

    /// The lanes' snapshots not yet released, which are then no longer this one's to release.
    fn take_lanes(&self) -> Vec<u64> {
        std::mem::take(&mut *self.others.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// The recorded calls as a JSON array, or `None` when recording was off.
    pub(crate) fn recording(&self) -> Option<Value> {
        let log = self.recording.as_ref()?.lock().ok()?;
        Some(Value::Array(log.clone()))
    }

    /// Requests sent so far, by method.
    /// The requests inside a batch count under their own methods and in all as `inBatches`, and the batch once as
    /// `batchRequests`. It is a profile, not a count of round trips: a batch and its requests are both in it.
    pub(crate) fn calls(&self) -> BTreeMap<&'static str, u64> {
        self.calls.lock().map(|calls| calls.clone()).unwrap_or_default()
    }

    /// One request, its answer as the JSON text it arrived as: [`decode`] parses it once, into the shape the caller
    /// expects. The parameters are written straight to the text the checker reads.
    async fn call(&self, params: &impl Request) -> Result<Box<RawValue>, CallError> {
        let method = params.method();
        if !params.frees_memory() {
            crate::heap::step()?;
        }
        if let Ok(mut calls) = self.calls.lock() {
            *calls.entry(method).or_default() += 1;
        }
        REQUESTS.try_with(|count| count.fetch_add(1, std::sync::atomic::Ordering::Relaxed)).ok();
        let params = serde_json::to_vec(params)
            .map_err(|error| CallError::Fatal(format!("failed to serialize a request: {error}")))?;
        let Some(log) = &self.recording else {
            return self.client.request_raw(method, params, self.deadline).await.map_err(classify);
        };
        let sent: Value = serde_json::from_slice(&params).unwrap_or(Value::Null);
        let answer = self.client.request_raw(method, params, self.deadline).await.map_err(classify)?;
        let (params, result): (Value, Value) = (sent, decode(&answer)?);
        if let Ok(mut log) = log.lock() {
            log.push(json!({ "method": method, "params": params, "result": result }));
        }
        Ok(answer)
    }

    async fn scoped<T: DeserializeOwned>(&self, ask: Ask<'_>) -> Result<T, CallError> {
        decode(&self.scoped_text(ask).await?)
    }

    /// A question about this snapshot and project, its answer left as text.
    async fn scoped_text(&self, ask: Ask<'_>) -> Result<Box<RawValue>, CallError> {
        self.call(&self.about(ask)).await
    }

    fn about<'a>(&'a self, ask: Ask<'a>) -> Scoped<'a> {
        Scoped { project: &self.project, snapshot: self.snapshot, ask }
    }

    pub(crate) async fn source_file_symbol(&self, file: &str) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped(Ask::SymbolOfSourceFile { file }).await
    }

    pub(crate) async fn exports_of_module(&self, module: &SymbolRef) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped(Ask::ExportsOfModule { symbol: module }).await
    }

    /// See [`Ask::AliasedSymbol`].
    pub(crate) async fn aliased_symbol(&self, symbol: &SymbolRef) -> Result<SymbolInfo, CallError> {
        self.scoped(Ask::AliasedSymbol { symbol }).await
    }

    pub(crate) async fn type_of_symbol(&self, symbol: &SymbolRef) -> Result<TypeInfo, CallError> {
        self.scoped(Ask::TypeOfSymbol { symbol }).await
    }

    pub(crate) async fn types_of_symbols(&self, symbols: &[&SymbolRef]) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped(Ask::TypesOfSymbols { symbols }).await
    }

    /// The call signatures of a type, or with `SignatureKind::Construct` its construct signatures.
    pub(crate) async fn signatures(&self, type_id: u64, kind: SignatureKind) -> Result<Vec<SignatureInfo>, CallError> {
        self.scoped(Ask::SignaturesOfType { type_id, kind: kind as u8 }).await
    }

    pub(crate) async fn type_parameters(&self, signature: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped(Ask::TypeParametersOfSignature { object_id: signature }).await
    }

    /// The type a type parameter defaults to (`"div"` in `E extends ElementType = "div"`); none when it declares none.
    pub(crate) async fn default_of_type_parameter(&self, type_id: u64) -> Result<Option<TypeInfo>, CallError> {
        self.scoped(Ask::DefaultFromTypeParameter { object_id: type_id }).await
    }

    /// The type a type alias or interface declares, `T` in `type A = T`.
    pub(crate) async fn declared_type_of_symbol(&self, symbol: &SymbolRef) -> Result<TypeInfo, CallError> {
        self.scoped(Ask::DeclaredTypeOfSymbol { symbol }).await
    }

    /// The type errors in a file of the program.
    pub(crate) async fn semantic_diagnostics(&self, file: &str) -> Result<Vec<FileDiagnostic>, CallError> {
        self.scoped(Ask::SemanticDiagnostics { files: &[file] }).await
    }

    /// The symbol that declares a type: the interface, class, enum member or unique symbol behind it.
    pub(crate) async fn symbol_of_type(&self, type_id: u64) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped(Ask::SymbolOfType { object_id: type_id }).await
    }

    /// The types a class or interface extends, instantiated as it names them (`Omit<HTMLAttributes<HTMLDivElement>, "color">`);
    /// none for a type that is not a class or interface.
    pub(crate) async fn base_types(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        let bases: Option<Vec<TypeInfo>> = self.scoped(Ask::BaseTypes { type_id }).await?;
        Ok(bases.unwrap_or_default())
    }

    pub(crate) async fn alias_type_arguments(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped(Ask::AliasTypeArgumentsOfType { object_id: type_id }).await
    }

    /// The generic type a reference instantiates; for a tuple reference, the tuple type that holds its element flags.
    pub(crate) async fn target_of_type(&self, type_id: u64) -> Result<TypeInfo, CallError> {
        self.scoped(Ask::TargetOfType { object_id: type_id }).await
    }

    pub(crate) async fn parent_of_symbol(&self, symbol: &SymbolRef) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped(Ask::ParentOfSymbol { symbol }).await
    }

    /// Whether each property is declared `readonly`, in one round trip.
    pub(crate) async fn readonly_symbols(&self, symbols: &[&SymbolRef]) -> Result<Vec<bool>, CallError> {
        let requests = symbols.iter().map(|symbol| Ask::ReadonlySymbol { symbol }).collect();
        self.batch(requests).await?.iter().map(|answer| decode(answer)).collect()
    }

    /// Several requests in one round trip. A request the checker rejects fails the batch: nothing here asks for one
    /// that may legitimately fail.
    pub(crate) async fn batch(&self, requests: Vec<Ask<'_>>) -> Result<Vec<Box<RawValue>>, CallError> {
        let wanted = requests.len();
        if let Ok(mut calls) = self.calls.lock() {
            for ask in &requests {
                *calls.entry(ask.method()).or_default() += 1;
            }
            *calls.entry("inBatches").or_default() += wanted as u64;
        }
        let scoped: Vec<Batched<'_>> = requests.into_iter().map(|ask| Batched::of(self.about(ask))).collect();
        let mut answers = Vec::with_capacity(wanted);
        let mut token: Option<String> = None;
        loop {
            let params = BatchRequests { requests: &scoped, continuation_token: token.as_deref() };
            let page: BatchPage = decode(&self.call(&params).await?)?;
            for response in page.responses {
                if let Some(error) = response.error {
                    return Err(CallError::Rejected(error));
                }
                answers.push(response.result.unwrap_or_else(|| RawValue::NULL.to_owned()));
            }
            match page.continuation_token {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        if answers.len() != wanted {
            return Err(CallError::Rejected(format!("a batch of {wanted} requests got {} answers", answers.len())));
        }
        Ok(answers)
    }

    pub(crate) async fn parameters(&self, signature: u64) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped(Ask::ParametersOfSignature { object_id: signature }).await
    }

    pub(crate) async fn return_type(&self, signature: u64) -> Result<TypeInfo, CallError> {
        self.scoped(Ask::ReturnTypeOfSignature { object_id: signature }).await
    }

    pub(crate) async fn properties(&self, type_id: u64) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped(Ask::PropertiesOfType { type_id }).await
    }

    /// The members of a union, intersection or template literal, as the text the checker answered (a list of types).
    pub(crate) async fn members_text(&self, type_id: u64) -> Result<Box<RawValue>, CallError> {
        self.scoped_text(Ask::TypesOfType { object_id: type_id }).await
    }

    /// The type arguments of a reference, as the text the checker answered (a list of types).
    pub(crate) async fn type_arguments_text(&self, type_id: u64) -> Result<Box<RawValue>, CallError> {
        self.scoped_text(Ask::TypeArguments { type_id }).await
    }

    /// The call and construct signatures, the properties and the index signatures of a type, in one round trip.
    pub(crate) async fn object_members(&self, type_id: u64) -> Result<ObjectMembers, CallError> {
        let requests = vec![
            Ask::SignaturesOfType { type_id, kind: SignatureKind::Call as u8 },
            Ask::SignaturesOfType { type_id, kind: SignatureKind::Construct as u8 },
            Ask::PropertiesOfType { type_id },
            Ask::IndexInfosOfType { type_id },
        ];
        let mut answers = self.batch(requests).await?.into_iter();
        let mut next = || answers.next().ok_or_else(|| CallError::Rejected("a short batch".to_string()));
        Ok(ObjectMembers {
            calls: decode(&next()?)?,
            constructs: decode(&next()?)?,
            properties: decode(&next()?)?,
            indexes: decode(&next()?)?,
        })
    }

    pub(crate) async fn alias_symbol(&self, type_id: u64) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped(Ask::AliasSymbolOfType { object_id: type_id }).await
    }

    pub(crate) async fn type_to_string(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped(Ask::TypeToString { type_id, flags: None }).await
    }

    /// The type as written inside a type alias declaration, and never cut short: an alias at the top is printed as what
    /// it stands for (`(value?: string) => void`, not `Handler`), and a long type is printed to its end.
    pub(crate) async fn type_to_string_expanded(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped(Ask::TypeToString { type_id, flags: Some(flag::PRINT_EXPANDED) }).await
    }
}

fn default_options() -> Value {
    // Numeric enum values: JSX preserve, ESNext modules, bundler resolution, ES2022.
    json!({ "jsx": 1, "module": 99, "moduleResolution": 100, "target": 9, "strict": true, "skipLibCheck": true, "noEmit": true })
}

/// The `fileSystem` of a request that adds `files` in the checker's memory, keyed by absolute path, over the disk.
fn in_memory_layer(files: &[(String, String)]) -> Value {
    let layer: serde_json::Map<String, Value> =
        files.iter().map(|(path, text)| (path.clone(), Value::String(text.clone()))).collect();
    json!({ "kind": "layer", "files": layer })
}

/// An answer parsed from its text straight into `T`, in one pass.
pub(crate) fn decode<T: DeserializeOwned>(answer: &RawValue) -> Result<T, CallError> {
    serde_json::from_str(answer.get())
        .map_err(|error| CallError::Rejected(format!("unexpected response shape: {error}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    use caliper_docgen_core::pipeline::{PipelineOptions, extract};
    use caliper_docgen_core::types::semantic::SemanticOutcome;
    use camino::{Utf8Path, Utf8PathBuf};

    use crate::checker::{Backend, Failure, Served};
    use crate::{SemanticRequest, answer_on, root_files, targets_of};

    #[test]
    fn semantic_diagnostics_checks_only_the_requested_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let valid = root.join("valid.ts").to_string();
        let invalid = root.join("invalid.ts").to_string();
        let files = [
            (valid.clone(), "export const valid: string = 'ok';".to_string()),
            (invalid.clone(), "export const invalid: string = 1;".to_string()),
        ];
        let checker = Backend::start(&root, 1).unwrap();
        let runtime = multi_thread();
        runtime.block_on(async {
            let deadline = Duration::from_secs(30);
            let client = checker.client();
            let options = Rpc::prepare(&client, deadline, None).await.unwrap();
            let rpc = Rpc::open_in_memory(client, deadline, &options, &files).await.unwrap();
            let diagnostics = |items: Vec<FileDiagnostic>| {
                items.into_iter().map(|d| (d.file_name.clone(), d.start_position.line, d.message())).collect::<Vec<_>>()
            };

            assert_eq!(diagnostics(rpc.semantic_diagnostics(&valid).await.unwrap()), []);
            assert_eq!(
                diagnostics(rpc.semantic_diagnostics(&invalid).await.unwrap()),
                [(invalid, 0, "Type 'number' is not assignable to type 'string'.".to_string())]
            );
            rpc.release().await;
        });
        runtime.block_on(checker.shutdown());
    }

    fn multi_thread() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    }

    /// Components over shared types whose props extend an interface the last file augments (`declare global`).
    fn lanes_project() -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::Builder::new().prefix("Lanes").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        std::fs::write(
            root.join("types.ts"),
            "export type Size = 'sm' | 'md' | 'lg';\nexport interface Base {\n  /** The id. */\n  id: string;\n  size?: Size;\n}\n\
             declare global { interface Augmented { fromTypes?: number } }\n",
        )
        .unwrap();
        for index in 0..6 {
            let augments =
                if index == 5 { "declare global { interface Augmented { fromLastFile?: boolean } }\n" } else { "" };
            std::fs::write(
                root.join(format!("C{index}.tsx")),
                format!(
                    "import type {{ Base, Size }} from './types';\n{augments}\
                     export interface C{index}Props extends Base, Augmented {{ own{index}: number; pick?: (size: Size) => void }}\n\
                     export function C{index}(props: C{index}Props) {{ return null; }}\n"
                ),
            )
            .unwrap();
        }
        (dir, root)
    }

    /// Asks `lanes` programs on a checker with as many lanes about the project's components: their snapshots, the
    /// answer, and whether each snapshot was still held after the first program was released.
    fn answered_on_lanes(root: &Utf8Path, lanes: usize) -> (Vec<u64>, SemanticOutcome, Vec<bool>) {
        let output = extract(&PipelineOptions { src_dirs: vec![root.to_owned()], ..PipelineOptions::default() });
        let request = SemanticRequest::new(root.to_owned(), targets_of(&output));
        let files = root_files(&request);
        let checker = Backend::start(root, lanes).unwrap();
        let runtime = multi_thread();
        let deadline = Duration::from_secs(120);
        let (snapshots, outcome, held) = runtime.block_on(async {
            let programs = Rpc::open_programs(checker.client(), deadline, None, &files, false, lanes).await.unwrap();
            let programs: Vec<Arc<Rpc>> = programs.into_iter().map(Arc::new).collect();
            let snapshots: Vec<u64> = programs.iter().map(|program| program.snapshot).collect();
            let outcome = answer_on(&programs, &request, &files).await.unwrap();
            programs[0].release().await;
            let (client, mut held) = (checker.client(), Vec::new());
            for snapshot in &snapshots {
                // Releasing a snapshot the checker no longer holds fails, as releasing an unknown one does.
                let params = json!({ "snapshot": snapshot }).to_string().into_bytes();
                held.push(client.request_raw("release", params, deadline).await.is_ok());
            }
            (snapshots, outcome, held)
        });
        runtime.block_on(checker.shutdown());
        (snapshots, outcome, held)
    }

    /// Each lane asks a snapshot of its own, derived from the first, and three lanes give the answer one gives: every
    /// program holds every root file, so what the last file declares reaches the components the other lanes answer.
    /// Releasing the first program releases every lane's snapshot. On the base the lanes shared one snapshot.
    #[test]
    fn each_lane_has_a_snapshot_of_its_own_and_they_answer_as_one_program_does() {
        let (_dir, root) = lanes_project();

        let (one_snapshots, one, one_held) = answered_on_lanes(&root, 1);
        let (snapshots, three, held) = answered_on_lanes(&root, 3);

        assert!(one.diagnostics.is_empty() && one.fallbacks.is_empty(), "{one:?}");
        assert_eq!(one.components.len(), 6);
        for component in &one.components {
            let names: Vec<&str> = component.props.iter().map(|prop| prop.name.as_str()).collect();
            assert!(names.contains(&"fromLastFile") && names.contains(&"fromTypes"), "{names:?}");
        }
        assert_eq!(one, three);
        assert_eq!(one_snapshots.len(), 1);
        assert_eq!(snapshots.len(), 3);
        let distinct: std::collections::BTreeSet<u64> = snapshots.iter().copied().collect();
        assert_eq!(distinct.len(), 3, "each lane has a snapshot of its own: {snapshots:?}");
        assert_eq!(
            (one_held, held),
            (vec![false], vec![false; 3]),
            "the first program's release releases every lane's"
        );
    }

    /// The generic components of a run on two lanes are answered in a snapshot derived from the first lane's, whose one
    /// program is reconfigured to take the probe file, not in a second program built from the disk beside both lanes';
    /// and the second lane's snapshot is released before that snapshot is made. The instantiation still forms. On the
    /// base the run made two `createSnapshot`s, the second rooted afresh at every root file, while both lanes lived.
    #[test]
    fn instantiations_are_answered_in_a_snapshot_derived_from_the_first_lane_after_the_others_are_released() {
        let root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic");
        let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
        let request = SemanticRequest::new(root.clone(), targets_of(&output));
        let files = root_files(&request);
        let tsconfig = root.join("tsconfig.json");
        let checker = Backend::start(&root, 2).unwrap();
        let runtime = multi_thread();
        let deadline = Duration::from_secs(120);
        let (outcome, lanes, recording, releases) = runtime.block_on(async {
            let programs =
                Rpc::open_programs(checker.client(), deadline, Some(tsconfig.as_str()), &files, true, 2).await.unwrap();
            let programs: Vec<Arc<Rpc>> = programs.into_iter().map(Arc::new).collect();
            let lanes: Vec<(u64, String)> =
                programs.iter().map(|program| (program.snapshot, program.project.clone())).collect();
            let outcome = answer_on(&programs, &request, &files).await.unwrap();
            let recording = programs[0].recording().unwrap();
            programs[0].release().await;
            let releases = programs[0].calls().get("release").copied();
            (outcome, lanes, recording, releases)
        });
        runtime.block_on(checker.shutdown());

        // A generic component comes back only from its instantiation: one that did not form is a fallback.
        let answered: Vec<&str> = outcome.components.iter().map(|component| component.key.as_str()).collect();
        assert!(answered.contains(&"Poly") && answered.contains(&"Anchor"), "{outcome:?}");
        assert!(outcome.fallbacks.iter().all(|fallback| fallback.key == "Configured"), "{:?}", outcome.fallbacks);
        let calls: Vec<(&str, &Value)> = recording
            .as_array()
            .unwrap()
            .iter()
            .map(|call| (call["method"].as_str().unwrap(), &call["params"]))
            .collect();
        let created = calls.iter().filter(|(method, _)| *method == "createSnapshot").count();
        assert_eq!(created, 1, "only the first lane's program is built from the disk");
        let layered = calls
            .iter()
            .position(|(method, params)| *method == "updateSnapshot" && params["changes"].get("fileSystem").is_some())
            .expect("the probe file is added by an update");
        let (first, second) = (&lanes[0], &lanes[1]);
        let changes = &calls[layered].1["changes"];
        assert_eq!(calls[layered].1["snapshot"], json!(first.0), "derived from the first lane's snapshot");
        assert_eq!(changes.get("createPrograms"), None, "no second program beside the first's");
        let reconfigured = changes["reconfigurePrograms"].as_array().unwrap();
        assert_eq!(reconfigured.len(), 1);
        assert_eq!(reconfigured[0]["id"], json!(first.1));
        let roots = reconfigured[0]["rootFiles"].as_array().unwrap();
        assert_eq!(roots.len(), files.len() + 1, "every root file and the probe file: {roots:?}");
        let released_second = calls
            .iter()
            .position(|(method, params)| *method == "release" && params["snapshot"] == json!(second.0))
            .expect("the second lane's snapshot is released");
        assert!(released_second < layered, "the second lane is released before the instantiation snapshot is made");
        // The second lane early, the instantiation snapshot, and the first lane; a release is counted even when it fails,
        // as a second release of the second lane's would.
        assert_eq!(releases, Some(3), "each snapshot is released once");
        // A release that succeeded is recorded; the recording was read before the first lane's own release. The second
        // lane's and the instantiation snapshot's, each once.
        let mut released: Vec<u64> = calls
            .iter()
            .filter(|(method, _)| *method == "release")
            .filter_map(|(_, params)| params["snapshot"].as_u64())
            .collect();
        released.sort_unstable();
        let layered_snapshot = recording.as_array().unwrap()[0..]
            .iter()
            .filter(|call| call["method"] == "updateSnapshot" && call["params"]["changes"].get("fileSystem").is_some())
            .find_map(|call| call["result"]["snapshot"].as_u64())
            .unwrap();
        let mut expected = vec![second.0, layered_snapshot];
        expected.sort_unstable();
        assert_eq!(released, expected);
    }

    /// Stands in for a session: makes snapshot 1 of as many programs as asked on `createSnapshot` (so a run that asked
    /// all the lanes' programs of one snapshot would open), and fails or panics on `updateSnapshot`, which
    /// derives each other lane's; records the snapshots it was told to release, and whether it was closed.
    struct Underived {
        panics: bool,
        released: Arc<Mutex<Vec<u64>>>,
        closed: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Served for Underived {
        fn handle(&self, method: &str, params: &[u8]) -> Result<String, Failure> {
            match method {
                "initialize" => Ok("null".to_string()),
                "createSnapshot" => {
                    // As many programs as asked for.
                    let params: Value = serde_json::from_slice(params).unwrap();
                    let count = params["createPrograms"].as_array().map_or(1, Vec::len);
                    let projects: Vec<Value> = (0..count).map(|n| json!({ "id": format!("p{n}") })).collect();
                    Ok(json!({ "snapshot": 1, "projects": projects }).to_string())
                }
                "updateSnapshot" if self.panics => panic!("checker invariant broken"),
                "updateSnapshot" => Err(Failure { message: "a file nests too deep".to_string(), panicked: false }),
                "release" => {
                    let params: Value = serde_json::from_slice(params).unwrap();
                    self.released.lock().unwrap().push(params["snapshot"].as_u64().unwrap());
                    Ok("null".to_string())
                }
                other => panic!("unexpected {other}"),
            }
        }

        fn discard(&self, _method: &str, _answer: &str) {}

        fn close(&self) {
            self.closed.store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// A lane whose snapshot cannot be made fails the opening, and the snapshot made before it is released, since no
    /// one will hold it; one that panics stops the checker, which closes its session and every snapshot with it.
    #[test]
    fn a_lane_whose_snapshot_cannot_be_made_releases_the_one_made_before_it() {
        for panics in [false, true] {
            let released = Arc::new(Mutex::new(Vec::new()));
            let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let session = Underived { panics, released: released.clone(), closed: closed.clone() };
            let checker = Backend::spawn(2, move || session).unwrap();
            let runtime = multi_thread();

            let opened = runtime.block_on(Rpc::open_programs(
                checker.client(),
                Duration::from_secs(5),
                None,
                &["/p/a.ts".to_string()],
                false,
                2,
            ));

            assert!(opened.is_err(), "panics: {panics}");
            runtime.block_on(checker.shutdown_within(Duration::from_secs(5)));
            if panics {
                assert!(closed.load(std::sync::atomic::Ordering::Acquire), "the session is closed");
            } else {
                assert_eq!(*released.lock().unwrap(), [1], "the first lane's snapshot is released");
            }
        }
    }

    #[test]
    fn a_name_with_three_or_more_leading_underscores_loses_the_one_the_checker_added() {
        assert_eq!(unescape_leading_underscores("___size"), "__size");
        assert_eq!(unescape_leading_underscores("____triple"), "___triple");
        assert_eq!(unescape_leading_underscores("___0"), "__0");
    }

    #[test]
    fn other_names_are_unchanged() {
        for name in ["size", "_single", "__call", "__type", "default", "", "a___b"] {
            assert_eq!(unescape_leading_underscores(name), name);
        }
    }

    fn member_name_of(raw: &str) -> String {
        let symbol: SymbolInfo = serde_json::from_value(
            serde_json::json!({ "reference": { "kind": 1, "id": 4, "file": "7" }, "name": raw, "flags": 0 }),
        )
        .unwrap();
        symbol.member_name().to_string()
    }

    /// The checker keys an ES-private member `__#<symbol id>@#<name>` and a symbol-keyed one `__@<name>@<symbol id>`,
    /// and the id depends on the order the checker was asked: the name an answer carries is the one the source writes.
    #[test]
    fn a_checker_internal_member_name_is_the_one_the_source_writes() {
        assert_eq!(member_name_of("__#8@#type"), "#type");
        assert_eq!(member_name_of("__#10@#type"), "#type");
        assert_eq!(member_name_of("__@iterator@17"), "[Symbol.iterator]");
        assert_eq!(member_name_of("__@brand@3"), "[brand]");
    }

    /// A member the source writes with two leading underscores arrives with a third, so a quoted name shaped like an
    /// internal one stays the member it is.
    #[test]
    fn a_quoted_member_shaped_like_an_internal_name_keeps_its_name() {
        assert_eq!(member_name_of("___#8@#type"), "__#8@#type");
        assert_eq!(member_name_of("___@brand@3"), "__@brand@3");
        assert_eq!(member_name_of("size"), "size");
        assert_eq!(member_name_of("__#nothing"), "__#nothing");
        assert_eq!(member_name_of("__call"), "__call");
    }

    /// A symbol's reference goes back as the text it arrived as, inside the parameters a request or a batch writes.
    #[test]
    fn a_reference_is_written_back_as_it_arrived() {
        let symbol: SymbolInfo =
            serde_json::from_str(r#"{"reference":{"kind":1,"id":4,"file":"7"},"name":"a","flags":0}"#).unwrap();
        let scoped = |ask| Scoped { project: "p0", snapshot: 3, ask };

        let one = serde_json::to_string(&scoped(Ask::TypeOfSymbol { symbol: &symbol.reference })).unwrap();
        let both = [&symbol.reference, &symbol.reference];
        let many = serde_json::to_string(&scoped(Ask::TypesOfSymbols { symbols: &both })).unwrap();
        let requests = [
            Batched::of(scoped(Ask::ReadonlySymbol { symbol: &symbol.reference })),
            Batched::of(scoped(Ask::TypesOfType { object_id: 12 })),
        ];
        let page =
            serde_json::to_string(&BatchRequests { requests: &requests, continuation_token: Some("t") }).unwrap();

        assert_eq!(symbol.reference.text(), r#"{"kind":1,"id":4,"file":"7"}"#);
        assert_eq!(one, r#"{"project":"p0","snapshot":3,"symbol":{"kind":1,"id":4,"file":"7"}}"#);
        assert_eq!(
            many,
            r#"{"project":"p0","snapshot":3,"symbols":[{"kind":1,"id":4,"file":"7"},{"kind":1,"id":4,"file":"7"}]}"#
        );
        assert_eq!(
            page,
            r#"{"requests":[{"method":"isReadonlySymbol","params":{"project":"p0","snapshot":3,"symbol":{"kind":1,"id":4,"file":"7"}}},{"method":"getTypesOfType","params":{"project":"p0","snapshot":3,"objectId":12}}],"continuationToken":"t"}"#
        );
        assert_eq!(
            serde_json::to_string(&scoped(Ask::SourceFileNames {})).unwrap(),
            r#"{"project":"p0","snapshot":3}"#
        );
    }

    /// Stands in for a session: answers what a session needs to go on (a snapshot, compiler options, a batch's page),
    /// `null` to every question, and records each request as its method and the text of its parameters.
    struct Wire {
        sent: Arc<Mutex<Vec<(String, String)>>>,
        snapshots: std::sync::atomic::AtomicU64,
    }

    impl Served for Wire {
        fn handle(&self, method: &str, params: &[u8]) -> Result<String, Failure> {
            let text = String::from_utf8(params.to_vec()).unwrap();
            self.sent.lock().unwrap().push((method.to_string(), text.clone()));
            Ok(match method {
                "parseConfigFile" => r#"{"options":{"strict":true}}"#.to_string(),
                "createSnapshot" | "updateSnapshot" => {
                    let n = self.snapshots.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    json!({ "snapshot": n, "projects": [{ "id": format!("p{n}") }] }).to_string()
                }
                "batchRequests" => {
                    let params: Value = serde_json::from_str(&text).unwrap();
                    let count = params["requests"].as_array().unwrap().len();
                    json!({ "responses": vec![json!({ "result": null }); count] }).to_string()
                }
                _ => "null".to_string(),
            })
        }

        fn discard(&self, _method: &str, _answer: &str) {}

        fn close(&self) {}
    }

    /// Every request a session sends, in order, as its method and the text of its parameters: each method of [`Rpc`]
    /// called once with fixed values (twice where its parameters vary), the batches [`crate::declared`] sends included.
    fn sent_by_every_call() -> Vec<(String, String)> {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let session = Wire { sent: sent.clone(), snapshots: std::sync::atomic::AtomicU64::new(1) };
        let checker = Backend::spawn(2, move || session).unwrap();
        let runtime = multi_thread();
        let deadline = Duration::from_secs(5);
        runtime.block_on(async {
            let roots = ["/p/a.ts".to_string(), "/p/b.tsx".to_string()];
            let probe = [("/p/probe.ts".to_string(), "export {};".to_string())];
            let lanes = Rpc::open_programs(checker.client(), deadline, Some("/p/tsconfig.json"), &roots, false, 2)
                .await
                .unwrap();
            let rpc = &lanes[0];
            let symbol: SymbolRef = serde_json::from_str(r#"{"kind":1,"id":4,"file":"7"}"#).unwrap();
            let other: SymbolRef = serde_json::from_str(r#"{"kind":1,"id":5}"#).unwrap();
            rpc.source_file_names().await.ok();
            rpc.source_file_symbol("/p/a.ts").await.ok();
            rpc.exports_of_module(&symbol).await.ok();
            rpc.aliased_symbol(&symbol).await.ok();
            rpc.type_of_symbol(&symbol).await.ok();
            rpc.types_of_symbols(&[&symbol, &other]).await.ok();
            rpc.signatures(11, SignatureKind::Call).await.ok();
            rpc.signatures(11, SignatureKind::Construct).await.ok();
            rpc.type_parameters(12).await.ok();
            rpc.default_of_type_parameter(13).await.ok();
            rpc.declared_type_of_symbol(&symbol).await.ok();
            rpc.semantic_diagnostics("/p/b.tsx").await.ok();
            rpc.symbol_of_type(14).await.ok();
            rpc.base_types(15).await.ok();
            rpc.alias_type_arguments(16).await.ok();
            rpc.target_of_type(17).await.ok();
            rpc.parent_of_symbol(&symbol).await.ok();
            rpc.readonly_symbols(&[&symbol, &other]).await.ok();
            rpc.batch(vec![
                Ask::DocumentationComment { symbol: &symbol },
                Ask::JsDocTags { symbol: &symbol },
                Ask::SymbolOfDeclaration { location: "12.171./p/b.tsx" },
            ])
            .await
            .ok();
            rpc.batch(vec![Ask::ParentOfSymbol { symbol: &other }]).await.ok();
            rpc.parameters(18).await.ok();
            rpc.return_type(19).await.ok();
            rpc.properties(20).await.ok();
            rpc.members_text(21).await.ok();
            rpc.type_arguments_text(22).await.ok();
            rpc.object_members(23).await.ok();
            rpc.alias_symbol(24).await.ok();
            rpc.type_to_string(25).await.ok();
            rpc.type_to_string_expanded(26).await.ok();
            let layered = rpc.layered(&roots, &probe).await.unwrap();
            layered.release().await;
            let memory =
                Rpc::open_in_memory(checker.client(), deadline, &json!({ "strict": true }), &probe).await.unwrap();
            memory.release().await;
            let changed = ["/p/a.ts".to_string()];
            let deleted = ["/p/gone.ts".to_string()];
            rpc.updated(&changed, &deleted, Some(&roots)).await.unwrap().release().await;
            rpc.updated(&changed, &[], None).await.unwrap().release().await;
            rpc.updated(&[], &[], Some(&roots[..1])).await.unwrap().release().await;
            rpc.release_lanes().await;
            rpc.release().await;
        });
        runtime.block_on(checker.shutdown_within(Duration::from_secs(5)));
        sent.lock().unwrap().clone()
    }

    /// What [`sent_by_every_call`] sends, one request a line. A change here changes what the checker is sent.
    const WIRE: &str = r#"initialize null
parseConfigFile {"file":"/p/tsconfig.json"}
createSnapshot {"createPrograms":[{"rootFiles":["/p/a.ts","/p/b.tsx"],"compilerOptions":{"strict":true}}]}
updateSnapshot {"snapshot":1,"changes":{"createPrograms":[{"rootFiles":["/p/a.ts","/p/b.tsx"],"compilerOptions":{"strict":true}}],"ensurePrograms":true}}
getSourceFileNames {"project":"p1","snapshot":1}
getSymbolOfSourceFile {"project":"p1","snapshot":1,"file":"/p/a.ts"}
getExportsOfModule {"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}
getAliasedSymbol {"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}
getTypeOfSymbol {"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}
getTypesOfSymbols {"project":"p1","snapshot":1,"symbols":[{"kind":1,"id":4,"file":"7"},{"kind":1,"id":5}]}
getSignaturesOfType {"project":"p1","snapshot":1,"type":11,"kind":0}
getSignaturesOfType {"project":"p1","snapshot":1,"type":11,"kind":1}
getTypeParametersOfSignature {"project":"p1","snapshot":1,"objectId":12}
getDefaultFromTypeParameter {"project":"p1","snapshot":1,"objectId":13}
getDeclaredTypeOfSymbol {"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}
getSemanticDiagnostics {"project":"p1","snapshot":1,"files":["/p/b.tsx"]}
getSymbolOfType {"project":"p1","snapshot":1,"objectId":14}
getBaseTypes {"project":"p1","snapshot":1,"type":15}
getAliasTypeArgumentsOfType {"project":"p1","snapshot":1,"objectId":16}
getTargetOfType {"project":"p1","snapshot":1,"objectId":17}
getParentOfSymbolForChecker {"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}
batchRequests {"requests":[{"method":"isReadonlySymbol","params":{"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}},{"method":"isReadonlySymbol","params":{"project":"p1","snapshot":1,"symbol":{"kind":1,"id":5}}}]}
batchRequests {"requests":[{"method":"getDocumentationComment","params":{"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}},{"method":"getJsDocTags","params":{"project":"p1","snapshot":1,"symbol":{"kind":1,"id":4,"file":"7"}}},{"method":"getSymbolOfDeclarationForChecker","params":{"project":"p1","snapshot":1,"location":"12.171./p/b.tsx"}}]}
batchRequests {"requests":[{"method":"getParentOfSymbolForChecker","params":{"project":"p1","snapshot":1,"symbol":{"kind":1,"id":5}}}]}
getParametersOfSignature {"project":"p1","snapshot":1,"objectId":18}
getReturnTypeOfSignature {"project":"p1","snapshot":1,"objectId":19}
getPropertiesOfType {"project":"p1","snapshot":1,"type":20}
getTypesOfType {"project":"p1","snapshot":1,"objectId":21}
getTypeArguments {"project":"p1","snapshot":1,"type":22}
batchRequests {"requests":[{"method":"getSignaturesOfType","params":{"project":"p1","snapshot":1,"type":23,"kind":0}},{"method":"getSignaturesOfType","params":{"project":"p1","snapshot":1,"type":23,"kind":1}},{"method":"getPropertiesOfType","params":{"project":"p1","snapshot":1,"type":23}},{"method":"getIndexInfosOfType","params":{"project":"p1","snapshot":1,"type":23}}]}
getAliasSymbolOfType {"project":"p1","snapshot":1,"objectId":24}
typeToString {"project":"p1","snapshot":1,"type":25}
typeToString {"project":"p1","snapshot":1,"type":26,"flags":9453569}
updateSnapshot {"snapshot":1,"changes":{"fileSystem":{"kind":"layer","files":{"/p/probe.ts":"export {};"}},"reconfigurePrograms":[{"id":"p1","rootFiles":["/p/a.ts","/p/b.tsx","/p/probe.ts"],"compilerOptions":{"strict":true}}],"ensurePrograms":true}}
release {"snapshot":3}
createSnapshot {"fileSystem":{"kind":"layer","files":{"/p/probe.ts":"export {};"}},"createPrograms":[{"rootFiles":["/p/probe.ts"],"compilerOptions":{"strict":true}}]}
release {"snapshot":4}
updateSnapshot {"snapshot":1,"changes":{"fileNotifications":{"changed":["/p/a.ts"],"deleted":["/p/gone.ts"]},"reconfigurePrograms":[{"id":"p1","rootFiles":["/p/a.ts","/p/b.tsx"],"compilerOptions":{"strict":true}}],"ensurePrograms":true}}
release {"snapshot":5}
updateSnapshot {"snapshot":1,"changes":{"fileNotifications":{"changed":["/p/a.ts"],"deleted":[]},"ensurePrograms":true}}
release {"snapshot":6}
updateSnapshot {"snapshot":1,"changes":{"reconfigurePrograms":[{"id":"p1","rootFiles":["/p/a.ts"],"compilerOptions":{"strict":true}}],"ensurePrograms":true}}
release {"snapshot":7}
release {"snapshot":2}
release {"snapshot":1}
"#;

    /// Every request is written byte for byte as pinned: key order, absent optional keys and `null` included.
    #[test]
    fn every_request_writes_the_pinned_bytes() {
        let sent: String = sent_by_every_call().iter().map(|(method, params)| format!("{method} {params}\n")).collect();

        assert_eq!(sent, WIRE);
    }

    /// What tsrs reads of each request's parameters, and of the objects nested in them: the name, the keys a request
    /// must send for its answer to be the one asked, and the keys it may leave out. tsrs reads a missing key as its
    /// zero value and ignores one it does not know, so neither a misspelled key nor a missing one fails a request.
    /// This table is a copy, so it guards our types against what tsrs 0.24 read; the guard against tsrs changing is
    /// `each_parameter_that_chooses_the_answer_changes_it`, which asks a real checker and fails when a key is ignored.
    ///
    /// Copied from `params_fields` in `tsrs_api` 0.24.0 (`src/paramfields.rs`, `pub(crate)`, so not importable): each
    /// method's arm, then the nested structs by Go name. `getSymbolOfDeclarationForChecker` and
    /// `getParentOfSymbolForChecker` have no arm; their keys are what their handlers read (`src/checker/dispatch.rs`
    /// names them, `handlers_symbols.rs` reads `location` or `symbol` and, in `setup`, `snapshot` and `project`). To
    /// refresh on a tsrs upgrade, read each method's arm in the new version's file and copy its `name`s; a method sent
    /// that is missing here fails [`every_key_sent_is_one_tsrs_reads`].
    const TSRS_READS: &[(&str, &[&str], &[&str])] = &[
        ("initialize", &[], &[]),
        ("parseConfigFile", &["file"], &[]),
        (
            "createSnapshot",
            &["createPrograms"],
            &[
                "openProjects",
                "closeProjects",
                "openFiles",
                "closeFiles",
                "reconfigurePrograms",
                "removePrograms",
                "ensurePrograms",
                "fileNotifications",
                "fileSystem",
            ],
        ),
        ("updateSnapshot", &["snapshot", "changes"], &[]),
        ("release", &["snapshot"], &[]),
        ("batchRequests", &["requests"], &["continuationToken", "maxResponseBytesPerPage"]),
        ("getSourceFileNames", &["snapshot", "project"], &[]),
        ("getSymbolOfSourceFile", &["snapshot", "project", "file"], &[]),
        ("getSemanticDiagnostics", &["snapshot", "project", "files"], &[]),
        ("getTypeOfSymbol", &["snapshot", "project", "symbol"], &[]),
        ("getDeclaredTypeOfSymbol", &["snapshot", "project", "symbol"], &[]),
        ("getAliasedSymbol", &["snapshot", "project", "symbol"], &[]),
        ("getExportsOfModule", &["snapshot", "project", "symbol"], &[]),
        ("getJsDocTags", &["snapshot", "project", "symbol"], &[]),
        ("getDocumentationComment", &["snapshot", "project", "symbol"], &[]),
        ("isReadonlySymbol", &["snapshot", "project", "symbol"], &[]),
        ("getTypesOfSymbols", &["snapshot", "project", "symbols"], &[]),
        ("getSignaturesOfType", &["snapshot", "project", "type", "kind"], &[]),
        ("typeToString", &["snapshot", "project", "type"], &["location", "flags"]),
        ("getSymbolOfType", &["snapshot", "project", "objectId"], &[]),
        ("getTargetOfType", &["snapshot", "project", "objectId"], &[]),
        ("getTypesOfType", &["snapshot", "project", "objectId"], &[]),
        ("getAliasTypeArgumentsOfType", &["snapshot", "project", "objectId"], &[]),
        ("getAliasSymbolOfType", &["snapshot", "project", "objectId"], &[]),
        ("getDefaultFromTypeParameter", &["snapshot", "project", "objectId"], &[]),
        ("getTypeParametersOfSignature", &["snapshot", "project", "objectId"], &[]),
        ("getParametersOfSignature", &["snapshot", "project", "objectId"], &[]),
        ("getReturnTypeOfSignature", &["snapshot", "project", "objectId"], &[]),
        ("getBaseTypes", &["snapshot", "project", "type"], &[]),
        ("getPropertiesOfType", &["snapshot", "project", "type"], &[]),
        ("getIndexInfosOfType", &["snapshot", "project", "type"], &[]),
        ("getTypeArguments", &["snapshot", "project", "type"], &[]),
        ("getSymbolOfDeclarationForChecker", &["snapshot", "project", "location"], &[]),
        ("getParentOfSymbolForChecker", &["snapshot", "project", "symbol"], &[]),
        ("BatchRequest", &["method", "params"], &[]),
        (
            "CreateSnapshotParams",
            &[],
            &[
                "openProjects",
                "closeProjects",
                "openFiles",
                "closeFiles",
                "createPrograms",
                "reconfigurePrograms",
                "removePrograms",
                "ensurePrograms",
                "userPreferences",
                "prepareAutoImports",
                "fileNotifications",
                "fileSystem",
            ],
        ),
        ("CreateSnapshotProgramParams", &["rootFiles", "compilerOptions"], &["options"]),
        ("ReconfigureSnapshotProgramParams", &["id", "rootFiles", "compilerOptions"], &["options"]),
        ("FileNotifications", &[], &["invalidateAll", "changed", "created", "deleted"]),
    ];

    /// Checks `params`, sent as `name`, against what tsrs reads of it, and the objects nested in it against theirs;
    /// returns each mismatch.
    fn mismatches(name: &str, params: &Value) -> Vec<String> {
        let Some((_, required, optional)) = TSRS_READS.iter().find(|(known, _, _)| *known == name) else {
            return vec![format!("{name}: not in the table of what tsrs reads")];
        };
        let Some(object) = params.as_object() else {
            return if required.is_empty() && params.is_null() { vec![] } else { vec![format!("{name}: {params}")] };
        };
        let mut wrong: Vec<String> = Vec::new();
        for key in object.keys() {
            if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
                wrong.push(format!("{name}: sends `{key}`, which tsrs does not read"));
            }
        }
        for key in *required {
            if !object.contains_key(*key) {
                wrong.push(format!("{name}: does not send `{key}`"));
            }
        }
        let each = |value: &'_ Value, key: &str| value.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        // `createSnapshot` takes the changes as its parameters, `updateSnapshot` under `changes`; either way the
        // objects nested in the changes are checked once, at the level that holds them.
        if let Some(changes) = object.get("changes") {
            wrong.extend(mismatches("CreateSnapshotParams", changes));
        }
        if let Some(notified) = object.get("fileNotifications") {
            wrong.extend(mismatches("FileNotifications", notified));
        }
        for program in each(params, "createPrograms") {
            wrong.extend(mismatches("CreateSnapshotProgramParams", &program));
        }
        for program in each(params, "reconfigurePrograms") {
            wrong.extend(mismatches("ReconfigureSnapshotProgramParams", &program));
        }
        for request in each(params, "requests") {
            wrong.extend(mismatches("BatchRequest", &request));
            wrong.extend(mismatches(request["method"].as_str().unwrap_or_default(), &request["params"]));
        }
        wrong
    }

    /// Every request a session sends, and every request in its batches, sends exactly the keys tsrs reads for its
    /// method: none it ignores, none it needs left out. And every method [`Ask`] names is among them.
    #[test]
    fn every_key_sent_is_one_tsrs_reads() {
        let sent = sent_by_every_call();
        let wrong: Vec<String> = sent
            .iter()
            .flat_map(|(method, params)| mismatches(method, &serde_json::from_str(params).unwrap()))
            .collect();
        let mut asked: Vec<String> = Vec::new();
        for (method, params) in &sent {
            asked.push(method.clone());
            let params: Value = serde_json::from_str(params).unwrap();
            for request in params.get("requests").and_then(Value::as_array).into_iter().flatten() {
                asked.push(request["method"].as_str().unwrap().to_string());
            }
        }
        let unasked: Vec<&str> =
            ASKED.iter().copied().filter(|method| !asked.iter().any(|sent| sent == method)).collect();
        let mut named = ASKED.to_vec();
        named.sort_unstable();
        named.dedup();

        assert_eq!(wrong, Vec::<String>::new());
        assert_eq!(unasked, Vec::<&str>::new(), "a method no call sends");
        assert_eq!(named.len(), ASKED.len(), "a method named twice: {ASKED:?}");
    }

    /// How an answer is printed to be compared: each handle in it as what it names.
    #[derive(Clone, Copy)]
    enum Shape {
        /// A symbol, as its name.
        Symbol,
        /// A type, as `typeToString` prints it.
        Type,
        /// A signature, as its parameters' names and its return type.
        Signature,
        /// An index signature, as its key and value types.
        Index,
        /// A diagnostic, as its file, line and message.
        Diagnostic,
        /// An answer that holds no handle, as its JSON; a string as itself.
        Plain,
    }

    /// What chooses a method's answer besides the snapshot and project every question sends.
    enum Chosen {
        /// These parameters, each varied by a case of [`each_parameter_that_chooses_the_answer_changes_it`]; and how
        /// the answer is printed.
        By(&'static [&'static str], Shape),
        /// Nothing: the method has no parameter of its own. Says why.
        ScopeOnly(&'static str),
    }

    /// Every [`Ask`] variant, classified. There is no wildcard arm, so a new method does not compile until it is here.
    fn chosen(ask: &Ask<'_>) -> Chosen {
        use Chosen::{By, ScopeOnly};
        match ask {
            Ask::SourceFileNames {} => ScopeOnly("it lists every file of the program the snapshot and project name"),
            Ask::SymbolOfSourceFile { .. } => By(&["file"], Shape::Symbol),
            Ask::ExportsOfModule { .. } => By(&["symbol"], Shape::Symbol),
            Ask::AliasedSymbol { .. } => By(&["symbol"], Shape::Symbol),
            Ask::TypeOfSymbol { .. } => By(&["symbol"], Shape::Type),
            Ask::TypesOfSymbols { .. } => By(&["symbols"], Shape::Type),
            Ask::DeclaredTypeOfSymbol { .. } => By(&["symbol"], Shape::Type),
            Ask::ParentOfSymbol { .. } => By(&["symbol"], Shape::Symbol),
            Ask::ReadonlySymbol { .. } => By(&["symbol"], Shape::Plain),
            Ask::DocumentationComment { .. } => By(&["symbol"], Shape::Plain),
            Ask::JsDocTags { .. } => By(&["symbol"], Shape::Plain),
            Ask::SymbolOfDeclaration { .. } => By(&["location"], Shape::Symbol),
            Ask::SemanticDiagnostics { .. } => By(&["files"], Shape::Diagnostic),
            Ask::SignaturesOfType { .. } => By(&["type", "kind"], Shape::Signature),
            Ask::BaseTypes { .. } => By(&["type"], Shape::Type),
            Ask::PropertiesOfType { .. } => By(&["type"], Shape::Symbol),
            Ask::IndexInfosOfType { .. } => By(&["type"], Shape::Index),
            Ask::TypeArguments { .. } => By(&["type"], Shape::Type),
            Ask::TypeToString { .. } => By(&["type", "flags"], Shape::Plain),
            Ask::SymbolOfType { .. } => By(&["objectId"], Shape::Symbol),
            Ask::TargetOfType { .. } => By(&["objectId"], Shape::Type),
            Ask::TypesOfType { .. } => By(&["objectId"], Shape::Type),
            Ask::AliasSymbolOfType { .. } => By(&["objectId"], Shape::Symbol),
            Ask::AliasTypeArgumentsOfType { .. } => By(&["objectId"], Shape::Type),
            Ask::DefaultFromTypeParameter { .. } => By(&["objectId"], Shape::Type),
            Ask::TypeParametersOfSignature { .. } => By(&["objectId"], Shape::Type),
            Ask::ParametersOfSignature { .. } => By(&["objectId"], Shape::Symbol),
            Ask::ReturnTypeOfSignature { .. } => By(&["objectId"], Shape::Type),
        }
    }

    /// The methods [`chosen`] calls scope-only, one question each.
    fn scope_only() -> Vec<Ask<'static>> {
        vec![Ask::SourceFileNames {}]
    }

    /// The keys every question sends, which name the program asked rather than what is asked of it.
    const SCOPE: &[&str] = &["snapshot", "project"];

    /// Keys tsrs reads that no request sends, so no case varies them: the method, the key and why.
    const LEFT_OUT: &[(&str, &str, &str)] = &[(
        "typeToString",
        "location",
        "it scopes the printing to a node; we print at the top level, chosen by `flags`",
    )];

    /// A method's parameter varied: two questions that differ in `param` alone, and the answer each gets.
    struct Varied<'a> {
        param: &'static str,
        asks: [Ask<'a>; 2],
        answers: [&'static str; 2],
    }

    /// An answer as [`Shape`] prints it, with `root` cut from the file names in it; `none` for `null`.
    async fn printed(rpc: &Rpc, shape: Shape, answer: &RawValue, root: &str) -> String {
        let text = match decode::<Value>(answer).unwrap() {
            Value::Null => "none".to_string(),
            Value::Array(items) => {
                let mut each = Vec::new();
                for item in items {
                    each.push(printed_one(rpc, shape, item).await);
                }
                format!("[{}]", each.join(", "))
            }
            single => printed_one(rpc, shape, single).await,
        };
        text.replace(root, "")
    }

    async fn printed_one(rpc: &Rpc, shape: Shape, value: Value) -> String {
        let type_text = async |id| rpc.type_to_string(id).await.unwrap();
        match shape {
            Shape::Symbol => serde_json::from_value::<SymbolInfo>(value).unwrap().name,
            Shape::Type => type_text(serde_json::from_value::<TypeInfo>(value).unwrap().id).await,
            Shape::Signature => {
                let signature: SignatureInfo = serde_json::from_value(value).unwrap();
                let names: Vec<String> =
                    rpc.parameters(signature.id).await.unwrap().into_iter().map(|parameter| parameter.name).collect();
                let returns = rpc.return_type(signature.id).await.unwrap().id;
                format!("({}) => {}", names.join(", "), type_text(returns).await)
            }
            Shape::Index => {
                let index: IndexInfo = serde_json::from_value(value).unwrap();
                format!("[{}]: {}", type_text(index.key_type.id).await, type_text(index.value_type.id).await)
            }
            Shape::Diagnostic => {
                let diagnostic: FileDiagnostic = serde_json::from_value(value).unwrap();
                format!("{}:{} {}", diagnostic.file_name, diagnostic.start_position.line, diagnostic.message())
            }
            Shape::Plain => match value {
                Value::String(text) => text,
                other => other.to_string(),
            },
        }
    }

    /// For every method [`Ask`] sends and each parameter that chooses its answer ([`chosen`]), two questions that
    /// differ in that parameter alone get the two different answers written here from a real checker, asked alone and
    /// in one batch. tsrs reads a missing key as its zero value and ignores one it does not know, so a parameter it
    /// does not read gets one answer for both. The questions are built here; `every_request_writes_the_pinned_bytes`
    /// pins that each [`Rpc`] method builds them from its arguments. Also: every method is classified, every key tsrs reads of it is varied, scope or
    /// left out on purpose ([`LEFT_OUT`]), and every case's two questions differ in its parameter and nothing else.
    #[test]
    fn each_parameter_that_chooses_the_answer_changes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let [a, b, bad] = ["a.ts", "b.ts", "bad.ts"].map(|name| root.join(name).to_string());
        std::fs::write(
            &a,
            "export interface Base1 { base1: string }\n\
             export interface Base2 { base2: string }\n\
             /** Alpha docs. @since 1 */\n\
             export interface Alpha extends Base1 {\n  (x: number): string;\n  new (x: string): Alpha;\n  a: number;\n  \
             readonly ra: number;\n  [key: string]: unknown;\n}\n\
             /** Beta docs. @deprecated use Alpha */\n\
             export interface Beta extends Base2 {\n  (y: string): number;\n  b: string;\n  [index: number]: string;\n}\n\
             export declare const one: Alpha;\n\
             export declare const two: Beta;\n\
             export type Handler = (value?: string) => void;\n\
             export declare const handler: Handler;\n\
             export interface Box<T> { value: T }\n\
             export interface Pair<T> { first: T; second: T }\n\
             export declare const boxS: Box<string>;\n\
             export declare const boxN: Box<number>;\n\
             export declare const pairS: Pair<string>;\n\
             export type U1 = 'x' | 'y';\n\
             export type U2 = 'p' | 'q';\n\
             export type Wrap<T> = { w: T };\n\
             export declare const wrapS: Wrap<string>;\n\
             export declare const wrapN: Wrap<number>;\n\
             export declare function f1<T1 = string>(a1: T1): T1;\n\
             export declare function f2<T2 = number, V2 = boolean>(b2: T2): T2;\n",
        )
        .unwrap();
        std::fs::write(&b, "export { Alpha as AliasA, Beta as AliasB } from './a';\n").unwrap();
        std::fs::write(&bad, "export const bad: string = 1;\n").unwrap();
        let cut = format!("{root}/");
        let checker = Backend::start(&root, 1).unwrap();
        let runtime = multi_thread();
        let wrong = runtime.block_on(async {
            let deadline = Duration::from_secs(60);
            let client = checker.client();
            let options = Rpc::prepare(&client, deadline, None).await.unwrap();
            let roots = [a.clone(), b.clone(), bad.clone()];
            let rpc = Rpc::open_with(client, deadline, &options, &roots, false).await.unwrap();
            let module = async |file: &str| rpc.source_file_symbol(file).await.unwrap().unwrap().reference;
            let (a_module, b_module, bad_module) = (module(&a).await, module(&b).await, module(&bad).await);
            let exports = rpc.exports_of_module(&a_module).await.unwrap();
            let aliases = rpc.exports_of_module(&b_module).await.unwrap();
            let named = |symbols: &[SymbolInfo], name: &str| {
                let found = symbols.iter().find(|symbol| symbol.name == name);
                let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
                found.unwrap_or_else(|| panic!("no {name} among {names:?}")).clone()
            };
            let export = |name| named(&exports, name);
            let typed = async |name| rpc.type_of_symbol(&export(name).reference).await.unwrap().id;
            let declared = async |name| rpc.declared_type_of_symbol(&export(name).reference).await.unwrap().id;
            let (alpha, beta) = (export("Alpha"), export("Beta"));
            let (one, two) = (export("one").reference, export("two").reference);
            let (alias_a, alias_b) = (named(&aliases, "AliasA").reference, named(&aliases, "AliasB").reference);
            let (alpha_type, beta_type) = (declared("Alpha").await, declared("Beta").await);
            let (u1, u2) = (declared("U1").await, declared("U2").await);
            let (one_type, two_type, handler) = (typed("one").await, typed("two").await, typed("handler").await);
            let (box_s, box_n, pair_s) = (typed("boxS").await, typed("boxN").await, typed("pairS").await);
            let (wrap_s, wrap_n) = (typed("wrapS").await, typed("wrapN").await);
            let alpha_members = rpc.properties(alpha_type).await.unwrap();
            let beta_members = rpc.properties(beta_type).await.unwrap();
            let a_member = named(&alpha_members, "a").reference;
            let ra_member = named(&alpha_members, "ra").reference;
            let b_member = named(&beta_members, "b").reference;
            let signature = async |name| rpc.signatures(typed(name).await, SignatureKind::Call).await.unwrap()[0].id;
            let (f1, f2) = (signature("f1").await, signature("f2").await);
            let first_parameter = async |signature| rpc.type_parameters(signature).await.unwrap()[0].id;
            let (t1, t2) = (first_parameter(f1).await, first_parameter(f2).await);
            let (both, second) = ([&one, &two], [&two]);
            let (in_a, in_bad) = ([a.as_str()], [bad.as_str()]);
            let (call, construct) = (SignatureKind::Call as u8, SignatureKind::Construct as u8);
            let (alpha_symbol, beta_symbol) = (&alpha.reference, &beta.reference);
            let (alpha_declaration, beta_declaration) = (&alpha.declarations[0], &beta.declarations[0]);

            let cases = || {
                let case = |param, asks, answers| Varied { param, asks, answers };
                vec![
                    case(
                        "file",
                        [Ask::SymbolOfSourceFile { file: &a }, Ask::SymbolOfSourceFile { file: &b }],
                        [r#""a""#, r#""b""#],
                    ),
                    case(
                        "symbol",
                        [Ask::ExportsOfModule { symbol: &b_module }, Ask::ExportsOfModule { symbol: &bad_module }],
                        ["[AliasA, AliasB]", "[bad]"],
                    ),
                    case(
                        "symbol",
                        [Ask::AliasedSymbol { symbol: &alias_a }, Ask::AliasedSymbol { symbol: &alias_b }],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "symbol",
                        [Ask::TypeOfSymbol { symbol: &one }, Ask::TypeOfSymbol { symbol: &two }],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "symbols",
                        [Ask::TypesOfSymbols { symbols: &both }, Ask::TypesOfSymbols { symbols: &second }],
                        ["[Alpha, Beta]", "[Beta]"],
                    ),
                    case(
                        "symbol",
                        [
                            Ask::DeclaredTypeOfSymbol { symbol: alpha_symbol },
                            Ask::DeclaredTypeOfSymbol { symbol: beta_symbol },
                        ],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "symbol",
                        [Ask::ParentOfSymbol { symbol: &a_member }, Ask::ParentOfSymbol { symbol: &b_member }],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "symbol",
                        [Ask::ReadonlySymbol { symbol: &ra_member }, Ask::ReadonlySymbol { symbol: &a_member }],
                        ["true", "false"],
                    ),
                    case(
                        "symbol",
                        [
                            Ask::DocumentationComment { symbol: alpha_symbol },
                            Ask::DocumentationComment { symbol: beta_symbol },
                        ],
                        ["Alpha docs.", "Beta docs."],
                    ),
                    case(
                        "symbol",
                        [Ask::JsDocTags { symbol: alpha_symbol }, Ask::JsDocTags { symbol: beta_symbol }],
                        [r#"[{"name":"since","text":"1"}]"#, r#"[{"name":"deprecated","text":"use Alpha"}]"#],
                    ),
                    case(
                        "location",
                        [
                            Ask::SymbolOfDeclaration { location: alpha_declaration },
                            Ask::SymbolOfDeclaration { location: beta_declaration },
                        ],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "files",
                        [Ask::SemanticDiagnostics { files: &in_a }, Ask::SemanticDiagnostics { files: &in_bad }],
                        ["[]", "[bad.ts:0 Type 'number' is not assignable to type 'string'.]"],
                    ),
                    case(
                        "type",
                        [
                            Ask::SignaturesOfType { type_id: alpha_type, kind: call },
                            Ask::SignaturesOfType { type_id: beta_type, kind: call },
                        ],
                        ["[(x) => string]", "[(y) => number]"],
                    ),
                    case(
                        "kind",
                        [
                            Ask::SignaturesOfType { type_id: alpha_type, kind: call },
                            Ask::SignaturesOfType { type_id: alpha_type, kind: construct },
                        ],
                        ["[(x) => string]", "[(x) => Alpha]"],
                    ),
                    case(
                        "type",
                        [Ask::BaseTypes { type_id: alpha_type }, Ask::BaseTypes { type_id: beta_type }],
                        ["[Base1]", "[Base2]"],
                    ),
                    case(
                        "type",
                        [Ask::PropertiesOfType { type_id: alpha_type }, Ask::PropertiesOfType { type_id: beta_type }],
                        ["[a, ra, base1]", "[b, base2]"],
                    ),
                    case(
                        "type",
                        [Ask::IndexInfosOfType { type_id: alpha_type }, Ask::IndexInfosOfType { type_id: beta_type }],
                        ["[[string]: unknown]", "[[number]: string]"],
                    ),
                    case(
                        "type",
                        [Ask::TypeArguments { type_id: box_s }, Ask::TypeArguments { type_id: box_n }],
                        ["[string]", "[number]"],
                    ),
                    case(
                        "type",
                        [
                            Ask::TypeToString { type_id: one_type, flags: None },
                            Ask::TypeToString { type_id: two_type, flags: None },
                        ],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "flags",
                        [
                            Ask::TypeToString { type_id: handler, flags: None },
                            Ask::TypeToString { type_id: handler, flags: Some(flag::PRINT_EXPANDED) },
                        ],
                        ["Handler", "(value?: string | undefined) => void"],
                    ),
                    case(
                        "objectId",
                        [Ask::SymbolOfType { object_id: one_type }, Ask::SymbolOfType { object_id: two_type }],
                        ["Alpha", "Beta"],
                    ),
                    case(
                        "objectId",
                        [Ask::TargetOfType { object_id: box_s }, Ask::TargetOfType { object_id: pair_s }],
                        ["Box<T>", "Pair<T>"],
                    ),
                    case(
                        "objectId",
                        [Ask::TypesOfType { object_id: u1 }, Ask::TypesOfType { object_id: u2 }],
                        [r#"["x", "y"]"#, r#"["p", "q"]"#],
                    ),
                    case(
                        "objectId",
                        [Ask::AliasSymbolOfType { object_id: u1 }, Ask::AliasSymbolOfType { object_id: u2 }],
                        ["U1", "U2"],
                    ),
                    case(
                        "objectId",
                        [
                            Ask::AliasTypeArgumentsOfType { object_id: wrap_s },
                            Ask::AliasTypeArgumentsOfType { object_id: wrap_n },
                        ],
                        ["[string]", "[number]"],
                    ),
                    case(
                        "objectId",
                        [
                            Ask::DefaultFromTypeParameter { object_id: t1 },
                            Ask::DefaultFromTypeParameter { object_id: t2 },
                        ],
                        ["string", "number"],
                    ),
                    case(
                        "objectId",
                        [
                            Ask::TypeParametersOfSignature { object_id: f1 },
                            Ask::TypeParametersOfSignature { object_id: f2 },
                        ],
                        ["[T1]", "[T2, V2]"],
                    ),
                    case(
                        "objectId",
                        [Ask::ParametersOfSignature { object_id: f1 }, Ask::ParametersOfSignature { object_id: f2 }],
                        ["[a1]", "[b2]"],
                    ),
                    case(
                        "objectId",
                        [Ask::ReturnTypeOfSignature { object_id: f1 }, Ask::ReturnTypeOfSignature { object_id: f2 }],
                        ["T1", "T2"],
                    ),
                ]
            };

            let mut wrong: Vec<String> = Vec::new();
            let mut varied: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
            let mut classified: BTreeMap<&'static str, &'static [&'static str]> = BTreeMap::new();
            for (index, Varied { param, asks: [left, right], answers }) in cases().into_iter().enumerate() {
                let method = left.method();
                let at = format!("{method} `{param}`");
                let (keys, shape) = match chosen(&left) {
                    Chosen::By(keys, shape) => (keys, shape),
                    Chosen::ScopeOnly(reason) => {
                        wrong.push(format!("{at}: varied, but classified scope-only: {reason}"));
                        continue;
                    }
                };
                let (mut sent_left, mut sent_right) =
                    (serde_json::to_value(&left).unwrap(), serde_json::to_value(&right).unwrap());
                if right.method() != method || !keys.contains(&param) || sent_left.get(param) == sent_right.get(param) {
                    wrong.push(format!("{at}: the two questions do not differ in it: {sent_left} {sent_right}"));
                }
                sent_left.as_object_mut().unwrap().remove(param);
                sent_right.as_object_mut().unwrap().remove(param);
                if sent_left != sent_right {
                    wrong.push(format!("{at}: the two questions differ in more than it: {sent_left} {sent_right}"));
                }
                varied.entry(method).or_default().push(param);
                classified.insert(method, keys);
                let mut answered: Vec<Result<Box<RawValue>, String>> = Vec::new();
                for ask in [left, right] {
                    answered.push(rpc.scoped_text(ask).await.map_err(|error| error.to_string()));
                }
                let Some(Varied { asks: [left, right], .. }) = cases().into_iter().nth(index) else { unreachable!() };
                match rpc.batch(vec![left, right]).await {
                    Ok(answers) => answered.extend(answers.into_iter().map(Ok)),
                    Err(error) => answered.extend([Err(error.to_string()), Err(error.to_string())]),
                }
                let mut got = Vec::new();
                for answer in &answered {
                    got.push(match answer {
                        Ok(answer) => printed(&rpc, shape, answer, &cut).await,
                        Err(error) => format!("rejected: {error}"),
                    });
                }
                let expected: Vec<&str> = answers.iter().chain(&answers).copied().collect();
                if got != expected {
                    wrong.push(format!("{at}: answered {got:?} (alone, then in a batch), not {expected:?}"));
                }
            }
            for ask in scope_only() {
                match chosen(&ask) {
                    Chosen::ScopeOnly(_) => drop(classified.insert(ask.method(), &[])),
                    Chosen::By(..) => wrong.push(format!("{}: listed scope-only, but classified as not", ask.method())),
                }
            }
            for method in ASKED {
                let Some(keys) = classified.get(method) else {
                    wrong.push(format!("{method}: no case varies it, and it is not listed scope-only"));
                    continue;
                };
                for key in *keys {
                    if !varied.get(method).is_some_and(|params| params.contains(key)) {
                        wrong.push(format!("{method} `{key}`: chooses the answer, but no case varies it"));
                    }
                }
                let (_, required, optional) = TSRS_READS.iter().find(|(name, _, _)| name == method).unwrap();
                for key in required.iter().chain(*optional) {
                    let left_out = LEFT_OUT.iter().any(|(name, left, _)| name == method && left == key);
                    if !SCOPE.contains(key) && !keys.contains(key) && !left_out {
                        wrong.push(format!("{method} `{key}`: tsrs reads it, but it is neither varied nor left out"));
                    }
                }
            }
            rpc.release().await;
            wrong
        });
        runtime.block_on(checker.shutdown());

        assert_eq!(wrong, Vec::<String>::new());
    }

    #[test]
    fn a_symbol_arrives_with_the_name_the_source_writes() {
        let symbol: SymbolInfo =
            serde_json::from_value(json!({ "reference": {}, "name": "___size", "flags": 0 })).unwrap();

        assert_eq!(symbol.name, "__size");
    }
}
