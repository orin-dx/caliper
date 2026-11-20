//! Spike: checker methods asked straight through `tsrs_api::checker::dispatch::handle`, the function `Session` itself
//! calls for them, with no JSON text built or parsed on either side. The parameters go in as a `tsrs_core::json::Value`
//! tree and the answer comes back as one, which [`decode`] reads into the typed structs of [`crate::rpc`].
//!
//! `Session` adapts itself to the checker lane through a private `SessionHost`; [`CaliperHost`] is the same adapter
//! written over `Session`'s public API. Of the nine `CheckerHost` methods, the six that symbol, type and signature
//! requests use are implemented as `SessionHost` implements them; the other three serve methods caliper does not route
//! here (file-owned references without a snapshot, node handles sent by the client, synthesized nodes, auto-imports)
//! and answer `Unsupported`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use tsrs_api::checker::{
    CachedFileScope, CheckerError, CheckerErrorKind, CheckerHost, CheckerResponse, CheckerResult, SnapshotScope,
    dispatch,
};
use tsrs_api::{ApiError, ErrorKind, Session};
use tsrs_ast::{Node, SourceFile};
use tsrs_compiler::Program;
use tsrs_core::P;
use tsrs_core::context::Context;
use tsrs_core::json::{JsonMap, Value};
use tsrs_project::Snapshot;

/// `CheckerHost` over a `Session`'s public API, as `tsrs_api`'s private `SessionHost` implements it.
pub(crate) struct CaliperHost<'a> {
    pub(crate) session: &'a Session,
}

fn unsupported(what: &str) -> CheckerError {
    CheckerError { kind: CheckerErrorKind::Unsupported, message: format!("{what} is not routed through a direct call") }
}

/// The session's error as its text: `Internal` displays the message as it is.
fn from_api(error: &ApiError) -> CheckerError {
    let kind = match error.kind {
        ErrorKind::InvalidRequest => CheckerErrorKind::InvalidRequest,
        ErrorKind::ClientError => CheckerErrorKind::Client,
        ErrorKind::Unsupported => CheckerErrorKind::Unsupported,
        ErrorKind::Internal => CheckerErrorKind::Internal,
    };
    CheckerError { kind, message: error.to_string() }
}

impl CheckerHost for CaliperHost<'_> {
    fn context(&self) -> Context {
        Context::background()
    }

    fn snapshot(&self, handle: u64) -> CheckerResult<SnapshotScope> {
        let data = self.session.snapshot_data(handle).map_err(|error| from_api(&error))?;
        let snapshot = Arc::clone(&data.snapshot);
        let registry = Arc::clone(&data.checker_state.registry);
        // The scope pins the snapshot's data until the request ends, as `SessionHost` does.
        Ok(SnapshotScope { handle, snapshot, registry, release: Some(Box::new(move || drop(data))) })
    }

    /// `Session::acquire_cached_source_file` is `pub(crate)`.
    fn acquire_cached_source_file(&self, _descriptor: &tsrs_core::json::Value) -> CheckerResult<CachedFileScope> {
        Err(unsupported("a file-owned symbol reference without a snapshot"))
    }

    fn source_file_descriptor(&self, file: P<SourceFile>) -> CheckerResult<Value> {
        Ok(tsrs_api::sourcefiles::source_file_descriptor(file))
    }

    fn source_file_descriptor_matches(&self, file: P<SourceFile>, descriptor: &Value) -> CheckerResult<bool> {
        Ok(tsrs_api::sourcefiles::source_file_descriptor_matches(file, descriptor))
    }

    /// `tsrs_api::sourcefiles::source_file_node_id` is `pub(crate)`; this is its body.
    fn source_file_node_id(&self, file: P<SourceFile>) -> CheckerResult<u64> {
        Ok(tsrs_ast::get_node_id(file.as_node()).0)
    }

    fn node_handle(&self, node: P<Node>) -> CheckerResult<String> {
        self.session.node_handle_from(node).map_err(|error| from_api(&error))
    }

    /// `Session::resolve_node_handle` is `pub(crate)`, as is the node index table cache it reads.
    fn resolve_node_handle(&self, _program: &'static Program, _handle: &str) -> CheckerResult<P<Node>> {
        Err(unsupported("a node handle"))
    }

    /// `Session::encode_node` is `pub(crate)`.
    fn encode_node(&self, _node: P<Node>) -> CheckerResult<Vec<u8>> {
        Err(unsupported("a synthesized node"))
    }

    /// `Session::snapshot_host` is `pub(crate)`.
    fn clone_snapshot_with_auto_imports(&self, _base: &Snapshot, _file_name: &str) -> CheckerResult<Arc<Snapshot>> {
        Err(unsupported("auto-imports"))
    }

    /// A session caliper makes has no language server attached.
    fn try_adopt_snapshot_in_background(&self, _base: &Arc<Snapshot>, _prepared: &Arc<Snapshot>) {}
}

/// Why a direct call did not answer: the message, and whether a panic unwound through the session.
pub(crate) struct Unanswered {
    pub(crate) message: String,
    pub(crate) panicked: bool,
}

/// One checker-lane request, answered as a tree. A panic is caught here, as `Session::handle_request` catches one.
pub(crate) fn ask(session: &Session, method: &str, params: &Value) -> Result<Value, Unanswered> {
    let host = CaliperHost { session };
    match catch_unwind(AssertUnwindSafe(|| dispatch::handle(&host, method, params))) {
        Ok(Some(Ok(CheckerResponse::Json(value)))) => Ok(value),
        Ok(Some(Ok(CheckerResponse::EncodedNode(_)))) => {
            Err(Unanswered { message: format!("{method} answered with an encoded node"), panicked: false })
        }
        Ok(Some(Err(error))) => Err(Unanswered { message: error.to_string(), panicked: false }),
        Ok(None) => Err(Unanswered { message: format!("{method} is not a checker method"), panicked: false }),
        Err(panic) => {
            let text = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            Err(Unanswered { message: format!("panic: {text}"), panicked: true })
        }
    }
}

/// Spike measurement switch: `CALIPER_SPIKE_TEXT` set sends the routed methods down the JSON text path again, so one
/// binary can be sampled both ways.
pub(crate) fn text_forced() -> bool {
    static FORCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| std::env::var_os("CALIPER_SPIKE_TEXT").is_some())
}

/// A request's parameters: the project and snapshot, then `fields`.
pub(crate) fn scoped(project: &str, snapshot: u64, fields: Vec<(&'static str, Value)>) -> Value {
    let mut map = JsonMap::with_capacity(2 + fields.len());
    map.insert("project", Value::String(project.to_string()));
    #[expect(clippy::cast_precision_loss, reason = "snapshot handles are small counters")]
    map.insert("snapshot", Value::Number(snapshot as f64));
    for (key, value) in fields {
        map.insert(key, value);
    }
    Value::Object(map)
}

/// A JSON text (a symbol reference as it arrived) as a tree.
pub(crate) fn tree_of(text: &str) -> Result<Value, String> {
    tsrs_core::json::unmarshal(text)
}

/// A tree read straight into `T`, as `serde_json` would read the text the session would have written for it.
pub(crate) fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    T::deserialize(Tree(value)).map_err(|error| error.0)
}

#[derive(Debug)]
pub(crate) struct DecodeError(String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

impl de::Error for DecodeError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        DecodeError(message.to_string())
    }
}

/// `serde_json`'s private name for `RawValue`: a raw value asks its deserializer for a one-entry map from this name to
/// the value's JSON text (as `serde_json::Value`'s own deserializer answers it).
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

#[derive(Clone, Copy)]
struct Tree<'a>(&'a Value);

/// A number as `serde_json` reads the text `tsrs` writes for it: an integral value below 1e21 is written in digits
/// (ES6 number formatting), which `serde_json` reads as a `u64` or an `i64` when it fits; anything else as an `f64`.
fn visit_number<'de, V: Visitor<'de>>(n: f64, visitor: V) -> Result<V::Value, DecodeError> {
    const TWO_64: f64 = 18_446_744_073_709_551_616.0;
    const MINUS_TWO_63: f64 = -9_223_372_036_854_775_808.0;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "range checked just before")]
    if n.fract() == 0.0 && n.abs() < 1e21 {
        if (0.0..TWO_64).contains(&n) {
            return visitor.visit_u64(n as u64);
        }
        if (MINUS_TWO_63..0.0).contains(&n) {
            return visitor.visit_i64(n as i64);
        }
    }
    visitor.visit_f64(n)
}

impl<'de> de::Deserializer<'de> for Tree<'_> {
    type Error = DecodeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Number(n) => visit_number(*n, visitor),
            Value::Integer(i) => visitor.visit_i64(*i),
            Value::String(s) => visitor.visit_str(s),
            Value::Array(items) => visitor.visit_seq(Seq(items.iter())),
            Value::Object(map) => visitor.visit_map(Map { iter: map.iter(), value: None }),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        if name == RAW_VALUE_TOKEN {
            let text = tsrs_core::json::marshal(self.0).map_err(DecodeError)?;
            return visitor.visit_map(Raw(Some(text)));
        }
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit unit_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

struct Seq<'a>(std::slice::Iter<'a, Value>);

impl<'de> SeqAccess<'de> for Seq<'_> {
    type Error = DecodeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, DecodeError> {
        self.0.next().map(|value| seed.deserialize(Tree(value))).transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

struct Map<'a, I> {
    iter: I,
    value: Option<&'a Value>,
}

impl<'de, 'a, I: Iterator<Item = (&'a str, &'a Value)>> MapAccess<'de> for Map<'a, I> {
    type Error = DecodeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, DecodeError> {
        let Some((key, value)) = self.iter.next() else { return Ok(None) };
        self.value = Some(value);
        seed.deserialize(IntoDeserializer::<DecodeError>::into_deserializer(key)).map(Some)
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, DecodeError> {
        let value = self.value.take().ok_or_else(|| DecodeError("a map value without its key".to_string()))?;
        seed.deserialize(Tree(value))
    }
}

/// The one-entry map a `RawValue` reads: its private name, then its JSON text.
struct Raw(Option<String>);

impl<'de> MapAccess<'de> for Raw {
    type Error = DecodeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, DecodeError> {
        if self.0.is_none() {
            return Ok(None);
        }
        seed.deserialize(IntoDeserializer::<DecodeError>::into_deserializer(RAW_VALUE_TOKEN)).map(Some)
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, DecodeError> {
        let text = self.0.take().ok_or_else(|| DecodeError("a raw value read twice".to_string()))?;
        seed.deserialize(IntoDeserializer::<DecodeError>::into_deserializer(text))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::rpc::{SymbolInfo, TypeInfo};

    /// Reading a tree gives what `serde_json` gives reading the text `tsrs` writes for that tree.
    #[test]
    fn a_tree_reads_as_its_text_does() {
        let text = r#"[{"reference":{"kind":1,"id":4,"file":{"fileName":"/a.ts","nodeId":"7"}},"name":"___size","flags":4,"declarations":["1.2./a.ts"],"parent":{"id":9,"file":"7"}}]"#;
        let tree = tree_of(text).unwrap();
        let from_tree: Vec<SymbolInfo> = decode(&tree).unwrap();
        let from_text: Vec<SymbolInfo> = serde_json::from_str(text).unwrap();
        assert_eq!(format!("{from_tree:?}"), format!("{from_text:?}"));
        assert_eq!(from_tree[0].reference.text(), r#"{"kind":1,"id":4,"file":{"fileName":"/a.ts","nodeId":"7"}}"#);

        let text = r#"{"id":12,"flags":1024,"value":-3,"texts":["a","b"],"symbol":{"id":2},"aliasTypeArguments":[1,2],"constraintType":5}"#;
        let tree = tree_of(text).unwrap();
        let from_tree: TypeInfo = decode(&tree).unwrap();
        let from_text: TypeInfo = serde_json::from_str(text).unwrap();
        assert_eq!(format!("{from_tree:?}"), format!("{from_text:?}"));
        for number in ["1.5", "0", "-0", "1e21", "123456789012345678", "-7"] {
            let from_tree: serde_json::Value = decode(&tree_of(number).unwrap()).unwrap();
            let marshalled = tsrs_core::json::marshal(&tree_of(number).unwrap()).unwrap();
            let from_text: serde_json::Value = serde_json::from_str(&marshalled).unwrap();
            assert_eq!(from_tree, from_text, "{number}");
        }
    }
}
