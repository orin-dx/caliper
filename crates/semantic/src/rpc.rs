//! Typed calls over one `tsc --api` session. Wire shapes are the ones the official 7.1 client sends (traced against
//! `7.1.0-beta`); a method this module does not name is not used.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use caliper_docgen_tsc_client::{Client, TscClientError};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

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
    pub name: String,
    pub flags: u64,
    #[serde(default)]
    pub declarations: Vec<String>,
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
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SignatureInfo {
    pub id: u64,
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
    pub(crate) const BOOLEAN: u64 = 256;
    pub(crate) const STRING_LITERAL: u64 = 1024;
    pub(crate) const NUMBER_LITERAL: u64 = 2048;
    pub(crate) const BOOLEAN_LITERAL: u64 = 8192;
    pub(crate) const ENUM_LITERAL: u64 = 32768;
    pub(crate) const ENUM: u64 = 65536;
    pub(crate) const NEVER: u64 = 262144;
    pub(crate) const TYPE_PARAMETER: u64 = 524288;
    pub(crate) const OBJECT: u64 = 1048576;
    pub(crate) const TEMPLATE_LITERAL: u64 = 4194304;
    pub(crate) const STRING_MAPPING: u64 = 8388608;
    pub(crate) const UNION: u64 = 134217728;
    pub(crate) const INTERSECTION: u64 = 268435456;
    pub(crate) const SYMBOL_ALIAS: u64 = 2097152;
    pub(crate) const SYMBOL_OPTIONAL: u64 = 16777216;
    pub(crate) const OBJECT_REFERENCE: u64 = 4;
}

/// One `tsc --api` session bound to a snapshot and project.
pub(crate) struct Rpc {
    client: Client,
    deadline: Duration,
    snapshot: u64,
    project: String,
    /// Every call and its answer, when the caller asked to record a replay fixture.
    recording: Option<Mutex<Vec<Value>>>,
    calls: Mutex<BTreeMap<String, u64>>,
}

impl Rpc {
    pub(crate) async fn open(
        client: Client,
        deadline: Duration,
        tsconfig: Option<&str>,
        root_files: &[String],
        record: bool,
    ) -> Result<Rpc, CallError> {
        let mut rpc = Rpc {
            client,
            deadline,
            snapshot: 0,
            project: String::new(),
            recording: record.then(|| Mutex::new(Vec::new())),
            calls: Mutex::new(BTreeMap::new()),
        };
        rpc.call("initialize", Value::Null).await?;
        // The beta panics on string-valued enum options such as `target: "es2022"`, so options come from
        // `parseConfigFile`, which yields numeric ones.
        let options = match tsconfig {
            Some(file) => {
                let parsed = rpc.call("parseConfigFile", json!({ "file": file })).await?;
                parsed.get("options").cloned().unwrap_or_else(default_options)
            }
            None => default_options(),
        };
        let created = rpc
            .call(
                "createSnapshot",
                json!({ "createPrograms": [{ "rootFiles": root_files, "compilerOptions": options }] }),
            )
            .await?;
        let response: SnapshotResponse = decode(created)?;
        let project = response
            .projects
            .into_iter()
            .next()
            .ok_or_else(|| CallError::Rejected("createSnapshot returned no project".to_string()))?;
        rpc.snapshot = response.snapshot;
        rpc.project = project.id;
        Ok(rpc)
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

    async fn call(&self, method: &str, params: Value) -> Result<Value, CallError> {
        if let Ok(mut calls) = self.calls.lock() {
            *calls.entry(method.to_string()).or_default() += 1;
        }
        let answer = self.client.request(method, params.clone(), self.deadline).await.map_err(classify)?;
        if let Some(log) = &self.recording {
            if let Ok(mut log) = log.lock() {
                log.push(json!({ "method": method, "params": params, "result": answer.value }));
            }
        }
        Ok(answer.value)
    }

    async fn scoped<T: DeserializeOwned>(&self, method: &str, extra: Value) -> Result<T, CallError> {
        let mut params = json!({ "snapshot": self.snapshot, "project": self.project });
        if let (Some(base), Value::Object(extra)) = (params.as_object_mut(), extra) {
            base.extend(extra);
        }
        decode(self.call(method, params).await?)
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

    pub(crate) async fn signatures(&self, type_id: u64) -> Result<Vec<SignatureInfo>, CallError> {
        self.scoped("getSignaturesOfType", json!({ "type": type_id, "kind": 0 })).await
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

    pub(crate) async fn members(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getTypesOfType", json!({ "objectId": type_id })).await
    }

    pub(crate) async fn type_arguments(&self, type_id: u64) -> Result<Vec<TypeInfo>, CallError> {
        self.scoped("getTypeArguments", json!({ "type": type_id })).await
    }

    pub(crate) async fn is_array(&self, type_id: u64) -> Result<bool, CallError> {
        self.scoped("isArrayType", json!({ "type": type_id })).await
    }

    pub(crate) async fn alias_symbol(&self, type_id: u64) -> Result<Option<SymbolInfo>, CallError> {
        self.scoped("getAliasSymbolOfType", json!({ "objectId": type_id })).await
    }

    pub(crate) async fn type_to_string(&self, type_id: u64) -> Result<String, CallError> {
        self.scoped("typeToString", json!({ "type": type_id })).await
    }

    pub(crate) async fn documentation(&self, symbol: &Value) -> Result<String, CallError> {
        self.scoped("getDocumentationComment", json!({ "symbol": symbol })).await
    }
}

fn default_options() -> Value {
    // Numeric enum values: JSX preserve, ESNext modules, bundler resolution, ES2022.
    json!({ "jsx": 1, "module": 99, "moduleResolution": 100, "target": 9, "strict": true, "skipLibCheck": true, "noEmit": true })
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, CallError> {
    serde_json::from_value(value).map_err(|error| CallError::Rejected(format!("unexpected response shape: {error}")))
}
