//! Process supervisor and JSON-RPC transport client for TypeScript 7's `tsc --api`.
//!
//! Standalone: this crate has no dependency on `caliper-docgen-core` and nothing in
//! the workspace consumes it yet. It exists to satisfy the Lifecycle gate in
//! `docs/type-checker-integration.md` ("Gates before it ships") — missing binary,
//! unsupported version, timeout, crash, restart storm, malformed/oversized frame,
//! partial project failure, shutdown reaping, stale notification — before any of the
//! actual `typescript/unstable/*` checker methods get wired up to a merge step.
//!
//! # Session invariants
//!
//! A session owns its child's I/O and ends exactly once. Keep it that way:
//!
//! - Only the writer task holds `ChildStdin`. Callers send whole frames over a bounded channel, so a request future
//!   dropped mid-flight has enqueued a full frame or nothing. A shared mutex around the pipe, or a write timeout, brings
//!   back partial frames that desync tsc's decoder and wedge every clone.
//! - The closed state is set once, with a reason, by reader exit, a write failure or `Supervisor::shutdown`. Setting it
//!   fails every pending waiter and stops the writer, so stdin closes even while `Client` clones live.
//! - The supervisor decides restarts from the closed state alone. Matching error variants or polling task handles lets
//!   a session end without a restart, as a fatal frame from a live child once did.
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
