use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwapOption;
use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::types::*;

use super::discover::{discover_dirs, discover_files};
use super::revision::{empty_output, output_delta, ComponentAnswer, Revision};
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

/// Lockfiles: a change means the installed packages, and so what imports resolve to, may have changed.
const LOCKFILES: [&str; 6] =
    ["pnpm-lock.yaml", "package-lock.json", "npm-shrinkwrap.json", "yarn.lock", "bun.lock", "bun.lockb"];

/// Whether a file of this name configures module resolution: a tsconfig or jsconfig, a `package.json`, or a lockfile.
fn is_resolution_config_name(name: &str) -> bool {
    name == "package.json"
        || name == "jsconfig.json"
        || LOCKFILES.contains(&name)
        || (name.starts_with("tsconfig") && name.ends_with(".json"))
}

/// The source directories, canonicalized the way `discover_files` reports paths.
fn canonical_src_dirs(options: &PipelineOptions) -> Vec<Utf8PathBuf> {
    options.src_dirs.iter().map(|dir| canonicalize_best_effort(dir)).collect()
}

/// What `stat` reported for a file or directory when it was recorded. `Unknown` never matches a fresh stamp: the path
/// may have changed after the revision read it, so it must be applied again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stamp {
    Missing,
    /// `hash` is set when the modification time was too recent to tell a later edit apart: a hash of a file's
    /// contents, or of a directory's entry names.
    Present {
        modified: SystemTime,
        len: u64,
        hash: Option<u64>,
    },
    Unknown,
}

/// Filesystems that keep modification times to the second, or to a kernel clock tick, give an edit made just after a
/// stat the same modification time.
const MTIME_GRANULARITY: Duration = Duration::from_secs(2);

fn stamp(path: &Utf8Path) -> Stamp {
    stamp_like(path, None)
}

/// A fresh stamp comparable with `recorded`: hashed when `recorded` was, or when the modification time is recent.
fn stamp_like(path: &Utf8Path, recorded: Option<&Stamp>) -> Stamp {
    let now = SystemTime::now();
    let Ok(meta) = std::fs::metadata(path) else { return Stamp::Missing };
    let Ok(modified) = meta.modified() else { return Stamp::Unknown };
    let hashed = matches!(recorded, Some(Stamp::Present { hash: Some(_), .. })) || modified + MTIME_GRANULARITY >= now;
    let hash = if hashed {
        match content_hash(path, meta.is_dir()) {
            Some(hash) => Some(hash),
            None => return Stamp::Unknown,
        }
    } else {
        None
    };
    Stamp::Present { modified, len: meta.len(), hash }
}

fn content_hash(path: &Utf8Path, is_dir: bool) -> Option<u64> {
    let mut hasher = rustc_hash::FxHasher::default();
    if is_dir {
        let mut names: Vec<String> =
            path.read_dir_utf8().ok()?.flatten().map(|entry| entry.file_name().to_owned()).collect();
        names.sort();
        names.hash(&mut hasher);
    } else {
        std::fs::read(path).ok()?.hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// The stamp of a path the revision built at `built_at` read, or `Unknown` when it may have changed since that build
/// started.
fn stamp_since(path: &Utf8Path, built_at: SystemTime) -> Stamp {
    match stamp(path) {
        Stamp::Present { modified, .. } if modified + MTIME_GRANULARITY >= built_at => Stamp::Unknown,
        stamp => stamp,
    }
}

/// Stamping is a stat call per path, thousands of them for a large project: done in parallel here and in `ensure_fresh`.
fn stamps_of(paths: impl IntoIterator<Item = Utf8PathBuf>, built_at: SystemTime) -> FxHashMap<Utf8PathBuf, Stamp> {
    paths
        .into_iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|path| {
            let stamp = stamp_since(&path, built_at);
            (path, stamp)
        })
        .collect()
}

/// The stamps `ensure_fresh` compares against: the published revision's source files, every directory discovery
/// walks (a file created or deleted there changes the directory's stamp), and every resolution config.
struct Stamps {
    files: FxHashMap<Utf8PathBuf, Stamp>,
    dirs: FxHashMap<Utf8PathBuf, Stamp>,
    configs: FxHashMap<Utf8PathBuf, Stamp>,
    /// The resolution configs inside the source directories; the ones above them are listed again on every call.
    inner_configs: Vec<Utf8PathBuf>,
}

/// Writer-side state, guarded by the writer lock.
struct Freshness {
    /// When the cold extraction behind the published revision started.
    built_at: SystemTime,
    /// Recorded on the first `ensure_fresh` after a cold extraction, so sessions that never call it never pay for it.
    stamps: Option<Stamps>,
}

impl Freshness {
    fn cold(built_at: SystemTime) -> Self {
        Self { built_at, stamps: None }
    }
}

fn unchanged(revision: u64) -> IncrementalUpdate {
    IncrementalUpdate {
        revision,
        updated_components: Vec::new(),
        updated_keys: Vec::new(),
        removed_keys: Vec::new(),
        affected_files: Vec::new(),
        diagnostics: Vec::new(),
        removed_diagnostics: Vec::new(),
        duration_ms: 0,
    }
}

/// The keys several lazy updates, applied one after another, left changed or gone.
fn merge_lazy_updates(steps: Vec<IncrementalUpdate>, revision: u64, start: Instant) -> IncrementalUpdate {
    let mut updated: std::collections::BTreeSet<String> = Default::default();
    let mut removed: std::collections::BTreeSet<String> = Default::default();
    let mut affected_files: Vec<Utf8PathBuf> = Vec::new();
    for step in steps {
        for key in step.removed_keys {
            updated.remove(&key);
            removed.insert(key);
        }
        for key in step.updated_keys {
            removed.remove(&key);
            updated.insert(key);
        }
        affected_files.extend(step.affected_files);
    }
    affected_files.sort();
    affected_files.dedup();
    IncrementalUpdate {
        updated_keys: updated.into_iter().collect(),
        removed_keys: removed.into_iter().collect(),
        affected_files,
        duration_ms: start.elapsed().as_millis() as u64,
        ..unchanged(revision)
    }
}

/// Stateful session for incremental watch-mode extraction.
///
/// The session holds one immutable `Revision` at a time. `update_file` builds the next revision from the current one
/// (re-parsing the changed file, re-resolving the components whose resolution read it) and publishes it with a single
/// atomic store, so a reader always sees one complete revision, never parts of two. Writers are serialized; readers
/// never wait on them.
///
/// A lazy session ([`WatchSession::lazy`], ADR 0016) parses and indexes every file and resolves a component only when
/// [`WatchSession::component`] asks for it; the rest of this API behaves as for an eager one.
pub struct WatchSession {
    pub options: PipelineOptions,
    current: ArcSwapOption<Revision>,
    writer: Mutex<Freshness>,
    lazy: bool,
}

/// What a lazy session has indexed and answered so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LazyStats {
    pub revision: u64,
    pub files: usize,
    /// Components found by parsing.
    pub components: usize,
    /// Components resolved since the last edit that could change them.
    pub resolved: usize,
    /// Dependency declaration files loaded so far, for the names resolved components reached.
    pub dependency_files: usize,
}

impl WatchSession {
    /// Create an empty WatchSession (call `initialize()` before first use).
    pub fn new(options: PipelineOptions) -> Self {
        Self {
            options,
            current: ArcSwapOption::empty(),
            writer: Mutex::new(Freshness::cold(SystemTime::UNIX_EPOCH)),
            lazy: false,
        }
    }

    /// Create an empty lazy session (call `initialize()` before first use). Its `initialize` parses and indexes every
    /// file and returns an output with no components; [`Self::component`] resolves one on request.
    ///
    /// This is the entry point for a host that links the library and asks about one component at a time (an editor
    /// server, a Storybook addon, an MCP server). The session is `Send + Sync`: share it behind an `Arc` and call it
    /// from any thread. Answers are owned values with the component's own diagnostics; no parser type crosses the
    /// API. Memory is bounded by the project and the dependencies it reaches (ADR 0016).
    ///
    /// ```
    /// use caliper_docgen_core::pipeline::{PipelineOptions, WatchSession};
    ///
    /// # let dir = tempfile::tempdir()?;
    /// # std::fs::write(
    /// #     dir.path().join("Button.tsx"),
    /// #     "export function Button(props: { /** Shown on the button. */ label: string; size?: 'sm' | 'lg' }) { return null; }",
    /// # )?;
    /// # let root = camino::Utf8PathBuf::from(dir.path().to_str().unwrap());
    /// let session = WatchSession::lazy(PipelineOptions {
    ///     src_dirs: vec![root.clone()],
    ///     // Resolve the types the project imports from packages, only for the names a request reaches.
    ///     load_dependencies: true,
    ///     ..Default::default()
    /// });
    /// let _ = session.initialize(); // parses and indexes every file; resolves nothing
    ///
    /// // By key, or by file and name when the key isn't known.
    /// let button = session.component("Button").expect("Button is a component");
    /// assert_eq!(button.entry.props.keys().collect::<Vec<_>>(), ["label", "size"]);
    /// assert!(button.diagnostics.is_empty());
    /// assert_eq!(session.component_in(&root.join("Button.tsx"), "Button"), Some(button));
    ///
    /// // As react-docgen-typescript documents it, for Storybook's argTypes.
    /// let docgen = session.docgen_info("Button").expect("Button is a component");
    /// assert_eq!(docgen["displayName"], "Button");
    ///
    /// // After an edit, only the components that read the file are resolved again.
    /// std::fs::write(root.join("Button.tsx"), "export function Button(props: { label: string }) { return null; }")?;
    /// let update = session.update_file(&root.join("Button.tsx"));
    /// assert_eq!(update.updated_keys, ["Button"]);
    /// assert_eq!(session.component("Button").map(|b| b.entry.props.len()), Some(1));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn lazy(options: PipelineOptions) -> Self {
        Self { lazy: true, ..Self::new(options) }
    }

    /// Whether this session resolves components on request rather than all at once.
    pub fn is_lazy(&self) -> bool {
        self.lazy
    }

    /// Run a full cold extraction to populate this session.
    ///
    /// Idempotent — concurrent or repeated calls return the current snapshot without re-running extraction. Use
    /// `rebuild` to force a fresh extraction.
    pub fn initialize(&self) -> ExtractionOutput {
        let mut writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = self.current.load_full() {
            return (*current.output).clone();
        }
        let revision = self.cold(&mut writer, 1);
        (*revision.output).clone()
    }

    /// Discard the session's state and extract everything again.
    pub fn rebuild(&self) -> ExtractionOutput {
        let mut writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = self.current.load().as_ref().map_or(1, |r| r.id + 1);
        let revision = self.cold(&mut writer, id);
        (*revision.output).clone()
    }

    /// Extract everything as revision `id` and publish it. The caller holds the writer lock.
    fn cold(&self, writer: &mut Freshness, id: u64) -> Arc<Revision> {
        *writer = Freshness::cold(SystemTime::now());
        let revision = Arc::new(match self.lazy {
            true => Revision::cold_lazy(&self.options, id),
            false => Revision::cold(&self.options, id),
        });
        self.current.store(Some(revision.clone()));
        revision
    }

    /// The published revision's number: 0 before `initialize`, then one higher for every change that produced a new
    /// revision. An update that leaves it unchanged changed nothing.
    pub fn revision(&self) -> u64 {
        self.current.load().as_ref().map_or(0, |revision| revision.id)
    }

    /// The current output: exactly what a cold `extract()` of the same files would return. Empty before `initialize`.
    pub fn snapshot(&self) -> ExtractionOutput {
        match self.current.load_full() {
            Some(revision) => (*revision.output).clone(),
            None => empty_output(),
        }
    }

    /// The current output, shared rather than copied. `None` before `initialize`.
    pub fn current_output(&self) -> Option<Arc<ExtractionOutput>> {
        self.current.load_full().map(|revision| revision.output.clone())
    }

    /// The component published under `key`, with its own diagnostics. A lazy session resolves it now, or answers from
    /// what an earlier request resolved; the answer is the one a batch extraction of the same files publishes under
    /// that key. `None` for a key with no component, and before `initialize`.
    pub fn component(&self, key: &str) -> Option<ComponentAnswer> {
        let revision = self.current.load_full()?;
        match revision.is_lazy() {
            true => revision.lazy_component(&self.options, key),
            false => revision.published_component(key),
        }
    }

    /// The component `name` declared in `file`, found without knowing its key (which depends on what else the project
    /// names the same). `file` may be relative or reach the project through a symlink. `None` when the file declares
    /// no such component, and before `initialize`.
    pub fn component_in(&self, file: &Utf8Path, name: &str) -> Option<ComponentAnswer> {
        let file = canonicalize_best_effort(file);
        let revision = self.current.load_full()?;
        let key = match revision.is_lazy() {
            true => revision.lazy_key_in(&self.options, &file, name)?,
            false => revision
                .output
                .components
                .iter()
                .find(|(_, entry)| entry.display_name == name && entry.file_path == file)
                .map(|(key, _)| key.clone())?,
        };
        self.component(&key)
    }

    /// The component published under `key` as react-docgen-typescript documents it (`__docgenInfo`, the shape
    /// Storybook converts to argTypes), resolving it first in a lazy session.
    pub fn docgen_info(&self, key: &str) -> Option<serde_json::Value> {
        Some(super::rdt::docgen_info(&self.component(key)?.entry))
    }

    /// [`Self::component`] for each of `keys`, resolved in parallel.
    pub fn components(&self, keys: &[String]) -> Vec<Option<ComponentAnswer>> {
        use rayon::prelude::*;
        let Some(revision) = self.current.load_full() else { return vec![None; keys.len()] };
        match revision.is_lazy() {
            true => revision.lazy_components(&self.options, keys),
            false => keys.par_iter().map(|key| revision.published_component(key)).collect(),
        }
    }

    /// Every key [`Self::component`] answers, sorted. A lazy session resolves only the imported-member aliases to
    /// decide; the rest come from parsing alone.
    pub fn component_keys(&self) -> Vec<String> {
        let Some(revision) = self.current.load_full() else { return Vec::new() };
        match revision.is_lazy() {
            true => revision.lazy_keys(&self.options),
            false => revision.output.components.keys().cloned().collect(),
        }
    }

    /// What a lazy session has indexed and resolved. `None` for an eager session, and before `initialize`.
    pub fn lazy_stats(&self) -> Option<LazyStats> {
        let revision = self.current.load_full()?;
        let (components, resolved, dependency_files) = revision.lazy_counts()?;
        Some(LazyStats {
            revision: revision.id,
            files: revision.source_files().len(),
            components,
            resolved,
            dependency_files,
        })
    }

    /// Whether a change to `path` can alter how imports resolve, so the session must be rebuilt rather than updated: a
    /// tsconfig, jsconfig, `package.json` or lockfile in a source directory, or in a directory above one. Anything
    /// under `node_modules` is not the project's configuration.
    pub fn is_resolution_config(&self, path: &Utf8Path) -> bool {
        let path = canonicalize_best_effort(path);
        let (Some(name), Some(dir)) = (path.file_name(), path.parent()) else { return false };
        if !is_resolution_config_name(name) || path.components().any(|part| part.as_str() == "node_modules") {
            return false;
        }
        canonical_src_dirs(&self.options).iter().any(|src| src.starts_with(dir) || dir.starts_with(src))
    }

    /// The resolution configs that exist in the directories above each source directory, for a watcher to add to what
    /// it watches; ones inside a source directory are already watched with it. See [`Self::is_resolution_config`].
    pub fn resolution_config_files(&self) -> Vec<Utf8PathBuf> {
        let mut files: Vec<Utf8PathBuf> = canonical_src_dirs(&self.options)
            .iter()
            .flat_map(|src| src.ancestors().skip(1).map(Utf8Path::to_owned).collect::<Vec<_>>())
            .filter_map(|dir| dir.read_dir_utf8().ok())
            .flat_map(|entries| entries.flatten().map(|entry| entry.into_path()).collect::<Vec<_>>())
            .filter(|path| path.file_name().is_some_and(is_resolution_config_name) && path.is_file())
            .collect();
        files.sort();
        files.dedup();
        files
    }

    /// Apply one file change: an edit, a new file, or a deletion. A path that is not a source file of this project
    /// (outside every source directory, excluded, or not TypeScript) changes nothing. A resolution config
    /// ([`Self::is_resolution_config`]) rebuilds the session, and the update carries every component that changed.
    pub fn update_file(&self, changed: &Utf8Path) -> IncrementalUpdate {
        // `discover_files` canonicalizes the paths it reports (e.g. macOS's `/var` -> `/private/var`), so a watcher's
        // path must be canonicalized the same way to match.
        let changed = canonicalize_best_effort(changed);
        let rebuild = self.is_resolution_config(&changed);
        let mut writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = match self.current.load_full() {
            Some(current) => current,
            None => self.cold(&mut writer, 1),
        };
        if rebuild {
            return self.rebuilt(&mut writer, &current);
        }
        // Stamped before it is read, so an edit made while it is applied still looks changed to `ensure_fresh`.
        let before = stamp(&changed);
        match current.next(&self.options, &changed) {
            Some((next, update)) => {
                if let Some(stamps) = &mut writer.stamps {
                    if next.source_files().contains(&changed) {
                        stamps.files.insert(changed, before);
                    } else {
                        stamps.files.remove(&changed);
                    }
                }
                self.current.store(Some(Arc::new(next)));
                update
            }
            None => unchanged(current.id),
        }
    }

    /// Bring the session up to date with the files on disk, for a host that has no file watcher: stat every file the
    /// published revision read, every directory discovery walks and every resolution config, then apply what changed
    /// since the session last saw it. A changed resolution config rebuilds the session, as in `update_file`.
    ///
    /// The update takes a client holding the previous snapshot to the new one, exactly as the `update_file` calls for
    /// the same changes would have, one after another. The revision is unchanged when nothing changed. Before
    /// `initialize`, it extracts everything and reports every component as updated.
    ///
    /// A change is noticed by its modification time or size, and by its contents while the modification time is too
    /// recent to tell edits apart. The first call after a cold extraction applies again every file modified shortly
    /// before it started, which can raise the revision with an empty update.
    pub fn ensure_fresh(&self) -> IncrementalUpdate {
        let start = Instant::now();
        let mut writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = self.current.load_full() else {
            let revision = self.cold(&mut writer, 1);
            return output_delta(&empty_output(), &revision.output, revision.id, start);
        };
        let recorded = match writer.stamps.take() {
            Some(stamps) => stamps,
            None => self.record(&current, writer.built_at),
        };

        let dirs: FxHashMap<Utf8PathBuf, Stamp> =
            recorded.dirs.par_iter().map(|(dir, was)| (dir.clone(), stamp_like(dir, Some(was)))).collect();
        let (dirs, inner_configs, discovered) = if dirs == recorded.dirs {
            (dirs, recorded.inner_configs.clone(), None)
        } else {
            // Stamped before the walk, so a file created during it changes the stamp again.
            let (walked, inner_configs) =
                discover_dirs(&self.options.src_dirs, &self.options.exclude_patterns, is_resolution_config_name);
            let dirs = walked.into_iter().map(|dir| (dir.clone(), dirs.get(&dir).copied().unwrap_or(Stamp::Unknown)));
            let (files, _) = discover_files(&self.options.src_dirs, &self.options.exclude_patterns);
            (dirs.collect(), inner_configs, Some(files))
        };
        let configs = self.config_stamps(&inner_configs, |path| stamp_like(path, recorded.configs.get(path)));
        if configs != recorded.configs {
            return self.rebuilt(&mut writer, &current);
        }

        let mut files = recorded.files;
        let paths: Vec<&Utf8PathBuf> =
            files.keys().chain(discovered.iter().flatten().filter(|path| !files.contains_key(*path))).collect();
        let mut changed: Vec<(Utf8PathBuf, Stamp)> = paths
            .into_par_iter()
            .map(|path| (path, stamp_like(path, files.get(path))))
            .filter(|(path, now)| files.get(*path) != Some(now))
            .map(|(path, now)| (path.clone(), now))
            .collect();
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        let mut next = current.clone();
        let mut steps = Vec::new();
        for (path, now) in changed {
            if let Some((revision, update)) = next.next(&self.options, &path) {
                next = Arc::new(revision);
                steps.push(update);
            }
            if next.source_files().contains(&path) {
                files.insert(path, now);
            } else {
                files.remove(&path);
            }
        }
        writer.stamps = Some(Stamps { files, dirs, configs, inner_configs });
        if next.id == current.id {
            return unchanged(current.id);
        }
        self.current.store(Some(next.clone()));
        match next.is_lazy() {
            true => merge_lazy_updates(steps, next.id, start),
            false => output_delta(&current.output, &next.output, next.id, start),
        }
    }

    /// Rebuild from `current` after a resolution config changed. The caller holds the writer lock.
    fn rebuilt(&self, writer: &mut Freshness, current: &Revision) -> IncrementalUpdate {
        *writer = Freshness::cold(SystemTime::now());
        let (next, update) = current.rebuilt(&self.options);
        self.current.store(Some(Arc::new(next)));
        update
    }

    /// The first stamps after a cold extraction that started at `built_at`: anything modified since then, and any
    /// source file gone since, is `Unknown`.
    fn record(&self, revision: &Revision, built_at: SystemTime) -> Stamps {
        let mut files = stamps_of(revision.source_files().iter().cloned(), built_at);
        for stamp in files.values_mut().filter(|stamp| **stamp == Stamp::Missing) {
            *stamp = Stamp::Unknown;
        }
        let (dirs, inner_configs) =
            discover_dirs(&self.options.src_dirs, &self.options.exclude_patterns, is_resolution_config_name);
        let configs = self.config_stamps(&inner_configs, |path| stamp_since(path, built_at));
        Stamps { files, dirs: stamps_of(dirs, built_at), configs, inner_configs }
    }

    /// Every resolution config: the ones inside the source directories, and the ones above them as listed now.
    fn config_stamps(
        &self,
        inner_configs: &[Utf8PathBuf],
        stamp: impl Fn(&Utf8Path) -> Stamp + Sync,
    ) -> FxHashMap<Utf8PathBuf, Stamp> {
        inner_configs
            .iter()
            .cloned()
            .chain(self.resolution_config_files())
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|path| {
                let stamp = stamp(&path);
                (path, stamp)
            })
            .collect()
    }
}

#[cfg(test)]
mod lazy_tests;
#[cfg(test)]
mod tests;
