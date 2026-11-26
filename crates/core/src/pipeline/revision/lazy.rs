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

/// Runs `work`, which may use rayon, on a thread and a pool of its own, and waits for it.
///
/// `grow` holds a lock while it loads dependencies, and the load fans out with `rayon::scope`. Called on a pool worker,
/// that wait steals other queued jobs, and a stolen request calls `grow` and blocks on the lock its own thread holds;
/// the other workers block on it too, so the scope's tasks have no worker left to run on. A plain thread blocks
/// without stealing, and its own pool keeps workers free for the load.
fn off_pool<R: Send>(work: impl FnOnce() -> R + Send) -> R {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    let pool = POOL
        .get_or_init(|| {
            rayon::ThreadPoolBuilder::new().thread_name(|index| format!("caliper-deps-{index}")).build().ok()
        })
        .as_ref();
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(|| match pool {
                Some(pool) => pool.install(work),
                None => work(),
            })
            .join()
    });
    outcome.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

#[cfg(test)]
mod off_pool_tests {
    use super::off_pool;

    fn thread_name() -> String {
        std::thread::current().name().unwrap_or_default().to_owned()
    }

    /// The deadlock needs the thread that waits for the loader's scope to steal an unrelated request, which only a
    /// worker of the caller's pool does. So the guarantee is where the work runs, scoped tasks included.
    #[test]
    fn scoped_work_runs_on_the_loader_pool_not_the_callers() {
        let caller = rayon::ThreadPoolBuilder::new().num_threads(2).build().expect("a small pool");
        let (own, scoped) = caller.install(|| {
            assert!(!thread_name().starts_with("caliper-deps-"), "the test starts on the caller's pool");
            off_pool(|| {
                let mut names = [String::new(), String::new(), String::new()];
                rayon::scope(|scope| {
                    for name in &mut names {
                        scope.spawn(move |_| *name = thread_name());
                    }
                });
                (thread_name(), names)
            })
        });
        assert!(own.starts_with("caliper-deps-"), "the work runs on a loader thread, not on {own}");
        assert!(scoped.iter().all(|name| name.starts_with("caliper-deps-")), "scoped tasks ran on {scoped:?}");
    }

    #[test]
    fn a_panic_in_the_work_reaches_the_caller() {
        let outcome = std::panic::catch_unwind(|| off_pool(|| -> usize { panic!("the loader failed") }));
        assert!(outcome.is_err());
    }
}

/// What a session answers for one key: the component as a batch run publishes it, and the diagnostics that run attaches
/// to that key.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentAnswer {
    /// The component's output key: its bare name, or `Name (file)` when two components share the name.
    pub key: String,
    /// The component's entry, the same one a batch run publishes under `key`.
    pub entry: ComponentEntry,
    /// This component's diagnostics only, each with `affects` set to `[key]`.
    pub diagnostics: Vec<Diagnostic>,
}

/// Where a component is declared, for a host that finds one by file and name before it asks for the props.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentLocation {
    /// The component's output key.
    pub key: String,
    /// The file that declares the component.
    pub file_path: Utf8PathBuf,
    /// The component's name as the extractor found it.
    pub display_name: String,
    /// How its module exports it ([`ComponentEntry::exports`]): what a host that finds a component by file and export
    /// reads.
    pub exports: Vec<ComponentExport>,
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
            Some(layer) => Arc::new(named_types.with_fallback(layer.served_types())),
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

/// How many times the layer grows for requests before the rest of the project's names load in the background. A
/// growth is cheap (it shares the project's indexes with the context it replaces), so a session that looks at a few
/// dozen components holds only the names they reach (shadcn: 181 declaration files after 50 requests, 450 once every
/// name is loaded, 30 MB more); one that reads across the project, as a static build does, reaches this and loads the
/// rest once instead of growing for every new component. Tests use a small project, so a small count.
const WHOLE_DEMAND_AFTER_GROWTHS: usize = 3;

pub(super) struct Lazy {
    /// Each project file's dependency demands; files that demand nothing are absent.
    demands: FxHashMap<PathKey, Arc<[Demand]>>,
    /// The demands the session's first load follows for every component ([`shared_demands`]): React's names and the
    /// modules the project imports, whose augmentations any component reading React's interfaces reads.
    shared: Arc<[Demand]>,
    /// The unread walks `shared` leads to in one layer, and that layer: asked once per layer, not once per resolution
    /// that reads React's augmentations.
    shared_unread: Mutex<Option<SharedUnread>>,
    /// Components by name, in output order, with their position in `GlobalSourceData::component_mappings`.
    by_name: FxHashMap<String, Vec<(ComponentId, usize)>>,
    memo: DashMap<ComponentId, Arc<Resolved>, FxBuildHasher>,
    engine: ArcSwap<Engine>,
    /// Serializes layer growth, so concurrent requests load a name once.
    growing: Mutex<()>,
    /// Whether the background load of every name the project takes from a package has started.
    whole_demand_started: std::sync::atomic::AtomicBool,
    /// The thread loading them, until [`Revision::wait_for_dependency_load`] joins it.
    whole_demand_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// How many times the layer grew (and the context was rebuilt or kept), for tests and stats.
    growths: std::sync::atomic::AtomicUsize,
    /// Components resolved through a walk that passed a file it could not read, with the unread walks they reached:
    /// answered until one of those is due, then resolved again. Never carried to the next revision, never persisted.
    provisional: DashMap<ComponentId, (Arc<Resolved>, dependency_types::Footprint), FxBuildHasher>,
    /// The id of the session's next dependency load, shared by its revisions: a footprint tells one load's failed read
    /// from a later one's.
    load_ids: Arc<dependency_types::LoadIds>,
    /// The session's clock for following again a walk that passed a file it could not read, shared by its revisions.
    clock: Arc<RetryClock>,
    /// How many edits the session has seen: one arriving makes every such walk due again.
    edits: dependency_types::Edits,
    /// How many times a component was resolved, for tests.
    #[cfg(test)]
    resolutions: std::sync::atomic::AtomicUsize,
    /// How many times the layer was loaded, whether or not it grew, for tests.
    #[cfg(test)]
    loads: std::sync::atomic::AtomicUsize,
    /// How many times [`Lazy::shared_unread`] asked a layer, for tests.
    #[cfg(test)]
    shared_unread_builds: std::sync::atomic::AtomicUsize,
    /// How many shared walks resolutions asked the ledger about one by one, for tests.
    #[cfg(test)]
    shared_walk_visits: std::sync::atomic::AtomicUsize,
    /// Run once by the next [`Revision::lazy_component`] between its resolution and its diagnostics, for tests.
    #[cfg(test)]
    between: Mutex<Option<Between>>,
}

/// A layer, and what the shared demands lead to in it ([`Lazy::shared_unread`]).
type SharedUnread = (Arc<dependency_types::DependencyLayer>, Arc<SharedWalks>);

/// The unread walks the shared demands lead to in one layer, and their footprint (`None` when one of them has a file
/// read since): one copy per layer, which every resolution that reads React's augmentations shares.
pub(super) struct SharedWalks {
    unread: Arc<[Demand]>,
    footprint: Option<dependency_types::Footprint>,
}

/// What a test runs inside a request ([`Lazy::between`]).
#[cfg(test)]
type Between = Box<dyn FnOnce(&Revision) + Send>;

impl Lazy {
    /// When this revision follows again a walk that passed a file it could not read: once it is due.
    fn retry(&self) -> dependency_types::Retry {
        dependency_types::Retry::Due(self.moment())
    }

    /// The unread walks the shared demands lead to in `layer` ([`Lazy::shared_unread`]).
    fn shared_unread(&self, layer: &Arc<dependency_types::DependencyLayer>) -> Arc<SharedWalks> {
        let mut held = self.shared_unread.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, unread)) = held.as_ref()
            && Arc::ptr_eq(at, layer)
        {
            return unread.clone();
        }
        #[cfg(test)]
        self.shared_unread_builds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unread: Arc<[Demand]> = Arc::from(layer.unread(&self.shared));
        let walks = Arc::new(SharedWalks { footprint: layer.footprint(&unread), unread });
        *held = Some((layer.clone(), walks.clone()));
        walks
    }

    fn moment(&self) -> dependency_types::Moment {
        dependency_types::Moment { at: self.clock.now(), edits: self.edits }
    }
}

/// A session's time for [`dependency_types::Retry`]; a test sets it.
#[derive(Default)]
pub(super) struct RetryClock {
    #[cfg(test)]
    set: Mutex<Option<Instant>>,
}

impl RetryClock {
    /// This clock as a load reads it at each failed read.
    fn reader(clock: &Arc<RetryClock>) -> dependency_types::Clock {
        let clock = clock.clone();
        Arc::new(move || clock.now())
    }

    fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(now) = *self.set.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) {
            return now;
        }
        Instant::now()
    }
}

/// The components of `global` by name, each name's components in output order.
fn group_by_name(global: &GlobalSourceData, options: &PipelineOptions) -> FxHashMap<String, Vec<(ComponentId, usize)>> {
    let mut by_name: FxHashMap<String, Vec<(ComponentId, usize)>> = FxHashMap::default();
    for (id, index) in ordered_ids(global, options) {
        by_name.entry(id.name.clone()).or_default().push((id, index));
    }
    by_name
}

/// Names each of `ids` in a delta: its name is touched and its file affected. Every path that reports answers which
/// may differ goes through this, for the memoized and the provisional ones alike.
fn touch(ids: impl Iterator<Item = ComponentId>, touched: &mut FxHashSet<String>, affected: &mut Vec<Utf8PathBuf>) {
    for id in ids {
        touched.insert(id.name);
        affected.push(id.file);
    }
}

/// The demands every component may read through: React's names and each module a project file imports.
fn shared_demands<'a>(file_demands: impl IntoIterator<Item = &'a [Demand]>) -> Arc<[Demand]> {
    let mut shared: Vec<Demand> = file_demands
        .into_iter()
        .flatten()
        .filter(|demand| &*demand.specifier == "react" || demand.name.is_empty())
        .cloned()
        .collect();
    shared.sort();
    shared.dedup();
    Arc::from(shared)
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
        // It also reads the persisted dependency index, so names a batch run already resolved cost no walk. The modules
        // the project imports start loaded as well: what they merge into React's interfaces reaches every component.
        let react: Vec<Demand> = file_demands
            .iter()
            .flat_map(|(_, demands)| {
                demands.iter().filter(|demand| &*demand.specifier == "react" || demand.name.is_empty()).cloned()
            })
            .collect();
        // The first load follows every walk, and stamps its failed reads with the session's own moment and load id.
        let (clock, load_ids, edits) = (
            Arc::<RetryClock>::default(),
            Arc::<dependency_types::LoadIds>::default(),
            dependency_types::Edits::default(),
        );
        let layer = options.load_dependencies.then(|| {
            let moment = dependency_types::Moment { at: clock.now(), edits };
            let stamp = dependency_types::Stamp { moment, load: load_ids.next() };
            let clock = RetryClock::reader(&clock);
            Arc::new(dependency_types::load_timed(react, options, None, dependency_types::Retry::Always, stamp, clock))
        });
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
            shared: shared_demands(file_demands.iter().map(|(_, demands)| demands.as_slice())),
            shared_unread: Mutex::new(None),
            demands: demands_by_file(file_demands),
            by_name: group_by_name(&global, options),
            memo: DashMap::default(),
            engine: ArcSwap::from_pointee(engine),
            growing: Mutex::new(()),
            whole_demand_started: std::sync::atomic::AtomicBool::new(false),
            whole_demand_thread: Mutex::new(None),
            growths: std::sync::atomic::AtomicUsize::new(0),
            provisional: DashMap::default(),
            clock,
            edits,
            load_ids,
            #[cfg(test)]
            resolutions: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            loads: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            shared_unread_builds: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            shared_walk_visits: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            between: Mutex::new(None),
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
            sites: Arc::default(),
            demands: Arc::default(),
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
        if !is_source_file(changed, &options.src_dirs, &options.exclude_patterns, self.source_files()) {
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
        // A provisional answer is never carried over: the edit makes every walk past an unreadable file due, so each
        // may answer differently, whatever file it read.
        touch(previous.provisional.iter().map(|entry| entry.key().clone()), &mut touched, &mut affected_files);
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
        // A kept answer is decided again against the new call sites, so an edit to a consumer can change its `used`
        // lists without touching its resolution. The new sites are the ones the next answer reads.
        if memo.iter().any(|entry| entry.value().evidence.is_some()) {
            let before = relevance_inputs::project_sites(&previous.engine.load().ctx);
            let after = relevance_inputs::project_sites(&engine.ctx);
            for entry in memo.iter().filter(|entry| entry.value().redecided(&before, &after)) {
                touched.insert(entry.key().name.clone());
            }
        }
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
            sites: Arc::default(),
            demands: Arc::default(),
            files: Arc::new(files),
            dts_files_parsed: self.dts_files_parsed,
            dts_cache_hits: self.dts_cache_hits,
            duration_ms: self.duration_ms,
            output: Arc::new(empty_output()),
            keys: FxHashMap::default(),
            lazy: Some(Lazy {
                shared: shared_demands(file_demands.values().map(|demands| demands.as_ref())),
                shared_unread: Mutex::new(None),
                demands: file_demands,
                by_name,
                memo,
                engine: ArcSwap::from_pointee(engine),
                growing: Mutex::new(()),
                whole_demand_started: std::sync::atomic::AtomicBool::new(false),
                whole_demand_thread: Mutex::new(None),
                growths: std::sync::atomic::AtomicUsize::new(0),
                // An edit makes every walk past an unreadable file due: no provisional answer carries over.
                provisional: DashMap::default(),
                clock: previous.clock.clone(),
                edits: previous.edits.next(),
                load_ids: previous.load_ids.clone(),
                #[cfg(test)]
                resolutions: std::sync::atomic::AtomicUsize::new(0),
                #[cfg(test)]
                loads: std::sync::atomic::AtomicUsize::new(0),
                #[cfg(test)]
                shared_unread_builds: std::sync::atomic::AtomicUsize::new(0),
                #[cfg(test)]
                shared_walk_visits: std::sync::atomic::AtomicUsize::new(0),
                #[cfg(test)]
                between: Mutex::new(None),
            }),
        };
        let update = self.lazy_delta(&next, options, &touched, affected_files, start);
        Some((next, update))
    }

    /// The revision after a change that can alter how every import resolves, rebuilt from scratch. Every name the
    /// previous revision had answered, memoized or provisional, is reported, since any answer may differ.
    pub(super) fn rebuilt_lazy(&self, options: &PipelineOptions) -> Option<(Revision, IncrementalUpdate)> {
        let previous = self.lazy.as_ref()?;
        let start = Instant::now();
        let next = Revision::cold_lazy(options, self.id + 1);
        let (mut touched, mut affected_files) = (FxHashSet::default(), Vec::new());
        let memo = previous.memo.iter().map(|entry| entry.key().clone());
        let provisional = previous.provisional.iter().map(|entry| entry.key().clone());
        touch(memo.chain(provisional), &mut touched, &mut affected_files);
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
        self.lazy_resolved_at(options, id, index).map(|(resolved, _)| resolved)
    }

    /// [`Self::lazy_resolved`], with the layer a provisional answer was read against: its read warnings are the ones
    /// that answer carries, whatever a later load recovered since. `None` for a memoized answer, which no unread walk
    /// reached.
    fn lazy_resolved_at(
        &self,
        options: &PipelineOptions,
        id: &ComponentId,
        index: usize,
    ) -> Option<(Arc<Resolved>, Option<Arc<dependency_types::DependencyLayer>>)> {
        let lazy = self.lazy.as_ref()?;
        if let Some(hit) = lazy.memo.get(id) {
            return Some((hit.clone(), None));
        }
        if let Some(hit) = self.provisional_hit(lazy, id) {
            return Some(hit);
        }
        let mapping = self.global.component_mappings.get(index)?;
        // The component's own file almost always reaches its own names; loading them first saves a resolution.
        if let Some(seed) = lazy.demands.get(mapping.file_path.as_str()) {
            self.grow(options, lazy, seed);
        }
        let mut asked_for: Vec<dependency_types::Demand> = Vec::new();
        loop {
            let engine = lazy.engine.load_full();
            #[cfg(test)]
            lazy.resolutions.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (entry, diagnostics, depends_on, evidence, attributes) =
                resolve_guarded(&[mapping], &engine.ctx).into_iter().next()?;
            // The names the resolution reached that the layer lacks, and the demands below them whose walk passed a
            // file that could not be read (the layer's ledger): the next load follows both, the second once due.
            let (missing, unread) = match &engine.layer {
                Some(layer) => {
                    let roots: Vec<Demand> = depends_on
                        .files
                        .iter()
                        .filter_map(|file| lazy.demands.get(file))
                        .flat_map(|demands| demands.iter().cloned())
                        .collect();
                    let retry = lazy.retry();
                    let unread = layer.unread(&roots);
                    // React's augmentations come from every module the project imports, whichever file imports it, and
                    // the first load followed those for every component: their unread walks and footprint are the
                    // layer's, built once and shared, and asked again only once that footprint is due.
                    let shared = depends_on.reads_augmentations.then(|| lazy.shared_unread(layer));
                    let shared = shared.filter(|walks| !walks.unread.is_empty());
                    let mut missing = layer.missing_at(&roots, retry);
                    missing.extend(layer.missing_at(&unread, retry));
                    if let Some(walks) = &shared
                        && !walks.footprint.as_ref().is_some_and(|held| layer.footprint_holds(held, lazy.moment()))
                    {
                        #[cfg(test)]
                        lazy.shared_walk_visits.fetch_add(walks.unread.len(), std::sync::atomic::Ordering::Relaxed);
                        missing.extend(layer.missing_at(&walks.unread, retry));
                    }
                    missing.sort();
                    missing.dedup();
                    (missing, (unread, shared))
                }
                None => (Vec::new(), (Vec::new(), None)),
            };
            // A resolution against a layer that lacked a name it reached is discarded, and so is its context's memo.
            // A load that left the same names missing answered none of them (a demand whose load panicked stays
            // unanswered, a file that stays unreadable), so asking again would never end.
            if missing.is_empty() || missing == asked_for {
                let resolved = Arc::new(Resolved { entry, diagnostics, depends_on, evidence, attributes });
                // A walk past a file that could not be read answers more once it is followed again: until then the
                // answer holds only provisionally, and never in the memo an edit carries over.
                // With no footprint, a file one of the walks could not read was read since in this same layer, so the
                // walk is due: this pass already asked for it (`missing == asked_for`) and the load left it as it was.
                // The answer and its warnings both come from `engine`; held nowhere, the next request asks again.
                let (own, shared) = unread;
                if !own.is_empty() || shared.is_some() {
                    let own = (!own.is_empty()).then(|| engine.layer.as_ref().and_then(|layer| layer.footprint(&own)));
                    let parts = own.into_iter().chain(shared.map(|walks| walks.footprint.clone()));
                    if let Some(footprint) = parts.collect::<Option<Vec<_>>>().map(dependency_types::Footprint::union) {
                        lazy.provisional.insert(id.clone(), (resolved.clone(), footprint));
                    }
                    return Some((resolved, engine.layer.clone()));
                }
                return Some((lazy.memo.entry(id.clone()).or_insert(resolved).clone(), None));
            }
            self.grow(options, lazy, &missing);
            asked_for = missing;
        }
    }

    /// The provisional answer for `id` while none of the unread walks it reached is due ([`Lazy::provisional`]): no
    /// retry has passed, no edit arrived, and no later load followed one of them again or read one of their files.
    fn provisional_hit(
        &self,
        lazy: &Lazy,
        id: &ComponentId,
    ) -> Option<(Arc<Resolved>, Option<Arc<dependency_types::DependencyLayer>>)> {
        let (resolved, footprint) = lazy.provisional.get(id).map(|entry| entry.value().clone())?;
        let layer = lazy.engine.load().layer.clone();
        let holds = layer.as_ref().is_some_and(|layer| layer.footprint_holds(&footprint, lazy.moment()));
        if holds {
            return Some((resolved, layer));
        }
        lazy.provisional.remove(id);
        None
    }

    /// Once the layer has grown [`WHOLE_DEMAND_AFTER_GROWTHS`] times, load every name the project's files take from a
    /// package on a thread of its own, once.
    ///
    /// Each growth of the layer rebuilds the resolution context, so a session that grows one request at a time pays
    /// that rebuild for every new component (Spectrum: 33 rebuilds, about 140 of the 300 ms a project read end to end
    /// took). A session that asks about a few components never gets here, and holds only the names they reach; one
    /// that reads across the project does, and the later requests find every name loaded. A request that needs a name
    /// before the thread has loaded it waits for the same lock and loads it, as it would have anyway.
    pub(crate) fn load_whole_demand_in_background(self: &Arc<Self>, options: &PipelineOptions) {
        use std::sync::atomic::Ordering::Relaxed;
        let Some(lazy) = &self.lazy else { return };
        if !options.load_dependencies
            || lazy.growths.load(Relaxed) < WHOLE_DEMAND_AFTER_GROWTHS
            || lazy.whole_demand_started.swap(true, Relaxed)
        {
            return;
        }
        let (revision, options) = (self.clone(), options.clone());
        let spawned = std::thread::Builder::new().name("caliper-whole-demand".into()).spawn(move || {
            drop(crate::panic_guard::contain_panic("load_whole_demand", || {
                let Some(lazy) = &revision.lazy else { return };
                let all: Vec<Demand> = lazy.demands.values().flat_map(|demands| demands.iter().cloned()).collect();
                revision.grow(&options, lazy, &all);
            }));
        });
        // A thread that cannot start costs the speed-up, never an answer.
        if let Ok(handle) = spawned {
            *lazy.whole_demand_thread.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(handle);
        }
    }

    /// Wait until the background load [`Self::load_whole_demand_in_background`] started has finished. Without one,
    /// returns at once.
    pub(crate) fn wait_for_dependency_load(&self) {
        let Some(lazy) = &self.lazy else { return };
        let handle = lazy.whole_demand_thread.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(handle) = handle {
            drop(handle.join());
        }
    }

    /// Whether the session's current context holds this revision's own project index, not a copy of it: growing the
    /// layer must not clone the project's declarations.
    #[cfg(test)]
    pub(crate) fn context_shares_project_index(&self) -> Option<bool> {
        let lazy = self.lazy.as_ref()?;
        Some(lazy.engine.load().ctx.named_types.shares_declarations_with(&self.named_types))
    }

    /// How many times the dependency layer grew since this revision was built.
    #[cfg(test)]
    pub(crate) fn layer_growths(&self) -> usize {
        self.lazy.as_ref().map_or(0, |lazy| lazy.growths.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Load `demands` into the dependency layer unless a concurrent request already did.
    fn grow(&self, options: &PipelineOptions, lazy: &Lazy, demands: &[Demand]) {
        let _growing = lazy.growing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let engine = lazy.engine.load_full();
        let Some(layer) = &engine.layer else { return };
        let moment = lazy.moment();
        let retry = dependency_types::Retry::Due(moment);
        let missing = layer.missing_at(demands, retry);
        if missing.is_empty() {
            return;
        }
        let stamp = dependency_types::Stamp { moment, load: lazy.load_ids.next() };
        let clock = RetryClock::reader(&lazy.clock);
        let extended =
            Arc::new(off_pool(|| dependency_types::load_timed(missing, options, Some(layer), retry, stamp, clock)));
        #[cfg(test)]
        lazy.loads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Names that were already loaded, or that no package declares, add nothing the resolver reads, so the
        // context and its memos stay; only the record of what was asked for moves on. Such a load (a retry of a file
        // that stays unreadable) is no growth either: it brings the background load of every name no nearer.
        if extended.resolves_like(layer) {
            lazy.engine.store(Arc::new(Engine { layer: Some(extended), ctx: engine.ctx.clone() }));
            return;
        }
        lazy.growths.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The new context shares the project's index, import map, resolvers and bare-name tables with the old one;
        // rebuilding them for every growth was most of what a session reading across a project spent.
        let named_types = Arc::new(self.named_types.with_fallback(extended.served_types()));
        let ctx = Arc::new(engine.ctx.over_layer(named_types, Some(extended.clone())));
        // What the load added that declares into React (an augmentation a retry read, React's own types) changes what
        // every resolution that reads React's augmentations sees, whichever files it recorded.
        let augmentations_changed = extended.changes_react_declarations(layer);
        let next = Engine { layer: Some(extended), ctx };
        // A memo that reached a project file whose names the old layer lacked was computed without them, and stays
        // behind; the rest hold for the larger layer, so the next component does not rebuild them. So does one that
        // read a file whose walk reached an unread one (the old layer's ledger): a project file whose demands did, or a
        // package file whose declarations did, since what lay past the file that could not be read is missing from it.
        let unread = layer.unread_walks();
        let unread_project: FxHashSet<&str> = match unread.is_empty() {
            true => FxHashSet::default(),
            false => lazy
                .demands
                .iter()
                .filter(|(_, demands)| unread.reached_by(demands))
                .map(|(file, _)| std::borrow::Borrow::<str>::borrow(file))
                .collect(),
        };
        let kept = |file: &str| {
            lazy.demands.get(file).is_none_or(|demands| layer.covers(demands))
                && !unread_project.contains(file)
                && !unread.declared_in(file)
        };
        // The one rule for every cache a growth carries over, the context's memos and the session's answers alike.
        let still_valid = |deps: &crate::resolver::Dependencies, read_in: &str| {
            !(augmentations_changed && deps.reads_augmentations)
                && kept(read_in)
                && deps.files.iter().all(|file| kept(std::borrow::Borrow::<str>::borrow(file)))
        };
        next.ctx.inherit_memos(&engine.ctx, still_valid);
        if augmentations_changed {
            lazy.memo.retain(|_, resolved| !resolved.depends_on.reads_augmentations);
            lazy.provisional.retain(|_, (resolved, _)| !resolved.depends_on.reads_augmentations);
        }
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
            if matches!(mapping.rule, DetectionRule::ImportedMemberAlias | DetectionRule::FactoryResult) && !assumed {
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

    /// Where each key [`Self::lazy_keys`] lists is declared, sorted by key. Parsing alone answers it; nothing is
    /// resolved.
    pub(crate) fn lazy_index(&self, options: &PipelineOptions) -> Vec<ComponentLocation> {
        let Some(lazy) = &self.lazy else { return Vec::new() };
        let mut index: Vec<ComponentLocation> = lazy
            .by_name
            .iter()
            .flat_map(|(name, group)| {
                self.group_positions(options, name, true).into_iter().filter_map(|(key, position)| {
                    let (id, index) = group.get(position)?;
                    let exports = self.global.component_mappings.get(*index)?.exports.clone();
                    Some(ComponentLocation { key, file_path: id.file.clone(), display_name: id.name.clone(), exports })
                })
            })
            .collect();
        index.sort_by(|a, b| a.key.cmp(&b.key));
        index
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
        let (resolved, read_against) = self.lazy_resolved_at(options, id, index)?;
        #[cfg(test)]
        if let Some(between) = self.lazy.as_ref().and_then(|lazy| lazy.between.lock().ok()?.take()) {
            between(self);
        }
        // A lazy revision counts call sites with its context, on the first answer that needs them.
        let sites = self.lazy.as_ref().map(|lazy| relevance_inputs::project_sites(&lazy.engine.load_full().ctx));
        let entry = resolved.answer(&sites.unwrap_or_default());
        let mut diagnostics = DiagnosticSet::default();
        diagnostics.extend(resolved.diagnostics.own.iter().cloned(), Some(key));
        for chunk in &resolved.diagnostics.shared {
            diagnostics.extend_shared(chunk, key);
        }
        // A panic in the dependency loader may have left any component's types unexpanded, so every answer carries it.
        // A provisional answer carries the read warnings of the layer it was read against: a later load that read the
        // file again ends the warning in the newest layer, not the gap in this answer.
        let layer = read_against.or_else(|| self.lazy.as_ref().and_then(|lazy| lazy.engine.load_full().layer.clone()));
        if let Some(layer) = layer {
            diagnostics.extend(layer.panic_diagnostics(), None);
        }
        Some(ComponentAnswer { key: key.to_owned(), entry, diagnostics: diagnostics.finish() })
    }
}

#[cfg(test)]
mod read_retry_tests {
    //! What a lazy session pays while a dependency file stays unreadable: a component behind it is followed again only
    //! once that is due, and no other component loses its memos. Unreadable bytes are not UTF-8, the same length as
    //! the text, so restoring the text and the modification time leaves the file's stamp as the failed read found it.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeSet;
    use std::sync::atomic::Ordering::Relaxed;
    use std::time::Duration;

    use camino::Utf8PathBuf;

    use super::*;
    use crate::dependency_types::DEGRADED_RETRY;
    use crate::pipeline::{CollapseInheritedProps, ComponentAnswer, PipelineOptions};

    const BAD_PROPS: &str = "export interface BadProps { bad: string }\n";
    const BAD2_PROPS: &str = "export interface Bad2Props { bad2: string }\n";
    const FIRST: &str = "export interface FirstProps { first: string }\n";

    fn warned(answer: &ComponentAnswer) -> Vec<String> {
        let file = |diagnostic: &crate::types::Diagnostic| diagnostic.file.clone().unwrap_or_default();
        let io = |diagnostic: &&crate::types::Diagnostic| diagnostic.code == crate::types::DiagnosticCode::IoError;
        answer.diagnostics.iter().filter(io).map(file).collect()
    }

    /// A walk that passes an unreadable `export *` target and finds the name in a later one gives the resolver that
    /// barrel's mapping for this layer: the props are in the answer, with the warning. The mapping is kept nowhere a
    /// complete answer is (`export_of_complete`, which the index writes). Once the file reads and an edit arrives, the
    /// answer is a cold run's.
    #[test]
    fn an_export_found_past_an_unreadable_star_target_reaches_the_resolver_for_its_layer() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let first = root.join("node_modules/barrel-kit/first.d.ts");
        std::fs::write(&first, [0xff; FIRST.len()]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        let answer = revision.lazy_component(&options, "S").unwrap();
        assert_eq!(props(&answer), ["own", "second"]);
        assert_eq!(warned(&answer), [first.to_string()]);
        let layer = revision.lazy.as_ref().unwrap().engine.load().layer.clone().unwrap();
        let barrel = root.join("node_modules/barrel-kit/index.d.ts");
        assert_eq!(layer.export_of_complete(&barrel, "SecondProps"), None, "not kept as a complete answer");
        assert!(layer.export_for_resolver(&barrel, "SecondProps").is_some());

        std::fs::write(&first, FIRST).unwrap();
        let s = root.join("src/s/S.tsx");
        std::fs::write(&s, std::fs::read_to_string(&s).unwrap() + "// edited\n").unwrap();
        let next = revision.next_lazy(&options, &s).unwrap().0;
        assert_eq!(next.lazy_component(&options, "S").unwrap(), cold(&root, "S"));
    }

    /// A component answered provisionally that also reads a project file: an edit to that file names it in the delta,
    /// and its next answer is a cold run's.
    #[test]
    fn an_edit_names_every_provisional_component_in_the_delta() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        std::fs::write(root.join("node_modules/bad-kit/props.d.ts"), [0xff; BAD_PROPS.len()]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        assert_eq!(props(&revision.lazy_component(&options, "A2").unwrap()), ["local", "own"]);
        let local = root.join("src/a2/local.ts");
        std::fs::write(&local, "export interface LocalProps { local: string; more: string }\n").unwrap();
        let (next, update) = revision.next_lazy(&options, &local).unwrap();
        assert!(update.updated_keys.contains(&"A2".to_owned()), "{:?}", update.updated_keys);
        let answer = next.lazy_component(&options, "A2").unwrap();
        assert_eq!(props(&answer), ["local", "more", "own"]);
        assert_eq!(answer, cold(&root, "A2"));
    }

    /// A load that only retries a file that stays unreadable adds nothing the resolver reads, so it is no growth:
    /// repeated retries of one component never bring the background load of every name nearer.
    #[test]
    fn retries_of_a_file_that_stays_unreadable_are_no_growth() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        std::fs::write(root.join("node_modules/bad-kit/props.d.ts"), [0xff; BAD_PROPS.len()]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        let start = Instant::now();
        set_clock(&revision, start);
        drop(revision.lazy_component(&options, "A"));
        let lazy = revision.lazy.as_ref().unwrap();
        let growths = lazy.growths.load(Relaxed);
        for retry in 1..=5_u32 {
            set_clock(&revision, start + DEGRADED_RETRY * retry);
            assert_eq!(cost(&revision, || drop(revision.lazy_component(&options, "A"))), (1, 1), "retry {retry}");
        }
        assert_eq!(lazy.growths.load(Relaxed), growths, "no retry is a growth");
    }

    /// Another request reads the unreadable file again between this request's resolution and its diagnostics: the
    /// answer still carries the warning of the layer it was read against, so incomplete props never come without it.
    #[test]
    fn an_answer_carries_the_warnings_of_the_layer_it_was_read_against() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let bad2 = root.join("node_modules/bad2-kit/props.d.ts");
        std::fs::write(&bad2, [0xff; BAD2_PROPS.len()]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        let (recover_file, recover_options) = (bad2.clone(), options.clone());
        *revision.lazy.as_ref().unwrap().between.lock().unwrap() = Some(Box::new(move |revision: &Revision| {
            std::fs::write(&recover_file, BAD2_PROPS).unwrap();
            assert_eq!(props(&revision.lazy_component(&recover_options, "C").unwrap()), ["bad2", "own"]);
        }));
        let answer = revision.lazy_component(&options, "B").unwrap();
        assert_eq!(props(&answer), ["own"]);
        assert_eq!(warned(&answer), [bad2.to_string()], "the gap in the props comes with its warning");
    }

    /// Two hundred demands in one load, each behind its own unreadable file: each mark holds exactly the file its own
    /// walk could not read, 200 entries in all, not 200 for each.
    #[test]
    fn each_unread_walk_holds_only_its_own_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        let kit = root.join("node_modules/many-kit");
        std::fs::create_dir_all(&kit).unwrap();
        std::fs::write(
            kit.join("package.json"),
            r#"{ "name": "many-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
        )
        .unwrap();
        let index: String = (0..200).map(|n| format!("export {{ P{n} }} from './p{n}';\n")).collect();
        std::fs::write(kit.join("index.d.ts"), index).unwrap();
        for n in 0..200 {
            std::fs::write(kit.join(format!("p{n}.d.ts")), [0xff; 8]).unwrap();
        }
        let demands: Vec<Demand> = (0..200)
            .map(|n| Demand {
                from: root.join("src/Card.tsx"),
                specifier: "many-kit".into(),
                name: format!("P{n}").into(),
            })
            .collect();
        let layer = dependency_types::load(demands.clone(), &options(&root, false), None);
        let suspects = layer.suspect_counts();
        assert_eq!(suspects.len(), 200, "one mark per unread walk");
        assert!(suspects.iter().all(|count| *count == 1), "each holds its own file only");
        assert_eq!(layer.unread(&demands).len(), 200);
    }

    /// The session's first load follows every walk and could not read React's types. After three edits, the first
    /// request behind them follows that walk again (an edit makes it due), and the requests after it are answered from
    /// that retry until the next one is due: one load and one resolution, then none.
    #[test]
    fn a_failure_the_first_load_recorded_is_followed_again_once_per_retry_after_edits() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let react = root.join("node_modules/react/index.d.ts");
        std::fs::write(&react, [0xff; 8]).unwrap();
        let mut revision = Revision::cold_lazy(&options, 0);
        let start = Instant::now();
        set_clock(&revision, start);
        let u1 = root.join("src/u1/U1.tsx");
        for edit in 0..3 {
            std::fs::write(&u1, std::fs::read_to_string(&u1).unwrap() + &format!("// edit {edit}\n")).unwrap();
            revision = revision.next_lazy(&options, &u1).unwrap().0;
        }
        let ask = || revision.lazy_component(&options, "M").unwrap();
        let mut first = None;
        assert_eq!(cost(&revision, || first = Some(ask())), (1, 1), "the edits made it due");
        assert_eq!(props(first.as_ref().unwrap()), ["own"]);
        assert_eq!(warned(first.as_ref().unwrap()), [react.to_string()]);
        for request in 0..5 {
            assert_eq!(cost(&revision, || drop(ask())), (0, 0), "request {request} inside the retry");
        }
        set_clock(&revision, start + DEGRADED_RETRY);
        assert_eq!(cost(&revision, || drop(ask())), (1, 1), "due again after the retry");
    }

    /// A component whose own demand is complete but whose package declaration extends a type from a file that could not
    /// be read: the memos read in its file do not outlive the next growth, since what lay past that file is missing.
    #[test]
    fn memos_read_in_a_file_whose_demand_reaches_an_unread_walk_do_not_outlive_a_growth() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        std::fs::write(root.join("node_modules/nest-kit/base.d.ts"), [0xff; 8]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        assert_eq!(props(&revision.lazy_component(&options, "N").unwrap()), ["own", "tone"]);
        assert!(memo_files(&revision).contains("N.tsx"));
        drop(revision.lazy_component(&options, "U2"));
        let after = memo_files(&revision);
        assert!(!after.contains("N.tsx") && after.contains("U2.tsx"), "{after:?}");
    }

    /// `D` loads `second.d.ts`'s declaration directly; then `S`, in the same directory, finds it through the barrel
    /// past an unreadable `export *` target. That load adds only the barrel's partial mapping, and the resolver reads
    /// it: `S` has the props a fresh session over the same unreadable file gives.
    #[test]
    fn a_barrel_mapping_to_a_declaration_loaded_directly_reaches_the_resolver() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let first = root.join("node_modules/barrel-kit/first.d.ts");
        std::fs::write(&first, [0xff; FIRST.len()]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        let start = Instant::now();
        set_clock(&revision, start);
        assert_eq!(props(&revision.lazy_component(&options, "D").unwrap()), ["own", "second"]);
        let answer = revision.lazy_component(&options, "S").unwrap();
        let fresh = Revision::cold_lazy(&options, 0);
        set_clock(&fresh, start);
        assert_eq!(props(&answer), ["own", "second"]);
        assert_eq!(answer, fresh.lazy_component(&options, "S").unwrap());
        // A retry that finds the same mapping again changes nothing the resolver reads: no growth.
        let growths = revision.layer_growths();
        set_clock(&revision, start + DEGRADED_RETRY);
        assert_eq!(cost(&revision, || drop(revision.lazy_component(&options, "S"))), (1, 1));
        assert_eq!(revision.layer_growths(), growths);
    }

    /// A provisional component, then a change that can alter how every import resolves (the rebuild path): its key is
    /// in the delta, and once the file reads its answer is a cold run's.
    #[test]
    fn a_rebuild_names_every_provisional_component_in_the_delta() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let bad = root.join("node_modules/bad-kit/props.d.ts");
        std::fs::write(&bad, [0xff; BAD_PROPS.len()]).unwrap();
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        assert_eq!(props(&revision.lazy_component(&options, "A").unwrap()), ["own"]);
        std::fs::write(&bad, BAD_PROPS).unwrap();
        let (next, update) = revision.rebuilt_lazy(&options).unwrap();
        assert!(update.updated_keys.contains(&"A".to_owned()), "{:?}", update.updated_keys);
        assert_eq!(next.lazy_component(&options, "A").unwrap(), cold(&root, "A"));
    }

    /// `setup.ts` imports a package whose React augmentation could not be read; `Box` never reaches `setup.ts` but
    /// reads React's augmentations. Its answer is held only until the retry: once the file reads and the retry is due,
    /// it has the augmented attribute, equal to a cold run.
    #[test]
    fn a_component_reading_augmentations_waits_on_the_shared_augmentation_walks() {
        shared_augmentation_scenario();
    }

    /// A retry that serves other provisional augmentations while the layer's counts stay as they were: `aug-kit`
    /// re-exports `one`, `two` and `three`, each augmenting `AriaAttributes`. While `one` and `two` cannot be read,
    /// `three` is served; once `one` reads, `one` and `three` are, still provisionally (`two` does not), so nothing is
    /// indexed; once `two` reads, all three are indexed. Each component equals a cold run at each step.
    #[test]
    fn a_retry_that_serves_other_provisional_augmentations_is_a_change() {
        let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        let augmentation = |attribute: &str| {
            format!(
                "import type {{}} from 'react';\n\
                 declare module 'react' {{ interface AriaAttributes {{ '{attribute}'?: string }} }}\n"
            )
        };
        let aria = |name: &str| {
            format!(
                "import * as React from 'react';\ninterface {name}Props extends React.AriaAttributes {{ own?: boolean }}\n\
                 export const {name} = (props: {name}Props) => <div />;\n"
            )
        };
        let first = aria("First");
        // `Second` also reads a package `First` does not, so asking for it grows the layer.
        let second = "import * as React from 'react';\nimport type { OtherProps } from 'other-kit';\n\
                      interface SecondProps extends React.AriaAttributes, OtherProps { own?: boolean }\n\
                      export const Second = (props: SecondProps) => <div />;\n"
            .to_owned();
        let bodies = ["one", "two", "three"].map(|name| augmentation(&format!("aria-{name}")));
        let files = [
            (
                "node_modules/aug-kit/package.json",
                r#"{ "name": "aug-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
            ),
            (
                "node_modules/aug-kit/index.d.ts",
                "export * from './one';\nexport * from './two';\nexport * from './three';\n",
            ),
            ("node_modules/aug-kit/one.d.ts", bodies[0].as_str()),
            ("node_modules/aug-kit/two.d.ts", bodies[1].as_str()),
            ("node_modules/aug-kit/three.d.ts", bodies[2].as_str()),
            (
                "node_modules/other-kit/package.json",
                r#"{ "name": "other-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
            ),
            ("node_modules/other-kit/index.d.ts", "export interface OtherProps { other?: string }\n"),
            ("src/setup.ts", "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n"),
            ("src/first/First.tsx", first.as_str()),
            ("src/second/Second.tsx", second.as_str()),
        ];
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let options = |use_cache| PipelineOptions {
            html_attributes: crate::pipeline::HtmlAttributeMode::Full,
            ..options(&root, use_cache)
        };
        let (one, two) = (root.join("node_modules/aug-kit/one.d.ts"), root.join("node_modules/aug-kit/two.d.ts"));
        let cold = |name: &str| Revision::cold_lazy(&options(false), 0).lazy_component(&options(false), name).unwrap();
        let served = |answer: &ComponentAnswer| -> Vec<String> {
            let names = ["aria-one", "aria-two", "aria-three"];
            props(answer).into_iter().filter(|prop| names.contains(&prop.as_str())).collect()
        };
        std::fs::write(&one, [0xff; 8]).unwrap();
        std::fs::write(&two, [0xff; 8]).unwrap();
        let revision = Revision::cold_lazy(&options(true), 0);
        let start = Instant::now();
        set_clock(&revision, start);
        for name in ["First", "Second"] {
            // `Second` grows the layer before any retry is due: what `First`'s load served is served still.
            let answer = revision.lazy_component(&options(true), name).unwrap();
            assert_eq!(served(&answer), ["aria-three"], "{name}");
            assert!(answer == cold(name), "{name}");
        }
        let steps = [(&one, &bodies[0], ["aria-one", "aria-three"].as_slice()), (&two, &bodies[1], &[])];
        for (step, (file, body, expected)) in steps.into_iter().enumerate() {
            std::fs::write(file, body).unwrap();
            set_clock(&revision, start + DEGRADED_RETRY * (step as u32 + 1));
            for name in ["First", "Second"] {
                let answer = revision.lazy_component(&options(true), name).unwrap();
                let cold = cold(name);
                if !expected.is_empty() {
                    assert_eq!(served(&answer), expected, "step {step}, {name}");
                }
                assert_eq!(served(&cold), served(&answer), "step {step}, {name}");
                assert!(answer == cold, "step {step}, {name}");
            }
        }
    }

    /// Full HTML mode: two props interfaces extend `React.AriaAttributes`, whose augmentation (imported only by
    /// `setup.ts`) could not be read, so the interface chain the first resolution cached lacks it. Once it reads and
    /// the retry loads it, no memo that read React's augmentations is carried into the larger context: both
    /// components equal a cold run, with the augmented attribute.
    #[test]
    fn a_retry_that_loads_an_augmentation_drops_every_memo_that_read_augmentations() {
        let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        let augmentation = "import type {} from 'react';\n\
                            declare module 'react' { interface AriaAttributes { 'aria-tw'?: string } }\n";
        let aria = |name: &str| {
            format!(
                "import * as React from 'react';\ninterface {name}Props extends React.AriaAttributes {{ own?: boolean }}\n\
                 export const {name} = (props: {name}Props) => <div />;\n"
            )
        };
        let (first, second) = (aria("First"), aria("Second"));
        let files = [
            (
                "node_modules/aug-kit/package.json",
                r#"{ "name": "aug-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
            ),
            ("node_modules/aug-kit/index.d.ts", augmentation),
            ("src/setup.ts", "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n"),
            ("src/first/First.tsx", first.as_str()),
            ("src/second/Second.tsx", second.as_str()),
        ];
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let options = |use_cache| PipelineOptions {
            html_attributes: crate::pipeline::HtmlAttributeMode::Full,
            ..options(&root, use_cache)
        };
        let aug = root.join("node_modules/aug-kit/index.d.ts");
        std::fs::write(&aug, [0xff; 8]).unwrap();
        let revision = Revision::cold_lazy(&options(true), 0);
        let start = Instant::now();
        set_clock(&revision, start);
        drop(revision.lazy_component(&options(true), "First"));
        drop(revision.lazy_component(&options(true), "Second"));
        std::fs::write(&aug, augmentation).unwrap();
        set_clock(&revision, start + DEGRADED_RETRY);
        for name in ["First", "Second"] {
            let answer = revision.lazy_component(&options(true), name).unwrap();
            let cold = Revision::cold_lazy(&options(false), 0).lazy_component(&options(false), name).unwrap();
            assert!(props(&cold).contains(&"aria-tw".to_owned()), "the fixture reaches the augmentation");
            assert!(answer == cold, "{name}: {}", props(&answer).contains(&"aria-tw".to_owned()));
        }
    }

    fn shared_augmentation_scenario() {
        // Inside the crate, so React's own types resolve from the workspace's `node_modules`.
        let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        let augmentation = "import type {} from 'react';\n\
                            declare module 'react' { interface HTMLAttributes<T> { tw?: string } }\n";
        let files = [
            (
                "node_modules/aug-kit/package.json",
                r#"{ "name": "aug-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
            ),
            ("node_modules/aug-kit/index.d.ts", augmentation),
            ("src/setup.ts", "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n"),
            (
                "src/box/Box.tsx",
                "import * as React from 'react';\n\
                 export const Box = (props: React.ComponentProps<'div'> & { own?: boolean }) => <div />;\n",
            ),
        ];
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let options = |use_cache| PipelineOptions {
            html_attributes: crate::pipeline::HtmlAttributeMode::Full,
            ..options(&root, use_cache)
        };
        let aug = root.join("node_modules/aug-kit/index.d.ts");
        std::fs::write(&aug, [0xff; 8]).unwrap();
        let revision = Revision::cold_lazy(&options(true), 0);
        let start = Instant::now();
        set_clock(&revision, start);
        assert!(!props(&revision.lazy_component(&options(true), "Box").unwrap()).contains(&"tw".to_owned()));
        std::fs::write(&aug, augmentation).unwrap();
        set_clock(&revision, start + DEGRADED_RETRY);
        let answer = revision.lazy_component(&options(true), "Box").unwrap();
        let cold = Revision::cold_lazy(&options(false), 0).lazy_component(&options(false), "Box").unwrap();
        assert!(props(&cold).contains(&"tw".to_owned()), "the fixture reaches the augmentation");
        assert!(answer == cold, "after the retry: {:?}", props(&answer).contains(&"tw".to_owned()));
    }

    /// Two hundred files import a package whose React augmentation could not be read, and two hundred components read
    /// React's augmentations: the shared walks are asked of each layer once, not once per resolution.
    #[test]
    fn the_shared_walks_are_asked_once_per_layer() {
        let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        let write = |path: &str, body: &[u8]| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        write(
            "node_modules/aug-kit/package.json",
            br#"{ "name": "aug-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
        );
        write("node_modules/aug-kit/index.d.ts", &[0xff; 8]);
        for n in 0..200 {
            write(&format!("src/setup{n}/setup.ts"), b"import * as Aug from 'aug-kit';\nexport const aug = Aug;\n");
            let component = format!(
                "import * as React from 'react';\nexport const Box{n} = (props: React.ComponentProps<'div'>) => <div />;\n"
            );
            write(&format!("src/box{n}/Box.tsx"), component.as_bytes());
        }
        let options = options(&root, false);
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        for n in 0..200 {
            assert!(!warned(&revision.lazy_component(&options, &format!("Box{n}")).unwrap()).is_empty());
        }
        let lazy = revision.lazy.as_ref().unwrap();
        let (builds, loads) = (lazy.shared_unread_builds.load(Relaxed), lazy.loads.load(Relaxed));
        assert!(builds <= loads + 1, "{builds} builds for {loads} loads");
        // While their footprint holds, no resolution asks about the 200 shared walks one by one.
        assert_eq!(lazy.shared_walk_visits.load(Relaxed), 0);
    }

    fn project() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        let kit = |name: &str| format!(r#"{{ "name": "{name}", "version": "1.0.0", "types": "index.d.ts" }}"#);
        let component = |name: &str, kit: &str, props: &str| {
            format!(
                "import type {{ {props} }} from '{kit}';\ninterface {name}Theme {{ tone: string }}\n\
                 interface {name}Props extends {props} {{ own: {name}Theme['tone'] }}\n\
                 export function {name}(props: {name}Props) {{ return null; }}\n"
            )
        };
        let files = [
            ("node_modules/bad-kit/package.json", kit("bad-kit")),
            ("node_modules/bad-kit/index.d.ts", "export * from './props';\n".to_owned()),
            ("node_modules/bad-kit/props.d.ts", BAD_PROPS.to_owned()),
            ("node_modules/good-kit/package.json", kit("good-kit")),
            ("node_modules/good-kit/index.d.ts", "export interface GoodProps { good: string }\n".to_owned()),
            ("node_modules/other-kit/package.json", kit("other-kit")),
            ("node_modules/other-kit/index.d.ts", "export interface OtherProps { other: string }\n".to_owned()),
            ("src/a/A.tsx", component("A", "bad-kit", "BadProps")),
            ("src/u1/U1.tsx", component("U1", "good-kit", "GoodProps")),
            ("src/u2/U2.tsx", component("U2", "other-kit", "OtherProps")),
            ("node_modules/bad2-kit/package.json", kit("bad2-kit")),
            ("node_modules/bad2-kit/index.d.ts", "export * from './props';\n".to_owned()),
            ("node_modules/bad2-kit/props.d.ts", BAD2_PROPS.to_owned()),
            ("node_modules/c-kit/package.json", kit("c-kit")),
            ("node_modules/c-kit/index.d.ts", "export * from '../bad2-kit/props';\n".to_owned()),
            ("src/b/B.tsx", component("B", "bad2-kit", "Bad2Props")),
            ("src/c/C.tsx", component("C", "c-kit", "Bad2Props")),
            ("src/a2/local.ts", "export interface LocalProps { local: string }\n".to_owned()),
            (
                "src/a2/A2.tsx",
                "import type { BadProps } from 'bad-kit';\nimport type { LocalProps } from './local';\n\
                 interface A2Props extends BadProps, LocalProps { own: string }\n\
                 export function A2(props: A2Props) { return null; }\n"
                    .to_owned(),
            ),
            ("node_modules/barrel-kit/package.json", kit("barrel-kit")),
            ("node_modules/barrel-kit/index.d.ts", "export * from './first';\nexport * from './second';\n".to_owned()),
            ("node_modules/barrel-kit/first.d.ts", FIRST.to_owned()),
            ("node_modules/barrel-kit/second.d.ts", "export interface SecondProps { second: string }\n".to_owned()),
            ("src/s/S.tsx", component("S", "barrel-kit", "SecondProps")),
            (
                "src/s/D.tsx",
                "import type { SecondProps } from 'barrel-kit/second';\nimport type { Missing } from 'barrel-kit';\n\
                 interface DProps extends SecondProps { own: string }\nexport function D(props: DProps) { return null; }\n"
                    .to_owned(),
            ),
            ("node_modules/nest-kit/package.json", kit("nest-kit")),
            (
                "node_modules/nest-kit/index.d.ts",
                "import type { Base } from './base';\nexport interface NestProps extends Base { tone: string }\n"
                    .to_owned(),
            ),
            ("node_modules/nest-kit/base.d.ts", "export interface Base { base: string }\n".to_owned()),
            ("src/n/N.tsx", component("N", "nest-kit", "NestProps")),
            ("node_modules/react/package.json", kit("react")),
            ("node_modules/react/index.d.ts", "export interface Attributes { key?: string }\n".to_owned()),
            (
                "src/m/M.tsx",
                "import type { Attributes } from 'react';\ninterface MProps extends Attributes { own: string }\n\
                 export function M(props: MProps) { return null; }\n"
                    .to_owned(),
            ),
        ];
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        (tmp, root)
    }

    fn options(root: &Utf8Path, use_cache: bool) -> PipelineOptions {
        PipelineOptions {
            src_dirs: vec![root.join("src")],
            cache_dir: Some(root.join("cache")),
            load_dependencies: true,
            use_cache,
            collapse_inherited_props: CollapseInheritedProps::None,
            ..Default::default()
        }
    }

    /// Writes `body` over `file` and gives it back the modification time it had, so its stamp is unchanged.
    fn rewrite_keeping_stamp(file: &Utf8Path, body: &[u8]) {
        let modified = std::fs::metadata(file).unwrap().modified().unwrap();
        std::fs::write(file, body).unwrap();
        std::fs::File::options().write(true).open(file).unwrap().set_modified(modified).unwrap();
    }

    /// The layer loads and component resolutions `request` cost.
    fn cost(revision: &Revision, request: impl FnOnce()) -> (usize, usize) {
        let lazy = revision.lazy.as_ref().unwrap();
        let before = (lazy.loads.load(Relaxed), lazy.resolutions.load(Relaxed));
        request();
        (lazy.loads.load(Relaxed) - before.0, lazy.resolutions.load(Relaxed) - before.1)
    }

    /// The files the context's member and props memos were read in (each component reads a member of its own theme).
    fn memo_files(revision: &Revision) -> BTreeSet<String> {
        let ctx = revision.lazy.as_ref().unwrap().engine.load().ctx.clone();
        let file = |file: &str| file.rsplit('/').next().unwrap_or_default().to_owned();
        let mut files: BTreeSet<String> =
            ctx.member_reads.iter().map(|entry| file(std::borrow::Borrow::borrow(&entry.key().0))).collect();
        files.extend(ctx.props_reads.iter().map(|entry| file(std::borrow::Borrow::borrow(&entry.key().0))));
        files
    }

    fn set_clock(revision: &Revision, now: Instant) {
        *revision.lazy.as_ref().unwrap().clock.set.lock().unwrap() = Some(now);
    }

    fn props(answer: &ComponentAnswer) -> Vec<String> {
        let mut names: Vec<String> = answer.entry.props.keys().cloned().collect();
        names.sort();
        names
    }

    fn cold(root: &Utf8Path, key: &str) -> ComponentAnswer {
        let options = options(root, false);
        Revision::cold_lazy(&options, 0).lazy_component(&options, key).unwrap()
    }

    /// One file stays unreadable (its stamp unchanged, as for a permission error). The component behind it costs one
    /// load and one resolution on its first request, then nothing until the retry is due (its provisional answer), then
    /// one load and one resolution once. A component that never reaches the file costs what it would anyway, and the
    /// memos of the components before it outlive the growths after the failure. Once the file reads and the retry is
    /// due, the answer equals a cold run's, and it is kept.
    #[test]
    fn a_file_that_stays_unreadable_is_followed_again_once_its_retry_is_due_and_costs_no_other_component_anything() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let props_file = root.join("node_modules/bad-kit/props.d.ts");
        rewrite_keeping_stamp(&props_file, &[0xff; BAD_PROPS.len()]);
        let revision = Revision::cold_lazy(&options, 0);
        let start = Instant::now();
        set_clock(&revision, start);
        let ask = |key: &str| revision.lazy_component(&options, key).unwrap();

        assert_eq!(cost(&revision, || assert_eq!(props(&ask("U1")), ["good", "own"])), (1, 1));
        assert_eq!(cost(&revision, || assert_eq!(props(&ask("A")), ["own"])), (1, 1), "the first request behind it");
        for request in 0..5 {
            assert_eq!(cost(&revision, || assert_eq!(props(&ask("A")), ["own"])), (0, 0), "request {request}");
        }
        let before = memo_files(&revision);
        assert!(before.contains("U1.tsx") && before.contains("A.tsx"), "{before:?}");
        assert_eq!(cost(&revision, || assert_eq!(props(&ask("U2")), ["other", "own"])), (1, 1));
        let after = memo_files(&revision);
        assert!(after.contains("U1.tsx"), "an unrelated component's memos are kept: {after:?}");
        assert!(!after.contains("A.tsx"), "the memos read behind the unread walk are not: {after:?}");
        assert_eq!(cost(&revision, || drop(ask("U1"))), (0, 0), "an unrelated component is memoized");

        set_clock(&revision, start + DEGRADED_RETRY - Duration::from_millis(1));
        assert_eq!(cost(&revision, || drop(ask("A"))), (0, 0), "not due yet");
        set_clock(&revision, start + DEGRADED_RETRY);
        assert_eq!(cost(&revision, || drop(ask("A"))), (1, 1), "due: followed again, still unreadable");
        assert_eq!(cost(&revision, || drop(ask("A"))), (0, 0), "the failure just now starts the retry again");

        std::fs::write(&props_file, BAD_PROPS).unwrap();
        assert_eq!(cost(&revision, || drop(ask("A"))), (0, 0), "readable, but nothing is read before the retry");
        set_clock(&revision, start + DEGRADED_RETRY * 2);
        let mut recovered = None;
        assert_eq!(cost(&revision, || recovered = Some(ask("A"))), (1, 1));
        assert_eq!(recovered.unwrap(), cold(&root, "A"), "once it reads, the answer is a cold run's");
        assert_eq!(cost(&revision, || drop(ask("A"))), (0, 0), "and it is kept");
    }

    /// An edit to the project, any file, makes every walk past an unreadable file due at once, without waiting for the
    /// retry, and drops the provisional answers: the next request answers as a cold run.
    #[test]
    fn an_edit_makes_the_walk_past_an_unreadable_file_due_at_once() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let props_file = root.join("node_modules/bad-kit/props.d.ts");
        rewrite_keeping_stamp(&props_file, &[0xff; BAD_PROPS.len()]);
        let first = Revision::cold_lazy(&options, 0);
        set_clock(&first, Instant::now());
        assert_eq!(props(&first.lazy_component(&options, "A").unwrap()), ["own"]);
        assert_eq!(cost(&first, || drop(first.lazy_component(&options, "A"))), (0, 0));

        std::fs::write(&props_file, BAD_PROPS).unwrap();
        let u1 = root.join("src/u1/U1.tsx");
        std::fs::write(&u1, std::fs::read_to_string(&u1).unwrap() + "// edited\n").unwrap();
        let revision = first.next_lazy(&options, &u1).unwrap().0;
        let mut answer = None;
        assert_eq!(cost(&revision, || answer = revision.lazy_component(&options, "A")), (1, 1));
        assert_eq!(answer.unwrap(), cold(&root, "A"));
    }

    /// Two components, followed in one load, each behind a file of its own that could not be read. One of the files is
    /// read since, by a third component's walk: only the component behind it is followed again, and the other keeps its
    /// provisional answer.
    #[test]
    fn only_the_walk_whose_own_file_was_read_since_is_followed_again() {
        let (_tmp, root) = project();
        let options = options(&root, true);
        let (bad, bad2) = (root.join("node_modules/bad-kit/props.d.ts"), root.join("node_modules/bad2-kit/props.d.ts"));
        rewrite_keeping_stamp(&bad, &[0xff; BAD_PROPS.len()]);
        rewrite_keeping_stamp(&bad2, &[0xff; BAD2_PROPS.len()]);
        let revision = Revision::cold_lazy(&options, 0);
        set_clock(&revision, Instant::now());
        let ask = |key: &str| revision.lazy_component(&options, key).unwrap();
        let both = ["A".to_owned(), "B".to_owned()];
        assert_eq!(cost(&revision, || drop(revision.lazy_components(&options, &both))), (1, 2), "one load for both");

        std::fs::write(&bad2, BAD2_PROPS).unwrap();
        assert_eq!(props(&ask("C")), ["bad2", "own"], "C's walk reads bad2 again");
        assert_eq!(cost(&revision, || assert_eq!(props(&ask("A")), ["own"])), (0, 0), "its own file still failed");
        let mut answer = None;
        assert_eq!(cost(&revision, || answer = Some(ask("B"))), (1, 1), "its own file was read since");
        assert_eq!(answer.unwrap(), cold(&root, "B"));
    }
}
