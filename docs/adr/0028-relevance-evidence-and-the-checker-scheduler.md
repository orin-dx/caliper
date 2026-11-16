# 0028. Show the props evidence makes relevant, and schedule the checker by what will be shown

**Status:** Proposed **Date:** 2026-11-15

## Context

Every surface returns every prop the checker resolves, minus what a collapsed layer hides. That is useless to Storybook and to agents: Chakra's `Button` had about 1,141 argTypes before the collapse-by-relevance PR (`feat/collapse-large-sources-by-default`) and 146 after it, and the rows still hidden are the ones consumers pass. Measured on origin/main 7b8f6d0d with the checker (trail 405 T5, `caliper-fixtures/shrink/relevance-baseline.md`), the share of the props a component's call sites pass that the output lists is 0.43 on shadcn (mean over 408 components; median 0.50), 0.86 on Mantine, 0.91 on ant-design and 0.96 on react-spectrum. Nearly every miss is a DOM attribute in a collapsed `React.ComponentProps<"el">` layer (shadcn: `className` alone on 243 components), and those layers carry `htmlElement` with no `members` and `totalProps` 0, so a reader cannot tell what is inside them. Today's choices come from size and from name lists (`resolver/ambient.rs`'s `*StyleProps`, `GlobalDOMEvents` clauses; `notable_html_attrs` in `react_types.rs:462`), which the owner rules out.

The checker's cost follows what it converts, not what is shown. A whole-project `extract` of Chakra's own repository built 42 GB of `PropType` trees converting its style system (`shrink/chakra-memory.md`), and the only memory guard (`OnDemandConfig::max_rss_mib`, `crates/semantic/src/on_demand.rs:93`) reads the checker's arenas (`crates/semantic/src/checker.rs:169`), not the converter's heap, and only between requests (`on_demand.rs:643`).

## Decision

### A. Relevance decides what is shown, from evidence

The owner's rules: returning all resolved props is useless for Storybook and agents; relevance decides what to show; it is derived from evidence, with no name lists; agent output is narrowed later.

**Signals.** Each prop a component's props type reaches gets evidence from four places.

- **The props type.** A member the props type declares itself is always shown. So is each key of `Pick<X, 'a' | 'b'>`, named on purpose; `Omit<X, 'k'>` says only that `k` is gone, nothing about the rest. A member that redeclares or narrows a base member is the component's own. A required member is shown (a story cannot render without it), and so is one with a `@default` tag or a destructuring default in the body (the component handles it). JSDoc and a literal-union type do not show a member on their own: as promoters they would list every documented member of a style system; they order the shown rows (open question 1).
- **Spreads in the body** (T1's `ComponentMapping::body`). `{...rest}` on a DOM tag passes the ambient DOM attributes: no evidence, ignored. A spread onto a component is one hop to that component's own non-ambient props, with the JSX tag resolved to the component by symbol and import (ADR 0027's identity, never the display name). An attribute set after the spread is fixed and subtracted; one set before it is an overridable default and kept; an attribute whose value is one of the component's own props adds nothing. A name in `reads` is shown when its source is not ambient.
- **Call sites.** The props consumers pass at JSX usages across the project (T1's `SourceData::jsx_usages`) and in stories' `args`, counted per component with a floor of `max(2, 20%)` of its sites. The baseline puts the cost at 0.30 (Mantine), 0.03 (Spectrum), 0.76 (shadcn) and 0.24 (antd) extra entries per component.
- **Unknown.** When `handed_on` is true, `reads` is incomplete (T1's contract), and the body says nothing: never guessed. The member falls back to today's single collapse decision with the reaching count (`CollapseInheritedProps::collapses` in the collapse PR; on main it is still `ResolutionContext::collapses`, `resolver/mod.rs:595`, by declared count, so `decide` is wired only after that PR merges), so missing evidence never hides more than today.

**Ambient** is decided by where a member is declared, not by its name: a member `@types/react` declares (by declaring path, as `is_react_declared` in `crates/semantic/src/component.rs` recognizes React terminals), or one that reaches the component only through a spread onto a DOM tag. Each lane computes the bit for each prop and hands it to `decide` in `Evidence` (`is_react_declared` is private to `caliper-docgen-semantic`, which core does not depend on); `decide` never recomputes it, and a model test in the style of the collapse PR's `collapse_model.rs` holds the two lanes to the same bits. `ambient.rs`'s name clauses and `notable_html_attrs` stay as the unknown-evidence fallback until the T5 gate shows evidence replaces them; then they are deleted (slice R9).

**Combination.** One function, `relevance::decide(&Evidence) -> Shown`, owns it, as `CollapseInheritedProps::collapses` owns collapsing; both lanes and every surface call it.

- Shown as a row: own, picked, overriding, required, defaulted; a non-ambient member that is forwarded, read, or passed at the floor.
- Named in the layer's `used` list: an ambient member passed at the floor or read in the body.
- Left in a collapsed layer: everything else with complete evidence, even from a source today's rule lists (its source becomes a layer). A non-ambient member with unknown body evidence and no other signal is listed or collapsed as today.

**Output.** A collapsed `InheritedLayer` (`crates/core/src/types/output.rs:83`) gains `used`: the names of its members that evidence put forward, written once as a name list (ADR 0018), not as typed rows. Universal props such as `className`, `style` and `id` cost a name each instead of a row each. A `React.ComponentProps<"el">` layer also gets its `members`, so a reader can check `used` against it. No per-prop flag in this ADR: the row/layer split already says shown or not, and why a row is shown is for the agent output, deferred. `SCHEMA_VERSION` goes from 7 to 8 (ADR 0017), with the RDT view, the JSON schema, `binding.d.ts` and a changeset.

**Consumers.** The Storybook addon makes argTypes from rows, then one name-only row per `used` name with the layer as its category; an option (`relevance: 'all'`) asks for the expanded output, which is today's `collapseInheritedProps: 'none'`. CLI JSON and TOON carry the same output. Agent output keeps its current form until the agent flow is narrowed.

**Acceptance gate** (T5's metric, ported into the weekly by T4). Coverage counts a passed prop as visible when it is a row or in `used`. Call-site evidence is computed from half of the files (by path hash) and coverage is measured on the other half, so the gate does not pass by construction. On the half the floor is `max(1, 20%)`, and only components with at least 6 sites are measured, so the gate does not test a stricter rule than the one shipped.

- Coverage mean rises: shadcn 0.43 to at least 0.70; Mantine 0.86 to at least 0.88; antd 0.91 to at least 0.93; Spectrum stays at least 0.96.
- Rows shown fall: the median rows per component does not rise on any corpus (Mantine 77, Spectrum 74, shadcn 3, antd 17), and total rows fall on Mantine and Spectrum. Chakra `Button`'s typed rows stay at or under 146 (its `used` names come on top and are reported apart).
- These targets are proposals. Only the extra-entry costs above were measured; the fall in rows is not predicted.

### A2. The listing follows the decision (slice T4, amended 2026-11-16)

Measured at de016f69 (`caliper-fixtures/oracle-run/diagnosis.md`): Storybook listed 437 Controls rows for a demo Chakra `Button` and 280 for a `ComponentProps<'span'>` `Badge`, because `decide` only wrote `used` lists while `htmlAttributes: 'full'` (ADR 0023) and the session merge (`checker_prop_used`) listed every `@types/react` attribute; the CLI merge dropped them all where a layer existed, so the lanes disagreed (Button 150, Badge 2). Changes to section A:

- **One placement, every lane.** `relevance::apply` keeps `decide`'s placement on the entry (`ComponentEntry::relevance`, not serialized) and lists by it (`relevance::narrow`); both checker merges call `narrow` again after laying out the checker's props, and take a `@types/react` prop only where the structural answer lists it (`react_prop_listed`), whatever the mode. A listed prop the placement leaves out moves to its source's layer as a name: the layer of its `parent`, else the element layer for an ambient source, else a new layer of that parent. A prop with no `parent` stays listed, so no name is lost.
- **Scope.** Only props a source outside the project declares (a dependency's, React's or the DOM's, by declaring path) are narrowed; the project's own sources keep the collapse rule until a later slice measures them. `collapseInheritedProps: none` narrows nothing: it is the expanded view (`relevance: 'all'`).
- **Rows.** A member passed at the call-site floor is a typed row even when ambient (a story needs its control; open question 2). An ambient member the body only reads is a `used` name. A member evidence puts forward that has no typed row (its source is collapsed) is named in `used`.
- **Unknown evidence.** An ambient member with unknown body evidence is a name in its layer, not a row: without call sites a component lists its own and declared props, never a whole DOM source. A non-ambient member with unknown evidence is listed as before.
- **Provenance, not names.** The placement keeps each left-out member with the source the structural answer gives it (`parent`, or the collapsed layer holding it); a merge narrows a listed prop only when name and source both match, so a same-named prop of another source is never hidden and a member with no structural source is narrowed in neither lane. A homomorphic mapped type's member (`{ [K in keyof Conditions]?: V }`) keeps its key's interface as `parent`, as the checker does.
- **Declined key sets.** A `Pick` or `Omit` (in place or behind an alias) whose keys are not a closed set of string literals narrows no non-ambient source: which members reach is unproven (findings D5). Keys a `Pick` names through `Omit`, `Partial`, `Required`, `Readonly` or a project alias are picked.
- **JSX children** pass `children` at a call site, after every attribute, as the JSX transform does; layout whitespace and empty expressions pass nothing.
- **`htmlAttributes`** (ADR 0023 is cited by `options.rs` but not in `docs/adr`; this amends its contract here) says which DOM attributes are known (as rows or layer names), not how many rows a reader sees; `full` stays the default because a passed attribute can be a typed row only if it is known. `notable_html_attrs` still feeds `curated` mode only; it no longer decides rows under the default, and R9 deletes it.

### B. The checker is scheduled by the props that will be shown

The owner asked for a queue. Its parts, against what exists:

- **Item.** The props of one component that relevance will show, at one revision, not the whole component; collapsed members are listed by name and parent and never converted. Exists: `SemanticTarget::react_wanted` and `described` (`crates/core/src/types/semantic.rs:169-175`) already narrow what one target converts. New: a `shown` set from `relevance::decide`.
- **Priority classes**, on-screen > neighbouring stories > background fill. Exists: two (`Priority::Interactive` and `Background`, `types/semantic.rs:221`), two lanes with a starvation bound and raising (`Queue`, `on_demand.rs:150-199`, `INTERACTIVE_STREAK` 4), a raise when an on-screen request joins a queued one (`pipeline/semantic_hook.rs:256`). The addon's prefetch queue holds the next `prefetchStories - 1` stories, replaced on every selection (`packages/storybook/src/background.ts:39`, `preset.ts:150`, `prefetch.ts:32`), one story at a time (`background.ts:81-89`), and asks at `Background` (`pipeline/watch.rs:464`). New: a third class, `Neighbour`, between them.
- **Preemption and cancel.** Exists: an edit forgets every kept and pending answer in the hook (`semantic_hook.rs:180`); the lifecycle reducer refreshes an answer superseded by an edit (`lifecycle.ts:196`); navigation replaces the addon's queue. Missing: nothing reaches `OnDemand`'s queue, which has no cancel, so requests nobody waits for still run. New: a request carries a cancel token tripped by an edit or by the navigation that replaced it, dropped when taken. Raising a request clears its token: an on-screen request that joined a queued prefetch (`Found::Joined`, `semantic_hook.rs:256`) is waited for, so a navigation never cancels it; and preemption between components of one request (a tsrs call cannot be interrupted).
- **Batching by snapshot.** Exists: `OnDemand` keeps one snapshot and re-roots it per request (`Checker::snapshot_for`, `on_demand.rs:551`); the one-shot CLI answers every component on one program with eight files in flight sharing one converter (`crates/semantic/src/lib.rs:49`, `session`). A request whose roots differ re-roots the snapshot (`rerooted`, `on_demand.rs:574`), which makes a new one with new type ids. New: the worker takes the queued items of one class together and roots one snapshot at the union of their roots, so they share the program and one converter memo; the memo is dropped with that snapshot, never carried to an updated or re-rooted one (type ids are valid within one snapshot, ADR 0026). The memo is per request today.
- **Admission on a memory budget.** Exists: the arena guard (`Upkeep::decide`, `on_demand.rs:709`), which misses the converter heap. In flight: the one-shot heap budget (heap-budget slice B, branch `feat/one-shot-heap-budget`, no commits yet), counting converted nodes inside conversion. New: the scheduler admits a background item only under a soft budget read from the allocator, holds background work above it, and stops the checker above the hard one, as `stop_past_bound` does.
- **Stream each result.** Exists: one `Delivery` per request (`types/semantic.rs:256`), and the reducer turns each settled key into a refresh (`lifecycle.ts:185-205`, invariants 1 to 6). New: a request of several keys delivers each key when converted. The CLI keeps one write: canonical JSON interns props across components (ADR 0009, 0018).

Not determined: whether tsrs's memory allows the converter memo to live as long as a snapshot over a long session (ADR 0026 measured 98 to 246 MB over 150 edits without it); whether `mi_process_info` reads the same under the napi host; how far listing collapsed members without converting them lowers the 42 GB case, since the collapse PR collapses `SystemProperties` after conversion.

## Consequences

- Output changes on every surface; the addon's default argTypes change again after the collapse PR. Complete evidence hides on purpose props today lists (a 10-prop dependency source the body never reads becomes a layer); that is how rows fall. Only a prop with unknown evidence is guaranteed to be shown as today.
- Agents read the same narrowed output until their flow is designed.
- Call-site evidence makes a component's answer depend on other files. Watch and the kept answer must invalidate on a consumer's edit, and the per-component closure is still unknown (ADR 0026), so invalidation stays whole-project.
- The scheduler adds a third class, cancel tokens and a long-lived memo to `OnDemand`; its determinism gate (gate harness) must hold with cancels.
- Breaking for readers of `InheritedLayer` (schema 8).

## Alternatives considered

- **A fixed universal set** (`className`, `style`, `id`, `aria-*`, `data-*`, `ref` plus handlers): about the same 1 to 3 rows, but a name list.
- **Expand everything** (`none`): 280 to 1,100 rows per component (`--no-checker` figures in the baseline).
- **Whole components as queue items**: converts collapsed members nobody sees, which is where the Chakra memory went.

## Slice plan

Files are disjoint within a wave; a file two slices touch is named with the order. Each slice runs `just lint doc-check machete typos`; a test that fails on main first.

**Wave 0, now:** R0.

- **R0, the decision function.** Owns `crates/core/src/relevance/mod.rs` (new), one line in `crates/core/src/lib.rs`. `Evidence`, `Shown`, `decide`, a table test per rule above. It also creates every wave-1 module empty and declares it (`relevance/props_type.rs`, `forwarding.rs`, `call_sites.rs`, `extractor/story_args.rs`, `extractor/factory_props.rs`, with their `mod` lines in `relevance/mod.rs` and `extractor/mod.rs`), so no wave-1 slice edits a shared `mod.rs`. About 200 lines. Depends on nothing. First failing test: `an_inherited_member_the_body_never_reads_stays_in_its_layer`. Gate: core nextest.

**Wave 1, after R0, the collapse PR and PR 470 (T1) merge; up to three at once, in this order of value:**

- **T3, forwarding.** Owns `relevance/forwarding.rs` (new), `crates/core/tests/relevance_forwarding.rs`. Resolves a spread's tag by symbol and import, takes one hop of own non-ambient props, subtracts attributes after the spread. About 400 lines. Depends on R0 and PR 470. First failing test: `a_rest_spread_onto_a_component_shows_its_own_props_except_those_set_after_it`. Gate: core nextest.
- **T6, call sites.** Owns `relevance/call_sites.rs`, `crates/core/src/extractor/story_args.rs` (new). Counts `jsx_usages` and stories' `args` per component at the floor. About 350 lines. Depends on R0 and PR 470. First failing test: `a_prop_passed_at_a_quarter_of_the_sites_is_used_and_one_at_a_tenth_is_not`. Gate: core nextest.
- **T2, props-type signals.** Owns `relevance/props_type.rs` (new), `crates/core/tests/relevance_props_type.rs`. Own, picked, overriding (the checker's `declarations`, ADR 0026 S7a), required, defaulted. About 300 lines. Depends on R0 and the collapse PR. First failing test: `a_member_that_narrows_a_base_member_is_a_row_when_its_source_collapses`. Gate: core nextest, `just test-checker-lane`.
- **F1, factory own props.** Owns `crates/core/src/extractor/factory_props.rs` (new), the field in `types/collected.rs` (after PR 470, which also edits it). `chakra("div", {variants})` gives its variant keys; Spectrum's `createLeafComponent` and `createBranchComponent` give the first parameter of their function as the props type. About 250 lines. First failing test: `a_chakra_factory_variant_key_is_an_own_prop`. Gate: core nextest; the census rows for Chakra's 11 and Spectrum's 4 factory components.
- **S1, classes and cancel.** Owns `Priority` and `SharedPriority` in `types/semantic.rs`, `crates/semantic/src/on_demand.rs`, `pipeline/semantic_hook.rs`, `pipeline/watch.rs`. Depends on the collapse PR (it edits these three core files). About 250 lines. First failing test: `a_neighbour_request_goes_after_an_on_screen_one_and_before_a_fill`, then `a_cancelled_request_is_never_answered`. Gate: semantic nextest, gate harness determinism and memory modes.

**Wave 2:**

- **T7, apply and output.** Owns `types/output.rs`, `pipeline/rdt.rs`, the JSON schema, `docs/architecture/serialization-formats.md`, `packages/napi/binding.d.ts`, the changeset, golden snapshots, and the call sites of `decide` in `types/semantic.rs`'s merge and `resolver/mod.rs` (after S1 and the collapse PR). Fills `used` and `members` of DOM layers; `SCHEMA_VERSION` 8. About 350 lines. Depends on R0 (T2, T3, T6 land as they come; unfilled evidence is unknown). First failing test: `a_collapsed_layer_names_the_ambient_members_consumers_pass`. Gate: checker lane, full-content diff on gate-large and chakra-only with listed changes.
- **S2, addon wiring.** Owns `crates/binding/src/lib.rs`, `packages/storybook/src/background.ts`, `session.ts`, `preset.ts`. Prefetch at `Neighbour`, cancel on a replaced queue. About 150 lines. Depends on S1. First failing test (Vitest): `a_replaced_prefetch_cancels_the_queued_requests`.
- **S3, stream per key.** Owns `on_demand.rs` (after S1). About 150 lines. First failing test: `a_request_of_two_keys_delivers_the_first_before_the_second_is_converted`.

**Wave 3:**

- **T4, consumers.** Owns `packages/storybook/src/arg-types.ts`, `options.ts`, `option-keys.ts`, `packages/storybook/README.md`, `scripts/weekly.py`, `scripts/test_weekly.py`, `docs/measurement.md`. The `used` rows, `relevance: 'all'`, and the weekly coverage row with the split-half method. About 300 lines. Depends on T7. First failing test: `a_used_name_becomes_an_arg_type_in_its_layer_category`. Gate: the acceptance gate above.
- **S4, memo per snapshot.** Owns `crates/semantic/src/lib.rs`, `convert.rs`'s memo lifetime, `on_demand.rs` (after S3). About 200 lines. First failing test: `two_requests_on_one_snapshot_convert_a_shared_type_once`. Gate: chakra-only session peak within 5% of main.
- **S5, admission.** Owns a new `crates/semantic/src/heap.rs`, `Upkeep` in `on_demand.rs` (after S4). Depends on heap-budget slice B. About 200 lines. First failing test: `background_work_waits_above_the_soft_heap_budget`.

**Wave 4:** **S6, items are the shown props** (`SemanticTarget::shown`, the converter's per-prop skip; after T7 and S5; first failing test `a_collapsed_member_is_listed_by_name_and_never_converted`; gate: Chakra's own repository extract under the heap budget), then **R9**, deleting `ambient.rs`'s name clauses and `notable_html_attrs` once the T4 gate holds without them.

Open questions: (1) do JSDoc or literal unions promote a member on their own; (2) does Storybook need a type for a `used` row to give it a control, which would make those typed rows instead; (3) the soft and hard heap budgets; (4) whether story `args` weigh more than a call site.
