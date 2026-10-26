# Pipeline orchestration

**Source:** `crates/core/src/pipeline/mod.rs`, `crates/core/src/pipeline/revision.rs`, `crates/core/src/pipeline/revision/lazy.rs`, `crates/core/src/pipeline/discover.rs`, `crates/core/src/pipeline/watch.rs`, `crates/core/src/resolver/deps.rs`, `crates/core/src/cache.rs`, `crates/core/src/import_map.rs`, `crates/core/src/dependency_types.rs`, `crates/core/src/panic_guard.rs`, `crates/core/src/plugin.rs`

This is the orchestration layer: it owns nothing about _how_ a component is parsed or resolved (that's `extractor/` and `resolver/`), only _when_ and _in what order_ files get discovered, parsed, merged, and resolved, and how failures at any step degrade instead of propagating.

## The 6-phase data flow

`Revision::cold()` (`pipeline/revision.rs`) is the single implementation behind `extract()`, `WatchSession::initialize()` and `WatchSession::rebuild()`. Every phase below runs exactly once per call:

- **Phase 0 — validate.** Checks `options.src_dirs` isn't empty and that at least one entry exists on disk. Sequential, trivial cost. Exists specifically so a typo'd `--src` produces an `Error` diagnostic instead of a silent zero-component, zero-diagnostic run (AGENTS.md non-negotiable #6).
- **Phase 1 — discover.** `discover_files()` walks `src_dirs` with the `ignore` crate's parallel walker, filters to `.ts`/`.tsx` and applies built-in and user excludes. It walks from each directory's canonical path, so only symlinked files need `canonicalize`. Per-file `realpath` calls cost more than parsing on Mantine. The final sort and dedup make the result independent of walker thread order (determinism invariant below).
- **Phase 2 — parse (parallel).** `src_files.par_iter().map(...)` via rayon. Each file either hits the `.d.ts` cache or gets parsed fresh by `extractor::parse_file`. This is the phase worth parallelizing: OXC parsing is pure CPU work, one file is fully independent of another, and this is typically the dominant cost of a run. Every closure body is wrapped in `panic_guard::contain_panic`.
- **Phase 3 — merge (+3.5/3.6), sequential.** Each file's `SourceData` (plus any plugin `on_file_extracted` mutation) is folded into one `GlobalSourceData` via hash-map insertions. This phase is sequential specifically because it's cheap (hash-map inserts, not real work) and because `GlobalSourceData` is a single shared structure being built incrementally — parallelizing it would require synchronization that costs more than the merge itself. Phase 3.5 (Full HTML-attribute mode) and Phase 3.6 (ambient TS lib files) extend the same merge with `@types/react` and `lib.es5.d.ts`/`lib.dom.d.ts`, both going through `merge_cached_dts_file` so their diagnostics aren't dropped (see Invariant 3).
- **Phase 3.7 — dependency declarations (parallel, `load_dependencies` only).** Before the merge consumes each file's `SourceData`, `dependency_types::project_demands` collects the names it imports from packages and uses in a type. `dependency_types::load` resolves each with the resolver's `ModuleResolvers`, follows re-exports to the declaring file, parses it, and follows the names that declaration references, in a rayon scope where each `(file, name)` is claimed once and each spawned demand runs under `contain_panic`. Only declaring files enter the resulting `DependencyLayer`: its own `GlobalSourceData`, `NamedTypeIndex` and `ImportResolutionMap`, which the project's index and import map fall back to for files they don't hold. Re-export answers are kept in the layer and in a per-package index next to the parse cache (`dependency-index-<resolution fingerprint>.msgpack`, ADR 0011), valid while each package's `package.json` keeps its mtime and size. Off by default (ADR 0010); with it off, no fallback is set and output is byte-identical.
- **Phase 4 — resolve (parallel).** `mappings.par_iter().map(|m| resolve_component(m, &ctx))`, again via rayon, again each closure wrapped in `contain_panic`. `ResolutionContext` wraps `Arc<GlobalSourceData>` so every thread reads the same immutable snapshot; per this crate's `AGENTS.md`, all resolver inputs must be owned or `Arc`-wrapped since rayon may run this on any thread.
- **Phase 5 — assemble.** `Revision::assemble()` folds the resolved components into the output `BTreeMap` in `(file, position)` order, running the display-name disambiguation (Invariant 1) and each plugin's `on_component_resolved` hook, then builds enums, diagnostics (deduplicated, with `affects`, in canonical order) and `ExtractionStats`. Watch revisions call the same function, so a watch snapshot and a cold extract cannot disagree on keys, hook output or diagnostic order.

**Why parallel at phases 1, 2 and 4, nowhere else:** discovery waits on directory reads, and parse and resolve are independent per item. Merge and collect both build one shared structure incrementally, which is cheaper done sequentially than synchronized.

**Resolution's hot lookups avoid `Path` hashing.** `Path`'s `Hash` and `Eq` walk components; the named-type index, the import map and the dependency recorder key on `PathKey`, which hashes the path string (paths are canonical, so string equality is path equality). Module resolutions are cached per `(directory, specifier)` in `ResolutionContext` for the life of one run or revision.

## Watch mode's incremental model

`WatchSession` (`pipeline/watch.rs`) holds one immutable `Revision` in an `ArcSwapOption` and a writer `Mutex`. `initialize()` builds the first revision with `Revision::cold()` and is idempotent; `rebuild()` replaces it with a fresh cold one. `update_file(changed)` takes the writer lock, builds the next revision with `Revision::next()`, and publishes it with one atomic store. Readers (`snapshot()`) never wait on the writer and always see one complete revision.

`ensure_fresh()` is for a host without a file watcher. Under the writer lock it compares stamps (modification time and size, plus a content hash while the time is within 2 s of the stat) of the revision's source files, every directory discovery walks, and every resolution config. A changed config rebuilds; a changed directory reruns discovery to find created and deleted files; each changed file goes through `Revision::next()` in path order. It returns one `output_delta()` from the starting output to the final one. Stamps are recorded on the first call after a cold extraction, and anything modified within 2 s of that extraction's start counts as changed, so an edit made while it ran is not lost.

`Revision::next(changed)`:

1. Ignores a path discovery would not include (outside every source directory, excluded, not TypeScript): `discover::is_source_file`.
2. Re-parses `changed` into copies of the previous global data and named-type index. A missing file is a deletion; any other read error (permissions, invalid UTF-8) declares nothing and records an `IoError`, exactly as a cold run does.
3. With `load_dependencies`, loads only the demands of `changed` the session's layer has not followed (`DependencyLayer::missing`) into a new layer built on the previous one's parsed files and answers, so a new package import costs one load and an unchanged one none.
4. Re-resolves the components that are new, live in `changed`, or whose recorded dependencies the change touches (below). Every other component keeps its previous `Arc<Resolved>`.
5. Calls `assemble()`, then reports the delta against the previous revision's output: `updated_keys` (re-resolved components, and kept ones whose key moved when a same-named component elsewhere appeared or disappeared), `removed_keys`, and the diagnostics added and removed. Applying every delta in revision order to the first snapshot yields the current snapshot; `every_kind_of_change_leaves_the_snapshot_equal_to_a_cold_extract` checks this after each step.

Each resolution records what it read (`resolver/deps.rs`, a thread-local recorder scoped to one `resolve_component` call):

| Dependency | Recorded by | Re-resolves when |
| --- | --- | --- |
| A source file | interface, alias and template lookups, re-export walks, resolved import targets, the consuming file | that file is edited, created or deleted |
| A bare enum or const-array name | `typeof X` and `(typeof X)[number]` lookups through the project-wide bare-name indexes | the changed file declared that name before or after the change |
| Module resolution | `resolve_import_specifier` | a new source file appears (it can redirect or satisfy a specifier) |

## Lazy sessions (ADR 0016)

`WatchSession::lazy(options)` builds `Revision::cold_lazy()`: phases 0 to 3.6 of the cold run (`Indexed::build`, shared with `Revision::cold()`), then nothing. No component is resolved and no dependency is loaded, except the names the project takes from `react` (see below). `WatchSession::component(key)`, `components(keys)` and `component_keys()` are the request API; an eager session answers the same calls from its published output.

**Request.** `component(key)` finds the same-named group (`Revision::lazy.by_name`), decides which of its members a batch run publishes under which key (the first keeps the bare name, later ones `Name (file)`; only an imported-member alias can be left out, so only those are resolved to decide), resolves the target if the memo lacks it, and returns `ComponentAnswer { key, entry, diagnostics }`. The entry is run through `on_component_resolved` as `assemble()` does, and `diagnostics` holds what a batch run attaches to that key, each with `affects == [key]`.

**Demand-driven dependencies.** `Lazy.demands` keeps each project file's `project_demands`. A resolution records the project files it read (`Dependencies::files`); the demands of those files not yet in the layer (`DependencyLayer::missing`) are the names the component reaches. If there are any, `Revision::grow` extends the layer with `dependency_types::load(missing, options, Some(layer))` under a lock, builds a new `Engine` (layer plus `ResolutionContext`) and resolves again. A resolution is memoized only when it found nothing missing, so the memo holds results whose closure was complete. The component's own file's demands are loaded before the first resolution, which saves one in most requests. Each growth rebuilds the `Engine`, which clones the project's `NamedTypeIndex` to attach the layer as its fallback, so a session that grows once per request pays that per new component. After `WHOLE_DEMAND_AFTER_GROWTHS` (3) growths, `Revision::load_whole_demand_in_background` loads every project demand on a thread of its own (the session starts it after an answer; `WatchSession::wait_for_dependency_load` joins it), and later requests find the layer complete.

**Memos across growth.** A `ResolutionContext`'s interface-chain recordings and member reads are computed against its layer. When the layer grows, `ResolutionContext::inherit_memos` copies into the new context the ones whose recorded files, and the file they were keyed in, have all their demands in the old layer; the rest stay behind, because a lookup that missed a name would otherwise answer the next component. File-system answers and shared props always carry over.

**Invalidation.** `Revision::next_lazy` applies the change with `apply_change` (shared with `next()`), keeps every memoized `Resolved` for which `id.file != changed && !affected_by(change)` and still exists, and reports the keys of the touched names as `updated_keys`/`removed_keys`; it resolves nothing and loads nothing. `updated_components` is empty: the consumer asks again. `ensure_fresh()` merges the per-file updates; `rebuilt` starts a new lazy revision and reports every memoized name.

**Bounds.** The memo holds at most one `Resolved` per component, and each revision starts a new one from the entries the edit left valid; a context's memos reset with the revision and are bounded by the project's and its dependencies' declarations. The dependency layer only grows within a session, as in watch mode.

**Concurrency.** Requests run on the caller's thread (`components()` on rayon's pool). The memo is a `DashMap`; the engine is an `ArcSwap`; layer growth takes one `Mutex`, so concurrent requests load a name once and a request that raced a growth re-checks against the new layer.

**Stays batch.** `extract()` and the CLI `extract`, `check` and `inspect` commands resolve everything with `Revision::cold_output`; the Vite plugin's snapshot uses `initialize_session`.

## Key types (verbatim)

```rust
pub struct WatchSession {
    pub options: PipelineOptions,
    current: ArcSwapOption<Revision>,
    writer: Mutex<()>,
}
```

```rust
pub(crate) struct Dependencies {
    pub(crate) files: FxHashSet<Utf8PathBuf>,
    pub(crate) bare_names: FxHashSet<CompactString>,
    pub(crate) resolves_modules: bool,
}
```

```rust
pub fn contain_panic<T>(label: &str, f: impl FnOnce() -> T) -> Result<T, Diagnostic>
```

```rust
pub trait DocgenPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn on_file_extracted(&self, _file_path: &camino::Utf8Path, _data: &mut SourceData) {}
    fn on_component_resolved(&self, _entry: &mut ComponentEntry) {}
}
```

## Invariants

1. **Component output keys are `display_name`, disambiguated to `"{name} ({file_path})"` on the 2nd+ occurrence of that name — this still collides for a genuine 3-way-or-more same-name-same-file duplicate.** Phase 5 tracks a `seen_names: HashMap<String, u32>` counter per base name; the _first_ occurrence gets the bare name, every later occurrence gets the disambiguated key. Since `file_path` is unique per file, two different libraries shipping a same-named `Button.tsx` never collide (each gets a distinct disambiguated key) — but three or more literal duplicate declarations of the same component in the _same file_ (`function Button() {}` repeated) all key identically after the first, and `components.insert` silently overwrites unless its `Option<ComponentEntry>` return is checked. It is checked (see `pipeline/mod.rs:456`), which is what turns this residual case into a `ComponentKeyCollision` diagnostic rather than a silent drop — but the entry itself is still lost from the map, only the _fact_ of the loss is preserved. See `test_duplicate_component_declarations_in_one_file_emit_a_collision_diagnostic`.

2. **`discover_files` must dedup identical canonical paths _before_ they become separate `ComponentMapping`s — the Phase-5 collision check alone cannot catch a 2-directory overlap.** Two overlapping `src_dirs` (e.g. `["./src", "./src/components"]`) walk the same physical file twice. Because the two resulting mappings key _differently_ under Invariant 1's numbering scheme (first occurrence → bare name, second → disambiguated name), they never collide at `insert` and would both silently appear as separate output entries. The actual fix is the `files.sort(); files.dedup();` at the end of `discover_files` (`discover.rs:94-95`), which only works because every path was canonicalized first — an uncanonicalized relative/absolute duplicate pair would sort non-adjacently and evade `dedup()`. See `test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing`.

3. **Every `.d.ts` merge into `GlobalSourceData` — main-loop, Full-mode `@types/react`, and ambient lib.d.ts — must drain the parsed file's own diagnostics before merging, because `GlobalSourceData` itself has no diagnostics field to catch them later.** Phase 3.5/3.6 route through the shared `merge_cached_dts_file` helper specifically to guarantee this; calling `global.merge()` directly on freshly-parsed data (as Phase 3.5/3.6 originally did) silently drops any excessive-nesting or parse-error diagnostic the extractor raised while parsing that file. See `merge_cached_dts_file_does_not_drop_parse_diagnostics`.

4. **Discovery output is deterministic across OS/filesystem: sorted, deduplicated, and canonicalized.** This is load-bearing for two independent reasons: diagnostic ordering and dedup-winner-selection in Phase 5 are reproducible only because rayon's `.collect()` on a `par_iter()` preserves input order and the sequential merge/resolve phases iterate that same order; and `parent.fileName` in resolver output would otherwise differ depending on whether `--src` was given relative or absolute, or what the invocation cwd was.

5. **A watch snapshot equals a cold `extract()` of the same files: components, enums and diagnostics, in the same order.** Stats carry timings and are excluded. Any resolver lookup that reads data outside the component's own mapping must record a dependency in `resolver/deps.rs`, or an edit to that data leaves the component stale. `every_kind_of_change_leaves_the_snapshot_equal_to_a_cold_extract` checks the equality after edits, renames, collisions, excluded files, syntax errors, deletions, moves, bare-name lookups and file creation; disabling any recorder fails it.

6. **A cache-persistence failure (`DtsCache::save_to_disk`) must degrade to `DiagnosticSeverity::Info`, never `Warning`.** `Warning` would flip `docgen check --strict`'s exit code to 1 purely because the _next_ run will have to re-parse `.d.ts` files — extraction output for _this_ run is completely correct either way. This is a deliberate severity choice, not an oversight; the comment at `cache.rs:143-148` explicitly calls out this is the same class of spurious exit-code flip already fixed once elsewhere in the resolver.

7. **Anything reachable from a rayon `.map()` closure or a `DocgenPlugin` hook call must be wrapped in `panic_guard::contain_panic`, individually, per item — not once around the whole batch.** The `.map()` closures at Phase 2 and Phase 4 wrap their _entire bodies_; `PluginRegistry::run_on_file_extracted`/`run_on_component_resolved` wrap _each individual plugin call_, not the surrounding loop, so one misbehaving plugin degrades only itself and every other registered plugin still runs (see `a_panicking_plugin_is_contained_and_tagged_with_its_name_others_still_run`). Violating the per-item granularity (e.g. wrapping the whole `.collect()` instead of each closure invocation) reintroduces the exact failure mode ADR 0005 was written to close: one bad file/plugin/component poisons the whole batch.

8. **`update_file`'s `changed` path must be canonicalized the same way `discover_files` canonicalizes at cold-start.** `discover_files` resolves symlinks (e.g. macOS's `/var` → `/private/var`), so every file key and recorded dependency is in that form. A raw watcher path that skips `canonicalize_best_effort` matches no dependency and silently under-updates.

## Locked design decisions

- **Parallel discovery, parse and resolve; everything else sequential.** Already covered above under "the 6-phase data flow" — restated here because it's the decision most likely to look "obviously improvable" by parallelizing merge/collect too. Don't: both are cheap relative to parse/resolve, and both build one shared mutable structure whose synchronization cost would exceed the sequential cost.
- **The DTS cache keys each entry on `(path, content hash)` and the whole cache on the build that wrote it.** The payload's header (schema version, crate version, source fingerprint from `build.rs`) must match the running build, or the cache is discarded (ADR 0008). `stats.dtsFilesParsed` and `stats.dtsCacheHits` come from the cache's own counters, so the TypeScript lib files merged in Phase 3.6 count too.
- **`contain_panic` uses `AssertUnwindSafe` internally, not at call sites.** Plugin hooks pass `&mut SourceData`/`&mut ComponentEntry`, and `&mut T` is unconditionally `!UnwindSafe` in std — requiring callers to prove unwind-safety themselves would make every call site either not compile or need its own `AssertUnwindSafe`. The justification is structural, not incidental: `SourceData`/`ComponentEntry` have no interior mutability (`Cell`/`RefCell`), so a caught panic mid-operation can't leave an observably-torn half-write behind — the whole operation's output is simply discarded. See ADR 0005 and `panic_guard.rs:14-19`.
- **`ComponentMapping`/`GlobalSourceData` construction happens once per cold run and is reused, immutable, across all of Phase 4's parallel resolve closures via `Arc`.** `ResolutionContext::new(global.clone(), options)` wraps the `Arc<GlobalSourceData>` once outside the `par_iter()`, not per-item — this is the mechanism that makes "all resolver inputs must be owned or `Arc`-wrapped" (crates/core/AGENTS.md) actually cheap rather than merely correct.
- **Plugin hooks are two narrow extension points (`on_file_extracted`, `on_component_resolved`), not a general AST-transform API.** Deliberately modeled after `michi`'s zero-dep trait pattern and `callisto`'s modular design (see `plugin.rs`'s module doc) — the hook surface is intentionally small so panic-containment and ordering guarantees stay easy to reason about per plugin.

## Known gaps

Cross-referencing `docs/edge-cases.md` and `docs/root-cause-analysis.md` for pipeline-specific findings — fixed vs. still open:

**Fixed** (all have regression tests in `pipeline/mod.rs`'s test module):

- Permission-denied subtrees and non-UTF8 filenames silently dropped during discovery — now surfaced as `IoError` diagnostics (`discover.rs`).
- Empty `src_dirs` bypassing the "missing dirs" diagnostic due to a `!is_empty()` boolean-logic bug — now its own distinct diagnostic message.
- Component-key collisions and overlapping-`src_dirs` duplication — see Invariants 1 and 2.
- No panic-containment anywhere in the repo — closed by ADR 0005 / `panic_guard.rs`, covering rayon closures, plugin hooks, and (per the ADR) NAPI entry points.
- Watch mode re-resolved by a string-built reverse import graph that missed barrels, `../` imports and new import edges, kept deleted files' components, ignored excludes and produced different keys and diagnostics from a cold run. Replaced by revisions assembled by the cold path's code and recorded dependencies (Invariant 5).
- Phase 3.5/3.6 dropping parse diagnostics on direct `global.merge()` calls — see Invariant 3.
- DTS cache staleness: mtime+size keys missed same-length edits within one clock tick (replaced by a content hash), and extractor changes without a schema bump served stale entries (fixed by the build fingerprint, ADR 0008).

**Still open** (deliberately deferred, not oversights):

- **A lazy answer depends on every project file's demands, not on a type checker.** A name a component's resolution reads that no project file's `project_demands` mentions (a type used only inside a function body, say) is loaded by a batch run when some other file mentions it, and by a lazy session only if the component's own closure does. `lazy_parity` found two such cases (React's imports, and memos computed before a name loaded) and fixed them; it checks every component of the fixtures and five corpora, in both orders.

- **The dependency layer only grows during a watch session.** A demand an edit removes leaves its declarations loaded; they change no output (a name nothing references is never looked up), and a rebuild (a configuration change below, or `rebuild()`) starts from a fresh layer.

- **Watch mode does not observe `node_modules`.** A change to a tsconfig, jsconfig, `package.json` or lockfile in or above a source directory (`WatchSession::is_resolution_config`) does reach `update_file`, which rebuilds the session from scratch (`Revision::rebuilt`) and returns the full delta; watchers add those files with `resolution_config_files()`. A package edited in place without a lockfile change still needs `rebuild()`.
- **`watch --out` write failures are silently discarded** and writes are non-atomic (`docs/edge-cases.md` line 72-73; `docs/root-cause-analysis.md` line 139). This is CLI-layer (`cli/commands/watch.rs`), outside this subsystem's scope (`crates/core` non-negotiable #6 doesn't reach the CLI), but it directly affects how trustworthy `WatchSession`'s output is once it leaves this module.
