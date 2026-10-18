use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseMessage {
    pub id: RequestId,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<RpcErrorObject>,
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
    pub method: String,
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
pub fn parse_incoming(body: &[u8]) -> Result<IncomingMessage, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_slice(body)?;
    let is_response = value.get("id").is_some() && (value.get("result").is_some() || value.get("error").is_some());
    if is_response {
        Ok(IncomingMessage::Response(serde_json::from_value(value)?))
    } else {
        Ok(IncomingMessage::Notification(serde_json::from_value(value)?))
    }
}
