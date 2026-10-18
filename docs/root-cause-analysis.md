# Root-cause analysis: `edge-cases.md` findings

Phase 0 of the edge-case remediation effort. Four parallel agents each took a candidate cluster from `edge-cases.md` and asked not "what's broken" (already catalogued) but "what's missing that let it happen" — reading the actual code at every cited site, not just trusting the audit's summary. Three of the four found their assigned cluster wasn't one mechanism, it was several; after splitting, **11 confirmed mechanism-level root causes** and **7 genuinely standalone findings** remain.

**The overall pattern:** this codebase has good conventions in places that got established once — a named constructor, a depth counter, a truncation helper, a documented precedence order — but nothing makes those conventions load-bearing. They're advisory, so a parallel or later code path silently diverges from them. The fix shape repeats across clusters: turn an advisory helper into the _only_ callable constructor, or add the one missing signal channel a class of code was never given. Two clusters (panic containment, schema derivation) are genuinely new architectural decisions and get an ADR; the rest are internal refactors of an existing-but-unenforced pattern.

**Adversarially verified 2026-08-04:** every root-cause diagnosis in this document was independently re-checked against the actual current source by a second, skeptical pass (7 agents, defaulting to refute). All diagnoses held up — no fabricated citations, no stale line numbers. Two mechanisms as originally drafted were **unsound** (would not compile, or would not stop the misuse they targeted) and have been rewritten below; several others were underspecified in ways an implementer would trip on and have been corrected inline, marked **[corrected]**. Where a correction narrows scope or changes the fix shape, the original claim is noted so the reasoning is traceable.

---

## Mechanism clusters

### 1. Resolver give-up constructor discipline `[corrected]`

**Root cause (narrowed on verification):** the original claim — "6 of ~10 `Opaque` construction sites bypass the named constructor" — overstated the blast radius. Verification found only **3 sites genuinely silent**: `func.rs:61` (`MultiParamFunction`, zero diagnostic anywhere nearby), `chain.rs:40` (cycle-detected `ResolvedChain::default()`, zero diagnostic), `alias.rs:116` (`LiteralUnion`-as-props-base, zero diagnostic, unlike the parallel diagnosed path at `alias.rs:234-253`). The two other sites originally named as bypassing the pattern — `primitives.rs:181` and `template.rs:39` — actually already push a correctly-shaped `Diagnostic` immediately before constructing the `Opaque`; they just don't call `collected.rs`'s helper, which is a private `fn` and structurally can't be called from those files today anyway. That's a duplication/style gap, not a silent-failure bug — don't conflate it with the real 3.

**Mechanism (rewritten — the original was unsound):** the original proposal — make `ResolvedChain`'s bare/`Default` construction private, and restructure `PropType::Opaque { raw, reason }` into a tuple variant `Opaque(OpaqueDetail)` with the only public constructor being a diagnostic-emitting `give_up()` — does not work, for two independent reasons verified against the real code:

1. **Reverse-dependency violation.** `known.rs` owns roughly 7 of the real `PropType::Opaque` construction sites and is required by `AGENTS.md`'s "no reverse dependencies" rule to never depend on `resolver::ResolveState`. A `give_up(state: &mut ResolveState, ...)` signature as the _only_ public constructor can't be called from `known.rs` at all. This split is intentional and already tested (`test_known_opaque_result_emits_diagnostic`, `test_known_opaque_result_emits_diagnostic_at_chain_level`): `known.rs` constructs the bare value, the _caller_ (`chain.rs`/`named.rs`) diagnoses it via `push_known_opaque_diagnostic` using context `known.rs` doesn't have. Collapsing construction and diagnosis into one constructor breaks this tested split. `types/output.rs`'s hand-written `Deserialize` impl also constructs `PropType::Opaque{..}` twice, reconstructing from wire JSON with no `ResolveState` in scope at all — deserialization isn't a give-up event and the original plan never accounted for it.
2. **`ResolvedChain` privacy is close to a non-issue, and harmful if forced.** The struct and its fields are already private to the `resolver` module tree — unreachable from outside `resolver/` today. Within `resolver/`, `chain.rs`, `alias.rs`, `named.rs`, and `extends.rs` all have _equal_ access to `resolver::mod.rs`'s private items — Rust's privacy model can't distinguish `chain.rs:40`'s illegitimate give-up call from `alias.rs`'s ~9 _legitimate_ accumulator-init call sites (later filled by `chain.merge_parent(...)` in a loop) at the same privacy tier. Only 1 of the 11 bare/`Default()` construction sites found is actually a silent give-up; the rest are legitimate accumulator seeds, deliberately-silent non-error short-circuits, or success-path struct-update literals. Forcing all of them through one diagnostic-emitting constructor would spuriously diagnose non-error paths.

**Corrected mechanism:** scope the fix to exactly the 3 confirmed sites. Add a small helper — callable wherever `ResolveState` is already in scope, which all 3 sites qualify for — e.g. `ResolvedChain::give_up(type_name: String, diag: Option<Diagnostic>, state: &mut ResolveState) -> Self` used _only_ at `chain.rs:40`, and an equivalent minimal fix at `func.rs:61` and `alias.rs:116` (push the missing diagnostic inline, matching the shape `alias.rs:234-253` already uses — doesn't need a new type). Leave `known.rs`'s construct-then-caller-diagnoses split untouched — it's correct and tested. Leave `ResolvedChain`'s and `PropType::Opaque`'s existing construction surface as-is; do not attempt to make either "give-up-only-constructible."

**Resolves:** `resolver-chain-cycle-detected-default`, `resolver-func-multiparam-no-diagnostic`, `resolver-literalunion-as-props-base` (P2 items in `edge-cases.md`).

**ADR:** No — enforcing an existing pattern, not introducing a new one.

### 2. Extractor missing give-up signal channel

**Root cause:** `classify_type_alias`, `extract_type_name_from_type`, and the component-detector `.or_else()` chains in `visit.rs` are `Option`-returning `?`-chains over AST shapes with no way to distinguish "wrong shape, fine" from "matched but malformed, worth flagging." `SourceData.diagnostics` exists and other extractor code already pushes into it (nesting-depth guard, parse errors) — it was simply never wired to these specific functions. This is a design gap, not helper drift: no helper could be "used inconsistently" because none was ever built for this call shape.

**Mechanism:** Add `fn record_skip(&mut self, code: DiagnosticCode, message: impl Into<String>, span: Span)` in `extractor/mod.rs`, plus a new `DiagnosticCode::SkippedCandidate` variant. Wire it into every malformed-but-matched arm in `alias.rs::classify_type_alias` and the end of each detector chain in `visit.rs` (lines 236-247, 262-293). This is a `crates/core/AGENTS.md` checklist requirement, not a lint — distinguishing "not this shape" from "this shape but unsupported" needs domain judgment per call site.

**Resolves:** `extractor-classify-type-alias-omit-pick-etc`, `extractor-extract-props-arg-terminal-none`, `extractor-failed-detector-silence`, `extractor-computed-property-keys`. New detector _coverage_ (anonymous default export, class components, `Object.assign`, class-expression components, `satisfies`) is a separate feature-gap task — `record_skip` only makes the existing miss visible, it doesn't add support.

**ADR:** No.

### 3. Pipeline discovery/merge diagnostic gaps

**Root cause:** Three independent bugs, not one mechanism. `discover_files` returns a bare `Vec<Utf8PathBuf>` with no diagnostic-return channel at all, so `walker.flatten()` silently drops every `ignore::Walk` error. `pipeline/mod.rs:220`'s empty-`src_dirs` guard (`!options.src_dirs.is_empty() && ...`) is a plain boolean logic error. `pipeline/mod.rs:386`'s `components.insert(key, entry)` discards the `Option<ComponentEntry>` return value that already reveals a collision — the data needed to diagnose it is sitting right there, unused.

**Mechanism `[corrected]`:** give `discover_files` a `&mut Vec<Diagnostic>` param, pushing on `Err` and on the non-UTF8-path fallback (reuse the pattern at `mod.rs:269-272`). Also update `discover_files`'s two direct test call sites (`mod.rs:542,562`, currently calling it with the old 2-arg signature). Fix the `mod.rs:220` guard with a _distinct diagnostic message_ for "no source directories were configured" vs. "the configured directories don't exist on disk" — dropping the `!is_empty()` conjunct alone routes an empty list into the "missing dirs" branch, which builds its message from `options.src_dirs.first()`/`.iter().join()` and would read as "None of the configured source directories exist: " with nothing after the colon, confusing for a genuinely-empty list. Check `components.insert`'s return at `mod.rs:386` and push a collision diagnostic on `Some(_)` — **this catches only a genuine 3+-way same-key collision, not the realistic 2-directory overlap case.** Tracing the actual keying logic: the disambiguation key is `format!("{} ({})", base_name, entry.file_path)` applied to every occurrence after the first, so with exactly two duplicate mappings from the same file (what `["./src", "./src/components"]`-style overlap actually produces via `component_mappings.extend()` with no dedup anywhere), the first gets the plain key and the second gets the disambiguated key — two _different_ keys, so `insert` never sees a collision. Overlapping `src_dirs` needs its own separate fix: dedup `src_files` by canonical path after sorting in `discover.rs` (e.g. `files.dedup()` post-sort), or dedup `component_mappings` by `(name, file_path)` before the keying loop in `mod.rs`. Implement both — the insert-return check for genuine key collisions, and the dedup for the overlap case — don't rely on either alone.

**Resolves:** `pipeline-permission-denied-files`, `pipeline-non-utf8-filenames`, `pipeline-empty-src-dirs`, `pipeline-same-name-same-file-collision`, `pipeline-overlapping-src-dirs`.

**ADR:** No.

### 4. No panic-containment boundary

**Root cause:** Zero `catch_unwind` anywhere in the repo (confirmed by grep). The three async NAPI entry points (`extract_all`, `extract_file_incremental`, `initialize_session`) are only panic-safe by _accident_ — they happen to run inside `tokio::task::spawn_blocking`, whose `JoinError` catches the unwind as a side effect of tokio's own plumbing, not a decision anyone made. Because it was never written down anywhere, every later concurrent/cross-boundary addition reinvented the answer independently and got inconsistent results: `pipeline/mod.rs:260,357`'s rayon `.map()` closures have no per-item boundary at all; `plugin.rs`'s hook loop calls `plugin.on_x(...)` directly with zero isolation between plugins or from the pipeline invoking them; `crates/binding/src/lib.rs`'s `create_session`/`close_session` skip `spawn_blocking` entirely, missing even the accidental protection their siblings get; and `pipeline/watch.rs:93`'s `std::sync::Mutex` can poison permanently, specifically because the unprotected rayon batches and plugin hooks from the first two run while that lock is held.

**Mechanism `[corrected]`:** add `crates/core/src/panic_guard.rs`: `pub fn contain_panic<T>(label: &str, f: impl FnOnce() -> T) -> Result<T, Diagnostic>` (public, not `pub(crate)` — `crates/binding` needs to call it across the crate boundary), plus a new `DiagnosticCode::InternalPanic` variant. Wrap each rayon closure body in `pipeline/mod.rs:260,357`. Wrap each individual `plugin.on_x(...)` call in `plugin.rs`'s loops (not the whole loop, so one bad plugin degrades only itself, tagged with `plugin.name()`). Wrap `create_session`/`close_session` bodies in `crates/binding/src/lib.rs`, and have the other three NAPI entry points call it inside their existing `spawn_blocking` closures too, so their safety stops being incidental. Defense in depth: replace `watch.rs:93`'s `.expect("init lock poisoned")` with `.unwrap_or_else(|p| p.into_inner())`. Document the rule in `crates/core/AGENTS.md`.

**Verified correction — the plugin-hook wrapping needs an explicit `UnwindSafe` decision.** `plugin.rs`'s two hook calls (`on_file_extracted(file_path, &mut data)`, `on_component_resolved(&mut entry)`) each capture a `&mut` reference, and `&mut T` is unconditionally `!UnwindSafe` in std — `contain_panic`'s closure bound as originally drafted (`+ UnwindSafe`) will not compile against these call sites; this is a hard compiler error, not a style nit. Resolve it inside `contain_panic` itself, not at each call site: wrap the closure in `std::panic::AssertUnwindSafe` internally, with a documented rationale — this codebase's `SourceData`/`ComponentEntry` have no interior mutability (`Cell`/`RefCell`) that could leave an inconsistent, observably-torn state behind after a caught panic; the whole per-item operation is what gets abandoned, not a partial mutation trusted afterward. `AssertUnwindSafe` is safe to apply as a blanket internal implementation detail of `contain_panic` specifically because of that data-shape guarantee — state it in the function's doc comment so a future caller with genuine interior mutability doesn't inherit the assumption unknowingly. Also worth noting: `napi-rs` (the pinned version) has its own opt-in `#[napi(catch_unwind)]` attribute that wraps a function body in `catch_unwind` at the FFI boundary specifically — a viable complementary/alternative mechanism for the NAPI entry points worth considering alongside (or instead of) routing them through `contain_panic` by hand.

**Resolves:** P1-3, P1-4, P1-5, P1-9.

**ADR: Yes — see draft below.**

### 5. Unbounded allocation from input-derived size

**Root cause:** `extractor/mod.rs` already solved exactly this shape of problem once (`MAX_SOURCE_NESTING_DEPTH` / `max_bracket_nesting_depth`, with a doc comment explaining why) — nobody generalized or even cross-referenced that precedent when `template.rs`'s Cartesian-product loop or `lsp.rs`'s header parsing were written later. A discoverability gap, not a missing type.

**Mechanism:** Add `const MAX_TEMPLATE_LITERAL_EXPANSIONS: usize = 4096` checked inside `try_expand_template_literal`'s accumulation loop (`resolver/template.rs`), falling through to the existing Opaque+diagnostic path on overflow. Add `const MAX_LSP_MESSAGE_BYTES` gating the `vec![0u8; len]` allocation in `cli/commands/lsp.rs:36`. No shared function is practical across crates with different data shapes — add a code-review checklist line instead: "does this size come from parsed/protocol input, and is it capped before use?"

**Resolves:** P1-1, P1-2.

**ADR:** No.

### 6. LSP scaffold immaturity

**Root cause:** `lsp.rs` is a new, zero-test, hand-rolled JSON-RPC framing layer with no capability/handler consistency check — it advertises `hoverProvider: true` with no hover handler behind it, and a malformed header desyncs the stream instead of resyncing.

**Mechanism `[corrected]`:** don't advertise `hoverProvider: true` until a handler exists. The `content_length: None` branch's originally-proposed fix — "resync by consuming to the next blank line" — is **ineffective as stated**: tracing the actual control flow, the header-reading loop already reads and breaks on the blank line _before_ the `content_length: None` check runs, so "consume to the next blank line" would be a no-op against code that's already past that point. The real problem is structural: with no `Content-Length`, there is no way to know how many bytes the message body occupies, so it cannot be skip-consumed at all — the next read will misinterpret body bytes as new headers. Fix: treat a headerless/no-`Content-Length` message as unrecoverable and close or reset the connection with a diagnostic, rather than attempting to skip past it — LSP framing gives no other resync signal once this state is reached. Longer-term, consider `lsp-server`/`lsp-types` instead of hand-rolled framing.

**Resolves:** P1-6, P1-7.

**ADR:** No.

### 7. Extractor depth-tracking proxy mismatch

**Root cause:** `max_bracket_nesting_depth` bounds raw-text _bracket_ depth as a stand-in for AST recursion depth, but `ts_type_to_collected` recurses with no depth counter of its own — unlike the resolver's `resolve_*` functions, which already thread an explicit `depth: u8` parameter and bail past a limit. Chained conditional types achieve deep type-nesting with proportionally fewer brackets per level than paren/object nesting, so the proxy metric undercounts exactly the case it's meant to catch.

**Mechanism:** Thread a depth counter through `ts_type_to_collected` and sibling functions in `extractor/mod.rs`, bailing with a diagnostic past e.g. depth 500 — bringing the extractor into the same convention the resolver already established.

**Resolves:** P1-8.

**ADR:** No.

### 8. Resolver precedence: two hand-copied orderings

**Root cause:** `named.rs` documents "check source-defined types before known-pattern shortcuts" as intentional, with an explicit comment explaining why. `chain.rs`'s `extends`-clause path re-implements the same sequence independently — and gets the order backwards, checking known-pattern shortcuts first. Nothing ties the two orderings together, so `chain.rs` was free to drift the moment it was written, and nothing since has caught it. This is the confirmed, demonstrable bug from the original audit (P0-1).

**Mechanism:** extract the shared logic into `resolve_source_defined_or_known(...)` in `resolver/mod.rs` (or a new `resolver/precedence.rs`): fixed order `resolve_to_canonical → lookup_type_alias → lookup_interface → resolve_known`, taking closures for each call site's differing terminal handling (`Named` vs `ResolvedChain`). Both `named.rs` and `chain.rs` call it — reordering becomes structurally impossible. Add a regression fixture: a project-defined `interface SxProps` extended via `extends`, asserting the hardcoded MUI shortcut is _not_ substituted. Verified zero regression risk: no fixture or snapshot anywhere in the test suite exercises an `extends`-clause over a known-pattern name (`SxProps`, `SlotProps`, `VariantProps`, etc.), so this fix changes no currently-passing output.

**Verified correction — this fix does not fully unify the two functions.** `chain.rs` and `named.rs` diverge in two further respects beyond the P0-1 step that the shared function above doesn't address: `named.rs` checks React-builtin status _before_ canonical resolution, while `chain.rs` checks it _after_ the known-pattern block; and `chain.rs` checks `is_ts_utility_type` very early (before known-pattern), while `named.rs`'s analogous check runs last (after known-pattern _and_ after interface/alias lookup). Neither divergence reproduces the P0-1 bug shape (a user-defined interface being shadowed), so they're out of scope for this fix, but don't describe the two functions as "unified" once this lands — flag both as open follow-up items if a future audit revisits resolver precedence.

**Resolves:** P0-1.

**ADR:** No.

### 9. TOON truncation indicator dropped in one branch

**Root cause:** `LiteralUnion`'s truncate-with-`"...(+N)"` pattern was never factored into a helper. When `Union`/`Intersection` needed the same truncate-with-indicator behavior — in the _same function_, `format_type_compact` — the indicator half was dropped. This isn't code aging apart over time; it's proof the codebase has no mechanism that makes "format a truncated list" a single call site, even within one function.

**Mechanism:** Add `fn truncate_with_indicator(parts: &[String], limit: usize, sep: &str) -> String` in `toon.rs`. Route `LiteralUnion`, `Union`, and `Intersection` through it.

**Resolves:** P0-4. Cheapest fix in this document — all three call sites are already colocated.

**ADR:** No.

### 10. Schema hand-maintained separately from the real structs

**Root cause:** `cli/commands/schema.rs` hand-writes a JSON Schema as a `serde_json::json!()` literal instead of deriving it from `ComponentEntry`/`ExtractionStats`/`Diagnostic`. Every new field on those structs requires someone to remember to edit a second, structurally unrelated file — and the only existing test checks that the hand-written JSON is syntactically valid, never that it matches real serialized output, so drift is invisible to CI. Concrete drift already present: `.methods`/`.tags`, 5 of 9 `ExtractionStats` fields, and `.line`/`.column`/`.help` are all undocumented in the schema today.

**Mechanism `[corrected]`:** derive `schemars::JsonSchema` on the plain structs (`ComponentEntry`, `ParsedProp`, `ExtractionStats`, `Diagnostic`, `InheritedLayer`); call `schemars::schema_for!(ExtractionOutput)` from `cmd_schema()`. Hand-write `JsonSchema` impls only for `PropType` and `CollectedType` — the two enums that are actually recursive and actually have hand-written `Serialize` impls today, per ADR-0002. **Verified correction: `OpaqueReason` is not recursive and does not hand-write `Serialize`.** It has an ordinary `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]` with no self-referential field — ADR-0002's own text claims otherwise (asserting all three types hand-write their impl), and that ADR text is itself stale relative to the current code; the root-cause analysis originally propagated that stale claim without checking the source directly, exactly the kind of unverified inference this document is meant to avoid. `OpaqueReason` can derive `JsonSchema` normally, alongside the other plain structs — the fix is cheaper than originally stated. Flag ADR-0002's stale claim about `OpaqueReason` for a correction the next time that file is touched. If `schemars` as a new dependency is undesirable, the floor-level fix is a CI test that serializes a real fixture and validates it against `schema.rs`'s hand-written output.

**Resolves:** P0-5.

**ADR: Yes — see draft below.**

### 11. CLI exit-code contract has no shared type

**Root cause:** The 0/1/2 exit-code mapping is a doc-comment convention re-typed per command, not a shared function — `extract.rs:68-71` and `check.rs:21-30` already implement the same check with syntactically different code. `ExtractionOutput` has no `exit_code()` method, plausibly because `crates/core/AGENTS.md`'s "no terminal/display code in `crates/core`" rule got over-applied to mean "no exit-code logic belongs in core either." `cmd_watch` returns `()`, not `Result<i32>` (main.rs hardcodes exit 0 for that arm); `cmd_inspect` never touches `output.diagnostics` at all. Both were plausibly written by copying the general command-handler shape without copying the exit-code block, because nothing — return type, trait, lint — forces a new command handler to remember it.

**Mechanism `[corrected]`:** add `impl ExtractionOutput { pub fn max_severity(&self) -> Option<DiagnosticSeverity>; pub fn exit_code(&self, strict: bool) -> i32 }` in `core/types/output.rs` — pure data-in/data-out, same category as the existing `html_element()` helper, so it doesn't violate the core/no-terminal-code rule. **Verified gap: `DiagnosticSeverity` has no ordering today** (`#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`, no `Ord`/`PartialOrd`, no manual impl) — `max_severity()` needs one added before it can be implemented as a one-liner. Add `Ord`/`PartialOrd`/`Eq` (with variant declaration order double-checked against "worst severity wins" semantics — derived `Ord` ranks by declaration order, not by any inherent worst-to-best meaning) or a private `fn rank(&self) -> u8` helper used inside `max_severity()` instead of relying on derived `Ord`. Route `extract.rs` and `check.rs` through the new method (collapsing their inline checks) — note `check.rs` additionally supports a `--strict` warnings-escalate-to-1 path that `extract.rs`'s args have no equivalent flag for today; preserve that distinction rather than assuming the two commands' checks are fully identical. Change `cmd_watch` to return `Result<i32>` and call `exit_code(false)` after each pass, or explicitly gate propagation behind a `--ci` flag if always-0 is intentional for interactive use — document the choice either way instead of leaving it silent. Call `exit_code(false)` in `cmd_inspect` after a successful lookup so an Error diagnostic elsewhere in the tree surfaces even when the inspected component itself is fine.

**Resolves:** P3-1.

**ADR:** No — a doc comment on `exit_code()` documenting the mapping is sufficient; this formalizes an already-informal contract, it doesn't create a new one.

---

## Standalone findings — no shared mechanism, treat as independent tasks

| Finding | Location | Why it doesn't cluster |
| --- | --- | --- |
| ~~Generic alias `.zip()` silently truncates unsubstituted trailing type params~~ — **fixed** | `resolver/substitute.rs:33-43` | Was a silent _truncation_ of a substitution map, not a give-up-and-return-nothing path. **Resolved:** `build_substitution` now pushes a `DiagnosticCode::GenericArgumentMismatch` diagnostic naming the unfilled parameter(s) (test: `build_substitution_diagnoses_unfilled_trailing_type_params`) — found stale while building `docs/architecture/resolver-type-resolution.md`. |
| `watch --out` write failure discarded via `let _ =` | `cli/commands/watch.rs:107-112` | CLI-layer, not extraction-pipeline degradation; non-negotiable #6 is scoped to `crates/core` — the real fix is routing through the CLI's own miette/`output.rs` error path, a different mechanism than the core-side clusters. **Verified correction:** this write happens inside a `Watchexec::new(move \|action\| { ...; action })` callback closure that returns `Action`, not `Result` — there is no `?`/return-propagation path out of it. The fix must surface the failure _inline_ (print via `output.rs`'s diagnostic printer or equivalent), not "propagate through miette" as originally phrased, since propagation out of this closure isn't structurally possible without restructuring the watch loop. Same file's non-atomic `fs::write` (no temp+rename) should be fixed alongside it. |
| ~~Cache staleness on same-tick, same-size edits~~ — **fixed 2026-08-13** | `cache.rs` (P0-2) | Was a genuine filesystem-timestamp-resolution limitation — `key_for` already extracted the maximum precision the OS exposed (nanosecond `mtime` + size), and the residual gap needed an independent signal the key didn't carry. **Resolved as this table itself suggested**: replaced `(size, mtime_ns)` with a content hash via `rustc_hash::FxHasher` (already a dependency, no new crate needed) over the file's bytes — a fast non-cryptographic hash, exactly the "far cheaper than 'content hash' implies" approach flagged above. `get`/`insert` now take the file's content as a parameter instead of stat-ing it internally, since every call site already reads the file to parse it on a miss — no call site pays for an extra read this fix didn't already need. `CACHE_SCHEMA_VERSION` bumped to invalidate old on-disk caches under the new key shape. Tests: `test_stale_key_after_content_change` (rewritten to prove the actual P0-2 scenario — same-length, different-content — misses), `test_identical_content_still_hits_even_with_a_different_mtime` (guards against the fix becoming over-conservative). |
| `ParsedProp` allows contradictory `required: true` + `default_value: Some(_)` | `types/output.rs:79-99` (P0-3) | Missing type invariant, not duplicated logic — there was never a shared constructor to unify. **Verified correction: the originally-proposed fix (`pub(crate)` fields) is unsound, not just imprecise.** `crates/cli` is a separate crate that reads `ParsedProp`'s fields directly across the crate boundary (`extract.rs`, `inspect.rs`) — `pub(crate)` would be a hard compile error there. And even ignoring that, `pub(crate)` grants visibility to the _entire defining crate_, which includes every one of the actual offending construction sites (`chain.rs`, `alias.rs`, `mod.rs`, `known.rs`) — it would stop nothing it's meant to stop, since none of the violators are in a different crate. Correct fix: a sealed-field approach — keep construction possible only through `ParsedProp::new(...)` (module-private field access, not crate-wide), while adding public read accessors so `crates/cli`'s existing field reads keep compiling. Add a line to the `rust-types` skill checklist: "if a struct has two fields whose combination can be contradictory, provide a validating constructor — and check field visibility is scoped to actually prevent the misuse, not just relocate it." |
| `DiagnosticCode::Unknown` never constructed | `types/diagnostic.rs:61` (P3-2) | Genuinely dead code but intentional headroom on a `#[non_exhaustive]` enum for external consumers — no fix needed. |
| `NumberLiteral` NaN/Infinity round-trips to `0.0` | `types/output.rs:314,423` | **Verified correction: severity was understated, not just optimistic.** The original framing ("no realistic reachable input") is wrong — an ordinary numeric-literal type like `type X = 1e400;` is valid TS syntax, parses to `f64::INFINITY` under normal IEEE-754 overflow at `extractor/mod.rs:328` with no range check, and reaches this exact null-round-trips-to-`0.0` path through completely normal source text, not an adversarial case. Worth raising from cosmetic/low-priority to "add a guard/diagnostic at the extraction boundary." |
| Zero-member `LiteralUnion` serializes as an empty RDT enum — **location corrected** | `cli/commands/extract.rs` (`rdt_type_json`) | Same tier as above — low-priority, cosmetic. **Correction:** the serialization layer already closes this — `PropType::is_literal_union()` requires `members.len() >= 2` for both `Union` and `LiteralUnion` (fixed in Part G Task 6 of the edge-case remediation), so a 0-member union never reaches the `{"name":"enum",...}` shape at all; it falls through to a plain `raw_string()`. The residual gap, if any, is upstream — a resolver construction site producing an unhelpfully empty raw type string, not a serialization-layer bug. Found while building `docs/architecture/serialization-formats.md`. |

---

## Draft ADRs

Two clusters are genuine new architectural decisions, not enforcement of an existing pattern. Drafted here; write the real `docs/adr/000N-*.md` file as part of implementing that cluster, per the ADR guide's "write it when the decision is made" convention — these are proposals, not yet accepted.

### Proposed: 0005. Contain panics at a single, per-item boundary

**Context:** No existing ADR covers panic/unwind policy. Panic safety today is an accident of `tokio::spawn_blocking`'s `JoinError`, inconsistent across NAPI entry points, plugin hooks, and rayon batches — `create_session`/`close_session` don't even get that accidental protection, and `watch.rs`'s `std::sync::Mutex` can poison permanently as a direct consequence. Picking the wrong granularity now (per-batch vs. per-file vs. per-plugin) is expensive to fix retroactively once call sites depend on it.

**Decision:** Panics reachable from a rayon `.map()`, a `DocgenPlugin` hook, or a NAPI entry point are contained at per-file/per-plugin/per-call granularity through one sanctioned helper, `panic_guard::contain_panic`, converting the payload into a `Diagnostic` (or `napi::Error`) instead of aborting a batch, killing the whole pipeline, or poisoning a session lock. `contain_panic` wraps its closure in `AssertUnwindSafe` internally (documented rationale: this codebase's data has no interior mutability that could leave an observably-torn state behind a caught panic), so callers never need to reason about `UnwindSafe` themselves — including the plugin-hook call sites, whose `&mut` captures are otherwise `!UnwindSafe`. At the NAPI boundary specifically, `napi-rs`'s own opt-in `#[napi(catch_unwind)]` attribute is a viable complementary or alternative mechanism worth considering alongside `contain_panic`.

**Consequences:**

- One bad file, plugin, or session call degrades to a diagnostic instead of taking down everything sharing its batch/pipeline/session.
- Every future concurrent, plugin, or FFI entry point has one obvious place to route through, instead of re-deriving the answer.
- `watch.rs`'s poisoned-mutex trap becomes structurally much less likely, since nothing panics while the lock is held anymore.

### Proposed: 0006. Derive JSON Schema instead of hand-writing it

**Context:** `schema.rs`'s hand-written schema has already drifted from `ComponentEntry`/`ExtractionStats`/`Diagnostic`'s real fields, undetected because the only test checks JSON syntax validity, not field-set parity. ADR-0002 already established the precedent of hand-writing `Serialize` for the three recursive enums (`PropType`, `CollectedType`, `OpaqueReason`) to avoid a recursion-limit blowup.

**Decision:** Derive `schemars::JsonSchema` on the plain output structs, including `OpaqueReason` (verified non-recursive, already using an ordinary `derive` for `Serialize` — not one of the hand-written cases despite ADR-0002's text suggesting otherwise); hand-write matching `JsonSchema` impls only for `PropType` and `CollectedType`, the two enums that are actually recursive and actually hand-write `Serialize` today. `cmd_schema()` calls `schemars::schema_for!(ExtractionOutput)` instead of building a `json!()` literal by hand.

**Consequences:**

- A second hand-maintained trait impl per recursive enum whenever a variant is added — the same "compiler won't remind you" tradeoff ADR-0002 already accepted for `Serialize`, now doubled. Scoped to 2 enums (`PropType`, `CollectedType`), not 3.
- Every plain-struct field addition is schema-correct automatically; only the two recursive enums need manual attention going forward.
- New dependency (`schemars`) — if that's undesirable, the floor-level alternative is a CI test that serializes a real fixture and validates it against the hand-written schema, catching drift without adding a dependency.

---

## Phase 1 task breakdown

Grouped by shared-file conflicts so independent clusters can run in parallel and conflicting ones get merged or sequenced.

| # | Task | Files touched | Scheduling |
| --- | --- | --- | --- |
| 1 | Resolver give-up constructor discipline (narrowed to 3 sites: `chain.rs:40`, `func.rs:61`, `alias.rs:116`) | `resolver/{mod,chain,func,alias}.rs` | Sequential with #8 (shared `chain.rs`) |
| 2 | Extractor diagnostic channel | `extractor/{mod,alias,visit}.rs`, `types/diagnostic.rs` | Sequential with #4, #7 (shared files) |
| 3 | Pipeline discovery/merge fixes | `pipeline/{discover,mod}.rs` | Sequential with #4 (shared `pipeline/mod.rs`) |
| 4 | Panic-containment boundary + ADR 0005 | new `panic_guard.rs`, `pipeline/mod.rs`, `plugin.rs`, `crates/binding/src/lib.rs`, `pipeline/watch.rs`, `types/diagnostic.rs` | **Do first if possible** — other tasks touching the same files should build on top of it |
| 5 | Allocation caps | `resolver/template.rs`, `cli/commands/lsp.rs` | Parallel-safe except vs #1, #6 |
| 6 | LSP scaffold hardening | `cli/commands/lsp.rs` | Merge with #5 (same file) |
| 7 | Extractor depth-tracking | `extractor/mod.rs` | Merge with #2 |
| 8 | Resolver precedence extraction | `resolver/mod.rs` or new file, `named.rs`, `chain.rs` | Merge with #1 |
| 9 | TOON truncation helper | `toon.rs` | Fully parallel, disjoint |
| 10 | Schema derivation + ADR 0006 | `cli/commands/schema.rs`, `core/types/*.rs` derives | Parallel-safe, additive derives only |
| 11 | CLI exit-code contract | `core/types/output.rs`, `extract.rs`, `check.rs`, `watch.rs`, `inspect.rs`, `main.rs` | Parallel-safe |
| 12 | Standalone fixes | `substitute.rs`, `watch.rs`, `cache.rs`, `types/output.rs` (`ParsedProp` ctor) | Each independently parallel; `watch.rs` and the `ParsedProp` fix should merge with #4/#1 respectively if scheduled concurrently |

**Practical grouping for implementation:** #4 first (unblocks nothing else but touches the most shared surface), then {#1+#8}, {#2+#7}, {#3}, {#5+#6}, {#9}, {#10}, {#11}, {#12} — seven parallelizable units after #4 lands.
