//! Extraction pipeline — Phase 3b.
//!
//! Orchestrates: file discovery → parallel parse → global merge → parallel resolve → output.
//! Also manages DTS cache and incremental watch-mode state.

use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use serde::Serialize;

use crate::cache::DtsCache;
use crate::resolver::ResolutionContext;
use crate::types::*;

mod discover;
pub mod rdt;
mod revision;
mod semantic_hook;
pub mod watch;

pub use crate::types::options::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions};
pub use revision::{ComponentAnswer, ComponentLocation};
pub use watch::{LazyStats, WatchSession};

/// Filename that trips a simulated panic in the Phase 2 parse closure —
/// exists only to let tests exercise `panic_guard::contain_panic`'s
/// containment without a plugin hook to inject a real one through.
#[cfg(test)]
const PARSE_PANIC_TEST_SENTINEL: &str = "__PANIC_TEST__.tsx";

/// Component name that trips a simulated panic in the Phase 4 resolve
/// closure — same rationale as `PARSE_PANIC_TEST_SENTINEL`, but for
/// `resolve_component`, which can't be made to panic without editing
/// non-test resolver code. Must be PascalCase — `is_pascal_case` is how the
/// extractor decides a function declaration is a component in the first
/// place, so a non-PascalCase sentinel would never reach Phase 4 at all.
#[cfg(test)]
const RESOLVE_PANIC_TEST_SENTINEL: &str = "ResolvePanicTestSentinel";

// ─── Incremental watch types ──────────────────────────────────────────────────

/// Result of processing a single file change in watch mode. Its JSON writes the updated components' props as ids, with
/// a `props` table holding every prop they refer to (ADR 0009).
#[derive(Debug)]
pub struct IncrementalUpdate {
    /// The session revision this update produced; a consumer can drop an update older than one it already applied.
    pub revision: u64,
    /// The current entry of every key this change affected, in the same order as `updated_keys`.
    pub updated_components: Vec<ComponentEntry>,
    /// Keys whose entry changed: re-resolved components, and kept ones whose key moved when a same-named component
    /// elsewhere appeared or disappeared (a bare name, or `Name (file)` when disambiguated).
    pub updated_keys: Vec<String>,
    /// Output keys that no longer exist: the component was renamed, deleted, or stopped being a component.
    pub removed_keys: Vec<String>,
    /// The files of the components in `updated_components`, sorted and without duplicates.
    pub affected_files: Vec<Utf8PathBuf>,
    /// Diagnostics this revision added. A client holding the previous output applies `removed_keys`, then
    /// `updated_keys`, then drops `removed_diagnostics` and adds these, and holds the new output.
    pub diagnostics: Vec<Diagnostic>,
    /// Diagnostics the previous revision had and this one does not: fixed, or replaced by a changed one.
    pub removed_diagnostics: Vec<Diagnostic>,
    /// How long building this revision took, in milliseconds.
    pub duration_ms: u64,
}

impl Serialize for IncrementalUpdate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use crate::types::output::interned::{InternedComponent, Tables, intern};
        use serde::ser::SerializeStruct;

        let IncrementalUpdate {
            revision,
            updated_components,
            updated_keys,
            removed_keys,
            affected_files,
            diagnostics,
            removed_diagnostics,
            duration_ms,
        } = self;
        let entries: Vec<&ComponentEntry> = updated_components.iter().collect();
        let (ids, mut tables) = intern::<S::Error>(&entries)?;
        let added = tables.intern_affects::<S::Error>(diagnostics)?;
        let removed = tables.intern_affects::<S::Error>(removed_diagnostics)?;
        let components: Vec<InternedComponent<'_>> =
            entries.iter().zip(&ids).map(|(entry, ids)| InternedComponent { entry, ids }).collect();
        let mut out = serializer.serialize_struct("IncrementalUpdate", 9 + Tables::FIELDS)?;
        out.serialize_field("schemaVersion", &crate::types::SCHEMA_VERSION)?;
        out.serialize_field("revision", revision)?;
        out.serialize_field("updatedComponents", &components)?;
        tables.write(&mut out)?;
        out.serialize_field("updatedKeys", updated_keys)?;
        out.serialize_field("removedKeys", removed_keys)?;
        out.serialize_field("affectedFiles", affected_files)?;
        out.serialize_field("diagnostics", &added)?;
        out.serialize_field("removedDiagnostics", &removed)?;
        out.serialize_field("durationMs", duration_ms)?;
        out.end()
    }
}

/// A session's index: where each component lives and the run's diagnostics, without props. A client that routes by file
/// and name reads this instead of the whole output.
struct SessionIndex<'a> {
    locations: &'a [ComponentLocation],
    output: &'a ExtractionOutput,
}

impl Serialize for SessionIndex<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use crate::types::output::interned::{InternedDiagnostics, NameLists};
        use serde::ser::SerializeStruct;

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Location<'a> {
            file_path: &'a str,
            display_name: &'a str,
            exports: &'a [crate::types::ComponentExport],
        }

        let mut lists = NameLists::new();
        let diagnostics = InternedDiagnostics::new::<S::Error>(&mut lists, &self.output.diagnostics)?;
        let locations: std::collections::BTreeMap<&str, Location<'_>> = self
            .locations
            .iter()
            .map(|at| {
                let location =
                    Location { file_path: at.file_path.as_str(), display_name: &at.display_name, exports: &at.exports };
                (at.key.as_str(), location)
            })
            .collect();
        let mut out = serializer.serialize_struct("SessionIndex", 4)?;
        out.serialize_field("schemaVersion", &crate::types::SCHEMA_VERSION)?;
        out.serialize_field("components", &locations)?;
        out.serialize_field("nameLists", &lists)?;
        out.serialize_field("diagnostics", &diagnostics)?;
        out.end()
    }
}

/// Serialize a session's index to a JSON string: `schemaVersion`, where each component lives (`components`), and the
/// run's `diagnostics` with the `nameLists` they refer to. The props stay in the session. `locations` is
/// [`WatchSession::component_index`]: a lazy session's output has no components, so the index cannot come from it.
pub fn session_index_to_json(
    locations: &[ComponentLocation],
    output: &ExtractionOutput,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SessionIndex { locations, output })
}

// ─── JSON serialization helpers ──────────────────────────────────────────────

/// Serialize an [`ExtractionOutput`] to a JSON string.
///
/// Defined in core (not in the NAPI crate) so the JSON-building logic for
/// `PropType`'s hand-written `Serialize` (see ADR 0002) happens once here,
/// not in every downstream crate.
pub fn extraction_output_to_json(output: &ExtractionOutput) -> Result<String, serde_json::Error> {
    let mut json = Vec::new();
    write_extraction_output_json(output, &mut json)?;
    String::from_utf8(json).map_err(|e| serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
}

/// Write an [`ExtractionOutput`] as compact JSON.
pub fn write_extraction_output_json(
    output: &ExtractionOutput,
    writer: impl std::io::Write,
) -> Result<(), serde_json::Error> {
    serde_json::to_writer(writer, output)
}

/// Serialize an [`IncrementalUpdate`] to a JSON string (same reason as above).
pub fn incremental_update_to_json(update: &IncrementalUpdate) -> Result<String, serde_json::Error> {
    serde_json::to_string(update)
}

// ─── Main cold extraction ─────────────────────────────────────────────────────

/// Stateless extraction — suitable for CLI and NAPI cold runs.
pub fn extract(options: &PipelineOptions) -> ExtractionOutput {
    revision::Revision::cold_output(options)
}

// ─── Helper functions ─────────────────────────────────────────────────────────

/// Parse-or-load-from-cache a single `.d.ts` file and merge it into `global`,
/// draining any diagnostics the parse produced into `diagnostics` first —
/// matching how the main per-project-file loop (Phase 3) already handles this.
/// Shared by the Full-mode `@types/react` merge (Phase 3.5) and the ambient
/// `lib.d.ts` merge (Phase 3.6), which previously each merged the parsed data
/// directly with no diagnostics drain at all: an excessive-nesting trip or a
/// parse error while parsing either file would have been silently discarded,
/// since `GlobalSourceData` itself has no diagnostics field to catch it later.
#[cfg(test)]
pub(super) fn merge_cached_dts_file(
    path: &Utf8Path,
    cache: &DtsCache,
    global: &mut GlobalSourceData,
    diagnostics: &mut Vec<Diagnostic>,
) {
    merge_parsed_dts_file(path, parse_cached_dts_file(path, cache), global, diagnostics);
}

/// Read and parse one declaration file through the cache. An unreadable file is an error and is not parsed: parsing ""
/// instead would cache an empty result under this path.
pub(super) fn parse_cached_dts_file(path: &Utf8Path, cache: &DtsCache) -> Result<SourceData, Diagnostic> {
    let source = std::fs::read_to_string(path).map_err(|error| Diagnostic::io_read_error(path, &error))?;
    Ok(match cache.get(path, &source) {
        Some(cached) => cached,
        None => {
            let data = crate::extractor::parse_file(path, &source);
            cache.insert(path, &source, data.clone());
            data
        }
    })
}

/// [`parse_cached_dts_file`]'s result merged into `global`, with the parse's diagnostics (or the read error) added to
/// `diagnostics`. Split from the parse so a run can parse these files while it parses the project's.
pub(super) fn merge_parsed_dts_file(
    path: &Utf8Path,
    parsed: Result<SourceData, Diagnostic>,
    global: &mut GlobalSourceData,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut data = match parsed {
        Ok(data) => data,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            return;
        }
    };
    diagnostics.append(&mut data.diagnostics);
    // These files exist to resolve types, not to document components: `@types/react`'s
    // own `class PureComponent<P, ...> extends Component<P, ...>` otherwise matches the
    // class-component detector and surfaces as a 0-prop component plus an unresolvable `P`.
    data.component_mappings.clear();
    global.merge(path, data);
}

/// Resolve `mappings` in parallel. The whole closure body is wrapped in `contain_panic` so one component's resolution
/// panicking degrades to a per-component diagnostic instead of poisoning the `.collect()` and losing every other
/// component. Shared by `extract()` and `WatchSession::update_file` so the two cannot drift.
pub(super) fn resolve_guarded(
    mappings: &[&ComponentMapping],
    ctx: &ResolutionContext,
) -> Vec<(ComponentEntry, ResolvedDiagnostics, crate::resolver::Dependencies)> {
    mappings
        .par_iter()
        .copied()
        .map(|mapping| {
            let label = format!("resolve:{}", mapping.component_name);
            crate::panic_guard::contain_panic(&label, || {
                #[cfg(test)]
                if mapping.component_name == RESOLVE_PANIC_TEST_SENTINEL {
                    panic!("simulated resolve-phase panic (test-only sentinel)");
                }

                crate::resolver::resolve_component_tracked(mapping, ctx)
            })
            .unwrap_or_else(|diag| {
                let stub = ComponentEntry {
                    display_name: mapping.component_name.clone(),
                    file_path: mapping.file_path.clone(),
                    description: String::new(),
                    props: Default::default(),
                    inheritance: vec![],
                    notable_inherited: Default::default(),
                    discriminant_prop: None,
                    composes: vec![],
                    tags: Default::default(),
                    methods: vec![],
                    detected_by: mapping.rule,
                    exported: mapping.exported(),
                    exports: mapping.exports.clone(),
                };
                // A panicked resolution re-runs on any edit to its own file.
                let mut deps = crate::resolver::Dependencies::default();
                deps.files.insert(crate::path_key::PathKey::from(mapping.file_path.as_path()));
                (stub, ResolvedDiagnostics { own: vec![diag], shared: Vec::new() }, deps)
            })
        })
        .collect()
}

/// Collect enum entries that are exported from their source files.
pub(super) fn collect_public_enums(
    global: &GlobalSourceData,
) -> (std::collections::BTreeMap<String, Vec<EnumEntry>>, Vec<Diagnostic>) {
    let mut result = std::collections::BTreeMap::new();
    let mut origin: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    let mut diagnostics = Vec::new();

    // Sorted so a name shared by two files resolves the same way on every run and in every input order.
    let mut keys: Vec<&String> = global.enums.keys().collect();
    keys.sort();

    for scoped_key in keys {
        // scoped_key = "{file_path}:{name}"; split from the right because a Windows path contains `:` too.
        let Some((file_path_str, name)) = scoped_key.rsplit_once(':') else { continue };
        let file_path = Utf8Path::new(file_path_str);
        let is_exported = global.re_export_map.get(file_path).is_some_and(|exports| {
            exports.iter().any(|e| matches!(e, LexedExport::LocalDeclaration { name: n, .. } if n == name))
        });
        if !is_exported {
            continue;
        }
        if let Some(kept) = origin.get(name) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "Enum `{name}` is exported from both {kept} and {file_path_str}; only the first is listed."
                ),
                file: Some(file_path_str.to_owned()),
                line: None,
                column: None,
                help: Some("Rename one of the enums; the public enum map is keyed by bare name.".into()),
                code: DiagnosticCode::DuplicateEnumName,
                affects: Vec::new(),
            });
            continue;
        }
        origin.insert(name, file_path_str);
        result.insert(name.to_owned(), global.enums[scoped_key].clone());
    }

    (result, diagnostics)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
