use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::error::{MalformedFrameReason, TscClientError};

/// A JSON-RPC request id. Newtype over `u64` so a raw index into another
/// collection can't be passed where a request id is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, Serialize)]
pub struct RequestMessage {
    pub id: RequestId,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ResponseMessage {
    pub id: RequestId,
    /// The `result` as its JSON text, unparsed: the caller parses it once, into the type it expects.
    pub result: Option<Box<RawValue>>,
    pub error: Option<RpcErrorObject>,
}

/// Every field a frame of either shape may carry, read in one pass over the body. `result` keeps its text: a large
/// answer (a union's members, an object's properties) is parsed once by the caller, not into a tree first.
#[derive(Deserialize)]
struct Incoming {
    #[serde(default)]
    id: Option<RequestId>,
    #[serde(default, deserialize_with = "present")]
    result: Option<Box<RawValue>>,
    #[serde(default)]
    error: Option<RpcErrorObject>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: serde_json::Value,
}

/// A field that is present is `Some`, even when it is `null`: `"result": null` is an answer.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationMessage {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A notification surfaced to a caller via [`crate::Client::take_notifications`].
#[derive(Debug, Clone)]
pub struct Notification {
    /// The notification's method name, as `tsc` sent it.
    pub method: String,
    /// The notification's parameters; `null` when it sent none.
    pub params: serde_json::Value,
}

#[derive(Debug)]
pub enum IncomingMessage {
    Response(ResponseMessage),
    Notification(NotificationMessage),
}

/// Dispatches a decoded frame body by shape: an `id` alongside `result`/`error` is a
/// response to one of our requests; anything else with a `method` is a notification.
/// tsc --api never sends us a request of its own (no server-initiated calls in this
/// protocol), so those are the only two shapes handled.
///
/// A body that does not decode is [`MalformedFrameReason::InvalidJson`]: the frame's length was honored, so the stream
/// can still be read.
pub fn parse_incoming(body: &[u8]) -> Result<IncomingMessage, TscClientError> {
    let invalid =
        |_source: serde_json::Error| TscClientError::MalformedFrame { reason: MalformedFrameReason::InvalidJson };
    // A struct also reads from a JSON array, field by field; a message is an object.
    if body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::InvalidJson });
    }
    let incoming: Incoming = serde_json::from_slice(body).map_err(invalid)?;
    match incoming {
        Incoming { id: Some(id), result, error, .. } if result.is_some() || error.is_some() => {
            Ok(IncomingMessage::Response(ResponseMessage { id, result, error }))
        }
        Incoming { method: Some(method), params, .. } => {
            Ok(IncomingMessage::Notification(NotificationMessage { method, params }))
        }
        _ => Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::InvalidJson }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_that_is_not_json_is_an_invalid_json_frame() {
        let result = parse_incoming(b"this is not json");
        assert!(
            matches!(result, Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::InvalidJson })),
            "got {result:?}"
        );
    }

    #[test]
    fn a_json_body_of_no_message_shape_is_an_invalid_json_frame() {
        for body in [&b"[1, 2]"[..], b"[1, 2, null]", b"{\"id\": 1}", b"3"] {
            let result = parse_incoming(body);
            assert!(
                matches!(result, Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::InvalidJson })),
                "{}: got {result:?}",
                String::from_utf8_lossy(body)
            );
        }
    }

    /// The response's id and its result's text, when `body` parses as a response.
    fn response(body: &[u8]) -> Option<(u64, Option<String>)> {
        match parse_incoming(body) {
            Ok(IncomingMessage::Response(response)) => {
                Some((response.id.0, response.result.as_deref().map(|raw| raw.get().to_string())))
            }
            _ => None,
        }
    }

    #[test]
    fn a_response_keeps_its_result_as_the_text_it_arrived_as() {
        assert_eq!(
            response(br#"{"id": 4, "result": {"b": [1, 2.50], "a": "x"}}"#),
            Some((4, Some(r#"{"b": [1, 2.50], "a": "x"}"#.to_string())))
        );
    }

    #[test]
    fn a_null_result_is_an_answer_and_not_a_notification() {
        assert_eq!(response(br#"{"id": 9, "result": null}"#), Some((9, Some("null".to_string()))));
    }

    #[test]
    fn an_error_without_a_result_is_an_answer() {
        assert_eq!(response(br#"{"id": 2, "error": {"code": -1, "message": "no"}}"#), Some((2, None)));
    }

    #[test]
    fn a_frame_with_a_method_and_no_answer_is_a_notification() {
        let parsed = parse_incoming(br#"{"method": "progress", "params": {"n": 1}}"#);
        assert!(
            matches!(&parsed, Ok(IncomingMessage::Notification(n)) if n.method == "progress" && n.params == serde_json::json!({ "n": 1 })),
            "got {parsed:?}"
        );
    }
}
