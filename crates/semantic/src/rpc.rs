//! Typed calls over one checker session, linked or a `tsc --api` process ([`crate::checker`]). Wire shapes are the
//! ones the official 7.1 client sends (traced against `7.1.0-beta`); a method this module does not name is not used.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use caliper_docgen_tsc_client::TscClientError;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::checker::Client;

/// Why a call failed, split by whether the rest of the run can go on.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The session is gone or stuck: every later call would fail the same way.
    Fatal(String),
    /// The checker rejected this one call; other components may still resolve.
    Rejected(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Fatal(message) | CallError::Rejected(message) => f.write_str(message),
        }
    }
}

fn classify(error: TscClientError) -> CallError {
    match error {
        TscClientError::RpcError { message, .. } => CallError::Rejected(message),
        other => CallError::Fatal(other.to_string()),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SymbolInfo {
    /// The full reference the checker wants back to name this symbol; opaque to us.
    pub reference: Value,
    /// The name the source writes: the checker's escaped name with the escaping removed (see
    /// [`unescape_leading_underscores`]).
    #[serde(deserialize_with = "unescaped_name")]
    pub name: String,
    pub flags: u64,
    #[serde(default)]
    pub declarations: Vec<String>,
    /// The symbol it is a member of: the interface or type literal that declares a property. Absent for a property a
    /// mapped type or an intersection made (`Omit<Base, "x">`, `A & B`), whose declarations still name the original.
    #[serde(default)]
    pub parent: Option<CompactSymbol>,
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

fn unescaped_name<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let name = String::deserialize(deserializer)?;
    Ok(unescape_leading_underscores(&name).to_string())
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
    calls: Arc<Mutex<BTreeMap<String, u64>>>,
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
        };
        rpc.call("initialize", Value::Null).await?;
        Ok(match tsconfig {
            Some(file) => {
                let parsed: Value = decode(&rpc.call("parseConfigFile", json!({ "file": file })).await?)?;
                parsed.get("options").cloned().unwrap_or_else(default_options)
            }
            None => default_options(),
        })
    }

    /// A snapshot of `count` programs over the same `root_files` (one when `count` is at most one), in a session it
    /// initializes first ([`Rpc::prepare`]), with one session bound to each program: a checker that answers requests
    /// about different programs at once ([`crate::checker::InProcess`]) answers the sessions in parallel. The programs
    /// read the same files, so each answers what any other would. They share the request counts and the recording; [`Rpc::release`] on the first releases the
    /// snapshot, and with it every program.
    pub(crate) async fn open_programs(
        client: Client,
        deadline: Duration,
        tsconfig: Option<&str>,
        root_files: &[String],
        record: bool,
        count: usize,
    ) -> Result<Vec<Rpc>, CallError> {
        let options = Rpc::prepare(&client, deadline, tsconfig).await?;
        if count <= 1 {
            return Ok(vec![Rpc::open_with(client, deadline, &options, root_files, record).await?]);
        }
        let first = Rpc {
            client,
            deadline,
            snapshot: 0,
            project: String::new(),
            options: options.clone(),
            recording: record.then(Arc::default),
            calls: Arc::default(),
        };
        let program = json!({ "rootFiles": root_files, "compilerOptions": options });
        let programs: Vec<Value> = std::iter::repeat_n(program, count).collect();
        let response: SnapshotResponse =
            decode(&first.call("createSnapshot", json!({ "createPrograms": programs })).await?)?;
        if response.projects.len() != count {
            return Err(CallError::Rejected(format!(
                "createSnapshot made {} programs of {count}",
                response.projects.len()
            )));
        }
        Ok(response
            .projects
            .into_iter()
            .enumerate()
            .map(|(lane, project)| Rpc {
                client: first.client.lane(lane),
                deadline,
                snapshot: response.snapshot,
                project: project.id,
                options: options.clone(),
                recording: first.recording.clone(),
                calls: first.calls.clone(),
            })
            .collect())
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
        };
        rpc.created(json!({ "createPrograms": [{ "rootFiles": root_files, "compilerOptions": options }] })).await
    }

    /// A new snapshot of the same program with `files` added, held in the checker's memory and never written to disk:
    /// each is a root file, keyed by its absolute path. The calls on it count, and are recorded, with this one's.
    pub(crate) async fn layered(&self, root_files: &[String], files: &[(String, String)]) -> Result<Rpc, CallError> {
        let rpc = Rpc {
            client: self.client.clone(),
            deadline: self.deadline,
            snapshot: 0,
            project: String::new(),
            options: self.options.clone(),
            recording: self.recording.clone(),
            calls: self.calls.clone(),
        };
        let mut roots = root_files.to_vec();
        roots.extend(files.iter().map(|(path, _)| path.clone()));
        let layer: serde_json::Map<String, Value> =
            files.iter().map(|(path, text)| (path.clone(), Value::String(text.clone()))).collect();
        rpc.created(json!({
            "fileSystem": { "kind": "layer", "files": layer },
            "createPrograms": [{ "rootFiles": roots, "compilerOptions": self.options }],
        }))
        .await
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
            decode(&self.call("updateSnapshot", json!({ "snapshot": self.snapshot, "changes": changes })).await?)?;
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
        })
    }

    /// Every file the snapshot's program read, as the checker names them.
    pub(crate) async fn source_file_names(&self) -> Result<Vec<String>, CallError> {
        self.scoped("getSourceFileNames", json!({})).await
    }

    /// This session bound to the snapshot `createSnapshot` makes from `params`, and to its one project.
    async fn created(mut self, params: Value) -> Result<Rpc, CallError> {
        let response: SnapshotResponse = decode(&self.call("createSnapshot", params).await?)?;
        let project = response
            .projects
            .into_iter()
            .next()
            .ok_or_else(|| CallError::Rejected("createSnapshot returned no project".to_string()))?;
        self.snapshot = response.snapshot;
        self.project = project.id;
        Ok(self)
    }

    pub(crate) async fn release(&self) {
        self.call("release", json!({ "snapshot": self.snapshot })).await.ok();
    }

    /// The recorded calls as a JSON array, or `None` when recording was off.
    pub(crate) fn recording(&self) -> Option<Value> {
        let log = self.recording.as_ref()?.lock().ok()?;
        Some(Value::Array(log.clone()))
    }

    /// Requests sent so far, by method.
    pub(crate) fn calls(&self) -> BTreeMap<String, u64> {
        self.calls.lock().map(|calls| calls.clone()).unwrap_or_default()
    }

    /// One request, its answer as the JSON text it arrived as: [`decode`] parses it once, into the shape the caller
    /// expects.
    async fn call(&self, method: &str, params: Value) -> Result<Box<RawValue>, CallError> {
        if let Ok(mut calls) = self.calls.lock() {
            *calls.entry(method.to_string()).or_default() += 1;
        }
        REQUESTS.try_with(|count| count.fetch_add(1, std::sync::atomic::Ordering::Relaxed)).ok();
        let Some(log) = &self.recording else {
            return self.client.request_raw(method, params, self.deadline).await.map_err(classify);
        };
        let answer = self.client.request_raw(method, params.clone(), self.deadline).await.map_err(classify)?;
        let result: Value = decode(&answer)?;
        if let Ok(mut log) = log.lock() {
            log.push(json!({ "method": method, "params": params, "result": result }));
        }
        Ok(answer)
    }

    async fn scoped<T: DeserializeOwned>(&self, method: &str, extra: Value) -> Result<T, CallError> {
        decode(&self.scoped_text(method, extra).await?)
    }

    /// A request about this snapshot and project, its answer left as text.
    async fn scoped_text(&self, method: &str, extra: Value) -> Result<Box<RawValue>, CallError> {
        let mut params = json!({ "snapshot": self.snapshot, "project": self.project });
        if let (Some(base), Value::Object(extra)) = (params.as_object_mut(), extra) {
            base.extend(extra);
        }
        self.call(method, params).await
    }

    pub(crate) async fn source_file_symbol(&self, file: &str) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getSymbolOfSourceFile", json!({ "file": file })).await
    }

    pub(crate) async fn exports_of_module(&self, module: &Value) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped("getExportsOfModule", json!({ "symbol": module })).await
    }

    /// Only for symbols flagged `Alias`: the beta panics on a plain `default` export.
    pub(crate) async fn aliased_symbol(&self, symbol: &Value) -> Result<SymbolInfo, CallError> {
        self.scoped("getAliasedSymbol", json!({ "symbol": symbol })).await
    }

    pub(crate) async fn type_of_symbol(&self, symbol: &Value) -> Result<TypeInfo, CallError> {
        self.scoped("getTypeOfSymbol", json!({ "symbol": symbol })).await
    }

    pub(crate) async fn types_of_symbols(&self, symbols: &[&Value]) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getTypesOfSymbols", json!({ "symbols": symbols })).await
    }

    /// The call signatures of a type, or with `SignatureKind::Construct` its construct signatures.
    pub(crate) async fn signatures(&self, type_id: u64, kind: SignatureKind) -> Result<Vec<SignatureInfo>, CallError> {
        self.scoped("getSignaturesOfType", json!({ "type": type_id, "kind": kind as u8 })).await
    }

    pub(crate) async fn type_parameters(&self, signature: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getTypeParametersOfSignature", json!({ "objectId": signature })).await
    }

    /// The type a type parameter defaults to (`"div"` in `E extends ElementType = "div"`); none when it declares none.
    pub(crate) async fn default_of_type_parameter(&self, type_id: u64) -> Result<Option<TypeInfo>, CallError> {
        self.scoped("getDefaultFromTypeParameter", json!({ "objectId": type_id })).await
    }

    /// The type a type alias or interface declares, `T` in `type A = T`.
    pub(crate) async fn declared_type_of_symbol(&self, symbol: &Value) -> Result<TypeInfo, CallError> {
        self.scoped("getDeclaredTypeOfSymbol", json!({ "symbol": symbol })).await
    }

    /// The type errors in a file of the program.
    pub(crate) async fn semantic_diagnostics(&self, file: &str) -> Result<Vec<FileDiagnostic>, CallError> {
        self.scoped("getSemanticDiagnostics", json!({ "file": file })).await
    }

    /// The symbol that declares a type: the interface, class, enum member or unique symbol behind it.
    pub(crate) async fn symbol_of_type(&self, type_id: u64) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getSymbolOfType", json!({ "objectId": type_id })).await
    }

    /// The types a class or interface extends, instantiated as it names them (`Omit<HTMLAttributes<HTMLDivElement>, "color">`);
    /// none for a type that is not a class or interface.
    pub(crate) async fn base_types(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        let bases: Option<Vec<TypeInfo>> = self.scoped("getBaseTypes", json!({ "type": type_id })).await?;
        Ok(bases.unwrap_or_default())
    }

    pub(crate) async fn alias_type_arguments(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getAliasTypeArgumentsOfType", json!({ "objectId": type_id })).await
    }

    /// The generic type a reference instantiates; for a tuple reference, the tuple type that holds its element flags.
    pub(crate) async fn target_of_type(&self, type_id: u64) -> Result<TypeInfo, CallError> {
        self.scoped("getTargetOfType", json!({ "objectId": type_id })).await
    }

    pub(crate) async fn parent_of_symbol(&self, symbol: &Value) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getParentOfSymbolForChecker", json!({ "symbol": symbol })).await
    }

    /// Whether each property is declared `readonly`, in one round trip.
    pub(crate) async fn readonly_symbols(&self, symbols: &[&Value]) -> Result<Vec<bool>, CallError> {
        let requests = symbols.iter().map(|symbol| ("isReadonlySymbol", json!({ "symbol": symbol }))).collect();
        self.batch(requests).await?.iter().map(|answer| decode(answer)).collect()
    }

    /// Several requests in one round trip. A request the checker rejects fails the batch: nothing here asks for one
    /// that may legitimately fail.
    pub(crate) async fn batch(&self, requests: Vec<(&str, Value)>) -> Result<Vec<Box<RawValue>>, CallError> {
        let wanted = requests.len();
        let scoped: Vec<Value> = requests
            .into_iter()
            .map(|(method, mut params)| {
                if let Value::Object(map) = &mut params {
                    map.insert("snapshot".to_string(), json!(self.snapshot));
                    map.insert("project".to_string(), json!(self.project));
                }
                json!({ "method": method, "params": params })
            })
            .collect();
        let mut answers = Vec::with_capacity(wanted);
        let mut token: Option<String> = None;
        loop {
            let mut params = json!({ "requests": scoped });
            if let (Some(token), Some(map)) = (&token, params.as_object_mut()) {
                map.insert("continuationToken".to_string(), json!(token));
            }
            let page: BatchPage = decode(&self.call("batchRequests", params).await?)?;
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
        self.scoped("getParametersOfSignature", json!({ "objectId": signature })).await
    }

    pub(crate) async fn return_type(&self, signature: u64) -> Result<TypeInfo, CallError> {
        self.scoped("getReturnTypeOfSignature", json!({ "objectId": signature })).await
    }

    pub(crate) async fn properties(&self, type_id: u64) -> Result<Vec<SymbolInfo>, CallError> {
        self.scoped("getPropertiesOfType", json!({ "type": type_id })).await
    }

    /// The members of a union, intersection or template literal, as the text the checker answered (a list of types).
    pub(crate) async fn members_text(&self, type_id: u64) -> Result<Box<RawValue>, CallError> {
        self.scoped_text("getTypesOfType", json!({ "objectId": type_id })).await
    }

    /// The type arguments of a reference, as the text the checker answered (a list of types).
    pub(crate) async fn type_arguments_text(&self, type_id: u64) -> Result<Box<RawValue>, CallError> {
        self.scoped_text("getTypeArguments", json!({ "type": type_id })).await
    }

    /// The call and construct signatures, the properties and the index signatures of a type, in one round trip.
    pub(crate) async fn object_members(&self, type_id: u64) -> Result<ObjectMembers, CallError> {
        let requests = vec![
            ("getSignaturesOfType", json!({ "type": type_id, "kind": SignatureKind::Call as u8 })),
            ("getSignaturesOfType", json!({ "type": type_id, "kind": SignatureKind::Construct as u8 })),
            ("getPropertiesOfType", json!({ "type": type_id })),
            ("getIndexInfosOfType", json!({ "type": type_id })),
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
        self.scoped("getAliasSymbolOfType", json!({ "objectId": type_id })).await
    }

    pub(crate) async fn type_to_string(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped("typeToString", json!({ "type": type_id })).await
    }

    /// The type as written inside a type alias declaration, and never cut short: an alias at the top is printed as what
    /// it stands for (`(value?: string) => void`, not `Handler`), and a long type is printed to its end.
    pub(crate) async fn type_to_string_expanded(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped("typeToString", json!({ "type": type_id, "flags": flag::PRINT_EXPANDED })).await
    }
}

fn default_options() -> Value {
    // Numeric enum values: JSX preserve, ESNext modules, bundler resolution, ES2022.
    json!({ "jsx": 1, "module": 99, "moduleResolution": 100, "target": 9, "strict": true, "skipLibCheck": true, "noEmit": true })
}

/// An answer parsed from its text straight into `T`, in one pass.
pub(crate) fn decode<T: DeserializeOwned>(answer: &RawValue) -> Result<T, CallError> {
    serde_json::from_str(answer.get())
        .map_err(|error| CallError::Rejected(format!("unexpected response shape: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_symbol_arrives_with_the_name_the_source_writes() {
        let symbol: SymbolInfo =
            serde_json::from_value(json!({ "reference": {}, "name": "___size", "flags": 0 })).unwrap();

        assert_eq!(symbol.name, "__size");
    }
}
