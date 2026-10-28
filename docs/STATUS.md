# Status

**Updated:** 2026-10-22

Core extraction, resolver, CLI, NAPI binding, and Vite plugin all work and are fully tested. Config file loading, watch mode (including rebuilds on config changes and polling with `ensure_fresh`), plugin architecture, TOON token-optimized output format, JSON schema export, and bounded atomic DTS caching are implemented and verified.

**Types from other packages.** Imports resolve to any file under a source directory, `.d.ts` included, so a monorepo whose packages are all source directories resolves across them. By default the declarations those names point to are loaded (ADR 0010, on by default since ADR 0019): a prop typed by an installed package resolves. `--no-load-dependencies` (config, NAPI and Vite `loadDependencies: false`) reads only `@types/react` and TypeScript's own lib files from `node_modules`, and then a prop whose type comes from any other installed package stays a named reference with an `UNRESOLVABLE_IMPORT` warning. Checked against RDT on five installed repos, loading reports far more of RDT's props and changes 11 matching types in 59,000. Chakra, the worst case, costs about 220 ms and 330 MiB against 50 ms and 100 MiB without (`docs/performance.md`).

## Numbers

- Rust tests (`cargo nextest` across the workspace, plus the NAPI binding tests) and vitest (unit with a mocked binding, integration against the real native binding), all green in CI
- `cargo clippy --workspace --all-targets -D warnings` clean with `unsafe_code = "deny"` enforced workspace-wide
- 20 real-world fixture libraries validated against `react-docgen-typescript` (shadcn, Radix, MUI, Chakra, Mantine, React Aria, antd, ariakit, ark-ui, base-ui, blueprint, day-picker, fluentui, headlessui, panda, react-final-form, react-resizable-panels, storybook-emotion, tanstack-table, zendesk-garden — see `rdt-coverage.md`)
- **Recall** against `react-docgen-typescript` with dependencies installed and loaded (`docs/parity.md`, 2026-10-23): share of RDT's non-React props reported Mantine 99.7%, Chakra 98.6%, shadcn/ui 39.5%, Ant Design 80.3%, react-spectrum 99.7%.
- **Precision** per detection rule (`docs/precision.md`, 2026-10-21): 1 component that isn't one in 154 sampled, `functionDeclaration` 96% and every other rule 100%. 25 samples per rule can't yet establish the 95% gate.
- **Speed** on five pinned public corpora (`docs/performance.md`, 2026-10-22): 34 to 81 ms wall time and 66 to 153 MB peak RSS for 41 to 490 kLOC, 17 to 39 times faster than `react-docgen-typescript` on the same files. CI fails on a 10% regression against main or a ratio under 10×.

## Features and improvements

- **`htmlAttributes` defaults to `full`** (ADR 0024) — every attribute `@types/react` declares is listed, `ref` for `forwardRef` and `ComponentProps<"div">` props; `curated` is an opt-in compact mode. Costs against the old default: wall time +9 to +31%, peak RSS +12 to +37%, canonical JSON +42 to +105%.
- **Testing Stack** — `cargo-nextest` process runner, `rstest` parameterized spec tables, `insta` snapshots, and `trycmd` executable Markdown CLI specs.
- **Plugin system** (`DocgenPlugin` / `PluginRegistry`) — Extensible AST and component resolution hooks in `crates/core/src/plugin.rs`.
- **TOON output format** (`--format toon` / `toon.rs`) — Token-optimized format for LLM agents, cutting context window token usage by ~65-75%.
- **JSON schema export** (`caliper-docgen schema`) — Machine-readable Draft-07 JSON Schema export for component metadata validation.
- **Atomic & Bounded DTS cache** (`cache.rs`) — Atomic temp-file swap writes, dirty flag tracking, and 5,000 entry eviction cap.
- **Strict Safe Rust** — `unsafe_code = "deny"` via `[workspace.lints]`, active across all three crates (`core`, `cli`, `binding`) — none currently use `unsafe`, including the NAPI FFI boundary (napi-rs's `#[napi]` macros generate the glue without user-visible `unsafe`).

## Known gaps that won't get fixed without a type checker

See the "Known gaps summary" table in `rdt-coverage.md` for the full, maintained list. The short version: conditional types that need inference (`infer`, an unbound generic), mapped types over an unbound generic, and a couple of `@emotion/styled`-specific call shapes all need real type inference OXC deliberately doesn't do. Not bugs. A conditional, indexed access or mapped type whose operands are known is evaluated (`resolver/evaluate.rs`).

An opt-in TypeScript 7 semantic backend is implemented as slice 1 (ADR 0006, ADR 0007), experimental and off by default: `extract --experimental-semantic-types`, see `type-checker-integration.md`. Accuracy is verified against real RDT — 0 missing/extra props on 13 hand-picked components, and re-verified on `7.1.0-beta` against the full 44-file fixture corpus (5,508/5,522 `required` matches, 5,266/5,522 type-string matches; the gap is explained noise, not a resolver bug) — and the RDT oracle that comparison depends on now runs on its own `typescript@6` install, separate from the semantic probe's `typescript@7.1.0-beta`. TypeScript 7.1's checker also runs, test-only, as an oracle for structural prop types (`docs/parity.md`). A missing, crashed or timed-out `tsc` falls back to the structural result with one warning; the open lifecycle items are in `type-checker-integration.md`'s Gates table.

## Experiments that didn't pan out

**Confidence routing (2026-10-25).** The idea: score each component's risk of wrong or missing props from Caliper's own output, and ask the TypeScript 7.1 checker only about the risky ones. Measured against the checker on 518,268 shared props over the five corpora and the fixtures, routing 10% of components caught 23% of the props only the checker reports with the existing warning rule (a component with a degrade warning or error), against 3% for a smoothed table and 6% for a logistic model. At a 20% budget neither model caught the one prop the oracle marks `wrong`. Most of the checker-only props were Next's ambient augmentation (1,154 of 1,182 on shadcn) or false component matches, which a score of structural risk can't learn. So no score ships: warnings stay the signal, and on-demand checker use asks about the requested component instead of predicting. The script was deleted; the method is `scripts/parity.py oracle` plus the numbers above.

## Where to look next

- **Open gaps** → `edge-cases.md` — the gaps still open. Add one when you find a gap you aren't fixing now; delete it when you fix it.
- **What broke and why, historically** → `rdt-coverage.md` — every bug found during real-library validation, root cause, and the fix. Keep this updated; it's the project's memory.
- **Why a hard-to-reverse decision was made** → `docs/adr/` — OXC over the TypeScript compiler, manual serde for `PropType`, positional msgpack encoding, panic containment, the TypeScript 7 semantic backend. Write a new one when you make a decision like these (see `docs/adr/README.md`).
- **How the pipeline fits together** → `ARCHITECTURE.md`, and `docs/architecture/` for per-subsystem invariants and known gaps (moved from `.claude/semantic-model/` so every coding agent can read it, not just Claude Code — `AGENTS.md` is now this repo's shared instructions file, with per-tool mirrors under `.codex/`, `.cursor/`, `.gemini/`, `.github/`, `.opencode/`).
- **Setup, testing, code style, commit conventions** → `CONTRIBUTING.md`
- **Migrating from react-docgen-typescript** → `MIGRATING.md`
