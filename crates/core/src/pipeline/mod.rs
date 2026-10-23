//! Extraction pipeline — Phase 3b.
//!
//! Orchestrates: file discovery → parallel parse → global merge → parallel resolve → output.
//! Also manages DTS cache and incremental watch-mode state.

use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::cache::DtsCache;
use crate::resolver::ResolutionContext;
use crate::types::*;

mod discover;
mod revision;
pub mod watch;

pub use watch::WatchSession;

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

// ─── PipelineOptions ─────────────────────────────────────────────────────────

/// How much of an inherited HTML element's attribute surface to expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HtmlAttributeMode {
    /// ~15-20 curated, commonly-documented attributes per element (onClick,
    /// disabled, aria-*, etc.) — the default. Matches RDT's shape for consumers
    /// that filter node_modules-sourced props, just with a smaller, hand-picked set.
    Curated,
    /// Actually resolve `@types/react`'s real `HTMLAttributes`/`AriaAttributes`/
    /// `DOMAttributes`/`<Element>HTMLAttributes` interface chain, matching RDT's
    /// full ~250-300 attributes per element.
    Full,
    /// Don't synthesize any inherited HTML attributes at all — own props only.
    None,
}

impl HtmlAttributeMode {
    /// The mode named by a config or binding string. `Err` carries the unrecognized value, so callers can name it
    /// instead of silently falling back to a default.
    pub fn parse(s: &str) -> Result<Self, &str> {
        match s {
            "curated" => Ok(Self::Curated),
            "full" => Ok(Self::Full),
            "none" => Ok(Self::None),
            other => Err(other),
        }
    }
}

/// Which inherited sources of more than 30 props are recorded as an `InheritedLayer` instead of expanded prop by prop
/// (ADR 0012). A component's own props type is always expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollapseInheritedProps {
    /// Sources declared in a loaded dependency (`load_dependencies`), as Storybook's react-component-meta shows them.
    #[default]
    Dependencies,
    /// Dependency sources and the project's own.
    All,
    /// Expand every source.
    None,
}

impl CollapseInheritedProps {
    /// The mode named by a config or binding string. `Err` carries the unrecognized value.
    pub fn parse(s: &str) -> Result<Self, &str> {
        match s {
            "dependencies" => Ok(Self::Dependencies),
            "all" => Ok(Self::All),
            "none" => Ok(Self::None),
            other => Err(other),
        }
    }
}

/// Configuration for a single extraction run.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    /// Source directories to scan.
    pub src_dirs: Vec<Utf8PathBuf>,
    /// Extra glob-like patterns to exclude (on top of built-in: stories, tests, snapshots).
    pub exclude_patterns: Vec<String>,
    /// Component name prefixes to skip.
    pub exclude_prefixes: Vec<String>,
    /// How much of an inherited HTML element's attributes to expose.
    pub html_attributes: HtmlAttributeMode,
    /// Path to the tsconfig that governs module resolution. When unset, each importing file uses the nearest
    /// `tsconfig.json` above it, and the one above the first source directory supplies `customConditions`.
    pub tsconfig_path: Option<Utf8PathBuf>,
    /// Additional builtin type names beyond the React standard set.
    pub extra_builtins: FxHashSet<compact_str::CompactString>,
    /// Cache directory. Defaults to `node_modules/.cache/caliper-docgen`.
    pub cache_dir: Option<Utf8PathBuf>,
    /// Custom docgen extension plugins.
    pub plugins: crate::plugin::PluginRegistry,
    /// Load the dependency declarations project types reference, by name, before resolving (ADR 0010). Off by default:
    /// without it a type imported from a package resolves only when it is a builtin or a known shortcut.
    pub load_dependencies: bool,
    /// Which large inherited sources to record as a layer instead of expanding.
    pub collapse_inherited_props: CollapseInheritedProps,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            src_dirs: vec![Utf8PathBuf::from("./src")],
            exclude_patterns: vec![],
            exclude_prefixes: vec![],
            html_attributes: HtmlAttributeMode::Curated,
            tsconfig_path: None,
            extra_builtins: Default::default(),
            cache_dir: None,
            plugins: crate::plugin::PluginRegistry::default(),
            load_dependencies: false,
            collapse_inherited_props: CollapseInheritedProps::default(),
        }
    }
}

// ─── Incremental watch types ──────────────────────────────────────────────────

/// Result of processing a single file change in watch mode. Its JSON writes the updated components' props as ids, with
/// a `props` table holding every prop they refer to (ADR 0009).
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
    pub affected_files: Vec<Utf8PathBuf>,
    /// Diagnostics this revision added. A client holding the previous output applies `removed_keys`, then
    /// `updated_keys`, then drops `removed_diagnostics` and adds these, and holds the new output.
    pub diagnostics: Vec<Diagnostic>,
    /// Diagnostics the previous revision had and this one does not: fixed, or replaced by a changed one.
    pub removed_diagnostics: Vec<Diagnostic>,
    pub duration_ms: u64,
}

impl Serialize for IncrementalUpdate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use crate::types::output::interned::{intern, InternedComponent};
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
        let (ids, props) = intern::<S::Error>(&entries)?;
        let components: Vec<InternedComponent<'_>> =
            entries.iter().zip(&ids).map(|(entry, ids)| InternedComponent { entry, ids }).collect();
        let mut out = serializer.serialize_struct("IncrementalUpdate", 9)?;
        out.serialize_field("revision", revision)?;
        out.serialize_field("updatedComponents", &components)?;
        out.serialize_field("props", &props)?;
        out.serialize_field("updatedKeys", updated_keys)?;
        out.serialize_field("removedKeys", removed_keys)?;
        out.serialize_field("affectedFiles", affected_files)?;
        out.serialize_field("diagnostics", diagnostics)?;
        out.serialize_field("removedDiagnostics", removed_diagnostics)?;
        out.serialize_field("durationMs", duration_ms)?;
        out.end()
    }
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
pub(super) fn merge_cached_dts_file(
    path: &Utf8Path,
    cache: &DtsCache,
    global: &mut GlobalSourceData,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // An unreadable file is reported and skipped; parsing "" instead would cache an empty result under this path.
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            diagnostics.push(Diagnostic::io_read_error(path, &error));
            return;
        }
    };
    let mut data = match cache.get(path, &source) {
        Some(cached) => cached,
        None => {
            let data = crate::extractor::parse_file(path, &source);
            cache.insert(path, &source, data.clone());
            data
        }
    };
    diagnostics.append(&mut data.diagnostics);
    // These files exist to resolve types, not to document components: `@types/react`'s
    // own `class PureComponent<P, ...> extends Component<P, ...>` otherwise matches the
    // class-component detector and surfaces as a 0-prop component plus an unresolvable `P`.
    data.component_mappings.clear();
    global.merge(path, data);
}

/// Canonicalize `src_dirs`' first entry to use as the "from" directory for
/// resolving @types/react and TypeScript's own lib.d.ts files — must match
/// what discovered file paths look like (always absolute — the `ignore`
/// walker absolutizes them regardless of whether --src was given as relative
/// or absolute), since the resolver looks this same specifier up again
/// per-component relative to each file's own absolute directory. A relative
/// src_dirs entry has too few path components for oxc_resolver's ancestor
/// walk to ever reach the real node_modules tree, so it silently finds
/// nothing instead of the intended real package.
pub(super) fn canonicalize_first_src_dir(src_dirs: &[Utf8PathBuf]) -> Option<Utf8PathBuf> {
    src_dirs.first().and_then(|dir| std::fs::canonicalize(dir).ok()).and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
}

/// Resolve `mappings` in parallel. The whole closure body is wrapped in `contain_panic` so one component's resolution
/// panicking degrades to a per-component diagnostic instead of poisoning the `.collect()` and losing every other
/// component. Shared by `extract()` and `WatchSession::update_file` so the two cannot drift.
pub(super) fn resolve_guarded(
    mappings: &[&ComponentMapping],
    ctx: &ResolutionContext,
) -> Vec<(ComponentEntry, Vec<Diagnostic>, crate::resolver::Dependencies)> {
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
                    exported: mapping.exported,
                };
                // A panicked resolution re-runs on any edit to its own file.
                let mut deps = crate::resolver::Dependencies::default();
                deps.files.insert(crate::path_key::PathKey::from(mapping.file_path.as_path()));
                (stub, vec![diag], deps)
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
