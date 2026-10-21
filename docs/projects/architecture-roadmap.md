# Architecture and semantic-model roadmap

Status: draft for review. Delete this file when the work ships (plans live here, specs in `docs/specs/`).

Goal: deliver shape-based component detection and exact props for wrapper-declared components (Mantine, Chakra, react-aria and the rest) while keeping the structural path 10 to 37 times faster than react-docgen and react-docgen-typescript. To get there, move the semantic models from strings and first-match chains to structured data, and pin the architecture with tests.

## Principles

1. Shapes, not names. Only React builtins are named in code. Library behavior comes from callee signatures, type shape, `@component`, or the checker.
2. Structure over strings. A type argument is a `CollectedType`, never display text that gets re-parsed.
3. Classify once, carry the reason. Detection returns a typed result with its rule and span. Every degrade emits a `Diagnostic` with a position.
4. The structural path is the default and stays fast. The `tsc --api` backend is opt-in at runtime, not behind a cargo feature (callisto #143: one tested build is shipped).
5. Measure before and after: recall, precision, time, memory, determinism.
6. Tests pin the architecture. A rule that exists only in `AGENTS.md` prose gets a source-scanning test or a lint.

## Where we are

Confirmed by the October 2026 audits and benchmarks.

| Area | Finding |
| --- | --- |
| Recall vs react-docgen-typescript (source-only mirrors, plausible components) | shadcn 98%, react-spectrum 98%, antd 88%, Mantine 29%, Chakra 9% |
| Misses by shape | withContext 370, factory 180, plain typed arrows 29, polymorphicFactory 24, withRootProvider 16, genericFactory 11, tableElement 7, opaque factories about 60 |
| Speed | 30 to 141ms and 34 to 78MB, against 0.5 to 2s and 300 to 590MB |
| Code layout | 52% of src lines are inline tests; `resolver/mod.rs` is 83% tests; no sibling `tests.rs` |
| Defects | D1 to D9 below |
| Docs | `STATUS.md`, the extractor architecture doc and `edge-cases.md` disagree; no `docs/specs/` |
| Semantic backend | designed (ADR 0006), `crates/tsc-client` exists but is not wired into core, CLI or NAPI |

### Defects to fix on the way

| Id | Defect | Evidence |
| --- | --- | --- |
| D1 | Type arguments are stringified then re-parsed; `FC<Box<'a'\|'b'>>` yields the string `a" \| "b` | reproduced |
| D2 | `extends Pick<Gen<number>,'x'>` and `Omit<Base, keyof Other>` silently drop props, no warning | reproduced |
| D3 | Eight `PipelineOptions` fields are never read; `--no-cross-package` is a no-op | grep |
| D4 | Same-named exported enums in two files: one vanishes by hash order; `split_once(':')` breaks Windows paths | reproduced / traced |
| D5 | Watch mode differs from `extract()`: enum keys, key disambiguation, `reverse_deps` never updated, unused `source_cache` | traced |
| D6 | Unreadable `@types/react` parsed as empty and cached; CLI JSON output can print nothing | traced |
| D7 | Parenthesized and non-null wrappers defeat detection (`preserve_parens` default) | reproduced |
| D8 | `MaxDepthExceeded` used for circular references; `UnresolvableImport` used for three unrelated misses | grep |
| D9 | Ambiguous bare enum names resolve by hash order, differing cold vs incremental | traced |

## Target semantic model

| Model | Today | Target | Why |
| --- | --- | --- | --- |
| Type arguments | `Vec<String>` in `ComponentMapping.props_type_args` and `ExtendsRef.type_args`, re-wrapped as `CollectedType::Raw` | `Vec<CollectedType>`; `Raw` only for unknown syntax, with a diagnostic | Fixes D1 and D2; object-literal arguments like `Factory<{ props: P }>` cannot survive strings |
| Props source | implicit in each detector (name plus args) | `ComponentMapping.props: PropsRef { type, selector }`, selector is `Whole` or `Member(name)` | Mantine payload `props` member; one resolver path for all shapes |
| Detection result | `Option` chain, first match wins, one side-effecting arm, span dropped | `classify() -> Detection { Component(mapping), Skip { reason, span }, NotCandidate }` with a `DetectionRule` on every mapping | Cluster 2 of the root-cause analysis; provenance and positions |
| Wrapper call | not representable | owned `WrapperCall` record (binding, structured type args, function-argument annotations by position, callee reference) | Extractor stays per-file; cross-file work happens later |
| Callee knowledge | none | promotion pass: resolve callee declaration, read where its type parameters reach the props position | Replaces name lists; yields the type-argument index |
| Utility types | knowledge in four places | `UtilityKind` enum | One implementation (callisto's "unify N copies") |
| Resolver options | resolver imports `pipeline::{PipelineOptions, HtmlAttributeMode}` | `ResolverOptions` and `HtmlAttributeMode` in `types/` | Removes a back edge before promotion needs the resolver |
| Resolver state | `ResolveState` forked 3 times (3 hash sets each); 10 `too_many_arguments` allows | `ResolveCx` struct; `visited` as an on-path set | Cost no longer grows with chain depth |
| Declaration keys | `"path:Name"` strings in six maps, duplicated by `NamedTypeIndex` | `DeclKey` newtype, one store | Later step, after the first six |
| Diagnostics | 39 literals, overloaded codes, `line: None` almost everywhere | `Diagnostic::new(..).at(pos)` builder; add `CircularReference`, `InferredComponent` | Provenance and positions |
| Output contract | no schema version | additive `schemaVersion` before the semantic backend merges | Open decision 1 in `type-checker-integration.md` |
| Options | two near-identical mappers (CLI, NAPI), already drifted | one `OptionsSpec` with `TryFrom` and a typed error in core | Fixes D3; NAPI returns `InvalidArg` |
| Watch state | three locks; `global` deep-cloned per save | one locked state with per-file `Arc` data; callee dependency edges | Parity with `extract()`; keeps the incremental edge |

## Phases

Each item is sized to one PR. Size: S under a day, M one to three days, L more. "Needs" lists hard dependencies.

### Phase 0: foundations (mechanical, no behavior change)

| Id | Item | Size | Needs | Exit criterion |
| --- | --- | --- | --- | --- |
| 0.1 | Move inline `mod tests` into sibling `tests.rs` for the 8 files with more than 300 inline test lines | M | none | Test count and results unchanged; `resolver/mod.rs` under about 600 lines. Do first: later phases edit the same files |
| 0.2 | Architecture tests (source scanning, allowlist with reasons, stale-entry check): extractor and `known.rs` never import resolver or pipeline; resolver never imports pipeline (after 2.3); `OpaqueDetail::new` only in `known.rs` and deserialization; output maps are `BTreeMap`; no `oxc_ast` type in a public extractor signature; no `panic = "abort"` | M | 0.1 | Tests pass; each rule has a negative check |
| 0.3 | Lints: `unwrap_used`, `expect_used`, `panic` deny with tests allowed; `print_stdout`, `print_stderr`, `dbg_macro` deny in core; `unused_must_use`, `let_underscore_must_use`, `map_err_ignore`; `unsafe_code = "forbid"` except binding; `yanked = "deny"`; `cargo update -p yoke-derive`; drop `pretty_assertions` | S | none | Clippy clean; resolve the CLI `println!` rule (route through `output.rs` or amend `crates/cli/AGENTS.md`) |
| 0.4 | CI: `cargo semver-checks` for core; build with the declared MSRV 1.94 or raise it; dependabot `oxc_*` group excludes `oxc_resolver` | S | none | Jobs green |
| 0.5 | Cache guards: golden-bytes `insta` test over a fully populated `SourceData`; `proptest` round-trip for `PropType` tagged values and the msgpack cache; `cargo-fuzz` targets for `parse_file` and resolution | M | none | A reordered field or added variant fails CI |
| 0.6 | Recall harness in the repo (from `recall2.py`, `accuracy.py`, `patterns.py`) plus one minimal fixture per declaration shape and per false-positive family, written red first | M | none | `just recall` reports per-shape recall; fixtures assert exact props with `assert_eq!` |
| 0.7 | Docs: `docs/specs/` and `docs/projects/` layout; fix drift between `STATUS.md`, the extractor architecture doc and `edge-cases.md`; correct the edge-cases figures (Mantine 29%, Chakra 9%, antd 88%); remove `SPEC-*`/`AC-*` citations from code (117) | M | none | One source of truth per fact |

### Phase 1: correctness defects

| Id | Item | Size | Needs | Exit criterion |
| --- | --- | --- | --- | --- |
| 1.1 | `preserve_parens: false` (D7) | S | 0.6 | Parenthesized and `!` wrappers detected |
| 1.2 | `OptionsSpec` with `TryFrom` and a typed error in core; wire or delete the eight dead fields; NAPI returns `InvalidArg` (D3) | M | none | No accepted-but-unread option; `--no-cross-package` works or is removed |
| 1.3 | Deterministic same-name enum handling with a diagnostic; consistent key parsing (`rsplit_once`) (D4, D9) | S | none | Reversed input order gives identical output |
| 1.4 | Report unreadable files instead of `unwrap_or_default()` (D6) | S | none | `io_read_error` on every path; CLI errors on serialization failure |
| 1.5 | Split diagnostic codes: `CircularReference` and separate missing-type vs missing-lib codes (D8). Append variants last, bump the cache version in the same commit | S | 0.5 | v4 blob is rejected by a test |
| 1.6 | Watch parity fixes: public enum keys, key disambiguation, `reverse_deps` rebuild, remove unused `source_cache`; one `resolve_guarded` shared by `extract()` and `update_file` (D5) | M | 0.1 | Parity tests pass for `extract()`, `update_file`, `create_session` |

### Phase 2: semantic-model groundwork

| Id | Item | Size | Needs | Exit criterion |
| --- | --- | --- | --- | --- |
| 2.1 | Structured type arguments (`Vec<CollectedType>`); delete the `Raw` round-trip; cache schema v5 (D1) | M | 0.5, 1.5 | The three reproduced D1 cases resolve correctly; time neutral or better |
| 2.2 | `Diagnostic` builder with position; `ComponentMapping::new`; keep the span in `record_skip` | S | none | 9 mapping literals become 1; skips carry line and column |
| 2.3 | Move `HtmlAttributeMode` and a `ResolverOptions` into `types/`; `ResolveCx` struct replaces the long argument lists | M | 0.2 | No `use crate::pipeline` in the resolver; the 10 `too_many_arguments` allows are gone |
| 2.4 | `UtilityKind` enum; fix nested-generic and `keyof` arguments to `Pick`/`Omit` (D2) | M | 2.1 | D2 repros give the right props or a diagnostic |
| 2.5 | `visited` as an on-path set; remove `ResolveState` forks | M | 2.3 | Diamond and cycle tests unchanged; no per-branch set clones |
| 2.6 | Minimal repro and fix for multi-hop barrel resolution (Mantine `core/index.ts` 51 of 79 unresolved warnings, Chakra `styled-system/index.ts` 24) | S to M | 0.6 | Cause stated; fixed or documented as a structural limit |
| 2.7 | Typed declaration keys and a single store (removes `NamedTypeIndex` duplication) | L | 2.1, 2.3 | Optional; do after Phase 3 if still worthwhile |

### Phase 3: shape-based detection

| Id | Item | Size | Needs | Exit criterion |
| --- | --- | --- | --- | --- |
| 3.1 | `classify()` and `Detection`; one `peel_expr`; `DetectionRule` on every mapping; `InferredComponent` diagnostic (Info, with position) | M | 2.2 | Behavior unchanged on existing fixtures |
| 3.2 | Easy shapes: typed arrow and function-expression components; function argument at any position; identifier argument naming a function; alias before base (RC7); `/** @component */` | M | 3.1, 0.6 | About 29 arrows plus react-aria wrappers found; no new false positives |
| 3.3 | Resolver: accept indexed access as a props base; select a member from an object-literal type argument; substitute generic interface type arguments (RC1 to RC3) | M | 2.1, 2.4 | Repro cases `FC<Fx['props']>`, `PropsOf<Fx>`, `Factory<{props: P}>` give exact props |
| 3.4 | `WrapperCall` record and the promotion pass, tier 1: resolve the callee declaration (alias, import, barrel, destructure-from-call) and read where its type parameters reach the props position. Pure function from `GlobalSourceData` and the record list to mapping candidates, run before resolution, deterministic order, inside `contain_panic` | L | 3.1, 3.3, 2.3, 2.6 | Mantine and Chakra component recall at least 80%; props asserted exactly |
| 3.5 | Tier 2 (type-shape guess) behind a flag, with the guard: function-valued or typed callee parameter, reject literal and `null` first arguments, SCREAMING_CASE, lowercase names, destructured bindings, namespace-member callees, React builtins | M | 3.4 | Hand-labeled precision of at least 95% per family (createContext, providers, helpers) |
| 3.6 | Watch: record callee and barrel-hop dependencies as `reverse_deps` edges; parity tests including callee-only edits | M | 3.4, 1.6 | Editing only the wrapper updates dependents |

### Phase 4: semantic backend (ADR 0006 follow-up)

| Id | Item | Size | Needs | Exit criterion |
| --- | --- | --- | --- | --- |
| 4.1 | Re-probe TypeScript 7.1.0-beta: package exports (`typescript/unstable` vs `./sync ./async ./proto ./ast/*`), checker methods, overloads, static sub-components, barrels; update `type-checker-integration.md` | S | none | Entry points and version pin recorded |
| 4.2 | ADR 0007: backend may discover components; always compiled, enabled at runtime (no cargo feature); structural stays default and fallback; trigger policy. Decide additive `schemaVersion` | S | 4.1 | Accepted ADR |
| 4.3 | Wire `tsc-client` into core (owned result type and merge in core, process in the backend crate) | L | 4.2, 2.1 | Failure leaves the structural result plus a diagnostic |
| 4.4 | Discovery by call-signature rule (RDT's algorithm), merge into structural output; covers opaque factories (about 60 declarations) | L | 4.3 | Accuracy gate extended to checker-discovered components |
| 4.5 | Determinism (clean vs incremental) and cost gates with the Rust client | M | 4.4 | Measured on the five corpora with dependencies installed |

### Phase 5: performance and dependencies

| Id | Item | Size | Needs | Exit criterion |
| --- | --- | --- | --- | --- |
| 5.1 | Profile allocations and thread scaling (Mantine: 44ms on 18 threads vs 68ms on one) before touching clone sites | S | none | Written profile with the top five sites |
| 5.2 | Parallel discovery; resolve the TypeScript lib once per run; serialize RDT output from typed structs instead of a `json!` tree; avoid per-lookup alias clones | M | 5.1 | Measured gain on the five corpora |
| 5.3 | Replace `watchexec` with `notify` and a debouncer; drop tokio from the CLI; trim tokio and tracing features | M | none | Pty watch test passes; crate count and build time recorded |
| 5.4 | Replace `dashmap` with `Mutex<FxHashMap>` where not on a hot path; benchmark the cache first | S | none | No regression |
| 5.5 | Release profile experiment (`lto = true`, `strip = true`); `mimalloc` for the CLI | S | none | Size and speed recorded; unwinding kept |
| 5.6 | JSON config fast path alongside node and `tsx`; append to `NODE_OPTIONS` instead of overwriting | M | none | Spawn skipped when a JSON config is present |

### Track R: release and product (parallel)

| Id | Item |
| --- | --- |
| R1 | Yank `oxc-react-docgen-core` 0.2.0 from your account |
| R2 | Update orin-dx.com/tools/caliper (published packages, `caliper-docgen` command names, remove "built, unpublished") |
| R3 | Diff the hand-written `release.yml` against what callisto 0.11 generates (napi roster-only builds, recovery fixes) |
| R4 | Docs guides: CLI reference, `docgen.config.ts`, output format and `PropType` kinds, Vite plugin options, Node API |
| R5 | File upstream callisto issues: verification window shorter than npm's new-package lag; no tag-push credential input |
| R6 | Release hygiene: do not merge workflow-changing PRs while a release is in flight |

## Dependency graph

```
0.1 -> 0.2 -> 2.3 -> 3.4
0.5 -> 1.5 -> 2.1 -> 2.4 -> 3.3 -> 3.4 -> 3.5
0.6 -> 1.1, 3.2
2.2 -> 3.1 -> 3.2, 3.4
2.6 -> 3.4
1.6 + 3.4 -> 3.6
4.1 -> 4.2 -> 4.3 -> 4.4 -> 4.5     (starts after 2.1)
5.x and Track R are independent
```

## Release shape

| Release | Contents | Notes |
| --- | --- | --- |
| 0.2.2 | Metadata, license, README rewrite (PR #45 merged; release PR #46 open) | Merging #46 publishes; the first release where package names already exist, so it tests the single-run publish |
| 0.2.3 | Phase 0 and Phase 1 | No user-visible behavior beyond fixed defects; cache version bump |
| 0.3.0 | Phase 2 and Phase 3 (3.1 to 3.4) | New components appear in output: a behavior change, call it out in the changeset. Tier 2 stays off |
| 0.4.0 | Phase 4 | Opt-in semantic backend |
| as ready | Phase 5 items | Each with measured numbers |

## Quality gates

| Gate | Threshold (proposed) |
| --- | --- |
| Component recall vs RDT, plausible components, source-only mirrors | Mantine at least 85%, Chakra at least 80%; shadcn, spectrum, antd not lower than 98%, 98%, 88% |
| Prop recall on found components | Mantine and Chakra at least 80% (inferred target, not yet measured) |
| Precision | At least 95% on a hand-labeled sample of about 50 new emissions per project, reported per false-positive family |
| With dependencies installed | Mantine and Chakra re-run with `node_modules` (dependencies installed with scripts disabled) |
| Time and memory | No project more than 15ms or 20% slower than the current figure; peak RSS tracked |
| Determinism | Reversed and repeated input order give byte-identical output; clean and incremental agree |
| Parity | `extract()`, `update_file`, `create_session` agree on components, props and diagnostics, including callee-only edits |
| Hygiene | Zero production `unwrap`/`expect`/`panic`; no `use crate::pipeline` in the resolver; no accepted-but-unread options |

## Decisions

Recommended defaults, assumed until confirmed.

| Id | Decision | Recommended default |
| --- | --- | --- |
| D1 | Does tier 2 (type-shape guess) emit by default? | No. Opt-in flag; tier 1 and `@component` emit by default |
| D2 | Declarative wrapper list in `docgen.config.ts`? | No for now; `@component` is enough. Revisit after Phase 3 data |
| D3 | May the semantic backend discover components? Wire `tsc-client`? | Yes (ADR 0007), runtime opt-in, wired in Phase 4, after Phase 3 shows what structural leaves |
| D4 | Provenance in output? | Info diagnostic with position; add an additive `schemaVersion` field to `ExtractionOutput` |
| D5 | Thresholds | As in Quality gates |
| D6 | Parity target | Plausible components (PascalCase, not hooks), not every RDT entry |
| D7 | Plain `.ts` files and underscore-leading names | Include underscore-leading names (`_3D`); keep `.ts` out |
| N1 | Adopt callisto's "no cargo features in shipped crates" rule | Yes; amend ADR 0006's "feature-gated" wording in ADR 0007 |
| N2 | Adopt `docs/specs/` and `docs/projects/` | Yes |
| N3 | Forbid spec or criterion IDs in code | Yes; clean up the 117 existing citations in 0.7 |

## Main risks

| Risk | Mitigation |
| --- | --- |
| Tier 2 matches the `createContext` family (427 sites in the corpora) | Off by default; structural discriminators; per-family precision; one negative fixture per family |
| Detection lands before props resolve and yields empty components | Order: 2.1, 2.4, 3.3 before 3.4; an unresolved props type emits an Info diagnostic and no entry |
| Phase 3 too large to attribute regressions | Split as above; each item has its own exit criterion |
| Watch staleness for cross-file callee resolution | 3.6, with callee-only parity tests |
| Positional cache decode (new field or variant) | Append last, bump version in the same commit, golden-bytes test (0.5) |
| New components change user output | Info diagnostics only; hand-review snapshot diffs; changeset note; 0.3.0 |
| Recall is measured against an oracle that over- and under-reports | Hand-labeled precision sample; dependency-installed run; per-shape reporting |
| TypeScript 7.1 API churn | 4.1 re-probe first; ADR kept narrow; version check degrades to structural with a diagnostic |

## Out of scope

`lazy` and `dynamic` components (props live in the target module), plugin registration from the CLI, replacing `compact_str` or the cache format (the audit found no case for it), Vue and Svelte, and a Storybook integration.

## Evidence

The October 2026 benchmark and audit reports, the research report on wrapper-declared components, and the pattern, dependency and callisto-practice reviews. Numbers marked "reproduced" were run against the release binary; "traced" means read from code only.
