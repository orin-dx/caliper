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

/// What a request asks about besides the snapshot and project [`Rpc`] adds: written as the fields of its parameters, a
/// symbol's reference as the text it arrived as.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum Asking<'a> {
    Symbol {
        symbol: &'a SymbolRef,
    },
    Symbols {
        symbols: &'a [&'a SymbolRef],
    },
    /// A declaration, by its node handle.
    Location {
        location: &'a str,
    },
    Type {
        #[serde(rename = "type")]
        type_id: u64,
    },
    Signatures {
        #[serde(rename = "type")]
        type_id: u64,
        kind: u8,
    },
    /// A type printed with `TypeFormatFlags`.
    Printed {
        #[serde(rename = "type")]
        type_id: u64,
        flags: u64,
    },
    /// A type or signature, by the handle the checker gave it.
    Object {
        #[serde(rename = "objectId")]
        object_id: u64,
    },
    File {
        file: &'a str,
    },
    Files {
        files: &'a [&'a str],
    },
    Nothing {},
}

/// A request's parameters: the snapshot and project, then the fields of what it asks.
#[derive(Serialize)]
struct Scoped<'a, E> {
    project: &'a str,
    snapshot: u64,
    #[serde(flatten)]
    extra: E,
}

/// One request of a batch.
#[derive(Serialize)]
struct Batched<'a, E> {
    method: &'static str,
    params: Scoped<'a, E>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchParams<'a, 'b, E> {
    requests: &'b [Batched<'a, E>],
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation_token: Option<&'b str>,
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
    /// What the answer carries besides the flags and declarations that [`Self::settled_readonly`] reads; None for an
    /// answer without check flags, which settles nothing. Kept raw so that a symbol nobody asks about is not classified.
    pub readonly: Option<ReadonlyEvidence>,
}

/// The parts of a symbol's answer that only [`SymbolInfo::settled_readonly`] reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReadonlyEvidence {
    check_flags: u64,
    value_declaration: bool,
}

impl SymbolInfo {
    /// The name of this member as the source writes it.
    pub(crate) fn member_name(&self) -> &str {
        self.member.as_deref().unwrap_or(&self.name)
    }

    /// What `isReadonlySymbol` answers for this symbol when its answer settles it ([`settled_readonly`]); None when
    /// only the checker can say.
    pub(crate) fn settled_readonly(&self) -> Option<bool> {
        let evidence = self.readonly?;
        settled_readonly(self.flags, evidence.check_flags, evidence.value_declaration, &self.declarations)
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
    /// Absent from an answer that does not carry it, which settles nothing ([`settled_readonly`]).
    check_flags: Option<u64>,
    /// Only its presence is read, so the handle is skipped, not kept.
    #[serde(default)]
    value_declaration: Option<serde::de::IgnoredAny>,
}

impl From<SymbolWire> for SymbolInfo {
    fn from(wire: SymbolWire) -> Self {
        let member = internal_member_name(&wire.name);
        let name = unescape_leading_underscores(&wire.name).to_string();
        let readonly = wire
            .check_flags
            .map(|check_flags| ReadonlyEvidence { check_flags, value_declaration: wire.value_declaration.is_some() });
        Self {
            reference: wire.reference,
            name,
            member,
            flags: wire.flags,
            declarations: wire.declarations,
            parent: wire.parent,
            readonly,
        }
    }
}

/// What the checker's `isReadonlySymbol` answers for a symbol, read off its flags, check flags, whether it has a value
/// declaration and its declarations' kinds; None when that depends on what the answer leaves out. The checker (tsrs
/// `is_readonly_symbol`, TypeScript's `isReadonlySymbol`) answers true for exactly:
/// - a transient symbol whose check flags say `Readonly` (a mapped, reverse-mapped or union property);
/// - a getter without a setter, and an enum member;
/// - a property whose value declaration has the `readonly` modifier: the answer has no modifiers, so asked;
/// - a variable declared `const`: the answer has no node flags, so asked;
/// - a symbol with an `Object.defineProperty` call among its declarations: settled false only when every declaration
///   is of a kind in [`NO_ASSIGNMENT_DECLARATION`].
///
/// A property's modifiers are read only off its value declaration (tsrs and TypeScript
/// `getDeclarationModifierFlagsFromSymbol`), so one without a value declaration has no `readonly` modifier.
fn settled_readonly(flags: u64, check_flags: u64, value_declaration: bool, declarations: &[String]) -> Option<bool> {
    #[cfg(test)]
    READONLY_CLASSIFIED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if check_flags & flag::CHECK_READONLY != 0
        || flags & flag::SYMBOL_GET_ACCESSOR != 0 && flags & flag::SYMBOL_SET_ACCESSOR == 0
        || flags & flag::SYMBOL_ENUM_MEMBER != 0
    {
        return Some(true);
    }
    // The answer omits `valueDeclaration` both when there is none and when it was not sent, so "none" is believed only
    // of a property the checker makes without one: a mapped, reverse-mapped, union or intersection property. Any other
    // check flag (`Instantiated` alone, `Late`) proves nothing about the declaration.
    let modifiers_unread = flags & flag::SYMBOL_PROPERTY != 0
        && (value_declaration || check_flags & flag::CHECK_MADE_WITHOUT_DECLARATION == 0);
    let constant_unread = flags & flag::SYMBOL_VARIABLE != 0;
    let declarations_known = declarations.iter().all(|declaration| {
        let kind = declaration.split('.').nth(1).and_then(|kind| kind.parse::<u32>().ok());
        kind.is_some_and(|kind| NO_ASSIGNMENT_DECLARATION.contains(&kind))
    });
    (!modifiers_unread && !constant_unread && declarations_known).then_some(false)
}

/// The syntax kinds (tsrs_ast 0.24.0 numbering, as node handles carry them) of the declarations that are not an
/// `Object.defineProperty` call: parameter, property signature and declaration, method signature and declaration, get
/// and set accessor, property assignment, shorthand property assignment. A kind not listed is asked.
///
/// The numbers are tsrs's, not ours: after a renumbering a listed number may name `CallExpression`, and a
/// `defineProperty` declaration would then be settled not readonly. So the test
/// `the_declaration_kinds_are_read_from_the_pinned_tsrs` fails on any tsrs version but the one they were read from,
/// and `a_settled_readonly_answer_is_the_checkers` pins each to a real declaration.
const NO_ASSIGNMENT_DECLARATION: [u32; 9] = [171, 173, 174, 175, 176, 179, 180, 304, 305];

/// How many symbols [`settled_readonly`] has classified in this process; nextest runs each test in a process of its own.
#[cfg(test)]
pub(crate) static READONLY_CLASSIFIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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
    /// `SymbolFlags`.
    pub(crate) const SYMBOL_VARIABLE: u64 = 1 | 2;
    pub(crate) const SYMBOL_PROPERTY: u64 = 4;
    pub(crate) const SYMBOL_ENUM_MEMBER: u64 = 8;
    pub(crate) const SYMBOL_GET_ACCESSOR: u64 = 32768;
    pub(crate) const SYMBOL_SET_ACCESSOR: u64 = 65536;
    /// `CheckFlags.Readonly`: a transient symbol made readonly.
    pub(crate) const CHECK_READONLY: u64 = 8;
    /// `CheckFlags.SyntheticProperty | SyntheticMethod | ReverseMapped | Mapped`: a property the checker makes for a
    /// union or intersection, a reverse-inferred mapped type or a mapped type.
    pub(crate) const CHECK_MADE_WITHOUT_DECLARATION: u64 = 2 | 4 | (1 << 16) | (1 << 21);
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
        rpc.call("initialize", &Value::Null).await?;
        Ok(match tsconfig {
            Some(file) => {
                let parsed: Value = decode(&rpc.call("parseConfigFile", &json!({ "file": file })).await?)?;
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
        let changes = json!({
            "createPrograms": [{ "rootFiles": root_files, "compilerOptions": self.options }],
            "ensurePrograms": true,
        });
        let response: SnapshotResponse =
            decode(&self.call("updateSnapshot", &json!({ "snapshot": self.snapshot, "changes": changes })).await?)?;
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
        rpc.created(json!({ "createPrograms": [{ "rootFiles": root_files, "compilerOptions": options }] })).await
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
        let layer = in_memory_layer(files);
        let roots: Vec<&String> = files.iter().map(|(path, _)| path).collect();
        session
            .created(json!({
                "fileSystem": layer,
                "createPrograms": [{ "rootFiles": roots, "compilerOptions": options }],
            }))
            .await
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
        let changes = json!({
            "fileSystem": in_memory_layer(files),
            "reconfigurePrograms": [{ "id": self.project, "rootFiles": roots, "compilerOptions": self.options }],
            "ensurePrograms": true,
        });
        let response: SnapshotResponse =
            decode(&self.call("updateSnapshot", &json!({ "snapshot": self.snapshot, "changes": changes })).await?)?;
        // The response lists the projects the update replaced: the reconfigured one, which may keep its id.
        let mut projects = response.projects.into_iter().map(|project| project.id);
        let project = match projects.len() {
            0 => self.project.clone(),
            1 => projects.next().unwrap_or_default(),
            _ => match projects.find(|id| *id == self.project) {
                Some(id) => id,
                None => {
                    // Nothing will hold this snapshot to release it later.
                    self.call("release", &json!({ "snapshot": response.snapshot })).await.ok();
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
    /// only when it is rebuilt. This snapshot stays alive until it is released.
    pub(crate) async fn updated(
        &self,
        changed: &[String],
        deleted: &[String],
        roots: Option<&[String]>,
    ) -> Result<Rpc, CallError> {
        let mut changes = serde_json::Map::new();
        if !changed.is_empty() || !deleted.is_empty() {
            changes.insert("fileNotifications".to_string(), json!({ "changed": changed, "deleted": deleted }));
        }
        if let Some(roots) = roots {
            changes.insert(
                "reconfigurePrograms".to_string(),
                json!([{ "id": self.project, "rootFiles": roots, "compilerOptions": self.options }]),
            );
        }
        changes.insert("ensurePrograms".to_string(), Value::Bool(true));
        let response: SnapshotResponse =
            decode(&self.call("updateSnapshot", &json!({ "snapshot": self.snapshot, "changes": changes })).await?)?;
        // The response lists only the projects it replaced; the synthetic program keeps its id.
        let project = response.projects.into_iter().next().map_or_else(|| self.project.clone(), |project| project.id);
        Ok(Rpc {
            client: self.client.clone(),
            deadline: self.deadline,
            snapshot: response.snapshot,
            project,
            options: self.options.clone(),
            recording: self.recording.clone(),
            calls: Arc::default(),
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
        self.scoped("getSourceFileNames", Asking::Nothing {}).await
    }

    /// This session bound to the snapshot `createSnapshot` makes from `params`, and to its one project.
    async fn created(mut self, params: Value) -> Result<Rpc, CallError> {
        let response: SnapshotResponse = decode(&self.call("createSnapshot", &params).await?)?;
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
            self.call("release", &json!({ "snapshot": snapshot })).await.ok();
        }
    }

    /// Releases the snapshots of the lanes opened beside this one ([`Rpc::open_programs`]) and keeps this one's: what
    /// they hold (each lane's program and its checker's caches) is freed before the work that follows the lanes. Each is
    /// released once, here or by [`Rpc::release`], never by both. Nothing may ask a lane after.
    pub(crate) async fn release_lanes(&self) {
        for snapshot in self.take_lanes() {
            self.call("release", &json!({ "snapshot": snapshot })).await.ok();
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
    async fn call(&self, method: &'static str, params: &impl Serialize) -> Result<Box<RawValue>, CallError> {
        // A release gives memory back, so it is sent past the heap budget too.
        if method != "release" {
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

    async fn scoped<T: DeserializeOwned>(&self, method: &'static str, extra: Asking<'_>) -> Result<T, CallError> {
        decode(&self.scoped_text(method, extra).await?)
    }

    /// A request about this snapshot and project, its answer left as text.
    async fn scoped_text(&self, method: &'static str, extra: Asking<'_>) -> Result<Box<RawValue>, CallError> {
        self.call(method, &Scoped { project: &self.project, snapshot: self.snapshot, extra }).await
    }

    pub(crate) async fn source_file_symbol(&self, file: &str) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getSymbolOfSourceFile", Asking::File { file }).await
    }

    pub(crate) async fn exports_of_module(&self, module: &SymbolRef) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped("getExportsOfModule", Asking::Symbol { symbol: module }).await
    }

    /// Only for symbols flagged `Alias`: the beta panics on a plain `default` export.
    pub(crate) async fn aliased_symbol(&self, symbol: &SymbolRef) -> Result<SymbolInfo, CallError> {
        self.scoped("getAliasedSymbol", Asking::Symbol { symbol }).await
    }

    pub(crate) async fn type_of_symbol(&self, symbol: &SymbolRef) -> Result<TypeInfo, CallError> {
        self.scoped("getTypeOfSymbol", Asking::Symbol { symbol }).await
    }

    pub(crate) async fn types_of_symbols(&self, symbols: &[&SymbolRef]) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getTypesOfSymbols", Asking::Symbols { symbols }).await
    }

    /// The call signatures of a type, or with `SignatureKind::Construct` its construct signatures.
    pub(crate) async fn signatures(&self, type_id: u64, kind: SignatureKind) -> Result<Vec<SignatureInfo>, CallError> {
        self.scoped("getSignaturesOfType", Asking::Signatures { type_id, kind: kind as u8 }).await
    }

    pub(crate) async fn type_parameters(&self, signature: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getTypeParametersOfSignature", Asking::Object { object_id: signature }).await
    }

    /// The type a type parameter defaults to (`"div"` in `E extends ElementType = "div"`); none when it declares none.
    pub(crate) async fn default_of_type_parameter(&self, type_id: u64) -> Result<Option<TypeInfo>, CallError> {
        self.scoped("getDefaultFromTypeParameter", Asking::Object { object_id: type_id }).await
    }

    /// The type a type alias or interface declares, `T` in `type A = T`.
    pub(crate) async fn declared_type_of_symbol(&self, symbol: &SymbolRef) -> Result<TypeInfo, CallError> {
        self.scoped("getDeclaredTypeOfSymbol", Asking::Symbol { symbol }).await
    }

    /// The type errors in a file of the program.
    pub(crate) async fn semantic_diagnostics(&self, file: &str) -> Result<Vec<FileDiagnostic>, CallError> {
        // Diagnostics take `files`; the singular `file` is ignored and checks the whole program.
        self.scoped("getSemanticDiagnostics", Asking::Files { files: &[file] }).await
    }

    /// The symbol that declares a type: the interface, class, enum member or unique symbol behind it.
    pub(crate) async fn symbol_of_type(&self, type_id: u64) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getSymbolOfType", Asking::Object { object_id: type_id }).await
    }

    /// The types a class or interface extends, instantiated as it names them (`Omit<HTMLAttributes<HTMLDivElement>, "color">`);
    /// none for a type that is not a class or interface.
    pub(crate) async fn base_types(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        let bases: Option<Vec<TypeInfo>> = self.scoped("getBaseTypes", Asking::Type { type_id }).await?;
        Ok(bases.unwrap_or_default())
    }

    pub(crate) async fn alias_type_arguments(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getAliasTypeArgumentsOfType", Asking::Object { object_id: type_id }).await
    }

    /// The generic type a reference instantiates; for a tuple reference, the tuple type that holds its element flags.
    pub(crate) async fn target_of_type(&self, type_id: u64) -> Result<TypeInfo, CallError> {
        self.scoped("getTargetOfType", Asking::Object { object_id: type_id }).await
    }

    pub(crate) async fn parent_of_symbol(&self, symbol: &SymbolRef) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getParentOfSymbolForChecker", Asking::Symbol { symbol }).await
    }

    /// Whether each property is declared `readonly`, in one round trip.
    pub(crate) async fn readonly_symbols(&self, symbols: &[&SymbolRef]) -> Result<Vec<bool>, CallError> {
        let requests = symbols.iter().map(|symbol| ("isReadonlySymbol", Asking::Symbol { symbol })).collect();
        self.batch(requests).await?.iter().map(|answer| decode(answer)).collect()
    }

    /// Several requests in one round trip. A request the checker rejects fails the batch: nothing here asks for one
    /// that may legitimately fail.
    pub(crate) async fn batch(
        &self,
        requests: Vec<(&'static str, Asking<'_>)>,
    ) -> Result<Vec<Box<RawValue>>, CallError> {
        let wanted = requests.len();
        if let Ok(mut calls) = self.calls.lock() {
            for (method, _) in &requests {
                *calls.entry(method).or_default() += 1;
            }
            *calls.entry("inBatches").or_default() += wanted as u64;
        }
        let scoped: Vec<Batched<'_, Asking<'_>>> = requests
            .into_iter()
            .map(|(method, extra)| Batched {
                method,
                params: Scoped { project: &self.project, snapshot: self.snapshot, extra },
            })
            .collect();
        let mut answers = Vec::with_capacity(wanted);
        let mut token: Option<String> = None;
        loop {
            let params = BatchParams { requests: &scoped, continuation_token: token.as_deref() };
            let page: BatchPage = decode(&self.call("batchRequests", &params).await?)?;
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
        self.scoped("getParametersOfSignature", Asking::Object { object_id: signature }).await
    }

    pub(crate) async fn return_type(&self, signature: u64) -> Result<TypeInfo, CallError> {
        self.scoped("getReturnTypeOfSignature", Asking::Object { object_id: signature }).await
    }

    pub(crate) async fn properties(&self, type_id: u64) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped("getPropertiesOfType", Asking::Type { type_id }).await
    }

    /// The members of a union, intersection or template literal, as the text the checker answered (a list of types).
    pub(crate) async fn members_text(&self, type_id: u64) -> Result<Box<RawValue>, CallError> {
        self.scoped_text("getTypesOfType", Asking::Object { object_id: type_id }).await
    }

    /// The type arguments of a reference, as the text the checker answered (a list of types).
    pub(crate) async fn type_arguments_text(&self, type_id: u64) -> Result<Box<RawValue>, CallError> {
        self.scoped_text("getTypeArguments", Asking::Type { type_id }).await
    }

    /// The call and construct signatures, the properties and the index signatures of a type, in one round trip.
    pub(crate) async fn object_members(&self, type_id: u64) -> Result<ObjectMembers, CallError> {
        let requests = vec![
            ("getSignaturesOfType", Asking::Signatures { type_id, kind: SignatureKind::Call as u8 }),
            ("getSignaturesOfType", Asking::Signatures { type_id, kind: SignatureKind::Construct as u8 }),
            ("getPropertiesOfType", Asking::Type { type_id }),
            ("getIndexInfosOfType", Asking::Type { type_id }),
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
        self.scoped("getAliasSymbolOfType", Asking::Object { object_id: type_id }).await
    }

    pub(crate) async fn type_to_string(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped("typeToString", Asking::Type { type_id }).await
    }

    /// The type as written inside a type alias declaration, and never cut short: an alias at the top is printed as what
    /// it stands for (`(value?: string) => void`, not `Handler`), and a long type is printed to its end.
    pub(crate) async fn type_to_string_expanded(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped("typeToString", Asking::Printed { type_id, flags: flag::PRINT_EXPANDED }).await
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

    /// Every form of property, of every kind of type that lists them: for each property whose answer settles
    /// `isReadonlySymbol` ([`SymbolInfo::settled_readonly`]), the checker answers the same, and which properties are
    /// settled true, settled false or asked is pinned by name. Also pins the syntax kinds [`NO_ASSIGNMENT_DECLARATION`]
    /// reads off the node handles to real declarations, and checks that an expando assignment's kind is not listed.
    #[test]
    fn a_settled_readonly_answer_is_the_checkers() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let file = root.join("forms.ts").to_string();
        let source = "\
            export interface I { a: string; readonly b: number; m(): void; get g(): number; get gs(): number; \
              set gs(v: number); set so(v: number); c?: boolean; 'quoted-key': 1; readonly [Symbol.iterator]: 1; }\n\
            export class K { x = 1; readonly y = 2; declare z: string; private p = 1; #q = 1; static s = 1; \
              static readonly t = 1; get gk() { return 1; } set gk(v) {} get only() { return 1; } \
              constructor(public pp: number, readonly rp: number) {} method() {} }\n\
            export type R = Readonly<I>;\n\
            export type P = Partial<I>;\n\
            export type Req = Required<Partial<R>>;\n\
            export type Mut = { -readonly [Key in keyof R]: R[Key] };\n\
            export type Pk = Pick<I, 'a' | 'b' | 'm' | 'g'>;\n\
            export type O = Omit<I, 'a'>;\n\
            export type Rec = Record<'u' | 'v', number>;\n\
            export type Ro = { readonly [Key in 'k1' | 'k2']: Key };\n\
            export type Renamed = { [Key in keyof I as `on${Capitalize<string & Key>}`]: I[Key] };\n\
            export type X = I & { readonly a: string; b: number; extra: 1 };\n\
            export type Y = R & { a: string };\n\
            export type U = I | { readonly a: string; b: number; m(): void; g: number; gs: number; c?: boolean };\n\
            export type Inst = InstanceType<typeof K>;\n\
            export type Tup = readonly [number, string];\n\
            export type MutTup = [number, ...string[]];\n\
            export type Str = String;\n\
            export type Arr = ReadonlyArray<number>;\n\
            export enum E { A, B = 'b' }\n\
            const short = 1;\n\
            export const ns = { lit: 1, readonly ro: 2, get gg() { return 1; }, set ss(v: number) {}, method() {}, \
              short, ['computed']: 1, ...{ spread: 1 } };\n\
            export const frozen = { a: 1, b: [1] } as const;\n\
            export const fn = Object.assign(() => 1, { attached: 1 });\n\
            export namespace N { export const c = 1; export let l = 2; export var v = 3; export function f() {} \
              export class Kn {} export enum En { A } }\n\
            export interface G<T> { readonly gr: T; gw: T }\n\
            export type Gi = G<number>;\n\
            declare function unbox<T>(boxed: { [Key in keyof T]: { v: T[Key] } }): T;\n\
            export const rev = unbox({ ra: { v: 1 }, rb: { v: 'b' } });\n\
            export function fx() { return 1; }\n\
            fx.dotted = 1;\n\
            fx['bracketed'] = 2;\n";
        let checker = Backend::start(&root, 1).unwrap();
        let runtime = multi_thread();
        let (compared, kinds) = runtime.block_on(async {
            let deadline = Duration::from_secs(60);
            let client = checker.client();
            let options = Rpc::prepare(&client, deadline, None).await.unwrap();
            let rpc =
                Rpc::open_in_memory(client, deadline, &options, &[(file.clone(), source.to_string())]).await.unwrap();
            let module = rpc.source_file_symbol(&file).await.unwrap().unwrap();
            let mut compared = Vec::new();
            let mut kinds = BTreeMap::new();
            for export in rpc.exports_of_module(&module.reference).await.unwrap() {
                // Type alias, interface, class: the declared type; variable, function, class, enum, namespace: the
                // value's.
                let mut types = Vec::new();
                if export.flags & (524_288 | 64 | 32) != 0 {
                    types.push(rpc.declared_type_of_symbol(&export.reference).await.unwrap());
                }
                if export.flags & (3 | 16 | 32 | 256 | 512) != 0 {
                    types.push(rpc.type_of_symbol(&export.reference).await.unwrap());
                }
                for ty in types {
                    let properties = rpc.properties(ty.id).await.unwrap();
                    let references: Vec<_> = properties.iter().map(|property| &property.reference).collect();
                    let asked = rpc.readonly_symbols(&references).await.unwrap();
                    for (property, asked) in properties.iter().zip(asked) {
                        let at = format!("{}.{}", export.name, property.member_name());
                        let declared =
                            property.declarations.iter().map(|d| d.split('.').nth(1).unwrap().parse().unwrap());
                        kinds.insert(at.clone(), declared.collect::<Vec<u32>>());
                        compared.push((at, property.settled_readonly(), asked));
                    }
                }
            }
            rpc.release().await;
            (compared, kinds)
        });
        runtime.block_on(checker.shutdown());

        let disagreements: Vec<_> =
            compared.iter().filter(|(_, settled, asked)| settled.is_some_and(|settled| settled != *asked)).collect();
        assert_eq!(disagreements, Vec::<&(String, Option<bool>, bool)>::new(), "{compared:#?}");
        assert_eq!(kinds.len(), compared.len(), "a property named twice: {compared:#?}");
        // Which properties the answer settles, and how: one settled that was asked, or settled the other way, fails
        // here by name.
        let partition = |want: Option<bool>| {
            let named = compared.iter().filter(|(_, settled, _)| *settled == want).map(|(at, ..)| at.as_str());
            named.collect::<Vec<_>>().join(" ")
        };
        let readonly = "I.g K.only R.a R.b R.m R.g R.gs R.so R.c R.quoted-key R.[Symbol.iterator] P.b P.g \
            P.[Symbol.iterator] Req.a Req.b Req.m Req.g Req.gs Req.so Req.c Req.quoted-key Req.[Symbol.iterator] \
            Pk.b Pk.g O.b O.g O.[Symbol.iterator] Ro.k1 Ro.k2 Renamed.onB Renamed.onG X.g Y.b Y.m Y.g Y.gs Y.so Y.c \
            Y.quoted-key Y.[Symbol.iterator] U.a U.b U.g Inst.only Tup.0 Tup.1 Tup.length E.A E.B frozen.a frozen.b";
        assert_eq!(partition(Some(true)), readonly, "settled readonly");
        let not_readonly = "I.m I.gs I.so K.gk K.method P.a P.m P.gs P.so P.c P.quoted-key Mut.a Mut.b Mut.m \
            Mut.g Mut.gs Mut.so Mut.c Mut.quoted-key Mut.[Symbol.iterator] Pk.a Pk.m O.m O.gs O.so O.c O.quoted-key \
            Rec.u Rec.v Renamed.onA Renamed.onC Renamed.onGs Renamed.onM Renamed.onQuoted-key Renamed.onSo X.a X.b \
            X.m X.gs X.so U.m U.gs U.c Inst.gk Inst.method Tup.toString Tup.toLocaleString Tup.concat Tup.join \
            Tup.slice Tup.indexOf Tup.lastIndexOf Tup.every Tup.some Tup.forEach Tup.map Tup.filter Tup.reduce \
            Tup.reduceRight Tup.find Tup.findIndex Tup.[Symbol.iterator] Tup.entries Tup.keys Tup.values \
            Tup.includes Tup.flatMap Tup.flat Tup.at MutTup.toString MutTup.toLocaleString MutTup.pop MutTup.push \
            MutTup.concat MutTup.join MutTup.reverse MutTup.shift MutTup.slice MutTup.sort MutTup.splice \
            MutTup.unshift MutTup.indexOf MutTup.lastIndexOf MutTup.every MutTup.some MutTup.forEach MutTup.map \
            MutTup.filter MutTup.reduce MutTup.reduceRight MutTup.find MutTup.findIndex MutTup.fill \
            MutTup.copyWithin MutTup.[Symbol.iterator] MutTup.entries MutTup.keys MutTup.values MutTup.includes \
            MutTup.flatMap MutTup.flat MutTup.at Str.toString Str.charAt Str.charCodeAt Str.concat Str.indexOf \
            Str.lastIndexOf Str.localeCompare Str.match Str.replace Str.search Str.slice Str.split Str.substring \
            Str.toLowerCase Str.toLocaleLowerCase Str.toUpperCase Str.toLocaleUpperCase Str.trim Str.substr \
            Str.valueOf Str.codePointAt Str.includes Str.endsWith Str.normalize Str.repeat Str.startsWith Str.anchor \
            Str.big Str.blink Str.bold Str.fixed Str.fontcolor Str.fontsize Str.italics Str.link Str.small \
            Str.strike Str.sub Str.sup Str.[Symbol.iterator] Str.padStart Str.padEnd Str.trimEnd Str.trimStart \
            Str.trimLeft Str.trimRight Str.matchAll Str.replaceAll Str.at Arr.toString Arr.toLocaleString Arr.concat \
            Arr.join Arr.slice Arr.indexOf Arr.lastIndexOf Arr.every Arr.some Arr.forEach Arr.map Arr.filter \
            Arr.reduce Arr.reduceRight Arr.find Arr.findIndex Arr.[Symbol.iterator] Arr.entries Arr.keys Arr.values \
            Arr.includes Arr.flatMap Arr.flat Arr.at rev.ra rev.rb";
        assert_eq!(partition(Some(false)), not_readonly, "settled not readonly");
        let unsettled = "I.a I.b I.c I.quoted-key I.[Symbol.iterator] K.x K.y K.z K.p K.#q K.pp K.rp K.s K.t \
            K.prototype X.c X.quoted-key X.[Symbol.iterator] X.extra Y.a Inst.x Inst.y Inst.z Inst.p Inst.#q Inst.pp \
            Inst.rp Tup.[Symbol.unscopables] MutTup.[Symbol.unscopables] MutTup.0 MutTup.length Str.length \
            Arr.length Arr.[Symbol.unscopables] ns.lit ns.ro ns.gg ns.ss ns.method ns.short ns.computed ns.spread \
            fn.attached N.c N.l N.v N.f N.Kn N.En G.gr G.gw Gi.gr Gi.gw fx.dotted fx.bracketed";
        assert_eq!(partition(None), unsettled, "asked");
        let pinned = [
            ("K.pp", 171),
            ("I.a", 173),
            ("K.x", 174),
            ("I.m", 175),
            ("K.method", 176),
            ("I.g", 179),
            ("I.so", 180),
            ("ns.lit", 304),
            ("ns.short", 305),
        ];
        for (at, kind) in pinned {
            assert_eq!(kinds.get(at), Some(&vec![kind]), "{at}: {kinds:#?}");
        }
        assert_eq!(pinned.map(|(_, kind)| kind), NO_ASSIGNMENT_DECLARATION);
        // The expando assignments' declarations are of a kind the list leaves out, so they are asked.
        for at in ["fx.dotted", "fx.bracketed"] {
            let declared = kinds.get(at).unwrap();
            assert!(declared.iter().all(|kind| !NO_ASSIGNMENT_DECLARATION.contains(kind)), "{at}: {declared:?}");
            assert!(ASSIGNMENT_DECLARATION.iter().any(|(_, kind)| declared.contains(kind)), "{at}: {declared:?}");
        }
    }

    /// The kinds (tsrs_ast 0.24.0's `Kind`, the version [`the_declaration_kinds_are_read_from_the_pinned_tsrs`] holds)
    /// of the nodes that make a declaration an assignment or `Object.defineProperty` one, after TypeScript's
    /// `getAssignmentDeclarationKind`, and of the JSDoc tags that declare a property.
    const ASSIGNMENT_DECLARATION: [(&str, u32); 7] = [
        ("PropertyAccessExpression", 213),
        ("ElementAccessExpression", 214),
        ("CallExpression", 215),
        ("BinaryExpression", 228),
        ("JSDocCallbackTag", 332),
        ("JSDocTypedefTag", 339),
        ("JSDocPropertyTag", 341),
    ];

    /// The syntax kind numbers above are tsrs's and change when it renumbers, which no check here can see; this fails
    /// on any tsrs but the one they were read from, until they are read again from its `tsrs_ast::Kind` and the
    /// version below updated. A real `defineProperty` declaration cannot stand in: tsrs 0.24 does not bind one.
    #[test]
    fn the_declaration_kinds_are_read_from_the_pinned_tsrs() {
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
        let versions: Vec<_> = lock
            .split("[[package]]")
            .filter(|package| package.contains("\nname = \"tsrs_ast\"\n"))
            .filter_map(|package| package.lines().find_map(|line| line.strip_prefix("version = ")))
            .collect();
        assert_eq!(
            versions,
            ["\"0.24.0\""],
            "tsrs_ast moved: re-read NO_ASSIGNMENT_DECLARATION and ASSIGNMENT_DECLARATION from its Kind"
        );
        for (name, kind) in ASSIGNMENT_DECLARATION {
            assert!(!NO_ASSIGNMENT_DECLARATION.contains(&kind), "{name} ({kind}) is listed");
        }
    }

    /// The answer settles only what its fields decide; an answer without check flags settles nothing.
    #[test]
    fn readonly_is_settled_only_from_what_the_answer_carries() {
        let settled = |extra: serde_json::Value| {
            let mut symbol = serde_json::json!({ "reference": { "kind": 1, "id": 4, "snapshot": 2 }, "name": "x" });
            if let (Some(symbol), serde_json::Value::Object(extra)) = (symbol.as_object_mut(), extra) {
                symbol.extend(extra);
            }
            serde_json::from_value::<SymbolInfo>(symbol).unwrap().settled_readonly()
        };
        let mapped = |check_flags: u64| serde_json::json!({ "flags": 4, "checkFlags": check_flags, "declarations": ["1.173./a.ts"] });
        assert_eq!(settled(mapped(8 | (1 << 21))), Some(true));
        // Mapped, union or intersection (property or method), reverse-mapped: made with no value declaration.
        for made in [1 << 21, 2, 4, 1 << 16, (1 << 21) | 1] {
            assert_eq!(settled(mapped(made)), Some(false), "{made}");
        }
        assert_eq!(settled(serde_json::json!({ "flags": 4, "declarations": ["1.173./a.ts"] })), None);
        // No value declaration, and no check flag that says the checker made it without one: the field may have gone
        // missing from a declared, instantiated, late-bound or index-signature property's answer, so its `readonly`
        // modifier is unread.
        for unproven in [0, 1, 1 << 15, 1 << 26] {
            assert_eq!(settled(mapped(unproven)), None, "{unproven}");
        }
        let synthetic_declared = serde_json::json!({
            "flags": 4, "checkFlags": 2, "declarations": ["1.173./a.ts"], "valueDeclaration": "1.173./a.ts"
        });
        assert_eq!(settled(synthetic_declared), None);
        let declared = serde_json::json!({
            "flags": 4, "checkFlags": 0, "declarations": ["1.173./a.ts"], "valueDeclaration": "1.173./a.ts"
        });
        assert_eq!(settled(declared), None);
        assert_eq!(settled(serde_json::json!({ "flags": 2, "checkFlags": 0 })), None);
        assert_eq!(settled(serde_json::json!({ "flags": 32768, "checkFlags": 0 })), Some(true));
        assert_eq!(settled(serde_json::json!({ "flags": 32768 | 65536, "checkFlags": 0 })), Some(false));
        assert_eq!(
            settled(serde_json::json!({ "flags": 8 | 4, "checkFlags": 0, "valueDeclaration": "1.307./a.ts" })),
            Some(true)
        );
        for (name, kind) in ASSIGNMENT_DECLARATION {
            let assigned =
                serde_json::json!({ "flags": 4, "checkFlags": 1 << 21, "declarations": [format!("1.{kind}./a.js")] });
            assert_eq!(settled(assigned), None, "{name}");
        }
        assert_eq!(settled(serde_json::json!({ "flags": 4, "checkFlags": 0, "declarations": ["bad"] })), None);
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
        let scoped = |extra| Scoped { project: "p0", snapshot: 3, extra };

        let one = serde_json::to_string(&scoped(Asking::Symbol { symbol: &symbol.reference })).unwrap();
        let both = [&symbol.reference, &symbol.reference];
        let many = serde_json::to_string(&scoped(Asking::Symbols { symbols: &both })).unwrap();
        let requests = [
            Batched { method: "isReadonlySymbol", params: scoped(Asking::Symbol { symbol: &symbol.reference }) },
            Batched { method: "getTypesOfType", params: scoped(Asking::Object { object_id: 12 }) },
        ];
        let page = serde_json::to_string(&BatchParams { requests: &requests, continuation_token: Some("t") }).unwrap();

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
        assert_eq!(serde_json::to_string(&scoped(Asking::Nothing {})).unwrap(), r#"{"project":"p0","snapshot":3}"#);
    }

    #[test]
    fn a_symbol_arrives_with_the_name_the_source_writes() {
        let symbol: SymbolInfo =
            serde_json::from_value(json!({ "reference": {}, "name": "___size", "flags": 0 })).unwrap();

        assert_eq!(symbol.name, "__size");
    }
}
