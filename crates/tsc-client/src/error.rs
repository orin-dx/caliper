use std::sync::Arc;
use std::time::Duration;

use camino::Utf8PathBuf;

/// One variant family per Lifecycle-gate item (docs/type-checker-integration.md,
/// "Gates before it ships") so a caller or test can tell failure modes apart.
///
/// `Clone` on purpose, not just for this crate's own internal fan-out (one session-
/// ending failure delivered to every pending caller):
/// a caller-facing error type that can't be cloned forces callers to choose between
/// logging it and propagating it. The only non-`Clone` sources are `io::Error` and
/// `serde_json::Error`, `Arc`-wrapped below so cloning the rare error path is just a
/// refcount bump, never a real cost on the common (success) path.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum TscClientError {
    #[error("tsc binary not found at {path} — install `typescript@next` (npm i typescript@next) or pass an explicit path ({source})")]
    SpawnFailed {
        path: Utf8PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },

    #[error("tsc at {path} reports version {found:?}, not a supported version (expected one of {expected:?})")]
    UnsupportedVersion { path: Utf8PathBuf, found: String, expected: &'static [&'static str] },

    #[error("could not determine tsc's version at {path}: {reason}")]
    VersionCheckFailed { path: Utf8PathBuf, reason: String },

    #[error("request {method:?} timed out after {after:?}")]
    Timeout { method: String, after: Duration },

    #[error("tsc exited unexpectedly (status: {status:?})")]
    Crashed { status: Option<std::process::ExitStatus> },

    #[error("tsc restarted {attempts} times within {window:?}; treating as permanently failed")]
    RestartStormDetected { attempts: u32, window: Duration },

    #[error("oversized frame: declared Content-Length {declared} exceeds the {cap}-byte cap")]
    OversizedFrame { declared: usize, cap: usize },

    #[error("malformed frame: {reason:?}")]
    MalformedFrame { reason: MalformedFrameReason },

    #[error("tsc returned a JSON-RPC error (code {code}): {message}")]
    RpcError { code: i64, message: String, data: Option<serde_json::Value> },

    #[error("session shut down")]
    SessionClosed,

    #[error("I/O error writing to tsc: {0}")]
    Write(#[source] Arc<std::io::Error>),

    #[error("I/O error reading from tsc: {0}")]
    Read(#[source] Arc<std::io::Error>),

    #[error("failed to serialize a request: {0}")]
    Serialize(#[source] Arc<serde_json::Error>),

    #[error("failed to reap tsc's process: {0}")]
    Reap(#[source] Arc<std::io::Error>),
}

/// A malformed **header** desyncs the stream (the next frame's byte offset becomes
/// unknowable) and is always fatal for the session. A malformed **body** is scoped to
/// one frame — the declared length was valid, so the reader can resync on the next
/// frame — and the caller decides whether that's fatal for its use case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MalformedFrameReason {
    NoContentLengthHeader,
    UnparsableContentLength,
    TruncatedBody,
    InvalidJson,
}

/// Required by `tokio_util::codec::Decoder::Error`: `FramedRead` surfaces a read-side
/// I/O failure through the decoder's own error type.
impl From<std::io::Error> for TscClientError {
    fn from(source: std::io::Error) -> Self {
        Self::Read(Arc::new(source))
    }
}
