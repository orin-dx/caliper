//! Process supervisor and JSON-RPC transport client for TypeScript 7's `tsc --api`.
//!
//! Standalone: this crate has no dependency on `oxc-react-docgen-core` and nothing in
//! the workspace consumes it yet. It exists to satisfy the Lifecycle gate in
//! `docs/type-checker-integration.md` ("Gates before it ships") — missing binary,
//! unsupported version, timeout, crash, restart storm, malformed/oversized frame,
//! partial project failure, shutdown reaping, stale notification — before any of the
//! actual `typescript/unstable/*` checker methods get wired up to a merge step.
//!
//! This crate deliberately has no `crates/core`-style panic containment
//! (`panic_guard::contain_panic`): that exists to contain panics from caller-supplied
//! closures run on rayon/NAPI boundaries. Nothing here calls an arbitrary caller
//! closure — the only untrusted input is bytes from a child process, handled through
//! `Result`, never `.unwrap()`/`.expect()`.

mod client;
mod error;
mod protocol;
mod supervisor;
pub mod transport; // also used by tests/bin/fake_tsc.rs, so client and fake server can't drift on wire format

pub use client::{Client, TaggedResponse};
pub use error::{MalformedFrameReason, TscClientError};
pub use protocol::Notification;
pub use supervisor::{Supervisor, SupervisorConfig};
