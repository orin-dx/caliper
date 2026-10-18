# crates/core

Pure extraction logic — no terminal, no NAPI, no async. Everything here must be `Send + Sync` for rayon.

## Module layout

```
types/          — shared data types (the contract between phases)
extractor/      — OXC AST visitor → SourceData
resolver/       — SourceData + GlobalSourceData → ComponentEntry
pipeline/       — orchestration: discover → parse → merge → resolve
cache.rs        — DTS parse cache (mtime+size invalidation, msgpack)
import_map.rs   — import resolution map
known.rs        — library-specific type shortcuts (SxProps, VariantProps…)
react_types.rs  — React builtin recognition + notable HTML attrs
```

## Types discipline

When adding a public type, run through the checklist in the `rust-types` skill. Canonical reference: `src/types/diagnostic.rs`.

## Resolver

`resolve_component()` is called in parallel via rayon. All inputs must be owned or `Arc`-wrapped — no borrowed data crosses thread boundaries. `ResolveState` accumulates diagnostics and visited-type tracking for a single resolution call; it is not shared.

## Panic containment

Anything reachable from a rayon `.map()` or a `DocgenPlugin` impl must go through `panic_guard::contain_panic` — see `pipeline/mod.rs` and `plugin.rs` for the call sites, and `docs/adr/0005-panic-containment-boundary.md` for why.

## Snapshot tests

Seven fixtures in `crates/core/tests/snapshots/`. If a change affects output, regenerate with `/snapshot` before committing.
