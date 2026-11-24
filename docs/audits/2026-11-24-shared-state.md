# Shared and long-lived mutable state: audit

Read-only audit of `origin/main` at `f4a88f93` (2026-11-24), crates `core`, `semantic`, `binding`, `cli`. Every finding below comes from reading the source. None was executed. Line numbers are for that commit.

Severity counts: **High 4**, **Medium 4**, **Low 10**. Seven areas were checked and found sound (see "Checked, no defect").

Ranking: first by whether a hit produces a wrong or non-reproducible output, then by wasted work, then by memory.

---

## Top 10 hits

### 1. HIGH. Session caches a checker outage as the component's answer, silently (f, c, d)

- **Where:** `crates/core/src/pipeline/semantic_hook.rs:302-321` (`ask` stores every `Settled::Answered` in `cache.answers`) and `:416-442` (`read_outcome`). Also `crates/semantic/src/on_demand.rs:550-556`, `:514-520` and `crates/semantic/src/checker.rs:346-350`.
- **Mechanism:**
  - A checker request can exceed `request_timeout`. The wall-clock deadline is in `checker.rs:303-320` and `:346-350`, and a timeout becomes `CallError::Fatal`. A request can also hit a checker crash or "the checker is not running".
  - Each of these produces `SemanticOutcome::unavailable(..)`, which has no components and no fallbacks.
  - `read_outcome` treats that as `unavailable`. It moves the Warning into `once` (not kept) and returns `Answered { component: None, diagnostics: [] }`.
  - `ask` then inserts that empty answer into `cache.answers` for the current generation.
- **Failing order:**
  1. Request A arrives while the machine is loaded, or the checker dies mid-request. A neighbour prefetch is enough.
  2. Request A gets the structural answer plus the one-time warning. `warned` is set (`:248`).
  3. Every later request for A until any source edit hits `Found::Cached`. `apply` returns early because `component` is `None` (`:333-336`).
  4. The result is structural props with no diagnostic at all. `awaits_checker` is false, so the host never asks again.
  - The checker has restarted (`ensure`, `on_demand.rs:447-458`), but A's answer is pinned.
- **Minimal fix:** in `ask`, store only when the outcome is not `unavailable`, meaning a component or a per-key fallback came back. Better, have `read_outcome` return a `Degraded` marker that is never inserted (see M3). Optionally re-emit `CHECKER_PENDING` so the host retries.
- **Proving test (fails on base):**
  - Use a fake `SemanticProvider` that delivers `SemanticOutcome::unavailable("timeout")` for the first request and a real component for the second.
  - Request 2 must carry the checker's props.
  - Differential oracle: a fresh `SemanticHook` per request, which has no shared answer cache.

### 2. HIGH. A thread-local nesting limit leaks into memo recordings, so output depends on component order (a, f)

- **Where:**
  - `crates/core/src/resolver/primitives.rs:290` (`ReadNesting::enter()?` returns `None` past `MAX_NESTED_PROPS_READS = 4`) and `:323-351` (thread-local `READS_IN_PROGRESS`).
  - `crates/core/src/resolver/memo.rs:146-171` (`applies_to` does not read nesting) and `:203-328`.
  - The memo wraps reach these reads through `chain.rs:506` and `named.rs:43`.
- **Mechanism:**
  - `read_from_props_chain` resolves the indexed chain in a fresh `ResolveState` (`:302-304`). That state's `interface_chains` and `named_results` recordings depend on the thread-local nesting, which `StateReads` does not record.
  - At nesting 4, an inner `ComponentProps<typeof X>["k"]` read returns `None`, so the member becomes opaque.
  - `props_reads` refuses to keep opaque results (`:309-312`). The **memo recordings of the enclosing chain do keep them**.
- **Failing order:**
  - Take five wrapper components, each typing one prop as the previous one's: `size?: ComponentProps<typeof Base>["size"]` through Button, IconButton, ToolbarButton and MenuButton. This shape is common in design systems.
  - If MenuButton resolves first, the chain of Button is recorded at nesting 3-4 with `size` opaque. Button, resolved directly afterwards at nesting 0, replays the opaque `size`.
  - If Button resolves first, `size` is correct for every component.
  - Rayon's scheduling decides which order happens.
- **Minimal fix:**
  - Add `nesting_fired: bool` to the recording, set when `ReadNesting::enter` refuses inside the capture (like `depth_fired`).
  - A recording that fired answers only the same nesting, or is not kept.
  - Equivalent: make the nesting part of `ResolveState` and of the mode key.
- **Proving test:**
  - Build the five-level fixture.
  - Run the `orders()` harness from `resolver/tests.rs` (`type_parameter_scope_answers_do_not_depend_on_the_order_components_resolve_in`) over all permutations, with 1 and N threads.
  - Oracle: each component resolved in its own fresh `ResolutionContext`.

### 3. HIGH. A session's dependency layer only grows, so React augmentations diverge from batch (d)

- **Where:**
  - `crates/core/src/pipeline/revision.rs:146-151` (`affected_by`), `:475-486` (`apply_change` extends the layer), `:629-632` (the eager keep rule), `:488` (`react_namespace` is set only when the edited file itself declares into React).
  - `crates/core/src/pipeline/revision/lazy.rs:299-310` and `:510-538`.
  - `crates/core/src/dependency_types.rs:803-860` (`module_demand` and `load_augmentations`).
- **Mechanism:**
  - Augmentations (`declare module "react" { interface Attributes { css?: … } }`, as `@emotion/react` ships) are global. They change every component that reads React's attributes (`reads_augmentations`).
  - An edit that **adds** `import '@emotion/react'` to file F extends the layer with the augmenter. Components in other files are kept, because they did not read F and `change.react_namespace` is false.
  - An edit that **removes** the import keeps the augmenter forever ("the new layer keeps everything the old one had", `:478`).
- **Failing orders:**
  - **Eager session:** start with B in `B.tsx` (a `<button>` wrapper). Add the import to `F.tsx`. B keeps its props without `css`, while a cold `extract` lists `css`. Removing the import gives the reverse.
  - **Lazy session:** B is answered from its memo, or, if B was evicted, its answer depends on whether a component of F was requested first. That request grows the layer through F's seed demands at `lazy.rs:422-424`.
- **Existing test that misses it:** `dependency_types/tests.rs:1221` (`watch_revisions_with_dependencies_match_a_cold_extract`) uses one component in the edited file only.
- **Minimal fix:**
  - When an extension's augmenter set differs from its parent's, or the changed file's module-only demands differ before and after the edit, mark the change `react_namespace = true`.
  - On removal of a module demand, rebuild the layer, or treat the change as `SourceChange::Everything` for `reads_augmentations` entries.
- **Proving test:**
  - Fixture: two files plus a fake package whose types augment `React.Attributes`.
  - Apply random sequences of add and remove import edits, under both eager and lazy sessions, with random request orders.
  - After every step the snapshot or answer must equal a cold `extract` (the oracle).

### 4. HIGH (persists across runs). A transient read failure becomes a persisted "not exported" answer (c, f)

- **Where:** `crates/core/src/dependency_types.rs`:
  - `:489-504`: `read` returns `None` on I/O error, and the `parsed` `OnceLock` stores `None`.
  - `:614`: `let data = self.parse(module)?` returns `None` without setting `cut`.
  - `:596-603`: the answer is cached and `learn`ed.
  - `:1419-1446`: `write_index` persists the `(module, name, None)` export row.
- **Failing timing:**
  - A cold load runs on a loaded machine: rayon fans out reads, so `EMFILE` is possible, or a package install races the run, or a file is temporarily unreadable.
  - That run warns once.
  - The index is written. Every later run reads it, because `package.json` is unchanged (`current_packages`, `:1311-1339`), and never walks the module again.
  - The type silently resolves as "not exported", with no read warning in those runs.
- **Minimal fix:** treat a failed or absent parse in `find_export_uncached` like a cycle cut (`*cut = true`). The answer is then returned but neither kept nor persisted. As a belt-and-braces rule, do not `write_index` when `panics` holds a read failure.
- **Proving test:**
  - On unix, `chmod 000` a dependency `.d.ts` and run `load` with the cache on.
  - Restore the permission and run `load` again.
  - The second run must find the export. Oracle: the same run with `use_cache = false`.

### 5. MEDIUM. Session checker answers outlive edits outside `src_dirs` (a, d)

- **Where:** `semantic_hook.rs:224-236`, where the cache key is the component key plus a hash of the component's own file bytes and its `Layering`. Also `:276-293`, and `watch.rs:370-374`, the only place that invalidates.
- **Mechanism:**
  - The checker's answer depends on the whole closure: monorepo packages reached through `paths`, other workspaces, and `node_modules`.
  - The session stamps only source files.
  - `OnDemand` would detect an untold change through its stamps (`on_demand.rs:571-607`). A `Found::Cached` hit never reaches the checker, so the drift check never runs.
- **Failing order:** answer A, edit `../shared/types.ts` (outside `src_dirs`, so no `update_file`), ask for A again. The stale props come back with no warning, indefinitely.
- **Fix:** key cached answers by an `OnDemand` snapshot generation. Bump the generation on untold drift, which `maintain` and `stamp` can check cheaply after each reply. Alternatively, re-validate stamps on a cache hit, rate-limited.
- **Test:** a session with a checker and an import from outside `src_dirs`. Edit that file without telling the session. The next request must return the new props or the untold warning.

### 6. MEDIUM. Binding session registry is check-then-insert, and the ids wrap (wasted work, wrong tracking, memory)

- **Where:**
  - `crates/binding/src/lib.rs:310-320`, `:347-357` and `:386-391`: `SESSIONS.get`, then `new_session`, then `SESSIONS.insert`.
  - `:178`: `IN_FLIGHT.insert` inside `new_session`.
  - `:89-98`: a 16-bit counter.
- **Failing interleaving:**
  - `initializeSession(id)` and `extractFileIncremental(id)` arrive together for a new id. Vite does this at start-up.
  - Both miss the registry and build sessions, which spawns **two warm checkers**. The second insert orphans the first, along with its tsgo process and its memory.
  - `IN_FLIGHT[id]` now points at whichever was inserted last, so `sessionCheckerSettled` can wait on the orphan's `InFlight` and report "settled" while the live session's checker is still answering.
  - Options passed to an existing id are silently ignored.
  - After 65,536 sessions in one process the id wraps and `insert` replaces a live session.
- **Fix:** `SESSIONS.entry(id).or_try_insert_with(..)`, with `IN_FLIGHT` registered inside the same entry closure. Use a plain 32-bit counter; the pid bits buy nothing in a per-process map.
- **Test:** 8 threads call `initialize_session` for one new id. Assert one `Arc` (via `ptr_eq`) and one checker started, counted through a test seam.

### 7. MEDIUM. Dependency-index keys omit inputs (a)

- **Where:** `crates/core/src/resolver/mod.rs:649-665` (`resolution_fingerprint`) and `dependency_types.rs:1265-1339`.
- **Missing inputs:**
  - **Tsconfig chain:** the fingerprint hashes the governing tsconfig's path and mtime only, not the configs it `extends` or `references`. Editing `customConditions`, `moduleResolution` or `paths` in `tsconfig.base.json` keeps the old resolutions.
  - **Declaration files:** export and augmenter rows are valid "while `package.json` keeps its mtime and size". In-place `.d.ts` edits (`patch-package`, a locally built `file:` package inside `node_modules`) do not touch `package.json`, so stale exports and augmenters are read back.
- **Fix:**
  - Fingerprint the resolved config chain. `oxc_resolver`'s tsconfig gives its extended paths; hash their mtimes, or hash the effective options.
  - Stamp the files an export row read (mtime and size) instead of the package manifest. That costs one `stat` per row; measure it.
- **Test:** write the index, edit the base config's conditions, then compare the second run with a `use_cache = false` run.

### 8. MEDIUM (memory, long-lived). Process-wide interner `key_id` is never freed (e)

- **Where:** `crates/core/src/types/global.rs:22-31`. It holds `static IDS: OnceLock<DashMap<Box<str>, u32>>` and `static NEXT`.
- **Memory:** every distinct `file:name<args>` key minted over the life of a NAPI dev-server process is retained. Each edit can mint new argument strings, and every session and every batch run in the process adds to it.
- **Order dependence:** ids depend on thread interleaving. That is harmless only while nothing in the output iterates a `FxHashSet<u32>` of ids, and no test pins that.
- **Fix:** an interner owned by `ResolutionContext`, `Arc`-shared by `over_layer` and dropped with the revision.
- **Test:** an allocation or length bound after N watch edits, where the interner length after a revision is dropped equals 0.

### 9. LOW-MEDIUM (wasted work, timing). The resolver `Memo` wait-for check is not atomic

- **Where:** `crates/core/src/resolver/memo.rs:225-246` checks `would_wait_in_a_cycle`, then inserts into `waiting`, as separate DashMap operations. Also `:55-69` (`MAX_WAIT` = 250 ms) and `:33` (`RECORDINGS_PER_KEY` = 4).
- **Interleaving:**
  - Threads A and B each lead one key and need the other's.
  - Both pass the cycle check before either records its wait, and both block.
  - Only the 250 ms timeout ends it. Each then resolves the key itself, doing duplicate work and stalling.
  - On a loaded machine, ordinary waits also time out.
- **Contrast:** `semantic/src/convert.rs:653-681` (`Flights`) checks and records under one lock.
- **Effect on output:** none if `applies_to` is complete. Hit 2 shows it is not, and timing decides which four states get recorded.
- **Fix:** port `Flights`: one `Mutex<FlightTable>` holding both `running` and `waiting`.
- **Test:** a shuttle or loom model of three threads with a key cycle. Assert no wait ever happens in a cycle and no timeout fires.

### 10. LOW (test flakiness). Process-wide test counters, the same class as recorded case 2 (e)

- **`FOLLOW_REEXPORTS_CALLS`** (`crates/core/src/resolver/import.rs:24-34`, `:99`) is a process-wide `Mutex`.
  - Two tests that call `reset_follow_reexports_call_count(scope)` race: the last reset wins, and the other test counts 0.
  - Safe under nextest, which runs one process per test. Flaky under `cargo test`.
- **`CELL_LOOKUPS`** (`dependency_types.rs:1009-1020`) is thread-local, but `probe_once` runs inside `rayon::scope` tasks. Lookups on worker threads are not counted by the test thread, so the counter under-counts. Unverified: check what the test asserts.
- **Fix:** move the counters onto the owning struct (`Loader`, `ResolutionContext`) under `#[cfg(test)]`.

---

## Other hits (low)

- **11.** `relevance_sites` and `mapping_index` `OnceLock`s are shared with every `over_layer` context (`resolver/mod.rs:554-555`).
  - Sites are computed with the first layer's import map (`relevance_inputs.rs:67-76` reads through `resolve_to_canonical`, whose fallback is the layer).
  - The code assumes the layer cannot change call-site counts. No test pins that.
- **12.** The `warned` flag in `semantic_hook.rs:153,248` reports checker unavailability once per session. Together with hit 1 it hides every later degradation.
- **13.** `DtsCache` eviction takes the first 100 entries of a `RandomState` `DashMap` (`cache.rs:291-296`), so a different set is evicted each process. Processes sharing a cache directory are last-writer-wins. Wasted work only; the key (path plus content hash) is complete.
- **14.** The `declared.rs` memo (`:48-74`) keys JSDoc by the joined declarations but asks the checker by the symbol, so the first asker's symbol decides the answer. That rests on the assumption that JSDoc is a function of the declarations, which `@inheritDoc` and instantiated or mapped symbols might break. Not pinned.
- **15.** A panicked dependency demand stays in `demands` (`dependency_types.rs:699`, `:1162`), so a session never retries it until a rebuild. This is documented, and every answer carries the panic diagnostic.
- **16.** A kept answer on disk (`kept.rs:334-337`) is saved whenever the outcome has no diagnostic. The fallback "the checker rejected a request" (`lib.rs:838-840`) carries none, so a transient `Rejected` persists across runs while the disk is unchanged.
- **17.** The on-demand snapshot's read set is `source_file_names` only (`on_demand.rs:637-649`, `stamps.rs:62`). Failed lookups, `tsconfig` and `package.json` reads are not stamped, so an untold new file that would resolve a failed import is invisible. `kept.rs::Recording` already records all of these, which makes the two read-set models inconsistent (recorded case 4).
- **18.** `off_pool`'s `static POOL` (`revision/lazy.rs:25`) is one rayon pool shared by every session in the process. It is never freed and serializes dependency loads across sessions.
- **19.** The `props_reads` key uses a 64-bit hash of the arguments (`primitives.rs:291-296`). A collision returns the wrong member type. The probability is negligible, but unlike `shared_props` there is no equality fallback.
- **20.** `SemanticHook::refine` computes the cache-key stamp from the disk (`semantic_hook.rs:224`) while the structural answer comes from the revision loaded earlier (`watch.rs:399`). The checker answer can be keyed by text the revision has not applied yet. A later `update_file` invalidates it, so the window is short and nothing wrong is cached.

## Checked, no defect

- **Separate resolver caches (recorded case 1):** `dependency_types::load` builds its own `ModuleResolvers` (`dependency_types.rs:1113-1116`), and the context checks `tsconfig_diagnostic` on its own fresh resolver before anything else (`resolver/mod.rs:359-361`). `over_layer` shares the context's resolvers only with contexts of the same consumer.
- **Re-export cycle cut:** the `exports` memo keeps only uncut answers (`dependency_types.rs:572-605`). `the_loader_is_deterministic_across_demand_order_and_threads` pins this.
- **Converter memos:** `Entry::holds_in`, `Flights` (atomic cycle check) and `Answers`/`BatchAnswers` (single-flight, failures not kept) are scoped to one `answer_on` call and therefore to one snapshot (`lib.rs:641`). Handles never cross snapshots: on-demand builds converters per request. `Trees` shares only equal trees.
- **Heap budget:** the meter (`heap.rs`) is process-wide, but only the CLI's batch `resolve` installs a `Watch` (`lib.rs:650`), so process equals run. Going past the budget yields one warning diagnostic. `kept::save` skips any outcome with a diagnostic, and conversion errors propagate before `State::memo` stores (`convert.rs:1126-1139`). The non-determinism is documented and marked.
- **Timeouts in batch:** a timeout is `Fatal`, which makes the whole outcome unavailable with a warning, and that outcome is not kept on disk. Non-reproducible under load, but marked. Only the session path (hit 1) caches it.
- **Snapshot lifecycle (g):**
  - Every error path of `snapshot_for` releases the kept snapshot (`on_demand.rs:586-609`).
  - `forget` releases it on a failed answer.
  - `checker.rs:375-391` (`discard`) releases snapshots whose answer cannot reach the caller.
  - When `reroot` partly fails (`lib.rs:448-468`), the snapshots it updated are dropped unreleased, but the caller shuts the checker down at `:422`.
  - The `Teardown::Session` paths release first and then the lanes.
  - No leak found.
- **Kept reads (recorded case 4):** reads are noted by the session VFS for its whole life, so the early start's pre-reroot reads are included, which is conservative. A program opened without notes is refused (`lib.rs:225-227`).

---

## Inventory

Legend: **SF** = single-flight (concurrent identical work waits); **CO** = completed-only (concurrent misses both compute); **Order** = whether the value can differ by which consumer touches it first; **Pinned** = a test pins order-independence.

### crates/core

**`ResolutionContext`** (`resolver/mod.rs:260-332`). Owner: one context per batch run, eager revision or lazy layer growth. Readers and writers: rayon resolve threads.

| State | Key | SF/CO | Invalidation | Order / Pinned |
| --- | --- | --- | --- | --- |
| `module_resolutions` `DashMap` | `(dir, specifier)`, complete given the context's resolvers | CO | new context per revision or growth; copied by `inherit_memos` | no / no |
| `react_types_files` | dir | CO | same | no / no |
| `shared_props` | structural hash, then equality | CO | same; copied | no / n.a. |
| `member_reads` | `(file, name, key)` plus deps | CO | `inherit_memos` keeps entries whose deps the old layer covered | no / no |
| `props_reads` | `(file, name<hash args>, key)` | CO | same | hit 19 (hash); opaque results not kept / no |
| `mapping_index` `OnceLock` | derived from `global` | SF | shared with `over_layer` contexts | no |
| `relevance_sites` `OnceLock` | derived from `global` and the import map | SF | shared with `over_layer` contexts | hit 11 / no |
| `interface_chains`, `named_results`, `attribute_reads` (`Memo`) | `(key, mode)` plus `StateReads` | SF with 250 ms timeout, non-atomic cycle check | per context; `copy_into` filtered | **hit 2** (thread-local nesting omitted), hit 9 / `type_parameter_scope_answers_do_not_depend_on_the_order_components_resolve_in` (partial) |
| `attribute_names` | element and mode string | CO | per context | no |
| `oxc_resolver`, `types_first_resolver` (oxc internal cache) | path | oxc | per context, shared by `over_layer` | case 1 fixed |

**Thread-locals and statics in `core`:**

| State | Location | Lifetime | Note |
| --- | --- | --- | --- |
| `deps::RECORDER` | `resolver/deps.rs:29` | thread-local, per `resolve_component` | `Memo` captures deps and replays them; fine |
| `READS_IN_PROGRESS` | `resolver/primitives.rs:323` | thread-local | **hit 2** |
| `key_id` `IDS`/`NEXT` | `types/global.rs:22` | process | **hit 8** |
| `NO_CALL_SITES` | `relevance/mod.rs:450` | process | immutable; fine |
| `cache.rs` `SEQ`, `dependency_types.rs` `WRITES` | `cache.rs:367`, `dependency_types.rs:1508` | process | temp-file name counters; fine |
| `off_pool` `POOL` | `revision/lazy.rs:25` | process | hit 18 |
| `TSCONFIG_CHECKS`, `CELL_LOOKUPS`, `FOLLOW_REEXPORTS_CALLS` | `resolver/mod.rs:695`, `dependency_types.rs:1009`, `resolver/import.rs:24` | test only | hit 10 |

**Dependency `Loader`** (`dependency_types.rs:419-451`). Owner: one `load` call. Writers: the rayon scope's demand tasks.

| State | Key | SF/CO | Note |
| --- | --- | --- | --- |
| `parsed` | path | SF (`OnceLock`) | stores `None` on read failure: **hit 4** |
| `resolutions` | `(dir, specifier)` | CO | deterministic |
| `exports` | `(module, name)` | CO | cut-aware; **hit 4** |
| `package_manifests`, `scope_marks` (`probe_once`) | dir | SF | fine |
| `packages`, `scopes` | dir | CO | fine |
| `module_demands` | `(scope, specifier)` | SF | fine |
| `re_exports` | path | SF | fine |
| `augmenters`, `augmenter_crossed` | module | CO | fine |
| `demands`, `claimed`, `unresolved` | demand key | CO | a panicked demand stays (hit 15) |
| `panics` | n.a. | n.a. | diagnostics |

**Other `core` state:**

| State | Owner and lifetime | Key / invalidation | Note |
| --- | --- | --- | --- |
| `DependencyLayer` chain | a session's revisions, grown, flattened at `MAX_CHAIN_DEPTH` | demand keys; never shrinks | **hit 3** |
| Dependency index on disk | `dependency-index-{fp}.msgpack`, across runs | fingerprint plus package stamps | **hits 4, 7** |
| `DtsCache` | `cache.rs:154`; per run, merged to disk | path plus content hash: complete | hit 13 |
| `Lazy.memo` | `revision/lazy.rs:148`; per revision, carried over by `affected_by` | `ComponentId` | **hit 3** |
| `Lazy.engine` (`ArcSwap`), `growing` mutex, whole-demand thread | `revision/lazy.rs:148-157` | replaced on layer growth | fine |
| `Revision.resolved` (eager) | per revision, carried over by `affected_by` | `ComponentId` | **hit 3** |
| `WatchSession.current` (`ArcSwap`) | `watch.rs:205-212` | swapped per revision | fine |
| `WatchSession.writer` (`Mutex<Freshness>`) | `watch.rs:205-212` | ensure-fresh stamps (mtime and size, source files only) | fine |
| `SemanticHook` `AnswerCache` | `semantic_hook.rs:92-100`; per session | key: component key plus hash of own-file bytes and layering; generation bumped per edit; pending requests SF | **hits 1, 5, 12, 20** |
| `PropInterner` | `types/semantic.rs:96`; per hook generation or per run | hash plus equality | fine |

### crates/semantic

**Converter `State`** (`convert.rs:796-858`). Lifetime: per `answer_on` call, so one snapshot and one run or one on-demand request. Owner: one lane; `Trees` is shared across lanes.

| State | Key | SF/CO | Order / Pinned |
| --- | --- | --- | --- |
| `memo` | `(handle, depth)` plus `holds_in(enclosing)` | SF through `Flights` | documented order-free; `reverse` flag test and `several_programs_answer_as_one_does` |
| `origins`, `printed`, `parts`, `callable`, `alias_arguments`, `bases`, `targets`, `field_types`, `readonly` (`Answers`) | handle or `(handle, listing)` | SF; failures not kept | `asked.rs` tests |
| `declared` (`BatchAnswers`) | declarations or symbol | SF | hit 14 |
| `maps` | `MapKey` | CO; stored only when `invariant` holds | not pinned |
| `props` (`KeptProps`) | `PropKey` (complete per its doc) | CO | `a_kept_symbol_type_is_the_type_of_that_symbol` |
| `symbol_types` | symbol text | CO | fine |
| `trees` | fingerprint plus equality | n.a. | fine |
| `hits`, `misses`, `walks` | counters | n.a. | fine |

**Other `semantic` state:**

| State | Lifetime | Note |
| --- | --- | --- |
| `heap::HEAP` task-local `Watch` | per batch run | "Checked, no defect" |
| `rpc::REQUESTS` task-local; `Rpc.calls`, `recording`, `others` | per component, per `Rpc` | counters; fine |
| `kept::Reads` and the on-disk `checker-answers.msgpack` | per backend session, across runs | key: request digest plus every FS question; hit 16 |
| `OnDemand` `Checker`: `live` snapshot, `told`, stamps, `down`, `restarts`, `served` | per session | **hits 1, 17**; `down` is permanent past `max_rss_mib`, documented |
| `checker::Progress` | per `run_on_thread` | wall-clock deadline |
| `checker::TsrsSession.refused` | per session | spoils the kept reads |
| `checker::TsrsSession.building` (`RwLock`) | per session | fine |

### crates/binding

| State                                         | Location             | Note      |
| --------------------------------------------- | -------------------- | --------- |
| `SESSIONS`, `IN_FLIGHT` (`LazyLock<DashMap>`) | `lib.rs:25-27`       | **hit 6** |
| `NEXT_SESSION_ID` (16-bit wrap)               | `lib.rs:28`          | **hit 6** |
| `InFlight.counts`                             | `in_flight.rs:17-20` | fine      |
| `ALLOCATOR`                                   | `lib.rs:21`          | fine      |

### crates/cli

Only the `GLOBAL` allocator static (`main.rs:26`). No other shared state.

---

## MODEL AND ARCHITECTURE CHANGES

Recorded cases referenced below: (1) shared oxc resolver cache, (2) process-wide test counter, (3) completed-only memos, (4) incomplete read recording, (5) non-determinism (heap wrap, leaked symbol ids, timeouts).

### M1. Ownership map: `docs/architecture/shared-state.md` plus a lint

- **What:**
  - One table, like the Inventory above, listing every shared or long-lived state with: owner, writers, lifetime, key (and what the value depends on), SF/CO, invalidation, and the order-independence test that pins it.
  - Add `clippy.toml` entries `disallowed-macros = ["std::thread_local"]` and `disallowed-types` for `std::sync::LazyLock` and `OnceLock` in statics. Each sanctioned site then carries `#[allow]` with a one-line pointer to its row.
  - A new memo without a row fails review. A new static fails clippy.
- **Cost:** about half a day for the doc and half a day for the lint plus allow annotations at about 10 sites.
- **Would have prevented:**
  - Case 2 directly, through the lint.
  - Case 1: the row for `oxc_resolver` would have had to name both consumers and the tsconfig-references input.
  - Hit 8 and hit 10.

### M2. One `Memo<K, V>` abstraction (sync for `core`, async twin over `asked.rs` for `semantic`)

- **Single-flight:** the atomic wait-for graph of `Flights`, with check and record under one lock. No timeout fallback, or a timeout that only logs.
- **Typed key:** the key type implements `MemoKey`, with an `inputs()` that a debug build hashes into the entry. Replaying an entry then asserts the inputs are equal (cheap in tests, off in release). This turns "the key omits an input" into a test failure.
- **Outcome-typed values:** `store(Exact(v))` is kept, `store(Degraded(v, why))` is returned but never kept. This generalizes the `cut` in `Loader::exports` and the `holds_opaque` gate on `props_reads`.
- **Invalidation hook:** a generation counter, and `retain(|deps| ..)` for the watch and lazy carry-over.
- **Replaces:** the resolver `Memo` flights; the context's `DashMap`s; `Loader` `resolutions`, `exports` and `packages`; `SemanticHook` `AnswerCache`; `symbol_types`; `maps`.
- **Cost:** 3 to 5 days, staged one consumer per PR. The deterministic-output gate and the allocation check are needed for each PR (`Memo` sits on the Chakra hot path).
- **Would have prevented:**
  - Case 3: single-flight by construction.
  - Hits 1, 2 and 4, since all three store a degraded value.
  - Hit 9, through the atomic cycle check.

### M3. Degraded results carry a marker end to end

- **What:**
  - Anything produced under a limit carries a reason in the type: nesting, depth, heap, timeout, I/O failure, the checker unavailable, or the 250 ms wait.
  - Examples: `SemanticOutcome::unavailable` is a variant, not "no components"; `Loader::parse` returns `Err(ReadFailed)`; a nesting-cut prop is `PropType::Opaque { reason: NestingLimit }`.
  - Caches refuse to store marked values.
  - `gate_harness determinism` compares outputs modulo marked entries, and fails if a marked entry appears in a run without load.
- **Cost:** 2 to 3 days.
- **Would have prevented:**
  - Case 5: heap wrap-around would have shown as a marked degrade in an unloaded run and failed the gate.
  - Hits 1, 2, 4 and 16.

### M4. Determinism is a tested property, with oracle = separate state per consumer

- **(a) Resolver differential:** generalize `orders()` in `resolver/tests.rs` into a harness over fixture × permutation × threads {1, N}. The oracle resolves each component in a fresh `ResolutionContext`. Add the wrapper-chain fixture (hit 2) and the layout × order matrix that proved case 1.
- **(b) Session ≡ batch model-based test:** random edit scripts (add or remove an import of an augmenting package, add or remove a file, edit a shared type, edit outside `src_dirs`), with eager and lazy sessions and random request orders, the checker stubbed. After each step, compare with a cold `extract` (hits 3 and 5). Extend `watch_revisions_with_dependencies_match_a_cold_extract` rather than adding a script.
- **(c) Interleaving tests:**
  - shuttle (or loom) models of `Flights::board`, resolver `Memo::get_or_resolve` and `BatchAnswers::take` and `wait`: no wait cycle, no lost wake-up, at most one leader per key.
  - This needs the sync primitives behind a `cfg(shuttle)` alias, about 1 day.
- **(d) `gate_harness determinism` under load:**
  - Run it twice concurrently with an artificially small `request_timeout` and `HeapBudget`.
  - Assert that equal inputs give equal unmarked output, and that marked output is identical in the set of keys it marks.
  - This needs the existing recipe only; no new script.
- **Cost:** 3 to 4 days in total.
- **Would have prevented:**
  - Case 1: this is the differential that proved the fix.
  - Case 3: (c).
  - Case 5: (d).
  - Hits 2, 3, 5, 6 and 9.

### M5. One read-set model for every checker cache

- **What:** use the `kept::Recording` VFS (every question, failed lookups and tsconfig included) for the on-demand session too. Derive drift from it instead of from `source_file_names` stamps, and key the hook's cached answers by the resulting snapshot generation.
- **Cost:** about 2 days, plus a measurement of the per-request stamp cost on Chakra (1,855 files today; the question set is larger).
- **Would have prevented:** case 4 (one model, tested once), hit 5 and hit 17.

### Order of work

1. Fix hit 1 and hit 4 now. Each is under 30 lines with one test.
2. M4(a) together with the hit 2 fix.
3. M4(b) together with the hit 3 fix.
4. M3 and M2 staged.
5. M1 alongside, since it is cheap.
6. M5 once on-demand is measured.

## Not verified

- No test or reproduction was run for any hit. Every failing order was derived by reading the code.
- Hit 2 assumes `holds_opaque` is recursive and that no other gate drops the recording. A five-wrapper fixture would confirm it.
- Hit 3 assumes the augmentation reaches components through `reads_augmentations`, as `html::note_augmentations` documents. Not traced end to end.
- Hit 10: what the `CELL_LOOKUPS` tests assert.
- Hit 17: whether tsrs `updateSnapshot` re-resolves failed imports when it is told about a new file.
- No `entire brain` recall or query was run. Historical decisions (for example why `SemanticHook` keeps unavailable answers, if that is deliberate) were not checked.
