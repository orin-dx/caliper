# 0006. Run the TypeScript backend as a second resolver, not a leaf patcher

**Status:** Accepted **Date:** 2026-09-26

## Context

The structural resolver can't substitute generics through alias chains or evaluate conditional and mapped types. Ten components account for 2,696 of the 2,720 props missing against react-docgen-typescript (RDT) in the 2026-07-25 baseline (`docs/benchmarks.md`). ADR 0004 deferred a fix until TypeScript 7 offered a usable route.

TypeScript 7.0 has no programmatic API. 7.1 adds one, available now as `tsc --api` in `typescript@next`. RDT's own algorithm runs on it and returned exactly RDT's props on 13 components (`docs/type-checker-integration.md`).

## Decision

Add an opt-in semantic resolver that runs RDT's algorithm on `tsc --api` and merges its result into the structural one per component. Structural stays the default and the fallback.

- `crates/core` defines an owned-data semantic result and does the merge. It stays free of I/O.
- A feature-gated crate spawns `tsc --api`, speaks its protocol and owns the process. Rust spawns it, in the CLI and the NAPI addon alike. The JS client is not involved.
- We pin an exact `typescript` build and check its version at startup, because the protocol has no version handshake.

## Consequences

- Components the structural resolver can't finish get RDT's prop set without a new heuristic per gap.
- The API is `typescript/unstable/*` and still changing. Pinning limits the damage, but a 7.1 change can force rework.
- The backend costs a process per run, so it is off by default.
- Users need `typescript` 7.1 or later. Without it, extraction still succeeds and emits a diagnostic.
- tsc sees a dependency edit only when told, so `WatchSession::update_file` must notify it, with a parity test against `extract()`.
- Checker types don't fit `PropType` losslessly, and `ExtractionOutput` has no schema version. A follow-up ADR decides the representation.

## Alternatives considered

- **Replace `Opaque` leaves with tsc answers** (ADR 0004's model). `OpaqueDetail` holds only a raw string and a reason, and can't be built or inspected outside `crates/core`. Some types resolve to `Named` on purpose with no diagnostic (bare generic parameters, TypeScript utility types), so no leaf marks them.
- **Call the JS client through NAPI.** The CLI is a standalone binary that uses Node only to load `docgen.config.ts`, the binding never calls into JS, and the resolver is synchronous rayon code.
- **Node sidecar**, as `config.rs` does for `docgen.config.ts`. It adds a process hop and a Node dependency. It is the fallback if protocol churn outpaces a Rust client.
- **Vendor typescript-go internals**, as tsgolint does. The repo was closed on 2026-08-20 and the API is the supported surface.
- **TypeScript 7 LSP.** Not evaluated further, because the API exposes each query the algorithm needs directly.
