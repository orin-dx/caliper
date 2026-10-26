# Status

**Updated:** 2026-10-22

Core extraction, resolver, CLI, NAPI binding, and Vite plugin all work and are fully tested. Config file loading, watch mode (including rebuilds on config changes and polling with `ensure_fresh`), plugin architecture, TOON token-optimized output format, JSON schema export, and bounded atomic DTS caching are implemented and verified.

**Types from other packages.** Imports resolve to any file under a source directory, `.d.ts` included, so a monorepo whose packages are all source directories resolves across them. By default the declarations those names point to are loaded (ADR 0010, on by default since ADR 0019): a prop typed by an installed package resolves. `--no-load-dependencies` (config, NAPI and Vite `loadDependencies: false`) reads only `@types/react` and TypeScript's own lib files from `node_modules`, and then a prop whose type comes from any other installed package stays a named reference with an `UNRESOLVABLE_IMPORT` warning. Checked against RDT on five installed repos, loading reports far more of RDT's props and changes 11 matching types in 59,000. Chakra, the worst case, costs about 220 ms and 330 MiB against 50 ms and 100 MiB without (`docs/performance.md`).

## Numbers

- 745 Rust tests (729 workspace tests under `cargo nextest`, 16 NAPI binding tests) and 46 vitest (41 unit with a mocked binding, 5 integration against the real native binding), all green
- `cargo clippy --workspace --all-targets -D warnings` clean with `unsafe_code = "deny"` enforced workspace-wide
- 20 real-world fixture libraries validated against `react-docgen-typescript` (shadcn, Radix, MUI, Chakra, Mantine, React Aria, antd, ariakit, ark-ui, base-ui, blueprint, day-picker, fluentui, headlessui, panda, react-final-form, react-resizable-panels, storybook-emotion, tanstack-table, zendesk-garden — see `rdt-coverage.md`)
- **Recall** against `react-docgen-typescript`, own props, five source-only mirrors with no dependencies installed (`docs/projects/architecture-roadmap.md`, after #69): components found Mantine 93%, Chakra 92%, Ant Design 97%, shadcn/ui 98%, react-spectrum 99%; share of RDT's props reported 92%, 85%, 95%, 94%, 97%.
- **Precision** per detection rule (`docs/precision.md`, 2026-10-21): 1 component that isn't one in 154 sampled, `functionDeclaration` 96% and every other rule 100%. 25 samples per rule can't yet establish the 95% gate.
- **Speed** on five pinned public corpora (`docs/performance.md`, 2026-10-22): 34 to 81 ms wall time and 66 to 153 MB peak RSS for 41 to 490 kLOC, 17 to 39 times faster than `react-docgen-typescript` on the same files. CI fails on a 10% regression against main or a ratio under 10×.

## Features and improvements

- **Testing Stack** — `cargo-nextest` process runner, `rstest` parameterized spec tables, `insta` snapshots, and `trycmd` executable Markdown CLI specs.
- **Plugin system** (`DocgenPlugin` / `PluginRegistry`) — Extensible AST and component resolution hooks in `crates/core/src/plugin.rs`.
- **TOON output format** (`--format toon` / `toon.rs`) — Token-optimized format for LLM agents, cutting context window token usage by ~65-75%.
- **JSON schema export** (`caliper-docgen schema`) — Machine-readable Draft-07 JSON Schema export for component metadata validation.
- **Atomic & Bounded DTS cache** (`cache.rs`) — Atomic temp-file swap writes, dirty flag tracking, and 5,000 entry eviction cap.
- **Strict Safe Rust** — `unsafe_code = "deny"` via `[workspace.lints]`, active across all three crates (`core`, `cli`, `binding`) — none currently use `unsafe`, including the NAPI FFI boundary (napi-rs's `#[napi]` macros generate the glue without user-visible `unsafe`).

## Known gaps that won't get fixed without a type checker

See the "Known gaps summary" table in `rdt-coverage.md` for the full, maintained list. The short version: conditional types that need inference (`infer`, an unbound generic), mapped types over an unbound generic, and a couple of `@emotion/styled`-specific call shapes all need real type inference OXC deliberately doesn't do. Not bugs. A conditional, indexed access or mapped type whose operands are known is evaluated (`resolver/evaluate.rs`).

An opt-in TypeScript 7 semantic backend is designed (ADR 0006) but not implemented; it will ship as an experimental opt-in, off by default. Accuracy is verified against real RDT — 0 missing/extra props on 13 hand-picked components, and re-verified on `7.1.0-beta` against the full 44-file fixture corpus (5,508/5,522 `required` matches, 5,266/5,522 type-string matches; the gap is explained noise, not a resolver bug) — and the RDT oracle that comparison depends on now runs on its own `typescript@6` install, separate from the semantic probe's `typescript@7.1.0-beta`. TypeScript 7.1's checker also runs, test-only, as an oracle for structural prop types (`docs/parity.md`). Lifecycle (crash/restart/timeout handling for the `tsc --api` process) hasn't started; see `type-checker-integration.md`'s Gates table.

## Where to look next

- **Edge cases, failure modes, and gaps not yet fixed** → `edge-cases.md` — comprehensive audit of crash/hang risks, silent data loss, and silent correctness bugs across every subsystem, prioritized. Update it as findings get fixed or new ones turn up.
- **Why those gaps happened, and the plan to fix them** → `root-cause-analysis.md` — the edge-case findings collapsed into 11 mechanism-level root causes with concrete fix proposals, plus a Phase 1 task breakdown.
- **What broke and why, historically** → `rdt-coverage.md` — every bug found during real-library validation, root cause, and the fix. Keep this updated; it's the project's memory.
- **Why a hard-to-reverse decision was made** → `docs/adr/` — OXC over the TypeScript compiler, manual serde for `PropType`, positional msgpack encoding, panic containment, the TypeScript 7 semantic backend. Write a new one when you make a decision like these (see `docs/adr/README.md`).
- **How the pipeline fits together** → `ARCHITECTURE.md`, and `docs/architecture/` for per-subsystem invariants and known gaps (moved from `.claude/semantic-model/` so every coding agent can read it, not just Claude Code — `AGENTS.md` is now this repo's shared instructions file, with per-tool mirrors under `.codex/`, `.cursor/`, `.gemini/`, `.github/`, `.opencode/`).
- **Setup, testing, code style, commit conventions** → `CONTRIBUTING.md`
- **Migrating from react-docgen-typescript** → `MIGRATING.md`
- **Old build-out specs, resolved questions, point-in-time analyses** → `docs/archive/` — historical, not current, kept for reference
