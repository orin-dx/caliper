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
    /// The `tsc` process could not be started: the binary is missing or not executable.
    #[error(
        "tsc binary not found at {path} — install `typescript@next` (npm i typescript@next) or pass an explicit path ({source})"
    )]
    SpawnFailed {
        /// The path that was tried.
        path: Utf8PathBuf,
        /// Why the operating system refused to start it.
        #[source]
        source: Arc<std::io::Error>,
    },

    /// The `tsc` is a version this crate was not traced against.
    #[error("tsc at {path} reports version {found:?}, not a supported version (expected one of {expected:?})")]
    UnsupportedVersion {
        /// The `tsc` binary that was checked.
        path: Utf8PathBuf,
        /// The version its `package.json` declares.
        found: String,
        /// The versions that were accepted: exact versions or `major.minor` series.
        expected: &'static [&'static str],
    },

    /// The `tsc`'s version could not be read, for example because there is no `package.json` beside it.
    #[error("could not determine tsc's version at {path}: {reason}")]
    VersionCheckFailed {
        /// The `tsc` binary that was checked.
        path: Utf8PathBuf,
        /// What went wrong reading the version.
        reason: String,
    },

    /// A request got no response within its deadline.
    #[error("request {method:?} timed out after {after:?}")]
    Timeout {
        /// The JSON-RPC method that was called.
        method: String,
        /// The deadline that passed.
        after: Duration,
    },

    /// The `tsc` process exited while the session was live.
    #[error("tsc exited unexpectedly (status: {status:?})")]
    Crashed {
        /// Its exit status, when the operating system reported one.
        status: Option<std::process::ExitStatus>,
    },

    /// A frame declared a body larger than the cap, so it was not read.
    #[error("oversized frame: declared Content-Length {declared} exceeds the {cap}-byte cap")]
    OversizedFrame {
        /// The body length the frame's header declared, in bytes.
        declared: usize,
        /// The largest body accepted, in bytes.
        cap: usize,
    },

    /// A frame could not be decoded; see [`MalformedFrameReason`] for whether the stream can recover.
    #[error("malformed frame: {reason:?}")]
    MalformedFrame {
        /// What was wrong with it.
        reason: MalformedFrameReason,
    },

    /// `tsc` answered a request with a JSON-RPC error.
    #[error("tsc returned a JSON-RPC error (code {code}): {message}")]
    RpcError {
        /// The JSON-RPC error code.
        code: i64,
        /// The error message `tsc` sent.
        message: String,
        /// Extra data the error carried, when it had any.
        data: Option<serde_json::Value>,
    },

    /// The session was shut down, so the request cannot be answered.
    #[error("session shut down")]
    SessionClosed,

    /// Writing a frame to `tsc`'s stdin failed.
    #[error("I/O error writing to tsc: {0}")]
    Write(#[source] Arc<std::io::Error>),

    /// Reading from `tsc`'s stdout failed.
    #[error("I/O error reading from tsc: {0}")]
    Read(#[source] Arc<std::io::Error>),

    /// A request could not be turned into JSON, so it was not sent.
    #[error("failed to serialize a request: {0}")]
    Serialize(#[source] Arc<serde_json::Error>),

    /// Waiting for `tsc`'s process to finish after shutdown failed.
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
    /// The header block has no `Content-Length` line. Header failure: fatal for the session.
    NoContentLengthHeader,
    /// The `Content-Length` value is not a number. Header failure: fatal for the session.
    UnparsableContentLength,
    /// The stream ended before the declared body length arrived.
    TruncatedBody,
    /// The body arrived in full but is not valid JSON; the next frame can still be read.
    InvalidJson,
}

/// Required by `tokio_util::codec::Decoder::Error`: `FramedRead` surfaces a read-side
/// I/O failure through the decoder's own error type.
impl From<std::io::Error> for TscClientError {
    fn from(source: std::io::Error) -> Self {
        Self::Read(Arc::new(source))
    }
}
