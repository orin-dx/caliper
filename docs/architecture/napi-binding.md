# NAPI binding

**Source:** `crates/binding/src/lib.rs`, `crates/binding/build.rs`

This is the only consumer-facing FFI boundary in the workspace: a thin `cdylib` that Node.js (concretely, a Vite plugin) calls into. It owns no extraction logic — every entry point marshals `JsExtractOptions` into `PipelineOptions` and delegates straight to `caliper_docgen_core::pipeline`. Its entire job is getting values across the Rust/JS boundary without ever letting a Rust panic reach the Node process, and without ever handing a caller anything other than a proper JS `Error` on failure.

## The six NAPI entry points

- `extract_all(options) -> String` — cold, one-shot extraction. Runs inside `tokio::task::spawn_blocking`.
- `create_session(options) -> u32` — allocates a session ID, builds a `WatchSession`, inserts it into `SESSIONS`. Runs **synchronously on the calling (JS) thread** — no `spawn_blocking`.
- `initialize_session(session_id, options) -> String` — looks up or lazily creates the session, then returns `WatchSession::initialize()`: the session's current output, running the cold extraction first if the session has none. The Vite plugin's virtual-module `load` uses it as the snapshot. Runs inside `spawn_blocking`.
- `extract_file_incremental(file_path, session_id, options) -> String` — looks up or lazily creates the session, then runs `WatchSession::update_file()`. Runs inside `spawn_blocking`.
- `session_docgen_info(session_id, keys) -> String` — the named components of the session's current revision in react-docgen-typescript's `__docgenInfo` shape (`pipeline::rdt::docgen_info`), for the Storybook provider (`packages/storybook`). Reads `WatchSession::current_output` without copying it; a key the session lacks, or any key before `initialize_session`, is left out. Runs synchronously on the calling thread, with `contain_panic` directly.
- `close_session(session_id) -> ()` — removes the session from `SESSIONS`. Runs synchronously on the calling thread, like `create_session`.

`SESSIONS` is `static SESSIONS: LazyLock<DashMap<u32, Arc<WatchSession>>>` — a single process-wide session table. `initialize_session` and `extract_file_incremental` both tolerate a missing session ID by silently constructing a fresh `WatchSession` from the passed `options` and inserting it (`lib.rs:171-179`, `200-208`) — this is a lazy-recovery path, not an error case, so a caller that races `create_session` against `initialize_session` (or never calls `create_session` at all) still gets a working session rather than an "unknown session ID" error.

## Panic-containment boundary (LOCKED DESIGN DECISION — ADR 0005)

Every one of the five entry points wraps its real body in `caliper_docgen_core::panic_guard::contain_panic(label, closure)`. This is not incidental defense — it is a deliberate, documented design decision (`docs/adr/0005-panic-containment-boundary.md`) with a specific and non-obvious shape:

- **The three `spawn_blocking`-wrapped entry points** (`extract_all`, `initialize_session`, `extract_file_incremental`) put `contain_panic` **inside** the `spawn_blocking` closure, not around it. Before ADR 0005, these three were only accidentally panic-safe: `tokio::task::spawn_blocking`'s `JoinError` happens to catch an unwind as a side effect of tokio's own plumbing, not because anyone decided panic-safety belonged there. Wrapping in `contain_panic` too makes the guarantee a designed one, converts the panic into a `Diagnostic`-derived `napi::Error` message instead of an opaque `JoinError`, and stops being contingent on tokio's internals never changing.
- **`create_session` and `close_session` get `contain_panic` directly**, with no `spawn_blocking` at all, because they run synchronously on the thread the JS call arrived on. Before ADR 0005 these two had **zero** panic protection — not even the accidental kind — documented as an open, unconfirmed risk in `docs/edge-cases.md` P1-9 ("_(Needs verification, not confirmed)_... Whether a panic here crashes the Node process depends on whether napi-rs's codegen auto-wraps in `catch_unwind`"). ADR 0005 closes that gap directly rather than relying on napi-rs's opt-in `#[napi(catch_unwind)]` attribute (considered and rejected in the ADR's "Alternatives considered" section, in favor of one mechanism — `contain_panic` — shared across rayon, plugin hooks, and FFI instead of two different ones). **Note for future readers of `docs/edge-cases.md`: P1-9 as currently written is stale** — it describes a state ADR 0005 has since fixed; it should be marked resolved rather than open next time that document is touched.

Do not remove or "simplify away" the `contain_panic` wrapping in `create_session`/`close_session` on the theory that synchronous code "can't really panic" — `next_session_id()`, `WatchSession::new()`, and `PipelineOptions::try_from` all run inside that closure, and `WatchSession::new` is not a leaf function under this crate's own non-negotiables (no `unwrap()` outside `#[cfg(test)]`, but a _panic_ — e.g. an indexing panic or arithmetic overflow in debug builds — is a different failure mode than `unwrap()` and is exactly what `contain_panic` exists to catch regardless of whether the non-negotiables are honored everywhere upstream).

## Why this crate's panic discipline is higher-stakes than `crates/cli`'s

`panic_guard::contain_panic` is shared code — the same function backs rayon `.map()` closures in `pipeline/mod.rs`, `DocgenPlugin` hook calls in `plugin.rs`, and all six entry points here. But the failure mode on the other side of an _uncaught_ panic differs by boundary. `crates/cli` is a standalone binary process: an uncaught panic there kills one CLI invocation, and the OS process boundary means nothing else is affected. This crate is a `cdylib` loaded **in-process** into a long-running Node.js server (a Vite dev server, per the doc comments on `create_session`/`initialize_session`/`close_session` referencing `configResolved`/`configureServer`/`buildEnd`). An uncaught Rust panic crossing the FFI boundary here does not fail one call — it is undefined behavior at best and a hard crash of the entire Node process at worst, taking down every other request the dev server was serving. That asymmetry is why `contain_panic` at this boundary is load-bearing in a way it is only "good hygiene" for `crates/cli`: bypass it here and the blast radius is the whole host process, not one invocation.

## Error conversion: everything converges on `napi::Error::from_reason(String)`

Every fallible path in this file — `PipelineOptions::try_from` validation failures, `contain_panic`'s caught-panic `Diagnostic`, `extraction_output_to_json`/`incremental_update_to_json` serialization errors, and `spawn_blocking`'s own `JoinError` — is mapped through `.map_err(|e| napi::Error::from_reason(e.to_string()))` or the equivalent `Err(diag) => Err(napi::Error::from_reason(diag.to_string()))` arm. There is no code path in this file that lets a `Result::Err` propagate to napi-rs's codegen in any form other than `napi::Error`.

**Invariant 1: no Node caller of any of the six entry points can ever receive anything other than a resolved value or a proper JS `Error` — never an opaque crash, never a native panic message, never a bare Rust `Result` leaking through.** This is what makes `extract_all`/`initialize_session`/`extract_file_incremental`'s doubled `Result` handling (`spawn_blocking`'s `JoinError` outside, `contain_panic`'s `Result<T, Diagnostic>` inside) necessary rather than redundant — two independent failure channels (task-join failure, application-level panic) both have to be funneled into the one JS-visible error type.

**Invariant 2: `htmlAttributes` never silently defaults.** `TryFrom<JsExtractOptions> for PipelineOptions` treats every `Option<T>` field as "use `PipelineOptions::default()`" when `None`, _except_ `html_attributes`, which goes through `HtmlAttributeMode::parse` and hard-errors, naming the bad value, if the caller (e.g. via a TypeScript `as any` cast bypassing the `'curated' | 'full' | 'none'` `ts_type`) passes something that isn't one of the three accepted strings. This is a deliberate application of AGENTS.md non-negotiable #6 ("never fail silently") at the boundary where TypeScript's type system stops being a real guarantee.

**Invariant 3: `extraPathsJson`/`knownTypeOverridesJson` malformed JSON is a hard `Err`, never a silently-empty map.** Both fields are accepted as raw JSON strings specifically "to avoid napi object complexity" (`lib.rs:50`) — `serde_json::from_str` failure is mapped to an error naming the field (`"extraPaths is not valid JSON: {e}"` / `"knownTypeOverrides is not valid JSON: {e}"`), not swallowed into `Default::default()`. Covered directly by `malformed_extra_paths_json_is_a_hard_error_not_a_silent_empty_map` and its `known_type_overrides` sibling in the test module.

## Session-ID scheme (LOCKED DESIGN DECISION, with an accepted gap)

```rust
fn next_session_id() -> u32 {
    let pid = std::process::id() as u64;
    let counter = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    // Avoids session-ID collisions across concurrent Vite dev server instances.
    (((pid & 0xFFFF) << 16) | (counter & 0xFFFF)) as u32
}
```

The `u32` session ID is split into two 16-bit halves: the low 16 bits of the OS PID in the high half, a process-local monotonic counter in the low half. This is a deliberate choice, not an arbitrary bit-packing — the comment states its purpose directly: it keeps session IDs from colliding when multiple Vite dev server processes (e.g. two projects being developed in parallel on the same machine) each load this binding and start allocating session IDs from zero independently.

**Invariant 4: within a single process, `next_session_id()` cannot collide until more than 65,536 sessions have been created in that process's lifetime.** The counter half is masked to 16 bits (`& 0xFFFF`), so it wraps after 65,536 allocations and reuses a session ID that may still be live in `SESSIONS`. This is a known, accepted gap, not a live bug: 65,536 `create_session` calls in one long-running dev-server process is far outside realistic usage (one session per Vite dev server run, not per file change — `update_file`/`extract_file_incremental` reuse the existing session ID), so the ADR-0005-era hardening effort scoped this as low-severity and left it as-is rather than widening the ID to `u64` or adding collision detection on insert. Note: this specific collision-probability tradeoff is not separately written up in `docs/edge-cases.md` at the time of writing — the closest existing entries there concern the panic-safety of `create_session`/`close_session` (P1-9, now stale per above), not this counter-wrap scenario; treat this file as the primary citation for it. If this ever needs to be revisited, the fix is a wider ID type or an explicit `SESSIONS.contains_key` retry loop in `next_session_id`, not a redesign of the packing scheme.

## `JsExtractOptions` → `PipelineOptions`: the shape of the boundary

`JsExtractOptions` (`lib.rs:34-55`) is deliberately flat — every field is a primitive, `String`, `Vec<String>`, or `Option` thereof, `TryFrom<JsExtractOptions> for PipelineOptions` (`Error = String`, not a structured error type) is the single place this flat shape gets reinflated into the richer `PipelineOptions` that `pipeline::extract`/`WatchSession` actually consume. Every one of the five entry points calls this same `TryFrom` before doing anything else — there is no code path that constructs a `PipelineOptions` from `JsExtractOptions` any other way, so a new validation rule (like Invariant 2's `reactVersion` check) added here is guaranteed to apply uniformly across cold extraction, session creation, and session initialization.

## Known gaps

- `docs/edge-cases.md` P1-9 (`crates/binding/src/lib.rs:132-138,193-195`) predates ADR 0005 and describes `create_session`/`close_session` as unverified/possibly-unprotected. That has since been fixed by the direct `contain_panic` wrapping described above; the doc entry itself has not been updated to reflect it.
- The session-ID counter wrap (Invariant 4) has no collision-detection or retry — see above. Accepted low-severity gap, not tracked as a numbered entry in `docs/edge-cases.md`.
- `build.rs` is a bare two-line `napi_build::setup()` call with no project-specific logic — there is nothing else to document about it; it exists only because napi-rs's codegen requires a build script to wire up its own `cdylib` linkage.
