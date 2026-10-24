//! One complete extraction state, and the single function that turns it into output.
//!
//! `extract()` builds a revision from scratch. Watch mode builds each next revision from the previous one: it re-parses
//! the changed file and re-resolves the components whose resolution read it, then publishes the whole revision at once.
//! Both paths produce output through `assemble`, so key disambiguation, plugin hooks, diagnostics and enums cannot
//! drift between them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::cache::DtsCache;
use crate::dependency_types::{self, Demand, DependencyLayer};
use crate::named_type_index::NamedTypeIndex;
use crate::resolver::{Dependencies, ResolutionContext};
use crate::types::output::severity_rank;
use crate::types::*;

use super::discover::{discover_files, is_source_file, should_skip};

mod lazy;
use super::{
    canonicalize_first_src_dir, collect_public_enums, merge_cached_dts_file, resolve_guarded, HtmlAttributeMode,
    IncrementalUpdate, PipelineOptions,
};
pub use lazy::{ComponentAnswer, ComponentLocation};

/// `@types/react` indexed on its own, for curated mode's HTML-attribute lookups: a prop that an HTML layer's element
/// also declares has both types, as in Full mode, while every other name still resolves without React's declarations.
/// No `@types/react` means no lookups, as before, and no diagnostic: curated mode never needed it.
fn curated_react_types(
    options: &PipelineOptions,
    cache: &DtsCache,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Arc<NamedTypeIndex>> {
    let from_dir = canonicalize_first_src_dir(&options.src_dirs)?;
    let react_dts = Utf8PathBuf::from(crate::resolver::resolve_package_dts_path(&from_dir, "react", options)?);
    let mut react = GlobalSourceData::default();
    merge_cached_dts_file(&react_dts, cache, &mut react, diagnostics);
    Some(Arc::new(NamedTypeIndex::build(&react)))
}

/// A component's identity across revisions: its file, its name, and which same-named declaration in that file it is.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ComponentId {
    file: Utf8PathBuf,
    name: String,
    ordinal: u32,
}

/// A resolved component, the diagnostics its resolution produced, and what that resolution depended on.
pub(crate) struct Resolved {
    entry: ComponentEntry,
    diagnostics: ResolvedDiagnostics,
    depends_on: Dependencies,
}

impl Resolved {
    /// The entry and diagnostics, moved out when nothing else shares this resolution.
    fn into_parts(shared: Arc<Resolved>) -> (ComponentEntry, ResolvedDiagnostics) {
        match Arc::try_unwrap(shared) {
            Ok(resolved) => (resolved.entry, resolved.diagnostics),
            Err(shared) => (shared.entry.clone(), shared.diagnostics.clone()),
        }
    }

    /// Whether this resolution can differ after `change`.
    fn affected_by(&self, change: &Change<'_>) -> bool {
        self.depends_on.files.contains(change.file.as_str())
            || (change.created && self.depends_on.resolves_modules)
            || change.bare_names.iter().any(|name| self.depends_on.bare_names.contains(name.as_str()))
    }
}

/// One file's change, as far as dependency tracking cares.
struct Change<'c> {
    file: &'c Utf8Path,
    /// The file did not exist in the previous revision.
    created: bool,
    /// Enum and const-array names the file declared before or after the change.
    bare_names: FxHashSet<String>,
}

/// The bare names `file` declares in an enum or const-array map keyed `file:name`.
fn bare_names_declared<V>(file: &Utf8Path, map: &FxHashMap<String, V>) -> Vec<String> {
    let prefix = format!("{file}:");
    map.keys().filter_map(|key| key.strip_prefix(prefix.as_str()).map(str::to_owned)).collect()
}

pub(crate) struct Revision {
    pub(crate) id: u64,
    global: Arc<GlobalSourceData>,
    named_types: Arc<NamedTypeIndex>,
    /// The loaded dependency declarations, with `load_dependencies`.
    dependencies: Option<Arc<DependencyLayer>>,
    /// React's own declarations in curated mode, indexed apart for HTML-attribute lookups (ADR 0010).
    react_types: Option<Arc<NamedTypeIndex>>,
    ambient_global_files: Vec<Utf8PathBuf>,
    /// Source-directory and discovery diagnostics.
    head_diagnostics: Vec<Diagnostic>,
    /// Read, parse and file-plugin diagnostics, per source file.
    file_diagnostics: BTreeMap<Utf8PathBuf, Vec<Diagnostic>>,
    /// `@types/react` and TypeScript lib diagnostics.
    lib_diagnostics: Vec<Diagnostic>,
    /// The parse-cache save result.
    tail_diagnostics: Vec<Diagnostic>,
    resolved: FxHashMap<ComponentId, Arc<Resolved>>,
    /// The project's source files.
    files: Arc<FxHashSet<Utf8PathBuf>>,
    /// `.d.ts` files the cold run parsed or loaded from the parse cache, and how many the cache answered.
    dts_files_parsed: u32,
    dts_cache_hits: u32,
    duration_ms: u64,
    pub(crate) output: Arc<ExtractionOutput>,
    /// The output key each component was published under.
    keys: FxHashMap<ComponentId, String>,
    /// Set on a lazy revision (ADR 0016): components resolve on request and `resolved` stays empty.
    lazy: Option<lazy::Lazy>,
}

/// The mapping identities of `global`, in output order: by file, then by position in the file.
fn component_ids<'g>(
    global: &'g GlobalSourceData,
    options: &PipelineOptions,
) -> Vec<(ComponentId, &'g ComponentMapping)> {
    ordered_ids(global, options)
        .into_iter()
        .filter_map(|(id, index)| Some((id, global.component_mappings.get(index)?)))
        .collect()
}

/// [`component_ids`] with each mapping's position in `global.component_mappings` instead of the mapping.
fn ordered_ids(global: &GlobalSourceData, options: &PipelineOptions) -> Vec<(ComponentId, usize)> {
    let mut ordinals: FxHashMap<(&Utf8Path, &str), u32> = FxHashMap::default();
    let mut ids: Vec<(ComponentId, usize)> = global
        .component_mappings
        .iter()
        .enumerate()
        .filter(|(_, m)| !should_skip(&m.component_name, &options.exclude_prefixes))
        .map(|(index, m)| {
            let ordinal = ordinals.entry((m.file_path.as_path(), m.component_name.as_str())).or_insert(0);
            let id = ComponentId { file: m.file_path.clone(), name: m.component_name.clone(), ordinal: *ordinal };
            *ordinal += 1;
            (id, index)
        })
        .collect();
    let span = |index: &usize| global.component_mappings.get(*index).map(|m| m.span_start);
    ids.sort_by(|(a, ia), (b, ib)| (&a.file, span(ia), a.ordinal).cmp(&(&b.file, span(ib), b.ordinal)));
    ids
}

fn resolve_all(
    mappings: &[(ComponentId, &ComponentMapping)],
    ctx: &ResolutionContext,
) -> FxHashMap<ComponentId, Arc<Resolved>> {
    let borrowed: Vec<&ComponentMapping> = mappings.iter().map(|(_, m)| *m).collect();
    resolve_guarded(&borrowed, ctx)
        .into_iter()
        .zip(mappings)
        .map(|((entry, diagnostics, depends_on), (id, _))| {
            (id.clone(), Arc::new(Resolved { entry, diagnostics, depends_on }))
        })
        .collect()
}

/// What a cold run knows once every file is discovered, parsed and merged, before any dependency is loaded or any
/// component resolved. A lazy revision stops here.
struct Indexed {
    start: Instant,
    cache: Arc<DtsCache>,
    src_files: Vec<Utf8PathBuf>,
    global: GlobalSourceData,
    head_diagnostics: Vec<Diagnostic>,
    file_diagnostics: BTreeMap<Utf8PathBuf, Vec<Diagnostic>>,
    lib_diagnostics: Vec<Diagnostic>,
    react_types: Option<Arc<NamedTypeIndex>>,
    /// Each project file's dependency demands, in file order; empty without `load_dependencies`.
    file_demands: Vec<(Utf8PathBuf, Vec<Demand>)>,
}

impl Indexed {
    fn build(options: &PipelineOptions) -> Indexed {
        let start = Instant::now();
        let mut head_diagnostics = source_dir_diagnostics(options);

        let cache = Arc::new(DtsCache::load_from_disk(options.cache_dir.as_deref()));
        head_diagnostics.extend(cache.load_diagnostic().cloned());

        let (src_files, discover_diagnostics) = discover_files(&options.src_dirs, &options.exclude_patterns);
        head_diagnostics.extend(discover_diagnostics);

        // The whole closure body is wrapped in `contain_panic` so one file's parse panicking degrades to a per-file
        // diagnostic instead of poisoning the `.collect()` and crashing the whole extraction run.
        let parsed: Vec<(Utf8PathBuf, SourceData, Option<Diagnostic>)> = src_files
            .par_iter()
            .map(|path| {
                let label = format!("parse:{path}");
                crate::panic_guard::contain_panic(&label, || {
                    #[cfg(test)]
                    if path.file_name() == Some(super::PARSE_PANIC_TEST_SENTINEL) {
                        panic!("simulated parse-phase panic (test-only sentinel)");
                    }
                    let (data, io_diag) = parse_source_file(path, &cache);
                    (path.clone(), data, io_diag)
                })
                .unwrap_or_else(|diag| (path.clone(), SourceData::default(), Some(diag)))
            })
            .collect();

        let mut global = GlobalSourceData::default();
        let mut file_diagnostics: BTreeMap<Utf8PathBuf, Vec<Diagnostic>> = BTreeMap::new();
        let mut file_demands = Vec::new();
        for (path, mut data, io_diag) in parsed {
            if options.load_dependencies {
                file_demands.push((path.clone(), dependency_types::project_demands(&path, &data)));
            }
            let mut diags: Vec<Diagnostic> = io_diag.into_iter().collect();
            diags.append(&mut data.diagnostics);
            diags.extend(options.plugins.run_on_file_extracted(&path, &mut data));
            if !diags.is_empty() {
                file_diagnostics.insert(path.clone(), diags);
            }
            global.merge(&path, data);
        }

        let mut lib_diagnostics = Vec::new();
        // Full HTML attribute mode needs @types/react's real interfaces merged before resolution, so the resolver can
        // look them up like any other interface instead of only recording an InheritedLayer.
        if options.html_attributes == HtmlAttributeMode::Full {
            let from_dir = canonicalize_first_src_dir(&options.src_dirs).unwrap_or_else(|| Utf8PathBuf::from("."));
            match crate::resolver::resolve_package_dts_path(&from_dir, "react", options) {
                Some(react_dts_path) => {
                    let react_dts_path = Utf8PathBuf::from(react_dts_path);
                    merge_cached_dts_file(&react_dts_path, &cache, &mut global, &mut lib_diagnostics);
                }
                None => lib_diagnostics.push(
                    Diagnostic::new(
                        DiagnosticSeverity::Warning,
                        DiagnosticCode::MissingTypeLibrary,
                        "HtmlAttributeMode::Full requested but @types/react could not be resolved",
                    )
                    .with_help("Check that @types/react is installed. Falling back to curated attributes."),
                ),
            }
        }
        let react_types = (options.html_attributes == HtmlAttributeMode::Curated)
            .then(|| curated_react_types(options, &cache, &mut lib_diagnostics))
            .flatten();
        // Ambient globals (Date, Element, ...) live in TS's lib files and are never imported, so merge them in every
        // mode. An install missing a lib file (TS 7 without its platform package) is reported here.
        if let Some(from_dir) = canonicalize_first_src_dir(&options.src_dirs) {
            let libs = crate::resolver::resolve_ts_lib_paths(&from_dir);
            lib_diagnostics.extend(libs.diagnostic);
            for lib_path in libs.paths {
                merge_cached_dts_file(&Utf8PathBuf::from(lib_path), &cache, &mut global, &mut lib_diagnostics);
            }
        }

        Indexed {
            start,
            cache,
            src_files,
            global,
            head_diagnostics,
            file_diagnostics,
            lib_diagnostics,
            react_types,
            file_demands,
        }
    }
}

/// One file's change applied to a copy of a revision's parsed state.
struct Applied<'c> {
    global: GlobalSourceData,
    named_types: NamedTypeIndex,
    file_diagnostics: BTreeMap<Utf8PathBuf, Vec<Diagnostic>>,
    files: FxHashSet<Utf8PathBuf>,
    dependencies: Option<Arc<DependencyLayer>>,
    change: Change<'c>,
    /// The changed file's dependency demands after the change; empty when it was deleted or demands are not loaded.
    demands: Vec<Demand>,
}

impl Revision {
    /// This revision's parsed state with `changed` re-read: re-parsed, created or deleted. With `extend_dependencies`,
    /// names the file newly references are loaded now; a lazy revision loads them when a component needs them.
    fn apply_change<'c>(
        &self,
        options: &PipelineOptions,
        changed: &'c Utf8Path,
        extend_dependencies: bool,
    ) -> Applied<'c> {
        let mut global = (*self.global).clone();
        let mut named_types = (*self.named_types).clone();
        let mut file_diagnostics = self.file_diagnostics.clone();
        let mut files = (*self.files).clone();
        let mut dependencies = self.dependencies.clone();
        let mut demands = Vec::new();
        let mut bare_names: FxHashSet<String> = bare_names_declared(changed, &self.global.enums).into_iter().collect();
        bare_names.extend(bare_names_declared(changed, &self.global.const_arrays));
        global.remove_file(changed);
        named_types.remove_file(changed);
        file_diagnostics.remove(changed);

        let read = match std::fs::read_to_string(changed) {
            Ok(source) => Some((source, None)),
            // Deleted: its declarations and diagnostics leave with it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            // Unreadable (permissions, invalid UTF-8): declares nothing, exactly as in a cold run.
            Err(e) => Some((String::new(), Some(Diagnostic::io_read_error(changed, &e)))),
        };
        let created = match read {
            None => {
                files.remove(changed);
                false
            }
            Some((source, io_diag)) => {
                let mut data = crate::extractor::parse_file(changed, &source);
                let mut diags: Vec<Diagnostic> = io_diag.into_iter().collect();
                diags.append(&mut data.diagnostics);
                diags.extend(options.plugins.run_on_file_extracted(changed, &mut data));
                if !diags.is_empty() {
                    file_diagnostics.insert(changed.to_owned(), diags);
                }
                bare_names.extend(bare_names_declared(changed, &data.enums));
                bare_names.extend(bare_names_declared(changed, &data.const_arrays));
                if options.load_dependencies {
                    demands = dependency_types::project_demands(changed, &data);
                }
                // Only names no earlier revision loaded cost a load; the new layer keeps everything the old one had.
                if let (true, Some(layer)) = (extend_dependencies, &self.dependencies) {
                    let missing = layer.missing(&demands);
                    if !missing.is_empty() {
                        let extended = dependency_types::load(missing, options, Some(layer));
                        named_types.fallback = Some(extended.named_types.clone());
                        dependencies = Some(Arc::new(extended));
                    }
                }
                named_types.merge_file(&data);
                global.merge(changed, data);
                files.insert(changed.to_owned())
            }
        };
        Applied {
            global,
            named_types,
            file_diagnostics,
            files,
            dependencies,
            change: Change { file: changed, created, bare_names },
            demands,
        }
    }
}

impl Revision {
    /// Discover, parse, merge, resolve and assemble every source file.
    pub(crate) fn cold(options: &PipelineOptions, id: u64) -> Revision {
        let mut revision = Revision::resolve_cold(options, id);
        let (output, keys) = revision
            .assemble(options, |id| revision.resolved.get(id).map(|r| (r.entry.clone(), r.diagnostics.clone())));
        revision.output = Arc::new(output);
        revision.keys = keys;
        revision
    }

    /// The output of a cold run that is never updated, so each entry moves into it instead of being copied.
    pub(crate) fn cold_output(options: &PipelineOptions) -> ExtractionOutput {
        let mut revision = Revision::resolve_cold(options, 0);
        let mut resolved = std::mem::take(&mut revision.resolved);
        let (output, _) = revision.assemble(options, |id| resolved.remove(id).map(Resolved::into_parts));
        // Freeing the parsed project takes milliseconds on a large one, and the caller only wants the output.
        let spawned = std::thread::Builder::new().name("caliper-drop".into()).spawn(move || drop(revision));
        // A failed spawn drops the closure, and the revision with it, right here; the output is unaffected.
        drop(spawned);
        output
    }

    /// Everything [`Revision::cold`] does except assembling the output.
    fn resolve_cold(options: &PipelineOptions, id: u64) -> Revision {
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
        let demands: Vec<Demand> = file_demands.into_iter().flat_map(|(_, demands)| demands).collect();

        let dependencies = options.load_dependencies.then(|| Arc::new(dependency_types::load(demands, options, None)));
        let global = Arc::new(global);
        let mut named_types = NamedTypeIndex::build(&global);
        named_types.fallback = dependencies.as_ref().map(|layer| layer.named_types.clone());
        let named_types = Arc::new(named_types);
        let ambient_global_files = crate::resolver::compute_ambient_global_files(options);
        let ctx = ResolutionContext::new_with_cached_ambient_global_files(
            global.clone(),
            options,
            ambient_global_files.clone(),
            named_types.clone(),
        )
        .with_dependencies(dependencies.clone())
        .with_react_types(react_types.clone());
        lib_diagnostics.extend(ctx.tsconfig_diagnostic.clone());
        let resolved = resolve_all(&component_ids(&global, options), &ctx);
        let files = Arc::new(src_files.into_iter().collect());

        let (dts_files_parsed, dts_cache_hits) = cache.counts();
        // Arc::try_unwrap succeeds because all rayon workers have finished.
        let tail_diagnostics: Vec<Diagnostic> = match Arc::try_unwrap(cache) {
            Ok(c) => c.save_to_disk(),
            Err(arc) => arc.save_to_disk(),
        }
        .into_iter()
        .collect();

        Revision {
            id,
            global,
            named_types,
            dependencies,
            react_types,
            ambient_global_files,
            head_diagnostics,
            file_diagnostics,
            lib_diagnostics,
            tail_diagnostics,
            resolved,
            files,
            dts_files_parsed,
            dts_cache_hits,
            duration_ms: start.elapsed().as_millis() as u64,
            output: Arc::new(empty_output()),
            keys: FxHashMap::default(),
            lazy: None,
        }
    }

    /// The revision after `changed` was edited, created or deleted, and the delta from this one. `None` when the path
    /// is not a source file of this project (outside every source directory, excluded, or not TypeScript).
    pub(crate) fn next(&self, options: &PipelineOptions, changed: &Utf8Path) -> Option<(Revision, IncrementalUpdate)> {
        if self.lazy.is_some() {
            return self.next_lazy(options, changed);
        }
        if !is_source_file(changed, &options.src_dirs, &options.exclude_patterns) {
            return None;
        }
        let start = Instant::now();

        let Applied { global, named_types, file_diagnostics, files, dependencies, change, .. } =
            self.apply_change(options, changed, true);

        let global = Arc::new(global);
        let named_types = Arc::new(named_types);
        let ids = component_ids(&global, options);
        let (stale, kept): (Vec<_>, Vec<_>) = ids.into_iter().partition(|(id, _)| match self.resolved.get(id) {
            None => true,
            Some(previous) => id.file == changed || previous.affected_by(&change),
        });
        let ctx = ResolutionContext::new_with_cached_ambient_global_files(
            global.clone(),
            options,
            self.ambient_global_files.clone(),
            named_types.clone(),
        )
        .with_dependencies(dependencies.clone())
        .with_react_types(self.react_types.clone());
        let fresh = resolve_all(&stale, &ctx);
        let mut resolved: FxHashMap<ComponentId, Arc<Resolved>> =
            kept.into_iter().filter_map(|(id, _)| self.resolved.get(&id).map(|r| (id, r.clone()))).collect();
        let fresh_ids: Vec<ComponentId> = fresh.keys().cloned().collect();
        resolved.extend(fresh);

        let mut next = Revision {
            id: self.id + 1,
            global,
            named_types,
            dependencies,
            react_types: self.react_types.clone(),
            ambient_global_files: self.ambient_global_files.clone(),
            head_diagnostics: self.head_diagnostics.clone(),
            file_diagnostics,
            lib_diagnostics: self.lib_diagnostics.clone(),
            tail_diagnostics: self.tail_diagnostics.clone(),
            resolved,
            files: Arc::new(files),
            dts_files_parsed: self.dts_files_parsed,
            dts_cache_hits: self.dts_cache_hits,
            duration_ms: self.duration_ms,
            output: Arc::new(empty_output()),
            keys: FxHashMap::default(),
            lazy: None,
        };
        let (output, keys) =
            next.assemble(options, |id| next.resolved.get(id).map(|r| (r.entry.clone(), r.diagnostics.clone())));
        next.output = Arc::new(output);
        next.keys = keys;

        // Updated: every key whose component this revision re-resolved, plus every kept component whose key moved (a
        // same-named component elsewhere appeared or disappeared, so the bare and disambiguated keys swapped).
        let fresh: FxHashSet<&ComponentId> = fresh_ids.iter().collect();
        let mut updated_keys: Vec<&String> = next
            .keys
            .iter()
            .filter(|(id, key)| fresh.contains(id) || self.keys.get(*id) != Some(*key))
            .map(|(_, key)| key)
            .collect();
        updated_keys.sort();
        updated_keys.dedup();
        let updated_components: Vec<(String, ComponentEntry)> = updated_keys
            .into_iter()
            .filter_map(|key| Some((key.clone(), next.output.components.get(key)?.clone())))
            .collect();
        let removed_keys: Vec<String> =
            self.output.components.keys().filter(|key| !next.output.components.contains_key(*key)).cloned().collect();
        let mut affected_files: Vec<Utf8PathBuf> = fresh_ids.iter().map(|id| id.file.clone()).collect();
        affected_files.sort();
        affected_files.dedup();
        let before: FxHashSet<&Diagnostic> = self.output.diagnostics.iter().collect();
        let after: FxHashSet<&Diagnostic> = next.output.diagnostics.iter().collect();
        let diagnostics: Vec<Diagnostic> =
            next.output.diagnostics.iter().filter(|d| !before.contains(d)).cloned().collect();
        let removed_diagnostics: Vec<Diagnostic> =
            self.output.diagnostics.iter().filter(|d| !after.contains(d)).cloned().collect();
        let update = IncrementalUpdate {
            revision: next.id,
            updated_components: updated_components.iter().map(|(_, entry)| entry.clone()).collect(),
            updated_keys: updated_components.into_iter().map(|(key, _)| key).collect(),
            removed_keys,
            affected_files,
            diagnostics,
            removed_diagnostics,
            duration_ms: start.elapsed().as_millis() as u64,
        };
        Some((next, update))
    }

    /// The revision after a change that can alter how every import resolves (a tsconfig, `package.json` or lockfile),
    /// rebuilt from scratch, and the delta from this one: every component whose entry differs, every key that went
    /// away, every diagnostic added or dropped.
    pub(crate) fn rebuilt(&self, options: &PipelineOptions) -> (Revision, IncrementalUpdate) {
        if let Some(rebuilt) = self.rebuilt_lazy(options) {
            return rebuilt;
        }
        let start = Instant::now();
        let next = Revision::cold(options, self.id + 1);
        let update = output_delta(&self.output, &next.output, next.id, start);
        (next, update)
    }

    /// The project's source files this revision parsed.
    pub(crate) fn source_files(&self) -> &FxHashSet<Utf8PathBuf> {
        &self.files
    }

    /// Build the public output from the revision, and the key each component was published under: components keyed
    /// with the same disambiguation rule everywhere, plugin hooks applied, diagnostics in a stable order, public
    /// enums. `take` yields a component's resolved entry and diagnostics, copied or moved out.
    fn assemble(
        &self,
        options: &PipelineOptions,
        mut take: impl FnMut(&ComponentId) -> Option<(ComponentEntry, ResolvedDiagnostics)>,
    ) -> (ExtractionOutput, FxHashMap<ComponentId, String>) {
        let mut diagnostics = DiagnosticSet::default();
        diagnostics.extend(self.head_diagnostics.iter().cloned(), None);
        for diags in self.file_diagnostics.values() {
            diagnostics.extend(diags.iter().cloned(), None);
        }
        diagnostics.extend(self.lib_diagnostics.iter().cloned(), None);

        let mut components = BTreeMap::new();
        let mut keys = FxHashMap::default();
        let mut seen_names: FxHashMap<String, u32> = FxHashMap::default();
        for (id, _) in component_ids(&self.global, options) {
            let Some((mut entry, resolved_diagnostics)) = take(&id) else { continue };
            // An alias of an imported component whose package isn't loaded has nothing to document.
            if entry.detected_by == DetectionRule::ImportedMemberAlias
                && entry.props.is_empty()
                && entry.inheritance.is_empty()
            {
                continue;
            }
            let base_name = entry.display_name.clone();
            let count = seen_names.entry(base_name.clone()).or_insert(0);
            *count += 1;
            // The first occurrence of a name keeps it bare; later ones take their file path, which is unique per file.
            let key = if *count == 1 { base_name } else { format!("{} ({})", base_name, entry.file_path) };
            diagnostics.extend(options.plugins.run_on_component_resolved(&mut entry), Some(&key));
            let new_file_path = entry.file_path.clone();
            if let Some(previous) = components.insert(key.clone(), entry) {
                diagnostics.add(
                    Diagnostic::new(
                        DiagnosticSeverity::Warning,
                        DiagnosticCode::ComponentKeyCollision,
                        format!(
                            "Duplicate component key '{key}' — colliding file paths: previously '{}', now '{}'",
                            previous.file_path, new_file_path
                        ),
                    )
                    .with_file(new_file_path.to_string())
                    .with_help(
                        "Two resolved components produced the same display name and disambiguation key — only the \
                         later one is kept in the output. Check for overlapping src_dirs or a genuine duplicate \
                         declaration.",
                    ),
                    None,
                );
            }
            diagnostics.extend(resolved_diagnostics.own, Some(&key));
            for chunk in &resolved_diagnostics.shared {
                diagnostics.extend_shared(chunk, &key);
            }
            keys.insert(id, key);
        }

        let (enums, enum_diagnostics) = collect_public_enums(&self.global);
        diagnostics.extend(enum_diagnostics, None);
        diagnostics.extend(self.tail_diagnostics.iter().cloned(), None);

        let components_extracted = components.len() as u32;
        let output = ExtractionOutput {
            components,
            enums,
            diagnostics: diagnostics.finish(),
            stats: ExtractionStats {
                components_extracted,
                files_parsed: self.files.len() as u32,
                dts_files_parsed: self.dts_files_parsed,
                dts_cache_hits: self.dts_cache_hits,
                duration_ms: self.duration_ms,
                ..Default::default()
            },
        };
        (output, keys)
    }
}

/// The delta that takes a client holding `previous` to `next`, published as `revision`: every component whose entry
/// differs, every key that went away, every diagnostic added or dropped. Revisions built one change at a time report
/// their own delta from `next`; this one is for a rebuild or several changes applied together.
pub(crate) fn output_delta(
    previous: &ExtractionOutput,
    next: &ExtractionOutput,
    revision: u64,
    start: Instant,
) -> IncrementalUpdate {
    let updated: Vec<(&String, &ComponentEntry)> =
        next.components.iter().filter(|(key, entry)| previous.components.get(*key) != Some(*entry)).collect();
    let removed_keys: Vec<String> =
        previous.components.keys().filter(|key| !next.components.contains_key(*key)).cloned().collect();
    let mut affected_files: Vec<Utf8PathBuf> = updated.iter().map(|(_, entry)| entry.file_path.clone()).collect();
    affected_files.sort();
    affected_files.dedup();
    let before: FxHashSet<&Diagnostic> = previous.diagnostics.iter().collect();
    let after: FxHashSet<&Diagnostic> = next.diagnostics.iter().collect();
    IncrementalUpdate {
        revision,
        updated_components: updated.iter().map(|(_, entry)| (*entry).clone()).collect(),
        updated_keys: updated.iter().map(|(key, _)| (*key).clone()).collect(),
        removed_keys,
        affected_files,
        diagnostics: next.diagnostics.iter().filter(|d| !before.contains(d)).cloned().collect(),
        removed_diagnostics: previous.diagnostics.iter().filter(|d| !after.contains(d)).cloned().collect(),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Diagnostics merged by identity: an identical diagnostic reported while resolving many components is kept once,
/// listing those components in `affects`.
#[derive(Default)]
struct DiagnosticSet {
    index: FxHashMap<Diagnostic, usize>,
    items: Vec<Diagnostic>,
    /// Where each shared chunk's diagnostics are in `items`, so a chunk many components share is hashed once.
    chunks: FxHashMap<*const Diagnostic, Vec<usize>>,
}

impl DiagnosticSet {
    fn add(&mut self, diagnostic: Diagnostic, component: Option<&str>) {
        let at = match self.index.get(&diagnostic) {
            Some(&at) => at,
            None => {
                self.index.insert(diagnostic.clone(), self.items.len());
                self.items.push(diagnostic);
                self.items.len() - 1
            }
        };
        if let Some(key) = component {
            self.items[at].affects.push(key.to_owned());
        }
    }

    fn extend_shared(&mut self, chunk: &Arc<[Diagnostic]>, component: &str) {
        let at = match self.chunks.get(&chunk.as_ptr()) {
            Some(at) => at.clone(),
            None => {
                let at: Vec<usize> = chunk
                    .iter()
                    .map(|diagnostic| match self.index.get(diagnostic) {
                        Some(&at) => at,
                        None => {
                            self.index.insert(diagnostic.clone(), self.items.len());
                            self.items.push(diagnostic.clone());
                            self.items.len() - 1
                        }
                    })
                    .collect();
                self.chunks.insert(chunk.as_ptr(), at.clone());
                at
            }
        };
        for at in at {
            self.items[at].affects.push(component.to_owned());
        }
    }

    fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>, component: Option<&str>) {
        for diagnostic in diagnostics {
            self.add(diagnostic, component);
        }
    }

    /// The diagnostics by file, position and message; run-wide ones (no file) first.
    fn finish(self) -> Vec<Diagnostic> {
        let mut items = self.items;
        for diagnostic in &mut items {
            diagnostic.affects.sort();
            diagnostic.affects.dedup();
        }
        // Severity by explicit rank, worst first, and code by name: neither enum's declaration order is an ordering.
        items.sort_by(|a, b| {
            (&a.file, a.line, a.column, &a.message)
                .cmp(&(&b.file, b.line, b.column, &b.message))
                .then_with(|| severity_rank(&b.severity).cmp(&severity_rank(&a.severity)))
                .then_with(|| format!("{:?}", a.code).cmp(&format!("{:?}", b.code)))
                .then_with(|| a.help.cmp(&b.help))
        });
        items
    }
}

/// Read and parse one source file, through the parse cache for `.d.ts`.
fn parse_source_file(path: &Utf8Path, cache: &DtsCache) -> (SourceData, Option<Diagnostic>) {
    let is_dts = path.as_str().ends_with(".d.ts");
    let (source, io_diag) = match std::fs::read_to_string(path) {
        Ok(s) => (s, None),
        Err(e) => (String::new(), Some(Diagnostic::io_read_error(path, &e))),
    };
    if is_dts {
        if let Some(cached) = cache.get(path, &source) {
            return (cached, io_diag);
        }
    }
    let data = crate::extractor::parse_file(path, &source);
    if is_dts {
        cache.insert(path, &source, data.clone());
    }
    (data, io_diag)
}

/// A typo'd `--src` (or a stale config `srcDirs`) must not silently produce an empty-but-valid-looking result.
fn source_dir_diagnostics(options: &PipelineOptions) -> Vec<Diagnostic> {
    let missing: Vec<&Utf8PathBuf> = options.src_dirs.iter().filter(|dir| !dir.as_std_path().is_dir()).collect();
    let help = "Check --src (or docgen.config.ts srcDirs) for a typo'd or stale path.";
    if options.src_dirs.is_empty() {
        return vec![Diagnostic::new(
            DiagnosticSeverity::Error,
            DiagnosticCode::IoError,
            "No source directories configured — src_dirs is empty",
        )
        .with_help("Set --src (or docgen.config.ts srcDirs) to at least one directory to scan.")];
    }
    if missing.len() == options.src_dirs.len() {
        let mut diag = Diagnostic::new(
            DiagnosticSeverity::Error,
            DiagnosticCode::IoError,
            format!(
                "None of the configured source directories exist: {}",
                options.src_dirs.iter().map(|dir| dir.as_str()).collect::<Vec<_>>().join(", ")
            ),
        )
        .with_help(help);
        if let Some(first) = options.src_dirs.first() {
            diag = diag.with_file(first.to_string());
        }
        return vec![diag];
    }
    missing
        .into_iter()
        .map(|dir| {
            Diagnostic::new(
                DiagnosticSeverity::Warning,
                DiagnosticCode::IoError,
                format!("Source directory does not exist: {dir}"),
            )
            .with_file(dir.to_string())
            .with_help(help)
        })
        .collect()
}

pub(crate) fn empty_output() -> ExtractionOutput {
    ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: Vec::new(),
        stats: ExtractionStats::default(),
    }
}
