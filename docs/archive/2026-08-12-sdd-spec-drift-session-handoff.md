# Session Handoff — 2026-08-12 SDD/Spec-Drift Initiative

This session ran a comprehensive spec-driven-development (SDD) initiative across the whole workspace: build a semantic-model reference layer, draft formal `spec@1` documents via the real `canon` plugin pipeline, validate current code against those specs, and fix what the validation found. It ran long and built a lot of context — this doc is the handoff so a fresh session can pick up without re-deriving any of it.

## Starter prompt for the next session

> Read `docs/superpowers/session-handoff-2026-08-12.md` for full context, then: (1) review the uncommitted changes with `git status`/`git diff` and commit them if they look right to you — commit message should summarize the bug fixes and test-coverage closure, not just say "SDD work"; (2) the CLI spec test-coverage gaps (see "What's still open" below) are the best next target if you want to keep closing gaps — SPEC-CLI-001a alone has ~15 uncovered criteria; (3) alternatively, run `canon-drift-checker` again against the newly-added tests from this session (nothing has adversarially re-verified them yet) if the priority is confidence over coverage. Ask me which before starting substantial work.

## What was done, in order

1. **Phase 1 — Semantic model.** `.claude/semantic-model/` (7 files + `INDEX.md`): one file per subsystem (pipeline, extractor, resolver, types, serialization, cli, binding), each grounded in real source with numbered invariants and known gaps. Load via `INDEX.md`'s routing table — never load all seven at once.

2. **Phase 2 — Spec drafting.** All 7 capabilities now have gated `spec@1` documents in `.claude/specs/`, drafted through the _real_ `canon` pipeline (`canon-drafter` → `canon-verifier` → `canon-auditor` → `canon-exit-gate`), not approximated:
   - `SPEC-PIPELINE-001.json`, `SPEC-EXTRACTOR-001.json`, `SPEC-RESOLVER-001.json`, `SPEC-TYPES-001.json`, `SPEC-SERIALIZATION-001.json`, `SPEC-BINDING-001.json`
   - `SPEC-CLI-001a/b/c/d.json` (split from one oversized draft: exit-codes / atomic writes / LSP framing / config loading) — **already committed** as `df68cf1`, unlike the rest of this session's changes.

   Several specs took many gate rounds (pipeline hit 9) because the adversarial gate kept catching real precision errors — worth knowing if you see `revision_note`/`reasoning` fields in the JSON that read like a long story. That's the gate working as intended, not noise.

3. **Phase 3 — Drift validation + fixes.** Ran `canon-drift-checker` against all 7 specs (via 6 parallel agents; CLI's 4 sub-specs bundled into one). Findings were split into covered / uncovered / **drifted**. Every drifted (= code contradicts spec) finding was investigated and either fixed in code or fixed in the spec, never left ambiguous.

4. **Gap-closing pass** (this was the last major chunk of work, after the user asked to "make all sets of improvements we can make to close the gaps extensively"). Went subsystem-by-subsystem adding the missing tests the drift pass identified, verifying each new test against a deliberately-broken version of the relevant code before trusting it.

## Real bugs found and fixed this session (7 total)

All have regression tests confirmed to fail against the pre-fix code and pass against the fix.

1. **Watch mode was permanently one edit behind.** `WatchSession::update_file` bound `new_global` directly to `ArcSwap::rcu(...)`'s return value — but `rcu` returns the _pre-swap_ value (`compare_and_swap` convention), not the closure's result. Every incremental update resolved against stale content. Fixed via `load_full()` after the swap. `crates/core/src/pipeline/watch.rs`.
2. **`watch --out`'s stats were always zero.** `WatchSession::snapshot()` used `ExtractionStats::default()`. Fixed for `components_extracted`; other stats fields (files_parsed, duration_ms, etc.) remain a narrower, documented gap — the session doesn't accumulate them incrementally. Same file.
3. **Relative `--config` paths broke when their parent wasn't the invocation cwd.** `try_load_config` changes the node subprocess's cwd to the config's parent but was passing the original relative path through unchanged. Fixed via canonicalization before handoff. `crates/cli/src/config.rs`.
4. **Panic containment gap in NAPI auto-vivification.** `initialize_session` / `extract_file_incremental` / `create_session` ran `WatchSession::new` construction (and, for `create_session`, the options conversion) _outside_ `panic_guard::contain_panic`. Fixed by moving construction inside the guarded closure in all three. `crates/binding/src/lib.rs`.
5. **`extract --out` was never actually atomic** — only `watch --out` got the `write_atomic` treatment originally; `extract.rs` still called bare `std::fs::write`. Fixed by extracting `write_atomic` into `crates/cli/src/output.rs` and sharing it.
6. **`watch`'s keyboard-quit (`q`/`c`) ignored the tracked exit code**, hardcoding `process::exit(0)` regardless of whether the run had errors. Fixed to read the tracked atomic. `crates/cli/src/commands/watch.rs`.
7. **LSP verbose-mode stdout corruption.** `tracing::error!` calls wrote ANSI-colored text to stdout under `-v`/`-vv` — the same channel LSP reserves exclusively for `Content-Length`-framed protocol messages. Fixed by routing all tracing output to stderr globally. `crates/cli/src/main.rs`. New integration test file: `crates/cli/tests/lsp_process.rs`.

Two smaller **code-vs-spec alignment fixes** (not bugs, just inconsistency):

- `chain.rs`'s non-cyclic depth-exhaustion path now routes through `ResolvedChain::give_up` (was hand-rolling an equivalent-but-inconsistent path).
- `SPEC-EXTRACTOR-001.json`'s AC-023 was corrected — chained conditional types emit one `MaxDepthExceeded` diagnostic _per child branch_ (up to 4), not one per chain; single-child nesting forms (the common case) still emit exactly one. Both behaviors now have tests.

## New process guidance added

`CLAUDE.md` gained a "Testing discipline" section with two rules, both directly motivated by what caused today's bugs to survive prior review passes:

1. **Sibling-path parity** — a function mirroring an already-tested one (e.g. `update_file()` mirrors `extract()`) needs its own equivalent test, not just "doesn't crash."
2. **Assert content, not presence** — prefer `assert_eq!` against exact values over `contains()`/`is_some()` liveness checks wherever the cost is comparable.

## Test/quality state as of end of session

- **357 tests passing** across the whole workspace (up from a much smaller baseline — see `docs/STATUS.md` for the pre-session count, which is now stale and due for an update).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo fmt --all -- --check` clean.
- New dev-dependency: `trybuild` (added to `crates/core/Cargo.toml`) for a compile-fail test proving `ParsedProp`'s sealed-constructor invariant.
- New test files: `crates/cli/tests/lsp_process.rs`, `crates/core/tests/compile_fail.rs` + `crates/core/tests/compile_fail/parsed_prop_seal.rs`.

## What's still open (tracked, not blocking)

All entries below are already recorded in `docs/edge-cases.md` — check there for exact citations before re-discovering them:

- **`resolve_named` has no cycle detection** (unlike `resolve_props_chain`) — bounded only by `MAX_DEPTH=20`, no dedicated diagnostic. `resolver/named.rs`.
- **`WatchSession::component_cache` staleness** — two related manifestations (disambiguated-key entries lingering after edits, renamed components leaving stale old-name entries) trace to the same root cause: `component_cache.insert(...)` never evicts a file's previous keys.
- **DTS cache same-tick/same-size staleness** (P0-2, deliberately deferred, documented in a code comment on `key_for`).
- **Config path with a literal backslash in a directory name** fails via Node's `pathToFileURL` percent-encoding — narrow edge case, not fixed.
- **CLI test coverage is uneven.** SPEC-CLI-001a (exit-code contract) alone still has roughly 15 of its ~21 criteria uncovered by tests, even after this session's gap-closing pass — triaged down in favor of the LSP fix and the weak-assertion strengthening, which were judged higher value for the time available. SPEC-CLI-001b/c/d are in better shape but not exhaustively covered either.
- **Today's own new tests haven't been adversarially re-verified.** Every one was confirmed to fail-then-pass against its specific fix, which is solid TDD discipline, but no independent `canon-drift-checker`/`canon-exit-gate` pass has tried to find holes in this session's own additions the way it did for the original 7 specs.

## Things to leave alone

- `.entire/`, `.gemini/`, `.moon/hooks/*` showing as untracked in `git status` are **not** from this session — don't assume ownership or clean them up without checking with the user first.
- `.claude/specs/SPEC-CLI-001a/b/c/d.json` are already committed (`df68cf1`) — everything else in this handoff is still uncommitted, staged nowhere, sitting in the working tree.

## Where things live

- Specs: `.claude/specs/*.json`
- Semantic model: `.claude/semantic-model/*.md` (start at `INDEX.md`)
- Bug/gap tracker: `docs/edge-cases.md` (kept current throughout this session — the audit trail convention is strikethrough + **FIXED**, not deletion)
- This doc: `docs/superpowers/session-handoff-2026-08-12.md`
