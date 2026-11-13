# Status

**Updated:** 2026-11-12

Core extraction, resolver, CLI, NAPI binding, and Vite plugin all work and are fully tested. Config file loading, watch mode (including rebuilds on config changes and polling with `ensure_fresh`), plugin architecture, TOON token-optimized output format, JSON schema export, and bounded atomic DTS caching are implemented and verified.

**Types from other packages.** Imports resolve to any file under a source directory, `.d.ts` included, so a monorepo whose packages are all source directories resolves across them. By default the declarations those names point to are loaded (ADR 0010): a prop typed by an installed package resolves. `--no-load-dependencies` (config, NAPI and Vite `loadDependencies: false`) reads only `@types/react` and TypeScript's own lib files from `node_modules`, and then a prop whose type comes from any other installed package stays a named reference with an `UNRESOLVABLE_IMPORT` warning. Checked against RDT on five installed repos, loading reports far more of RDT's props and changes 11 matching types in 59,000. Chakra, the worst case, costs about 220 ms and 330 MiB against 50 ms and 100 MiB without (`docs/performance.md`).

## Numbers

- Rust tests (`cargo nextest` across the workspace, plus the NAPI binding tests) and vitest (unit with a mocked binding, integration against the real native binding), all green in CI
- `cargo clippy --workspace --all-targets -D warnings` clean with `unsafe_code = "deny"` enforced workspace-wide
- 20 real-world fixture libraries validated against `react-docgen-typescript` (shadcn, Radix, MUI, Chakra, Mantine, React Aria, antd, ariakit, ark-ui, base-ui, blueprint, day-picker, fluentui, headlessui, panda, react-final-form, react-resizable-panels, storybook-emotion, tanstack-table, zendesk-garden — see `rdt-coverage.md`)
- **Recall** against `react-docgen-typescript` with dependencies installed and loaded: the share of RDT's reference components and props that Caliper also reports, not a measure of correctness (the checker judges that; on the props both list, the checker contradicts Caliper on 0 to 1.09% and RDT on 0.006 to 5.02%, `docs/rdt-differences.md`, 2026-11-06). The weekly record and the current definition of the reference are in `docs/measurement.md`.
- **Precision** per detection rule (`docs/precision.md`, 2026-10-21): 1 component that isn't one in 154 sampled, `functionDeclaration` 96% and every other rule 100%. 25 samples per rule can't yet establish the 95% gate.
- **Speed, with the checker (the default):** `extract` of 938 Chakra UI and MUI wrappers takes 6.05 s and about 970 MiB, 787 Chakra wrappers alone 5.4 s and 770 MiB, 2,814 components 15.4 s and 1,053 MiB (`docs/gate-measurements.md`, 2026-11-12). A Storybook story gets the structural props at once and the checker's within 150 ms when prefetched or asked before; a cold `ChakraButton` takes 171 ms.
- **Speed, structural resolver (`--no-checker`)** on five pinned public corpora (`docs/performance.md`, 2026-10-22): 34 to 81 ms wall time and 66 to 153 MB peak RSS for 41 to 490 kLOC, 17 to 39 times faster than `react-docgen-typescript` on the same files; the 938 wrappers above take 0.10 s and 200 MiB. No pull-request gate enforces it: a weekly workflow records speed, memory (of the checker, the default) and recall against RDT and react-docgen, and `just bench` compares a branch with `main` locally (`docs/measurement.md`). These structural figures are by hand; the weekly's checker budgets are set after its first run.

## Features and improvements

- **`htmlAttributes` defaults to `full`** — every attribute `@types/react` declares is listed, `ref` for `forwardRef` and `ComponentProps<"div">` props; `curated` is an opt-in compact mode. Costs against the old default: wall time +9 to +31%, peak RSS +12 to +37%, canonical JSON +42 to +105%.
- **Testing Stack** — `cargo-nextest` process runner, `rstest` parameterized spec tables, `insta` snapshots, and `trycmd` executable Markdown CLI specs.
- **TOON output format** (`--format toon` / `toon.rs`) — Token-optimized format for LLM agents, cutting context window token usage by ~65-75%.
- **JSON schema export** (`caliper-docgen schema`) — Machine-readable Draft-07 JSON Schema export for component metadata validation.
- **Atomic & Bounded DTS cache** (`cache.rs`) — Atomic temp-file swap writes, dirty flag tracking, and 5,000 entry eviction cap. Keyed by content hash, and discarded whenever the build that wrote it differs.
- **Lazy sessions** (ADR 0016) — `WatchSession::lazy` parses and indexes every file and resolves a component when a request names it; the Storybook addon uses it by default (NAPI `lazy`, off by default). Each answer equals the batch extraction's for the same key. Numbers in `docs/performance.md`.
- **Dependency declarations loaded by default** (ADR 0010) — a prop typed by an installed package resolves; `--no-load-dependencies` turns it off.
- **Strict Safe Rust** — `unsafe_code = "deny"` via `[workspace.lints]`, active across all five crates — none currently use `unsafe`, including the NAPI FFI boundary (napi-rs's `#[napi]` macros generate the glue without user-visible `unsafe`).

## Known gaps of the structural resolver (the fallback, and `--no-checker`)

See the "Known gaps summary" table in `rdt-coverage.md` for the full, maintained list. The short version: conditional types that need inference (`infer`, an unbound generic), mapped types over an unbound generic, and a couple of `@emotion/styled`-specific call shapes all need real type inference OXC deliberately doesn't do: the checker, on by default, answers them. Not bugs. A conditional, indexed access or mapped type whose operands are known is evaluated (`resolver/evaluate.rs`).

**The TypeScript checker is the default** (ADR 0026, S8 done 2026-11-12). `extract`, `check` and `inspect` merge the built-in checker's answer (the `tsrs` crates, the only checker since trail 369 slice 4 removed `--tsc-path`) over the structural one prop by prop, napi sessions and the Storybook addon ask it per component within `semanticWaitMs` (ADR 0022), and `--no-checker` (`checker: false`) turns it off; `watch` and the Vite plugin stay structural. A prop the checker could not read keeps the structural type with `degradedBy`, and a checker that fails to start or times out falls back to the structural result with one warning. The gates before the flip (checker lane, determinism, Linux CI, memory past 600 edits, tsrs-against-Go parity) are recorded in ADR 0026. It began as the opt-in slice 1 (ADR 0006, ADR 0007, `type-checker-integration.md`), whose accuracy was verified against real RDT: 0 missing/extra props on 13 hand-picked components, and on `7.1.0-beta` against the full 44-file fixture corpus (5,508/5,522 `required` matches, 5,266/5,522 type-string matches). TypeScript 7.1's Go checker also runs, test-only, as an oracle for structural prop types (`docs/measurement.md`); the weekly record measures the checker, the default (`oracle.universe_version` 5).

## Experiments that didn't pan out

**Confidence routing (2026-10-25).** The idea: score each component's risk of wrong or missing props from Caliper's own output, and ask the TypeScript 7.1 checker only about the risky ones. Measured against the checker on 518,268 shared props over the five corpora and the fixtures, routing 10% of components caught 23% of the props only the checker reports with the existing warning rule (a component with a degrade warning or error), against 3% for a smoothed table and 6% for a logistic model. At a 20% budget neither model caught the one prop the oracle marks `wrong`. Most of the checker-only props were Next's ambient augmentation or false component matches, which a score of structural risk can't learn (the 1,154 of 1,182 on shadcn quoted then came from a run that no longer reproduces: the weekly record of 2026-11-06 finds 55 checker-only props on shadcn, all `tw`, and `docs/rdt-differences.md` has the current account). So no score ships: warnings stay the signal, and on-demand checker use asks about the requested component instead of predicting. The script was deleted, so the routing numbers above cannot be reproduced; the method is the oracle in `docs/measurement.md`.

## Where to look next

- **Open gaps** → `edge-cases.md` — the gaps still open. Add one when you find a gap you aren't fixing now; delete it when you fix it.
- **Where Caliper's types differ from RDT and the checker, and what that costs** → `rdt-differences.md`.
- **What broke and why, historically** → `rdt-coverage.md` — every bug found during real-library validation, root cause, and the fix. Keep this updated; it's the project's memory.
- **Why a hard-to-reverse decision was made** → `docs/adr/` — OXC over the TypeScript compiler, manual serde for `PropType`, positional msgpack encoding, panic containment, the TypeScript 7 semantic backend. Write a new one when you make a decision like these (see `docs/adr/README.md`).
- **How the pipeline fits together** → `ARCHITECTURE.md`, and `docs/architecture/` for per-subsystem invariants and known gaps (moved from `.claude/semantic-model/` so every coding agent can read it, not just Claude Code — `AGENTS.md` is now this repo's shared instructions file, with per-tool mirrors under `.codex/`, `.gemini/`, `.github/`).
- **Setup, testing, code style, commit conventions** → `CONTRIBUTING.md`
- **Migrating from react-docgen-typescript** → `MIGRATING.md`
