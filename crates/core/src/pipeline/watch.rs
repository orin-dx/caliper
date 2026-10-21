use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use camino::{Utf8Path, Utf8PathBuf};

use crate::types::*;

use super::revision::Revision;
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
/// The session holds one immutable `Revision` at a time. `update_file` builds the next revision from the current one
/// (re-parsing the changed file, re-resolving the components whose resolution read it) and publishes it with a single
/// atomic store, so a reader always sees one complete revision, never parts of two. Writers are serialized; readers
/// never wait on them.
pub struct WatchSession {
    pub options: PipelineOptions,
    current: ArcSwapOption<Revision>,
    writer: Mutex<()>,
}

impl WatchSession {
    /// Create an empty WatchSession (call `initialize()` before first use).
    pub fn new(options: PipelineOptions) -> Self {
        Self { options, current: ArcSwapOption::empty(), writer: Mutex::new(()) }
    }

    /// Run a full cold extraction to populate this session.
    ///
    /// Idempotent — concurrent or repeated calls return the current snapshot without re-running extraction. Use
    /// `rebuild` to force a fresh extraction.
    pub fn initialize(&self) -> ExtractionOutput {
        let _writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = self.current.load_full() {
            return (*current.output).clone();
        }
        let revision = Arc::new(Revision::cold(&self.options, 1));
        let output = (*revision.output).clone();
        self.current.store(Some(revision));
        output
    }

    /// Discard the session's state and extract everything again.
    pub fn rebuild(&self) -> ExtractionOutput {
        let _writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = self.current.load().as_ref().map_or(1, |r| r.id + 1);
        let revision = Arc::new(Revision::cold(&self.options, id));
        let output = (*revision.output).clone();
        self.current.store(Some(revision));
        output
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
            None => ExtractionOutput {
                components: BTreeMap::new(),
                enums: BTreeMap::new(),
                diagnostics: Vec::new(),
                stats: ExtractionStats::default(),
            },
        }
    }

    /// Apply one file change: an edit, a new file, or a deletion. A path that is not a source file of this project
    /// (outside every source directory, excluded, or not TypeScript) changes nothing.
    pub fn update_file(&self, changed: &Utf8Path) -> IncrementalUpdate {
        // `discover_files` canonicalizes the paths it reports (e.g. macOS's `/var` -> `/private/var`), so a watcher's
        // path must be canonicalized the same way to match.
        let changed = canonicalize_best_effort(changed);
        let _writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = match self.current.load_full() {
            Some(current) => current,
            None => {
                let revision = Arc::new(Revision::cold(&self.options, 1));
                self.current.store(Some(revision.clone()));
                revision
            }
        };
        match current.next(&self.options, &changed) {
            Some((next, update)) => {
                self.current.store(Some(Arc::new(next)));
                update
            }
            None => IncrementalUpdate {
                revision: current.id,
                updated_components: Vec::new(),
                updated_keys: Vec::new(),
                removed_keys: Vec::new(),
                affected_files: Vec::new(),
                diagnostics: Vec::new(),
                removed_diagnostics: Vec::new(),
                duration_ms: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests;
