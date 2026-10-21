use std::sync::{Arc, Mutex};
use std::time::Instant;

use arc_swap::ArcSwap;
use camino::{Utf8Path, Utf8PathBuf};
use dashmap::DashMap;
use rayon::prelude::*;

use crate::resolver::{resolve_component, ResolutionContext};
use crate::types::*;

use super::super::types::{ComponentMapping, GlobalSourceData};
use super::ReverseDeps;
use super::{IncrementalUpdate, PipelineOptions};

/// Canonicalize `path`, falling back to canonicalizing its parent directory
/// and rejoining the file name when the full path can't be resolved (e.g. the
/// file doesn't exist yet — created and then immediately edited before this
/// session ever saw it read successfully). Falls back to `path` unchanged only
/// when even the parent doesn't exist. Deliberately stable across a file's
/// create/delete transitions: the parent directory is what actually needs a
/// consistent identity across repeated `update_file` calls for the same file,
/// not the file itself.
fn canonicalize_best_effort(path: &Utf8Path) -> Utf8PathBuf {
    if let Ok(p) = std::fs::canonicalize(path.as_std_path()) {
        if let Ok(p) = Utf8PathBuf::from_path_buf(p) {
            return p;
        }
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(p) = std::fs::canonicalize(parent.as_std_path()) {
            if let Ok(mut p) = Utf8PathBuf::from_path_buf(p) {
                p.push(file_name);
                return p;
            }
        }
    }
    path.to_owned()
}

/// Stateful session for incremental watch-mode extraction.
///
/// `global`, `reverse_deps`, and `component_cache` are updated atomically /
/// concurrently on each file change. `reverse_deps` is rebuilt in full on
/// structural changes (new file, deleted file) via a new `WatchSession` — it
/// does not track incremental import-graph edits within a session.
pub struct WatchSession {
    pub options: PipelineOptions,
    /// Current merged source data — swapped atomically via `ArcSwap`.
    pub global: ArcSwap<GlobalSourceData>,
    /// Reverse-dependency graph built at initialisation time — swapped
    /// atomically via `ArcSwap` so `initialize()` can populate it from the
    /// first fully-merged `global` (it doesn't exist yet at `new()`).
    pub reverse_deps: ArcSwap<ReverseDeps>,
    /// Per-file SourceData cache (avoids re-reading files that haven't changed).
    pub source_cache: DashMap<Utf8PathBuf, SourceData>,
    /// Latest resolved component entries, keyed by display name.
    pub component_cache: DashMap<String, ComponentEntry>,
    /// Latest diagnostics per file, replaced (not appended) on each re-update so a
    /// fixed file's stale diagnostics don't linger — mirrors GlobalSourceData::remove_file's
    /// per-file replacement semantics.
    pub diagnostics: DashMap<Utf8PathBuf, Vec<Diagnostic>>,
    /// TypeScript's own lib.d.ts paths, computed once here at `initialize()`
    /// rather than by every `ResolutionContext::new` inside `update_file` —
    /// `options.src_dirs` (the only input this depends on) never changes
    /// within a session, so re-walking the filesystem for it on every single
    /// file save would be pure waste.
    ambient_global_files: std::sync::OnceLock<Vec<Utf8PathBuf>>,
    /// Guards initialize() so concurrent callers don't race to build caches.
    initialized: Mutex<bool>,
    /// Fast `(file, name)` interface/type-alias index — mirrors `self.global`'s
    /// incremental per-file patching (`remove_file` + merge on each edit)
    /// rather than being rebuilt from the whole project on every
    /// `update_file` call. `Mutex<Arc<_>>` (not `ArcSwap`, unlike `global`):
    /// `update_file` mutates it in place via `Arc::make_mut` when nothing else
    /// still holds a reference (the common case — no full clone needed), and
    /// falls back to cloning only if a concurrent call is still using the
    /// previous snapshot. `ArcSwap::rcu` can't express "mutate in place when
    /// possible" — its closure always produces a brand new value.
    named_types: Mutex<Arc<crate::named_type_index::NamedTypeIndex>>,
}

impl WatchSession {
    /// Create an empty WatchSession (call `initialize()` before first use).
    pub fn new(options: PipelineOptions) -> Self {
        Self {
            options,
            global: ArcSwap::new(Arc::new(GlobalSourceData::default())),
            reverse_deps: ArcSwap::new(Arc::new(ReverseDeps { inner: Default::default() })),
            source_cache: DashMap::new(),
            component_cache: DashMap::new(),
            diagnostics: DashMap::new(),
            ambient_global_files: std::sync::OnceLock::new(),
            initialized: Mutex::new(false),
            named_types: Mutex::new(Arc::new(crate::named_type_index::NamedTypeIndex::default())),
        }
    }

    /// Run a full cold extraction to populate this session's caches.
    ///
    /// Idempotent — concurrent or repeated calls return the existing snapshot
    /// without re-running extraction.
    pub fn initialize(&self) -> ExtractionOutput {
        let mut guard = self.initialized.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if *guard {
            return self.snapshot();
        }

        // Single real pipeline run (discover, parse, merge, Full-mode
        // @types/react, ambient lib.d.ts, resolve) shared with the one-shot
        // extract() path — see extract_with_global's doc comment. Previously
        // this rebuilt GlobalSourceData by hand here, which skipped the
        // Full-mode/ambient-lib.d.ts merges entirely: the cold `output` below
        // was correct (it came from a real extract() call), but the `global`
        // this session persists for every subsequent update_file() was not.
        let (output, global, per_file_data) = super::extract_with_global(&self.options, true);

        for (path, data) in per_file_data {
            self.source_cache.insert(path, data);
        }

        let _ = self.ambient_global_files.set(crate::resolver::compute_ambient_global_files(&self.options));
        self.reverse_deps.store(Arc::new(ReverseDeps::build(&global)));
        *self.named_types.lock().unwrap_or_else(|p| p.into_inner()) =
            Arc::new(crate::named_type_index::NamedTypeIndex::build(&global));
        self.global.store(global);

        // Seed component cache from the cold output.
        for (name, entry) in &output.components {
            self.component_cache.insert(name.clone(), entry.clone());
        }

        *guard = true;
        output
    }

    /// Return the current full extraction state from the in-memory caches.
    /// Suitable for writing to `--out` after each incremental update.
    pub fn snapshot(&self) -> ExtractionOutput {
        let components: std::collections::BTreeMap<String, ComponentEntry> =
            self.component_cache.iter().map(|r| (r.key().clone(), r.value().clone())).collect();
        let global = self.global.load();
        let enums: std::collections::BTreeMap<String, Vec<EnumEntry>> =
            global.enums.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        // DashMap iteration order is arbitrary — sort by path for deterministic output.
        let mut by_path: Vec<(Utf8PathBuf, Vec<Diagnostic>)> =
            self.diagnostics.iter().map(|r| (r.key().clone(), r.value().clone())).collect();
        by_path.sort_by(|a, b| a.0.cmp(&b.0));
        let diagnostics: Vec<Diagnostic> = by_path.into_iter().flat_map(|(_, ds)| ds).collect();

        // Only `components_extracted` is populated here — the other ExtractionStats
        // fields (files_parsed, dts_cache_hits, duration_ms, tier1/tier3/opaque
        // counts) describe a single extract() run's own bookkeeping, which this
        // session doesn't accumulate across incremental update_file() calls. A
        // consumer reading watch --out's stats block for anything but the
        // component count sees zeros — a known, narrower gap than reporting
        // componentsExtracted itself as 0 regardless of how many components exist.
        let stats = ExtractionStats { components_extracted: components.len() as u32, ..ExtractionStats::default() };

        ExtractionOutput { components, enums, diagnostics, stats }
    }

    /// Handle a single file change — re-resolve only affected components.
    pub fn update_file(&self, changed: &Utf8Path) -> IncrementalUpdate {
        let start = Instant::now();

        // `discover_files`'s directory walk (via the `ignore` crate) resolves
        // symlinks in the paths it reports — e.g. macOS's `/var` -> `/private/var`
        // — so every path already stored in `global`/`reverse_deps` is in that
        // resolved form. A file-watcher-reported `changed` path is not
        // guaranteed to match unless it's canonicalized the same way; without
        // this, `reverse_deps.affected()` below would look up the wrong key and
        // silently find no dependents at all.
        let changed = canonicalize_best_effort(changed);
        let changed = changed.as_path();

        // 1. Re-parse the changed file.
        let mut io_diagnostic: Option<Diagnostic> = None;
        let source = match std::fs::read_to_string(changed) {
            Ok(s) => s,
            Err(e) => {
                io_diagnostic = Some(Diagnostic::io_read_error(changed, &e));
                String::new()
            }
        };
        let mut new_data = crate::extractor::parse_file(changed, &source);
        // Drained here as in the cold path's merge, so both paths cache and merge the same `SourceData`.
        let parse_diagnostics = std::mem::take(&mut new_data.diagnostics);
        self.source_cache.insert(changed.to_owned(), new_data.clone());

        // 2. Patch GlobalSourceData atomically using rcu (read-copy-update).
        // rcu retries if the pointer was swapped by a concurrent update_file call,
        // preventing lost-update races under parallel file change events.
        // rcu's return value is the PRE-swap value (arc-swap's compare_and_swap
        // convention), not the value the closure just produced — the closure's
        // result must be re-read via load_full() to see the post-update state.
        self.global.rcu(|old| {
            let mut g = (**old).clone();
            g.remove_file(changed);
            g.merge(changed, new_data.clone());
            g
        });
        let new_global = self.global.load_full();

        // Keep the named-type index in sync with `global`'s own unconditional
        // per-file patch above (same remove-then-merge shape, same lack of a
        // read-failure gate — this index derives from `global`, not from
        // `component_cache`'s separately-decided AC-019 persistence policy).
        // `Arc::make_mut` mutates in place when this call holds the only
        // reference (the common case), and clones only if a concurrent
        // `update_file` call is still using the previous snapshot.
        let named_types = {
            let mut guard = self.named_types.lock().unwrap_or_else(|p| p.into_inner());
            let index = Arc::make_mut(&mut guard);
            index.remove_file(changed);
            index.merge_file(&new_data);
            guard.clone()
        };

        // 3. Find all transitively affected files via the reverse dep graph.
        let affected = self.reverse_deps.load().affected(changed);

        // 4. Re-resolve affected components.
        // Borrowed, not cloned — same reasoning as the one-shot `extract()`
        // path's identical fix (pipeline/mod.rs): `resolve_component` already
        // takes `&ComponentMapping`, and `.par_iter()` below only ever
        // produces borrows anyway.
        let affected_mappings: Vec<&ComponentMapping> =
            new_global.component_mappings.iter().filter(|m| affected.contains(&m.file_path)).collect();

        let ambient_global_files = self.ambient_global_files.get().cloned().unwrap_or_default();
        let ctx = ResolutionContext::new_with_cached_ambient_global_files(
            new_global.clone(),
            &self.options,
            ambient_global_files,
            named_types,
        );
        // Wrapped in `contain_panic` for the same reason as the one-shot `extract()`
        // path (pipeline/mod.rs) — one component's resolution panicking must degrade
        // to a per-component diagnostic instead of poisoning this `.collect()` and
        // losing every other affected component's re-resolution for this file change.
        let results: Vec<(ComponentEntry, Vec<Diagnostic>)> = affected_mappings
            .par_iter()
            .copied()
            .map(|m| {
                let label = format!("resolve:{}", m.component_name);
                crate::panic_guard::contain_panic(&label, || {
                    #[cfg(test)]
                    if m.component_name == super::RESOLVE_PANIC_TEST_SENTINEL {
                        panic!("simulated resolve-phase panic (test-only sentinel)");
                    }

                    resolve_component(m, &ctx)
                })
                .unwrap_or_else(|diag| {
                    let stub = ComponentEntry {
                        display_name: m.component_name.clone(),
                        file_path: m.file_path.clone(),
                        description: String::new(),
                        props: Default::default(),
                        inheritance: vec![],
                        notable_inherited: Default::default(),
                        discriminant_prop: None,
                        composes: vec![],
                        tags: Default::default(),
                        methods: vec![],
                    };
                    (stub, vec![diag])
                })
            })
            .collect();

        // A failed read (deletion, permission error, non-UTF8 content) must
        // NOT evict this file's cache entries — SPEC-PIPELINE-001's AC-019
        // deliberately specifies that a deleted file's previously-resolved
        // component stays in component_cache/snapshot() until a new
        // WatchSession is constructed, a documented (not accidental)
        // staleness gap distinct from the rename case below. Only a
        // successful re-read, which can genuinely produce a different set of
        // component names for this file, should evict what it no longer owns.
        let file_read_failed = io_diagnostic.is_some();

        let mut updated_components = Vec::new();
        let mut diagnostics: Vec<Diagnostic> = io_diagnostic.into_iter().chain(parse_diagnostics).collect();

        // Evict every existing component_cache entry that belonged to this
        // file — by whatever key it was stored under, bare or disambiguated —
        // before inserting its freshly-resolved entries. A bare `insert(
        // entry.display_name, ...)` here left two kinds of stale entries
        // behind forever: a renamed component's old-name key never got
        // removed, and a disambiguated key from initialize()'s cold Phase 5
        // pass was never touched by an update that only ever wrote the bare
        // key. Both traced to the same root cause — no eviction step existed
        // at all. This resolves the file's keys as a batch first so a
        // same-file rename doesn't race against its own insert below.
        if !file_read_failed {
            let stale_keys: Vec<String> = self
                .component_cache
                .iter()
                .filter(|r| r.value().file_path == changed)
                .map(|r| r.key().clone())
                .collect();
            for key in stale_keys {
                self.component_cache.remove(&key);
            }
        }

        for (entry, diags) in results {
            // Disambiguate against whatever's left in the cache after this
            // file's own stale keys were just evicted above — mirrors Phase
            // 5's bare-then-disambiguated rule (extract()'s one-shot path),
            // checked incrementally instead of via a session-wide counter.
            // Deliberately narrower than Phase 5 in one respect: if a rename
            // elsewhere frees up a bare key another file's component was
            // disambiguated against earlier, that other entry stays under
            // its disambiguated key until its own file is next touched —
            // still correctly retrievable, just not repacked to the
            // now-available bare key. Re-optimizing every other cached
            // entry's key on every single-file edit would defeat the point
            // of incremental updates; only this file's own stale keys are
            // this method's responsibility.
            let key = if self.component_cache.iter().any(|r| r.value().display_name == entry.display_name) {
                format!("{} ({})", entry.display_name, entry.file_path)
            } else {
                entry.display_name.clone()
            };
            self.component_cache.insert(key, entry.clone());
            updated_components.push(entry);
            diagnostics.extend(diags);
        }

        // Replace (not append) this file's diagnostics — a fixed file's prior
        // errors shouldn't linger in subsequent snapshots.
        if diagnostics.is_empty() {
            self.diagnostics.remove(changed);
        } else {
            self.diagnostics.insert(changed.to_owned(), diagnostics.clone());
        }

        IncrementalUpdate {
            updated_components,
            affected_files: affected.into_iter().collect(),
            diagnostics,
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests;
