# 0005. Contain panics at a single, per-item boundary

**Status:** Accepted **Date:** 2026-08-05

## Context

There was no `catch_unwind` anywhere in the repo. What safety existed was accidental:

- `extract_all`, `extract_file_incremental`, and `initialize_session` happened to run inside `tokio::task::spawn_blocking`, whose `JoinError` catches an unwind as a side effect of tokio's own plumbing — not a decision anyone made.
- `pipeline/mod.rs`'s rayon `.map()` closures had no per-item boundary.
- `plugin.rs` called each `DocgenPlugin` hook with zero isolation between plugins.
- `create_session`/`close_session` skipped `spawn_blocking` entirely and got no protection at all.
- `watch.rs`'s `std::sync::Mutex` could poison permanently — a direct consequence of an unprotected panic happening while the lock was held.

## Decision

Panics reachable from a rayon `.map()`, a `DocgenPlugin` hook, or a NAPI entry point are caught at `crates/core/src/panic_guard.rs`'s `pub fn contain_panic<T>(label: &str, f: impl FnOnce() -> T) -> Result<T, Diagnostic>`, converting the payload into a `Diagnostic` tagged `DiagnosticCode::InternalPanic` instead of aborting a batch, killing the whole pipeline, or poisoning a lock.

- **`AssertUnwindSafe` lives inside `contain_panic`**, so callers never reason about `UnwindSafe` themselves. This matters because plugin-hook call sites pass `&mut SourceData`/`&mut ComponentEntry`, and `&mut T` is unconditionally `!UnwindSafe` in std. It's safe here because `SourceData`/`ComponentEntry` have no interior mutability (`Cell`/`RefCell`) that could leave a torn, observable half-write behind a caught panic — the whole per-item operation is abandoned, not a partial mutation trusted afterward.
- **Call sites:** the rayon parse-phase and resolve-phase closure bodies in `pipeline/mod.rs`; each individual `plugin.on_x(...)` call inside `PluginRegistry::run_on_file_extracted`/`run_on_component_resolved` in `plugin.rs` (not the whole loop, so one bad plugin degrades only itself); all five NAPI entry points in `crates/binding/src/lib.rs`.
- **Defense in depth:** `watch.rs`'s poisoned-lock `.expect()` was replaced with `.unwrap_or_else(|p| p.into_inner())`.

## Consequences

- One bad file, plugin, or session call degrades to a diagnostic instead of taking down its batch, the pipeline, or the session.
- `PluginRegistry::run_on_file_extracted` and `run_on_component_resolved` changed their return type from `()` to `Vec<Diagnostic>` to carry caught-panic diagnostics back to the pipeline — a real API change, not just internal wiring.
- `extract_all`, `extract_file_incremental`, and `initialize_session` were already panic-safe via `spawn_blocking`'s `JoinError`; wrapping their bodies in `contain_panic` makes that guarantee documented and consistent with the rest of the codebase rather than incidental. `create_session` and `close_session` were genuinely unprotected before this and are now wrapped directly.
- `watch.rs`'s `std::sync::Mutex` poisoning is now defense-in-depth rather than a live risk — panics upstream (rayon batches, plugin hooks) are contained before they'd reach that lock while held.
- Every future concurrent, plugin, or FFI entry point has one obvious place to route through instead of re-deriving the answer.

## Alternatives considered

napi-rs (the pinned version) has its own opt-in `#[napi(catch_unwind)]` attribute that wraps a function body in `catch_unwind` at the FFI boundary. It would work for the five NAPI entry points, but we used `contain_panic` explicitly there too, for one mechanism across rayon, plugins, and FFI instead of two.
