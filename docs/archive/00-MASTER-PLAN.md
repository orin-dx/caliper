# oxc-react-docgen — Master Implementation Plan

## What We're Building

A Rust-powered React prop extraction tool that replaces `react-docgen-typescript`.

- **10-100x faster** via OXC (no TypeScript compiler program)
- **Cross-package monorepo support** via import graph + .d.ts parsing
- **Drop-in RDT output** for existing Storybook setups
- **World-class CLI** with miette diagnostics
- **Vite plugin** — primary delivery mechanism

## Repository Structure

```
oxc-react-docgen/
├── Cargo.toml                 # workspace root
├── crates/
│   ├── core/                  # extraction logic — ZERO terminal/NAPI deps
│   ├── binding/               # thin NAPI wrapper over core (was crates/napi/)
│   └── cli/                   # clap + miette + indicatif
├── packages/
│   ├── napi/                  # npm: @oxc-react-docgen/napi (TS types + dev binary loader)
│   └── vite-plugin/           # npm: @oxc-react-docgen/vite-plugin
├── apps/
│   └── validate/              # comparison harness vs react-docgen + react-docgen-typescript
├── fixtures/                  # real library test fixtures
│   ├── radix/
│   ├── shadcn/
│   ├── mui/
│   ├── react-aria/
│   └── panda/
└── .github/
    └── workflows/             # CI

```

## Dependency Graph Between Agents

```
Phase 0: Repository Setup (1 agent, must finish first)
    └─► Phase 1: Types + Fixtures (2 agents, parallel)
            └─► Phase 2: Extractor + ImportMap + Known (3 agents, parallel)
                    └─► Phase 3: Resolver + Pipeline (2 agents, parallel)
                            └─► Phase 4: NAPI + CLI (2 agents, parallel)
                                    └─► Phase 5: Vite Plugin (1 agent)
                                            └─► Phase 6: Integration + Tests (1 agent)
```

## Phase Summary

| Phase | Agents         | Depends On | Deliverable                            |
| ----- | -------------- | ---------- | -------------------------------------- |
| 0     | Repo Setup     | —          | Workspace, CI, tooling                 |
| 1a    | Types          | Phase 0    | `types.rs` — the shared contract       |
| 1b    | Fixtures       | Phase 0    | Real library .d.ts + component files   |
| 2a    | Extractor      | Phase 1    | OXC parse loop → SourceData            |
| 2b    | Import Map     | Phase 1    | ImportMap + ReExportMap                |
| 2c    | Known Patterns | Phase 1    | known.rs match function                |
| 3a    | Resolver       | Phase 2    | PropResolver → ComponentEntry          |
| 3b    | Pipeline       | Phase 2    | Rayon orchestration + GlobalSourceData |
| 4a    | NAPI           | Phase 3    | napi crate + TS types                  |
| 4b    | CLI            | Phase 3    | clap/miette CLI                        |
| 5     | Vite Plugin    | Phase 4a   | @oxc-react-docgen/vite                 |
| 6     | Integration    | Phase 5    | E2E tests, benchmarks, docs            |

## Non-Negotiable Constraints (All Agents Must Respect)

1. **No AST refs escape the parse function** — OXC allocator is per-file, per-call
2. **No `HashMap` in `core/`** — use `FxHashMap` from `rustc-hash` for internal maps, `BTreeMap` for JSON-facing output (determinism)
3. **No `String` for type names in hot paths** — use `CompactString` from `compact_str`
4. **No trait objects (`Box<dyn Trait>`) for pattern dispatch** — use match arms
5. **No `unwrap()` in library code** — propagate errors with `?` and `thiserror`
6. **No terminal/display code in `crates/core/`** — zero dependency on indicatif, console, ratatui
7. **No `Tier 4 / Corsa / tsgo` implementation** — out of scope, add TODO comments only
8. **No topological sort** — not needed; collect all, then resolve
9. **`miette` without `fancy` feature in core** — fancy only in CLI

## Key Crate Versions

```toml
oxc_allocator = "0.60" # pin to same version as oxc_parser
oxc_parser = "0.60"
oxc_ast = "0.60"
oxc_span = "0.60"
oxc_module_lexer = "0.60"
oxc_resolver = "2.0" # separate versioning from main oxc
rayon = "1.10"
dashmap = "6.1"
arc-swap = "1.7"
compact_str = "0.8"
rustc-hash = "2.1"
camino = "1.1"
serde = "1.0"
serde_json = "1.0"
rmp-serde = "1.3"
thiserror = "2.0"
miette = "7.2"
ignore = "0.4"
lsp-types = "0.95"
clap = "4.5"
indicatif = "0.17"
watchexec = "5.0"
napi = "2.16"
napi-derive = "2.16"
divan = "0.1"
insta = "1.42"
```
