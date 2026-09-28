# Status

**Updated:** 2026-08-13

Core extraction, resolver, CLI, NAPI binding, and Vite plugin all work and are fully tested. Config file loading, cross-package `.d.ts` resolution, watch mode, plugin architecture, TOON token-optimized output format, JSON schema export, LSP server scaffold, and bounded atomic DTS caching are all fully implemented and verified.

## Numbers

- 545 Rust tests (77 cli + 443 core unit + 8 snapshot + 1 core compile-fail + 16 napi binding), 36 vitest (33 unit, mocked NAPI + 3 integration against the real native binding — `packages/vite-plugin/tests/integration/`, gated on a locally-built `.node` binary, auto-skips otherwise) — all green
- `cargo clippy --workspace --all-targets -D warnings` clean with `unsafe_code = "deny"` enforced workspace-wide
- 20 real-world fixture libraries validated against `react-docgen-typescript` (shadcn, Radix, MUI, Chakra, Mantine, React Aria, antd, ariakit, ark-ui, base-ui, blueprint, day-picker, fluentui, headlessui, panda, react-final-form, react-resizable-panels, storybook-emotion, tanstack-table, zendesk-garden — see `rdt-coverage.md`)

## Features and improvements

- **Testing Stack** — `cargo-nextest` process runner, `rstest` parameterized spec tables, `insta` snapshots, and `trycmd` executable Markdown CLI specs.
- **Plugin system** (`DocgenPlugin` / `PluginRegistry`) — Extensible AST and component resolution hooks in `crates/core/src/plugin.rs`.
- **TOON output format** (`--format toon` / `toon.rs`) — Token-optimized format for LLM agents, cutting context window token usage by ~65-75%.
- **JSON schema export** (`oxc-react-docgen schema`) — Machine-readable Draft-07 JSON Schema export for component metadata validation.
- **Atomic & Bounded DTS cache** (`cache.rs`) — Atomic temp-file swap writes, dirty flag tracking, and 5,000 entry eviction cap.
- **LSP server scaffold** (`oxc-react-docgen lsp`) — bounded framing plus initialize/shutdown/exit lifecycle. It currently advertises `hoverProvider: false` and does not serve component prop hovers yet.
- **Strict Safe Rust** — `unsafe_code = "deny"` via `[workspace.lints]`, active across all three crates (`core`, `cli`, `binding`) — none currently use `unsafe`, including the NAPI FFI boundary (napi-rs's `#[napi]` macros generate the glue without user-visible `unsafe`).

## Known gaps that won't get fixed without a type checker

See the "Known gaps summary" table in `rdt-coverage.md` for the full, maintained list. The short version: conditional types, mapped types over an unbound generic, and a couple of `@emotion/styled`-specific call shapes all need real type inference OXC deliberately doesn't do. Not bugs. An opt-in TypeScript 7 semantic backend is designed but not implemented: see ADR 0006 and `type-checker-integration.md`.

## Where to look next

- **Edge cases, failure modes, and gaps not yet fixed** → `edge-cases.md` — comprehensive audit of crash/hang risks, silent data loss, and silent correctness bugs across every subsystem, prioritized. Update it as findings get fixed or new ones turn up.
- **Why those gaps happened, and the plan to fix them** → `root-cause-analysis.md` — the edge-case findings collapsed into 11 mechanism-level root causes with concrete fix proposals, plus a Phase 1 task breakdown.
- **What broke and why, historically** → `rdt-coverage.md` — every bug found during real-library validation, root cause, and the fix. Keep this updated; it's the project's memory.
- **Why a hard-to-reverse decision was made** → `docs/adr/` — OXC over the TypeScript compiler, manual serde for `PropType`, positional msgpack encoding, panic containment, the TypeScript 7 semantic backend. Write a new one when you make a decision like these (see `docs/adr/README.md`).
- **How the pipeline fits together** → `ARCHITECTURE.md`
- **Setup, testing, code style, commit conventions** → `CONTRIBUTING.md`
- **Migrating from react-docgen-typescript** → `MIGRATING.md`
- **Old build-out specs, resolved questions, point-in-time analyses** → `docs/archive/` — historical, not current, kept for reference
