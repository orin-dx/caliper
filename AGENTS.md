# caliper-docgen

Faster than react-docgen-typescript; measured numbers are in `docs/performance.md`. OXC parses TypeScript/TSX; we extract React component props and emit RDT-compatible JSON via `--format rdt`. See `docs/STATUS.md` for current state and known gaps.

## Commands

```bash
just lint doc-check machete typos          # what CI runs; pass before pushing
cargo test -p caliper-docgen-core          # unit + snapshot tests
cargo clippy -p caliper-docgen-core -- -D warnings
cargo build --release                         # full build including CLI
```

Or use `/check` and `/snapshot` for the common workflows.

## Non-negotiables

1. No `unwrap()` outside `#[cfg(test)]` — use `?`
2. `FxHashMap` for internal maps; `BTreeMap` for JSON-output maps
3. `CompactString` for type/prop names in hot paths
4. No AST refs escape `parse_file()` — allocator is local per call
5. No terminal/display code in `crates/core`
6. Always emit `Diagnostic` when degrading — never fail silently

## Architecture

```
OXC parse → SourceData (extractor/) → GlobalSourceData (pipeline/) → ComponentEntry (resolver/) → ExtractionOutput
```

No reverse dependencies. `pub(crate)` for implementation modules; `pub` only for consumer-facing API (`pipeline/`, `types/`, `react_types`).

See `ARCHITECTURE.md` for the full pipeline/crate-graph overview, and `docs/architecture/` for per-subsystem invariants and known gaps (pipeline orchestration, extractor, resolver, types, serialization, CLI, NAPI binding) — start at `docs/architecture/README.md`.

## Decisions

Read `docs/adr/README.md` before changing the parser choice, `PropType` serialization, the DTS cache format, panic containment or the semantic backend. Reversing an ADR needs a new ADR.

## Testing discipline

Two habits that repeatedly caught real bugs during the 2026-08-12 spec-validation pass (a stale-by-one-edit watch mode, always-zero watch stats — both had zero test coverage despite extensive existing suites):

1. **Sibling-path parity.** When a function parallels an already-tested one (e.g. `WatchSession::update_file` mirrors `extract()`'s resolve/panic-containment logic; `create_session`'s guard boundary mirrors `initialize_session`/`extract_file_incremental`'s), the new path needs a test asserting the _same_ contract, not just "it runs without error." A sibling path is exactly where coverage silently lapses.
2. **Assert content, not presence.** Prefer `assert_eq!` against an exact expected value over `contains(...)`/`is_some()`/liveness checks wherever the cost is comparable. A test asserting "component X exists" survives X's props resolving to stale or wrong data; a test asserting X's exact prop set does not.

## Crate guide

- `crates/core` — pure extraction logic; @crates/core/AGENTS.md
- `crates/cli` — clap CLI, no logic; @crates/cli/AGENTS.md
- `crates/binding` — thin NAPI wrapper, delegates to core
- `crates/semantic` — TypeScript 7.1 checker backend, on by default (`docs/type-checker-integration.md`)

## Scripts

No new sh, py or mjs script unless a CI job, the weekly or a documented recipe calls it; prefer a Rust test or the existing oracle TypeScript. A measurement run once is recorded in docs (`docs/measurement.md`, `docs/gate-measurements.md`), not committed as a script. Each script's caller is listed in `docs/measurement.md`.

## Plugins

Start work through these commands so their checks run:

- Options before deciding: `/guide:explore`. Does it work: `/guide:spike`.
- Bug or review findings: `/guide:fix`. Real feature or shared change: `/guide:build`.
- Verify or find bugs: `/guide:check`. Commit message: `/guide:ship`.

Pull requests are opened by Entire from the branch's trail, so `/guide:ship` stops at the commit: push the branch and create or link its trail (`entire trail create`). Do not open the PR with `gh`.

## Writing and commits

- Succinct: short sentences, bullets, tables. No backstory, hedging or filler. Banned words are in `docs/adr/README.md`.
- Changesets live in `.changeset/` as one file per consumer-visible topic, with a front matter line per affected package (`cargo/caliper-docgen: patch`, `npm/caliper-docgen: minor`). Changesets become the project's changelog, so write them for the people who use it, not for the implementer.
  - Open with a bold lead-in naming the change as the user sees it, then one line on its value: what they can now do or what changed for them.
  - When there is more to say, add short bullets in this order: why it matters, how to adopt it (commands, configuration, migration), and limits they will hit. Use sub-bullets for detail.
  - Group a longer entry under bold labels such as **Upgrading** or **Scripting**. Do not use `#` headings; the changelog supplies those.
  - Mark a breaking change with a **Breaking** label that states the old behavior, the new behavior and what to do.
  - One idea per bullet. Leave out internals unless the user must know them.
- Never hard-wrap Markdown, commit messages or PR descriptions: one line per paragraph.
- Comments: only the non-obvious why, no history (the commit records it). Fill to 120 columns (`.rustfmt.toml`). Rewriting a comment block counts as writing it: re-fill and cut it, don't inherit the old wrapping.
- Commits: imperative subject, at most one or two sentences of why, no list of what changed, no "This commit...". Skip the body when the subject says it all.

<!-- entire-agent:begin -->
Read .entire/agent-guide.md for this repository's workflow, source inspection, and verification guidance.
@.entire/agent-guide.md
<!-- entire-agent:end -->
