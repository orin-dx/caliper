# 0004. Defer full type-checker integration

**Status:** Superseded by 0006 **Date:** 2026-07-22 (retroactive)

## Context

OXC's structural, no-type-checking approach (`0001`) can't resolve conditional types, mapped types over an unbound generic, or a few `@emotion/styled`-specific call shapes — these need real type inference. TypeScript 7 is now a stable native compiler and LSP language service, but a stable programmatic contract for the per-node facts this project needs is still a separate question.

## Decision

Don't integrate a type checker now. Every gap that would need one degrades to a `Diagnostic` plus an `Opaque` prop type instead of a wrong answer, and gets tracked in `docs/rdt-coverage.md`'s "Known gaps summary" table. Revisit when either a stable public programmatic API or the TypeScript 7 LSP can supply the required facts and passes the accuracy, lifecycle, and cost gates in `docs/type-checker-integration.md`.

## Consequences

- A real, bounded set of real-world types (see `docs/type-checker-integration.md` for the current list) shows up as `Opaque` in extraction output until this changes.
- No dependency on an unstable, fast-moving API in the meantime.
- The eventual semantic process/client stays outside `crates/core`; core retains an owned-data enrichment contract and no I/O/async responsibility.
- The gap list needs to stay current in `docs/type-checker-integration.md` and `docs/rdt-coverage.md` as new patterns get found — otherwise this ADR quietly goes stale.
