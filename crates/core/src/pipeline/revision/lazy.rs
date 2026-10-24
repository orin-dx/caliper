//! Demand-driven revisions (ADR 0016).
//!
//! A lazy revision parses and indexes every file, as a cold one does, and stops: no dependency is loaded and no
//! component resolved. `Revision::lazy_component` resolves one component when asked, loads only the dependency names
//! its resolution reached, and memoizes the result. The memo follows the same dependency tracking watch mode uses
//! (`Resolved::affected_by`), so an edit drops exactly the entries it can change.

use std::sync::Mutex;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use rustc_hash::FxBuildHasher;

use crate::path_key::PathKey;

use super::*;

/// What a session answers for one key: the component as a batch run publishes it, and the diagnostics that run attaches
/// to that key.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentAnswer {
    pub key: String,
    pub entry: ComponentEntry,
    /// This component's diagnostics only, each with `affects` set to `[key]`.
    pub diagnostics: Vec<Diagnostic>,
}

/// The dependency layer a context was built over, and the context. Replaced whole when the layer grows: the context's
/// memos (interface chains, member reads) were computed against the old layer and must not outlive it.
struct Engine {
    layer: Option<Arc<DependencyLayer>>,
    ctx: Arc<ResolutionContext>,
}

impl Engine {
    fn new(
        global: &Arc<GlobalSourceData>,
        options: &PipelineOptions,
        ambient_global_files: &[Utf8PathBuf],
        named_types: &Arc<NamedTypeIndex>,
        layer: Option<Arc<DependencyLayer>>,
        react_types: &Option<Arc<NamedTypeIndex>>,
    ) -> Engine {
        let named_types = match &layer {
            Some(layer) => {
                let mut with_fallback = (**named_types).clone();
                with_fallback.fallback = Some(layer.named_types.clone());
                Arc::new(with_fallback)
            }
            None => named_types.clone(),
        };
        let ctx = ResolutionContext::new_with_cached_ambient_global_files(
            global.clone(),
            options,
            ambient_global_files.to_vec(),
            named_types,
        )
        .with_dependencies(layer.clone())
        .with_react_types(react_types.clone());
        Engine { layer, ctx: Arc::new(ctx) }
    }
}

pub(super) struct Lazy {
    /// Each project file's dependency demands; files that demand nothing are absent.
    demands: FxHashMap<PathKey, Arc<[Demand]>>,
    /// Components by name, in output order, with their position in `GlobalSourceData::component_mappings`.
    by_name: FxHashMap<String, Vec<(ComponentId, usize)>>,
    memo: DashMap<ComponentId, Arc<Resolved>, FxBuildHasher>,
    engine: ArcSwap<Engine>,
    /// Serializes layer growth, so concurrent requests load a name once.
    growing: Mutex<()>,
}

/// The components of `global` by name, each name's components in output order.
fn group_by_name(global: &GlobalSourceData, options: &PipelineOptions) -> FxHashMap<String, Vec<(ComponentId, usize)>> {
    let mut by_name: FxHashMap<String, Vec<(ComponentId, usize)>> = FxHashMap::default();
    for (id, index) in ordered_ids(global, options) {
        by_name.entry(id.name.clone()).or_default().push((id, index));
    }
    by_name
}

fn demands_by_file(file_demands: Vec<(Utf8PathBuf, Vec<Demand>)>) -> FxHashMap<PathKey, Arc<[Demand]>> {
    file_demands
        .into_iter()
        .filter(|(_, demands)| !demands.is_empty())
        .map(|(file, demands)| (PathKey::from(file.as_path()), Arc::from(demands)))
        .collect()
}

impl Revision {
    /// Discover, parse and merge every source file, and nothing more.
    pub(crate) fn cold_lazy(options: &PipelineOptions, id: u64) -> Revision {
        let Indexed {
            start,
            cache,
            src_files,
            global,
            head_diagnostics,
            file_diagnostics,
            mut lib_diagnostics,
            react_types,
            file_demands,
        } = Indexed::build(options);
        let global = Arc::new(global);
        let named_types = Arc::new(NamedTypeIndex::build(&global));
        let ambient_global_files = crate::resolver::compute_ambient_global_files(options);
        // The layer starts with the names the project takes from React. A batch run loads them for every component,
        // and loading them follows React's own imports (csstype), which any component's HTML attributes can reach
        // without its file naming them; a component's answer would otherwise depend on what was asked before it.
        // It also reads the persisted dependency index, so names a batch run already resolved cost no walk.
        let react: Vec<Demand> = file_demands
            .iter()
            .flat_map(|(_, demands)| demands.iter().filter(|demand| &*demand.specifier == "react").cloned())
            .collect();
        let layer = options.load_dependencies.then(|| Arc::new(dependency_types::load(react, options, None)));
        let engine = Engine::new(&global, options, &ambient_global_files, &named_types, layer, &react_types);
        lib_diagnostics.extend(engine.ctx.tsconfig_diagnostic.clone());
        let (dts_files_parsed, dts_cache_hits) = cache.counts();
        let tail_diagnostics: Vec<Diagnostic> = match Arc::try_unwrap(cache) {
            Ok(c) => c.save_to_disk(),
            Err(arc) => arc.save_to_disk(),
        }
        .into_iter()
        .collect();
        let lazy = Lazy {
            demands: demands_by_file(file_demands),
            by_name: group_by_name(&global, options),
            memo: DashMap::default(),
            engine: ArcSwap::from_pointee(engine),
            growing: Mutex::new(()),
        };
        Revision {
            id,
            global,
            named_types,
            dependencies: None,
            react_types,
            ambient_global_files,
            head_diagnostics,
            file_diagnostics,
            lib_diagnostics,
            tail_diagnostics,
            resolved: FxHashMap::default(),
            files: Arc::new(src_files.into_iter().collect()),
            dts_files_parsed,
            dts_cache_hits,
            duration_ms: start.elapsed().as_millis() as u64,
            output: Arc::new(empty_output()),
            keys: FxHashMap::default(),
            lazy: Some(lazy),
        }
    }

    pub(crate) fn is_lazy(&self) -> bool {
        self.lazy.is_some()
    }

    /// The component an eager revision published under `key`, as a lazy revision answers for it.
    pub(crate) fn published_component(&self, key: &str) -> Option<ComponentAnswer> {
        let entry = self.output.components.get(key)?.clone();
        let diagnostics = self
            .output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.affects.iter().any(|affected| affected == key))
            .map(|diagnostic| Diagnostic { affects: vec![key.to_owned()], ..diagnostic.clone() })
            .collect();
        Some(ComponentAnswer { key: key.to_owned(), entry, diagnostics })
    }

    /// How many components this lazy revision indexed, how many it has resolved, and how many dependency declaration
    /// files it has loaded.
    pub(crate) fn lazy_counts(&self) -> Option<(usize, usize, usize)> {
        let lazy = self.lazy.as_ref()?;
        let loaded = lazy.engine.load().layer.as_ref().map_or(0, |layer| layer.file_count());
        Some((lazy.by_name.values().map(Vec::len).sum(), lazy.memo.len(), loaded))
    }

    /// The revision after `changed` was edited, created or deleted, and the keys whose answers may differ. The memo
    /// keeps every entry whose resolution did not read `changed`; the dependency layer carries over as it stands.
    pub(super) fn next_lazy(
        &self,
        options: &PipelineOptions,
        changed: &Utf8Path,
    ) -> Option<(Revision, IncrementalUpdate)> {
        let previous = self.lazy.as_ref()?;
        if !is_source_file(changed, &options.src_dirs, &options.exclude_patterns) {
            return None;
        }
        let start = Instant::now();
        let Applied { global, named_types, file_diagnostics, files, change, demands, .. } =
            self.apply_change(options, changed, false);
        let global = Arc::new(global);
        let named_types = Arc::new(named_types);

        let mut file_demands = previous.demands.clone();
        match demands.is_empty() {
            true => file_demands.remove(change.file.as_str()),
            false => file_demands.insert(PathKey::from(change.file), Arc::from(demands)),
        };
        let by_name = group_by_name(&global, options);
        let memo: DashMap<ComponentId, Arc<Resolved>, FxBuildHasher> = DashMap::default();
        let mut touched: FxHashSet<String> = FxHashSet::default();
        let mut affected_files: Vec<Utf8PathBuf> = Vec::new();
        for entry in previous.memo.iter() {
            let (id, resolved) = (entry.key(), entry.value());
            let kept = id.file != change.file
                && !resolved.affected_by(&change)
                && by_name.get(&id.name).is_some_and(|group| group.iter().any(|(candidate, _)| candidate == id));
            if kept {
                memo.insert(id.clone(), resolved.clone());
            } else {
                touched.insert(id.name.clone());
                affected_files.push(id.file.clone());
            }
        }
        // A name the changed file declared before or after keeps no memo, but its keys can still move.
        for groups in [&previous.by_name, &by_name] {
            for (name, group) in groups {
                if group.iter().any(|(id, _)| id.file == change.file) {
                    touched.insert(name.clone());
                }
            }
        }
        affected_files.sort();
        affected_files.dedup();

        let layer = previous.engine.load().layer.clone();
        let engine = Engine::new(&global, options, &self.ambient_global_files, &named_types, layer, &self.react_types);
        let next = Revision {
            id: self.id + 1,
            global,
            named_types,
            dependencies: None,
            react_types: self.react_types.clone(),
            ambient_global_files: self.ambient_global_files.clone(),
            head_diagnostics: self.head_diagnostics.clone(),
            file_diagnostics,
            lib_diagnostics: self.lib_diagnostics.clone(),
            tail_diagnostics: self.tail_diagnostics.clone(),
            resolved: FxHashMap::default(),
            files: Arc::new(files),
            dts_files_parsed: self.dts_files_parsed,
            dts_cache_hits: self.dts_cache_hits,
            duration_ms: self.duration_ms,
            output: Arc::new(empty_output()),
            keys: FxHashMap::default(),
            lazy: Some(Lazy {
                demands: file_demands,
                by_name,
                memo,
                engine: ArcSwap::from_pointee(engine),
                growing: Mutex::new(()),
            }),
        };
        let update = self.lazy_delta(&next, options, &touched, affected_files, start);
        Some((next, update))
    }

    /// The revision after a change that can alter how every import resolves, rebuilt from scratch. Every name the
    /// previous revision had resolved is reported, since any answer may differ.
    pub(super) fn rebuilt_lazy(&self, options: &PipelineOptions) -> Option<(Revision, IncrementalUpdate)> {
        let previous = self.lazy.as_ref()?;
        let start = Instant::now();
        let next = Revision::cold_lazy(options, self.id + 1);
        let touched: FxHashSet<String> = previous.memo.iter().map(|entry| entry.key().name.clone()).collect();
        let mut affected_files: Vec<Utf8PathBuf> = previous.memo.iter().map(|entry| entry.key().file.clone()).collect();
        affected_files.sort();
        affected_files.dedup();
        let update = self.lazy_delta(&next, options, &touched, affected_files, start);
        Some((next, update))
    }

    /// The delta from this lazy revision to `next`: the keys of every `touched` name there, and the keys it had that
    /// `next` does not. Answers are not part of it; a consumer asks again.
    fn lazy_delta(
        &self,
        next: &Revision,
        options: &PipelineOptions,
        touched: &FxHashSet<String>,
        affected_files: Vec<Utf8PathBuf>,
        start: Instant,
    ) -> IncrementalUpdate {
        let mut updated_keys: Vec<String> = Vec::new();
        let mut removed_keys: Vec<String> = Vec::new();
        for name in touched {
            let now = next.group_keys(options, name, true);
            removed_keys.extend(self.group_keys(options, name, true).into_iter().filter(|key| !now.contains(key)));
            updated_keys.extend(now);
        }
        updated_keys.sort();
        updated_keys.dedup();
        removed_keys.sort();
        removed_keys.dedup();
        IncrementalUpdate {
            revision: next.id,
            updated_components: Vec::new(),
            updated_keys,
            removed_keys,
            affected_files,
            diagnostics: Vec::new(),
            removed_diagnostics: Vec::new(),
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// `component` resolved, memoized. `None` when the position is not one of this revision's mappings.
    fn lazy_resolved(&self, options: &PipelineOptions, id: &ComponentId, index: usize) -> Option<Arc<Resolved>> {
        let lazy = self.lazy.as_ref()?;
        if let Some(hit) = lazy.memo.get(id) {
            return Some(hit.clone());
        }
        let mapping = self.global.component_mappings.get(index)?;
        // The component's own file almost always reaches its own names; loading them first saves a resolution.
        if let Some(seed) = lazy.demands.get(mapping.file_path.as_str()) {
            self.grow(options, lazy, seed);
        }
        loop {
            let engine = lazy.engine.load_full();
            let (entry, diagnostics, depends_on) = resolve_guarded(&[mapping], &engine.ctx).into_iter().next()?;
            let missing = match &engine.layer {
                Some(layer) => {
                    let mut missing = Vec::new();
                    for file in &depends_on.files {
                        if let Some(demands) = lazy.demands.get(file) {
                            missing.extend(layer.missing(demands));
                        }
                    }
                    missing
                }
                None => Vec::new(),
            };
            // A resolution against a layer that lacked a name it reached is discarded, and so is its context's memo.
            if missing.is_empty() {
                let resolved = Arc::new(Resolved { entry, diagnostics, depends_on });
                return Some(lazy.memo.entry(id.clone()).or_insert(resolved).clone());
            }
            self.grow(options, lazy, &missing);
        }
    }

    /// Load `demands` into the dependency layer unless a concurrent request already did.
    fn grow(&self, options: &PipelineOptions, lazy: &Lazy, demands: &[Demand]) {
        let _growing = lazy.growing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let engine = lazy.engine.load_full();
        let Some(layer) = &engine.layer else { return };
        let missing = layer.missing(demands);
        if missing.is_empty() {
            return;
        }
        let extended = Arc::new(dependency_types::load(missing, options, Some(layer)));
        // Names that were already loaded, or that no package declares, add nothing the resolver reads, so the
        // context and its memos stay; only the record of what was asked for moves on.
        if extended.resolves_like(layer) {
            lazy.engine.store(Arc::new(Engine { layer: Some(extended), ctx: engine.ctx.clone() }));
            return;
        }
        let next = Engine::new(
            &self.global,
            options,
            &self.ambient_global_files,
            &self.named_types,
            Some(extended),
            &self.react_types,
        );
        // A memo that reached a project file whose names the old layer lacked was computed without them, and stays
        // behind; the rest hold for the larger layer, so the next component does not rebuild them.
        let covered = |file: &str| lazy.demands.get(file).is_none_or(|demands| layer.covers(demands));
        next.ctx.inherit_memos(&engine.ctx, |deps, read_in| {
            covered(read_in) && deps.files.iter().all(|file| covered(std::borrow::Borrow::<str>::borrow(file)))
        });
        lazy.engine.store(Arc::new(next));
    }

    /// The keys a batch run publishes the components named `name` under, with each key's position in the group.
    /// Only an imported-member alias can be left out (when its package isn't loaded it has nothing to show), so only
    /// those are resolved to decide. With `listing`, a session that loads dependencies assumes they are all published
    /// instead: deciding would load the package of every alias in the project.
    fn group_positions(&self, options: &PipelineOptions, name: &str, listing: bool) -> Vec<(String, usize)> {
        let Some(group) = self.lazy.as_ref().and_then(|lazy| lazy.by_name.get(name)) else { return Vec::new() };
        let mut keyed: Vec<(String, usize)> = Vec::with_capacity(group.len());
        let mut published = 0;
        for (position, (id, index)) in group.iter().enumerate() {
            let Some(mapping) = self.global.component_mappings.get(*index) else { continue };
            let assumed = listing && options.load_dependencies;
            if mapping.rule == DetectionRule::ImportedMemberAlias && !assumed {
                let empty = self
                    .lazy_resolved(options, id, *index)
                    .is_some_and(|resolved| resolved.entry.props.is_empty() && resolved.entry.inheritance.is_empty());
                if empty {
                    continue;
                }
            }
            published += 1;
            // The first occurrence of a name keeps it bare; later ones take their file path, which is unique per file.
            let key = if published == 1 { name.to_owned() } else { format!("{name} ({})", mapping.file_path) };
            // Two components with one key: the later one is kept, as in the output map.
            match keyed.iter_mut().find(|(existing, _)| *existing == key) {
                Some(slot) => slot.1 = position,
                None => keyed.push((key, position)),
            }
        }
        keyed
    }

    fn group_keys(&self, options: &PipelineOptions, name: &str, listing: bool) -> Vec<String> {
        self.group_positions(options, name, listing).into_iter().map(|(key, _)| key).collect()
    }

    /// Every key a batch run would publish, sorted. Without `load_dependencies` it resolves the imported-member
    /// aliases to decide which are published; with it, it lists them all and never loads a dependency, so the list
    /// can include an alias whose package has nothing to show, which `lazy_component` then answers `None` for.
    pub(crate) fn lazy_keys(&self, options: &PipelineOptions) -> Vec<String> {
        let Some(lazy) = &self.lazy else { return Vec::new() };
        let mut keys: Vec<String> = lazy.by_name.keys().flat_map(|name| self.group_keys(options, name, true)).collect();
        keys.sort();
        keys
    }

    /// [`Self::lazy_component`] for each of `keys`. The dependency names of every requested component's own file are
    /// loaded once up front and the first component is resolved alone, which warms the memos the rest share; resolving
    /// them all at once instead makes every thread rebuild the same chains and restart on each layer growth.
    pub(crate) fn lazy_components(&self, options: &PipelineOptions, keys: &[String]) -> Vec<Option<ComponentAnswer>> {
        use rayon::prelude::*;
        let Some(lazy) = &self.lazy else { return vec![None; keys.len()] };
        let mut seed: Vec<Demand> = Vec::new();
        let mut seeded: FxHashSet<&Utf8Path> = FxHashSet::default();
        for key in keys {
            let Some((id, index)) = self.lazy_target(options, key) else { continue };
            let Some(mapping) = self.global.component_mappings.get(index) else { continue };
            if let Some(demands) = lazy.demands.get(mapping.file_path.as_str()).filter(|_| seeded.insert(&id.file)) {
                seed.extend(demands.iter().cloned());
            }
        }
        if !seed.is_empty() {
            self.grow(options, lazy, &seed);
        }
        let mut answers: Vec<Option<ComponentAnswer>> = vec![None; keys.len()];
        let Some((first, rest)) = keys.split_first() else { return answers };
        let (head, tail) = answers.split_at_mut(1);
        if let Some(slot) = head.first_mut() {
            *slot = self.lazy_component(options, first);
        }
        tail.par_iter_mut().zip(rest.par_iter()).for_each(|(slot, key)| *slot = self.lazy_component(options, key));
        answers
    }

    /// The key a batch run publishes the component `name` declared in `file` under. `None` when `file` declares no
    /// such component, or a batch run would leave it out.
    pub(crate) fn lazy_key_in(&self, options: &PipelineOptions, file: &Utf8Path, name: &str) -> Option<String> {
        let group = self.lazy.as_ref()?.by_name.get(name)?;
        self.group_positions(options, name, false)
            .into_iter()
            .rev()
            .find(|(_, position)| group.get(*position).is_some_and(|(id, _)| id.file == file))
            .map(|(key, _)| key)
    }

    /// The component a batch run publishes under `key`, and its place in the revision: the id and the position of
    /// its mapping. `None` for a key with no component.
    fn lazy_target(&self, options: &PipelineOptions, key: &str) -> Option<(&ComponentId, usize)> {
        let lazy = self.lazy.as_ref()?;
        // `key` is a bare name, or `Name (file)`; a name may itself contain " (" only in a path, never a component.
        let names = std::iter::once(key).chain(key.match_indices(" (").map(|(at, _)| &key[..at]));
        let (name, position) = names.into_iter().find_map(|name| {
            let position =
                self.group_positions(options, name, false).into_iter().find(|(candidate, _)| candidate == key)?.1;
            Some((name, position))
        })?;
        let (id, index) = lazy.by_name.get(name)?.get(position)?;
        Some((id, *index))
    }

    /// The component a batch run publishes under `key`, resolved now (or answered from the memo), with its
    /// diagnostics. `None` for a key the project has no component under.
    pub(crate) fn lazy_component(&self, options: &PipelineOptions, key: &str) -> Option<ComponentAnswer> {
        let (id, index) = self.lazy_target(options, key)?;
        let resolved = self.lazy_resolved(options, id, index)?;
        let mut entry = resolved.entry.clone();
        let mut diagnostics = DiagnosticSet::default();
        diagnostics.extend(options.plugins.run_on_component_resolved(&mut entry), Some(key));
        diagnostics.extend(resolved.diagnostics.own.iter().cloned(), Some(key));
        for chunk in &resolved.diagnostics.shared {
            diagnostics.extend_shared(chunk, key);
        }
        Some(ComponentAnswer { key: key.to_owned(), entry, diagnostics: diagnostics.finish() })
    }
}
