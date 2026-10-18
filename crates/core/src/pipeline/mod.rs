//! Extraction pipeline — Phase 3b.
//!
//! Orchestrates: file discovery → parallel parse → global merge → parallel resolve → output.
//! Also manages DTS cache and incremental watch-mode state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::cache::DtsCache;
use crate::resolver::{resolve_component, ResolutionContext};
use crate::types::*;

mod discover;
pub mod watch;

use discover::{discover_files, should_skip};
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

/// User-supplied known type override for custom library patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KnownTypeOverride {
    Opaque { label: Option<String> },
    Alias { arg_index: usize },
    Skip,
}

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

/// Configuration for a single extraction run.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    /// Source directories to scan.
    pub src_dirs: Vec<Utf8PathBuf>,
    /// Extra glob-like patterns to exclude (on top of built-in: stories, tests, snapshots).
    pub exclude_patterns: Vec<String>,
    /// Component name prefixes to skip.
    pub exclude_prefixes: Vec<String>,
    /// React version — default: React 19.
    pub react_version: crate::react_types::ReactVersion,
    /// Whether to resolve cross-package types.
    pub cross_package: bool,
    /// PandaCSS generated output dir, if applicable.
    pub pandacss_outdir: Option<Utf8PathBuf>,
    /// Extra function names to treat as cva-like variant functions.
    pub variant_functions: Vec<String>,
    /// How much of an inherited HTML element's attributes to expose.
    pub html_attributes: HtmlAttributeMode,
    // ── Fields from architectural review ─────────────────────────────────────
    /// Path to tsconfig.json (auto-detected if None).
    pub tsconfig_path: Option<Utf8PathBuf>,
    /// Monorepo path aliases: package name → list of candidate root dirs.
    pub extra_paths: FxHashMap<String, Vec<Utf8PathBuf>>,
    /// User-supplied overrides for specific named types.
    pub known_type_overrides: FxHashMap<String, KnownTypeOverride>,
    /// Additional builtin type names beyond the React standard set.
    pub extra_builtins: FxHashSet<compact_str::CompactString>,
    /// Enable vanilla-extract CSS-in-JS support.
    pub vanilla_extract: bool,
    /// Cache directory. Defaults to `node_modules/.cache/oxc-react-docgen`.
    pub cache_dir: Option<Utf8PathBuf>,
    /// Future: opt-in to typescript-go for conditional/mapped type resolution.
    pub resolve_complex_types: bool,
    /// Custom docgen extension plugins.
    pub plugins: crate::plugin::PluginRegistry,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            src_dirs: vec![Utf8PathBuf::from("./src")],
            exclude_patterns: vec![],
            exclude_prefixes: vec![],
            react_version: crate::react_types::REACT_19,
            cross_package: true,
            pandacss_outdir: None,
            variant_functions: vec!["cva".into(), "tv".into(), "defineRecipe".into(), "recipe".into()],
            html_attributes: HtmlAttributeMode::Curated,
            tsconfig_path: None,
            extra_paths: Default::default(),
            known_type_overrides: Default::default(),
            extra_builtins: Default::default(),
            vanilla_extract: false,
            cache_dir: None,
            resolve_complex_types: false,
            plugins: crate::plugin::PluginRegistry::default(),
        }
    }
}

// ─── Incremental watch types ──────────────────────────────────────────────────

/// Result of processing a single file change in watch mode.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncrementalUpdate {
    pub updated_components: Vec<ComponentEntry>,
    pub affected_files: Vec<Utf8PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    pub duration_ms: u64,
}

/// Reverse dependency graph: file → list of files that import from it.
/// Built once from GlobalSourceData; used by WatchSession for BFS propagation.
pub struct ReverseDeps {
    pub(super) inner: FxHashMap<Utf8PathBuf, Vec<Utf8PathBuf>>,
}

impl ReverseDeps {
    /// Build from a fully-merged GlobalSourceData.
    pub fn build(global: &GlobalSourceData) -> Self {
        let mut inner: FxHashMap<Utf8PathBuf, Vec<Utf8PathBuf>> = Default::default();

        for (consumer_file, imports) in &global.import_map {
            for import in imports {
                // Only chase relative imports — package imports can't be naively reversed.
                if import.specifier.starts_with('.') {
                    if let Some(parent) = consumer_file.parent() {
                        let target = parent.join(&import.specifier);
                        // Approximate extension normalization without I/O.
                        for ext in &[".ts", ".tsx", "/index.ts", "/index.tsx"] {
                            let candidate = Utf8PathBuf::from(format!("{}{}", target, ext));
                            inner.entry(candidate).or_default().push(consumer_file.clone());
                        }
                    }
                }
            }
        }

        Self { inner }
    }

    /// BFS: collect all files transitively affected by a change to `changed`.
    pub fn affected(&self, changed: &Utf8Path) -> FxHashSet<Utf8PathBuf> {
        let mut visited: FxHashSet<Utf8PathBuf> = Default::default();
        let mut queue: VecDeque<Utf8PathBuf> = VecDeque::new();
        queue.push_back(changed.to_owned());

        while let Some(file) = queue.pop_front() {
            if visited.insert(file.clone()) {
                for dep in self.inner.get(&file).into_iter().flatten() {
                    queue.push_back(dep.clone());
                }
            }
        }

        visited
    }
}

// ─── JSON serialization helpers ──────────────────────────────────────────────

/// Serialize an [`ExtractionOutput`] to a JSON string.
///
/// Defined in core (not in the NAPI crate) so the JSON-building logic for
/// `PropType`'s hand-written `Serialize` (see ADR 0002) happens once here,
/// not in every downstream crate.
pub fn extraction_output_to_json(output: &ExtractionOutput) -> Result<String, serde_json::Error> {
    serde_json::to_string(output)
}

/// Serialize an [`IncrementalUpdate`] to a JSON string (same reason as above).
pub fn incremental_update_to_json(update: &IncrementalUpdate) -> Result<String, serde_json::Error> {
    serde_json::to_string(update)
}

// ─── Main cold extraction ─────────────────────────────────────────────────────

/// Stateless extraction — suitable for CLI and NAPI cold runs.
pub fn extract(options: &PipelineOptions) -> ExtractionOutput {
    extract_with_global(options, false).0
}

/// Shared implementation for `extract()` and `WatchSession::initialize()`.
/// Runs every phase (discover, parse, merge, Full-mode `@types/react`,
/// ambient lib.d.ts, resolve) exactly once and hands back everything a caller
/// might need beyond the public-facing output: the fully-merged
/// `GlobalSourceData` (so watch mode doesn't have to hand-rebuild a second,
/// easily-incomplete one just for incremental updates — see `WatchSession`'s
/// prior bug, where its own rebuild skipped the Full-mode/ambient-lib.d.ts
/// merges entirely) and each file's own parsed `SourceData` (so watch mode can
/// seed its per-file cache without re-parsing). `capture_source_data` is
/// `false` for the one-shot `extract()` path, which never looks at the third
/// tuple element — skip the extra per-file clone in that case.
pub(crate) fn extract_with_global(
    options: &PipelineOptions,
    capture_source_data: bool,
) -> (ExtractionOutput, Arc<GlobalSourceData>, Vec<(Utf8PathBuf, SourceData)>) {
    let start = Instant::now();
    let mut diagnostics = Vec::new();

    // Phase 0: Validate that configured source directories exist. A typo'd --src (or a
    // stale docgen.config.ts srcDirs) must not silently produce an empty-but-valid-looking
    // result — see AGENTS.md non-negotiable #6.
    let missing_src_dirs: Vec<&Utf8PathBuf> =
        options.src_dirs.iter().filter(|dir| !dir.as_std_path().is_dir()).collect();
    if options.src_dirs.is_empty() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "No source directories configured — src_dirs is empty".into(),
            file: None,
            line: None,
            column: None,
            help: Some("Set --src (or docgen.config.ts srcDirs) to at least one directory to scan.".into()),
            code: DiagnosticCode::IoError,
        });
    } else if missing_src_dirs.len() == options.src_dirs.len() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "None of the configured source directories exist: {}",
                options.src_dirs.iter().map(|dir| dir.as_str()).collect::<Vec<_>>().join(", ")
            ),
            file: options.src_dirs.first().map(ToString::to_string),
            line: None,
            column: None,
            help: Some("Check --src (or docgen.config.ts srcDirs) for a typo'd or stale path.".into()),
            code: DiagnosticCode::IoError,
        });
    } else {
        for dir in missing_src_dirs {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("Source directory does not exist: {dir}"),
                file: Some(dir.to_string()),
                line: None,
                column: None,
                help: Some("Check --src (or docgen.config.ts srcDirs) for a typo'd or stale path.".into()),
                code: DiagnosticCode::IoError,
            });
        }
    }

    // Load the DTS parse-result cache (silently empty if missing/stale).
    let cache = Arc::new(DtsCache::load_from_disk(options.cache_dir.as_deref()));
    let cache_ref = Arc::clone(&cache);

    // Phase 1: Discover source files.
    let (src_files, mut discover_diagnostics) = discover_files(&options.src_dirs, &options.exclude_patterns);
    diagnostics.append(&mut discover_diagnostics);
    let files_parsed = src_files.len() as u32;

    // Counter for cache hits (atomic so rayon closures can increment safely).
    let cache_hits = AtomicU32::new(0);

    // Phase 2: Parallel parse with rayon — check DTS cache for .d.ts files.
    // The whole closure body is wrapped in `contain_panic` so one file's
    // parse panicking (a bug in the OXC visitor, an unanticipated AST shape)
    // degrades to a per-file diagnostic instead of poisoning the entire
    // `.collect()` and crashing the whole extraction run for every file.
    let source_data_vec: Vec<(Utf8PathBuf, SourceData, Option<Diagnostic>)> = src_files
        .par_iter()
        .map(|path| {
            let label = format!("parse:{path}");
            crate::panic_guard::contain_panic(&label, || {
                #[cfg(test)]
                if path.file_name() == Some(PARSE_PANIC_TEST_SENTINEL) {
                    panic!("simulated parse-phase panic (test-only sentinel)");
                }

                let is_dts = path.as_str().ends_with(".d.ts");
                let (source, io_diag) = match std::fs::read_to_string(path) {
                    Ok(s) => (s, None),
                    Err(e) => (String::new(), Some(Diagnostic::io_read_error(path, &e))),
                };
                // The cache key is a content hash (see cache.rs's CacheKey doc
                // comment for why, vs. mtime+size), so the file must already be
                // read before a cache lookup can happen — this is still exactly
                // one read either way (hit or miss), never two.
                if is_dts {
                    if let Some(cached) = cache_ref.get(path, &source) {
                        cache_hits.fetch_add(1, Ordering::Relaxed);
                        return (path.clone(), cached, io_diag);
                    }
                }
                let data = crate::extractor::parse_file(path, &source);
                if is_dts {
                    cache_ref.insert(path, &source, data.clone());
                }
                (path.clone(), data, io_diag)
            })
            .unwrap_or_else(|diag| (path.clone(), SourceData::default(), Some(diag)))
        })
        .collect();

    let dts_cache_hits = cache_hits.load(Ordering::Relaxed);

    // Phase 3: Merge into GlobalSourceData (sequential — fast hash-map insertions).
    let mut global = GlobalSourceData::default();
    let mut per_file_data = Vec::new();
    for (path, mut data, io_diag) in source_data_vec {
        if let Some(d) = io_diag {
            diagnostics.push(d);
        }
        // Surface any diagnostics the extractor raised while parsing this file
        // (excessive nesting, syntax errors) — never drop them silently.
        diagnostics.append(&mut data.diagnostics);
        diagnostics.extend(options.plugins.run_on_file_extracted(&path, &mut data));
        if capture_source_data {
            per_file_data.push((path.clone(), data.clone()));
        }
        global.merge(&path, data);
    }

    // Phase 3.5: Full HTML attribute mode needs @types/react's real interfaces
    // (HTMLAttributes, AriaAttributes, ButtonHTMLAttributes, etc.) merged in
    // before resolution runs, so the resolver can look them up like any other
    // interface instead of just recording an InheritedLayer. Cached the same way
    // as any other .d.ts — this cost is paid once per @types/react version, not
    // per extraction run.
    if options.html_attributes == HtmlAttributeMode::Full {
        let from_dir = canonicalize_first_src_dir(&options.src_dirs).unwrap_or_else(|| Utf8PathBuf::from("."));
        match crate::resolver::resolve_package_dts_path(&from_dir, "react") {
            Some(react_dts_path) => {
                let react_dts_path = Utf8PathBuf::from(react_dts_path);
                merge_cached_dts_file(&react_dts_path, &cache, &mut global, &mut diagnostics);
            }
            None => {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: "HtmlAttributeMode::Full requested but @types/react could not be resolved".into(),
                    file: None,
                    line: None,
                    column: None,
                    help: Some("Check that @types/react is installed. Falling back to curated attributes.".into()),
                    code: DiagnosticCode::UnresolvableImport,
                });
            }
        }
    }

    // Phase 3.6: ambient globals (Date, Element, ...) live in TS's lib.es5.d.ts/lib.dom.d.ts and are never imported, so
    // merge them in every mode. No `typescript` at all stays silent; an install missing a lib file (TS 7 without its
    // platform package) is reported here, since the per-type "cannot resolve" warnings blame the type, not the install.
    if let Some(from_dir) = canonicalize_first_src_dir(&options.src_dirs) {
        let libs = crate::resolver::resolve_ts_lib_paths(&from_dir);
        diagnostics.extend(libs.diagnostic);
        for lib_path in libs.paths {
            let lib_path = Utf8PathBuf::from(lib_path);
            merge_cached_dts_file(&lib_path, &cache, &mut global, &mut diagnostics);
        }
    }

    let global = Arc::new(global);

    // Phase 4: Resolve all components in parallel.
    // Borrowed, not cloned: `resolve_component` already takes `&ComponentMapping`,
    // and the parallel `.par_iter()` below only ever produces borrows anyway —
    // cloning here just to immediately re-borrow deep-cloned every mapping's
    // Strings/Vecs/param_defaults map for no reason, once per component.
    let mappings: Vec<&ComponentMapping> = global
        .component_mappings
        .iter()
        .filter(|m| !should_skip(&m.component_name, &options.exclude_prefixes))
        .collect();

    // The whole closure body is wrapped in `contain_panic` so one component's
    // resolution panicking degrades to a per-component diagnostic instead of
    // poisoning the entire `.collect()` and crashing the whole extraction
    // run for every component.
    let ctx = Arc::new(ResolutionContext::new(global.clone(), options));
    let results: Vec<(ComponentEntry, Vec<Diagnostic>)> = mappings
        .par_iter()
        .copied()
        .map(|mapping| {
            let label = format!("resolve:{}", mapping.component_name);
            crate::panic_guard::contain_panic(&label, || {
                #[cfg(test)]
                if mapping.component_name == RESOLVE_PANIC_TEST_SENTINEL {
                    panic!("simulated resolve-phase panic (test-only sentinel)");
                }

                resolve_component(mapping, &ctx)
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
                };
                (stub, vec![diag])
            })
        })
        .collect();

    // Phase 5: Collect output.
    let mut components = std::collections::BTreeMap::new();
    let mut seen_names: std::collections::HashMap<String, u32> = Default::default();

    for (entry, diags) in results {
        let base_name = entry.display_name.clone();
        let count = seen_names.entry(base_name.clone()).or_insert(0);
        *count += 1;

        let key = if *count == 1 {
            base_name
        } else {
            // File stem alone isn't unique across different src_dirs — two
            // different libraries each shipping a same-named file (e.g.
            // "Button.tsx" in one directory and "Button.d.ts" in another,
            // ubiquitous across real component libraries) produced the
            // identical disambiguated key too, silently overwriting one
            // another in `components` with zero diagnostic. The full file
            // path is always unique per file, so it can't repeat this failure
            // mode for distinct files (only a genuine duplicate declaration of
            // the same name in the very same file would still collide, a
            // materially different and much rarer situation).
            format!("{} ({})", base_name, entry.file_path)
        };

        let mut entry = entry;
        diagnostics.extend(options.plugins.run_on_component_resolved(&mut entry));
        // Capture the cheap Utf8PathBuf clone before `entry` moves into
        // `insert` — the alternative (`entry.clone()`) deep-clones the whole
        // resolved ComponentEntry (props map, inheritance chain, tags, ...)
        // on every component just to keep `file_path` readable for a
        // diagnostic that only fires on the rare collision case.
        let new_file_path = entry.file_path.clone();
        if let Some(previous) = components.insert(key.clone(), entry) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "Duplicate component key '{key}' — colliding file paths: previously '{}', now '{}'",
                    previous.file_path, new_file_path
                ),
                file: Some(new_file_path.to_string()),
                line: None,
                column: None,
                help: Some(
                    "Two resolved components produced the same display name and disambiguation key — only \
                     the later one is kept in the output. Check for overlapping src_dirs or a genuine \
                     duplicate declaration."
                        .into(),
                ),
                code: DiagnosticCode::ComponentKeyCollision,
            });
        }
        diagnostics.extend(diags);
    }

    let enums = collect_public_enums(&global);

    // Persist cache for the next run.
    // Arc::try_unwrap succeeds because all rayon workers have finished; fallback
    // calls save_to_disk via the Arc deref since DtsCache::save_to_disk takes &self.
    let cache_save_diagnostic = match Arc::try_unwrap(cache) {
        Ok(c) => c.save_to_disk(),
        Err(arc) => arc.save_to_disk(),
    };
    diagnostics.extend(cache_save_diagnostic);

    let duration_ms = start.elapsed().as_millis() as u64;
    let components_extracted = components.len() as u32;

    let output = ExtractionOutput {
        components,
        enums,
        diagnostics,
        stats: ExtractionStats {
            components_extracted,
            files_parsed,
            dts_cache_hits,
            duration_ms,
            ..Default::default()
        },
    };
    (output, global, per_file_data)
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
fn merge_cached_dts_file(
    path: &Utf8Path,
    cache: &DtsCache,
    global: &mut GlobalSourceData,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let source = std::fs::read_to_string(path).unwrap_or_default();
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
fn canonicalize_first_src_dir(src_dirs: &[Utf8PathBuf]) -> Option<Utf8PathBuf> {
    src_dirs.first().and_then(|dir| std::fs::canonicalize(dir).ok()).and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
}

/// Collect enum entries that are exported from their source files.
fn collect_public_enums(global: &GlobalSourceData) -> std::collections::BTreeMap<String, Vec<EnumEntry>> {
    let mut result = std::collections::BTreeMap::new();

    for (scoped_key, entries) in &global.enums {
        // scoped_key = "{file_path}:{name}"
        if let Some((file_path_str, name)) = scoped_key.split_once(':') {
            let file_path = Utf8Path::new(file_path_str);
            let is_exported = global
                .re_export_map
                .get(file_path)
                .map(|exports| {
                    exports.iter().any(|e| match e {
                        LexedExport::LocalDeclaration { name: n, .. } => n == name,
                        _ => false,
                    })
                })
                .unwrap_or(false);

            if is_exported {
                result.insert(name.to_owned(), entries.clone());
            }
        }
    }

    result
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn html_attribute_mode_parses_its_three_names_and_rejects_anything_else() {
        assert_eq!(HtmlAttributeMode::parse("curated"), Ok(HtmlAttributeMode::Curated));
        assert_eq!(HtmlAttributeMode::parse("full"), Ok(HtmlAttributeMode::Full));
        assert_eq!(HtmlAttributeMode::parse("none"), Ok(HtmlAttributeMode::None));
        for bad in ["all", "Full", "", "curated "] {
            assert_eq!(HtmlAttributeMode::parse(bad), Err(bad));
        }
    }

    #[test]
    fn merge_cached_dts_file_does_not_drop_parse_diagnostics() {
        // Adversarial review finding: Phase 3.5 (@types/react Full-mode merge)
        // and Phase 3.6 (ambient lib.d.ts merge) each called global.merge()
        // directly on freshly-parsed data with no diagnostics drain first —
        // GlobalSourceData has no diagnostics field, so anything the extractor
        // flagged while parsing either file (an excessive-nesting trip, a
        // parse error) was silently lost. Mirrors
        // extractor::tests::test_excessive_nesting_guard's exact repro shape.
        let tmp = TempDir::new().unwrap();
        let nested = "(".repeat(2500) + &")".repeat(2500);
        let source = format!("const x = {nested};");
        let path = write_file(&tmp, "lib.d.ts", &source);

        let cache_dir = Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap();
        let cache = DtsCache::load_from_disk(Some(&cache_dir));
        let mut global = GlobalSourceData::default();
        let mut diagnostics = Vec::new();

        merge_cached_dts_file(&path, &cache, &mut global, &mut diagnostics);

        assert!(
            diagnostics.iter().any(|d| d.code == DiagnosticCode::ExcessiveNesting),
            "expected the excessive-nesting diagnostic to survive the merge, got {:?}",
            diagnostics
        );
    }

    /// Write a file into a temp dir and return its Utf8PathBuf.
    fn write_file(dir: &TempDir, name: &str, content: &str) -> Utf8PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        Utf8PathBuf::from_path_buf(path).unwrap()
    }

    // ── a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash ──────
    //
    // No plugin system exists in this codebase yet to inject a panic via a
    // hook (that's a separate, later task), so this drives the real Phase 2
    // rayon closure via the sentinel filename it checks for under
    // `#[cfg(test)]` (see the `PARSE_PANIC_TEST_SENTINEL` check above).

    #[test]
    fn a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, PARSE_PANIC_TEST_SENTINEL, "export const Other = () => null;");

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        // Must not panic the test process — that's the whole point.
        let output = extract(&options);

        assert_eq!(output.stats.files_parsed, 2, "both files should still be counted as discovered");
        assert!(
            output.diagnostics.iter().any(|d| d.code == DiagnosticCode::InternalPanic),
            "expected an InternalPanic diagnostic, got {:?}",
            output.diagnostics
        );
    }

    // ── a_panic_during_resolve_phase_degrades_to_a_diagnostic_not_a_crash ────
    //
    // Same rationale as the parse-phase test above: `resolve_component` can't
    // be made to panic without editing non-test resolver code, so this drives
    // the real Phase 4 rayon closure via the sentinel component name it
    // checks for under `#[cfg(test)]`.

    #[test]
    fn a_panic_during_resolve_phase_degrades_to_a_diagnostic_not_a_crash() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");
        write_file(
            &tmp,
            "Other.tsx",
            &format!("export function {RESOLVE_PANIC_TEST_SENTINEL}(props: {{ label: string }}) {{ return null; }}\n"),
        );

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        // Must not panic the test process — that's the whole point.
        let output = extract(&options);

        assert!(
            output.components.contains_key("Button"),
            "the component whose sibling panicked during resolution should still be extracted"
        );
        assert!(
            output.diagnostics.iter().any(|d| d.code == DiagnosticCode::InternalPanic),
            "expected an InternalPanic diagnostic, got {:?}",
            output.diagnostics
        );
    }

    // ── test_discover_files ───────────────────────────────────────────────────

    #[test]
    fn test_discover_files() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, "types.ts", "export type Foo = string;");
        write_file(&tmp, "styles.css", ".btn {}");
        write_file(&tmp, "README.md", "# docs");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"), "should find .tsx files");
        assert!(names.contains(&"types.ts"), "should find .ts files");
        assert!(!names.contains(&"styles.css"), "should skip .css files");
        assert!(!names.contains(&"README.md"), "should skip .md files");
    }

    // ── test_exclude_stories ──────────────────────────────────────────────────

    #[test]
    fn test_exclude_stories() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, "Button.stories.tsx", "export default {};");
        write_file(&tmp, "Button.test.tsx", "it('works', () => {});");
        write_file(&tmp, "Button.spec.ts", "");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"), "implementation should be included");
        assert!(!names.contains(&"Button.stories.tsx"), ".stories. should be excluded");
        assert!(!names.contains(&"Button.test.tsx"), ".test. should be excluded");
        assert!(!names.contains(&"Button.spec.ts"), ".spec. should be excluded");
    }

    // ── test_discover_files_reports_diagnostic_for_permission_denied_subtree ─
    //
    // Bug A (root-cause-analysis.md): `discover_files` used `walker.flatten()`,
    // silently dropping every `ignore::Walk` `Err` — a permission-denied
    // subtree, a broken symlink, or any other I/O error mid-walk vanished with
    // no diagnostic, no warning, nothing. This exercises the permission-denied
    // case with a real unreadable subdirectory.

    #[test]
    #[cfg(unix)]
    fn test_discover_files_reports_diagnostic_for_permission_denied_subtree() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let restricted = tmp.path().join("restricted");
        fs::create_dir(&restricted).unwrap();
        fs::write(restricted.join("Hidden.tsx"), "export const Hidden = () => null;").unwrap();
        write_file(&tmp, "Visible.tsx", "export const Visible = () => null;");

        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o000)).unwrap();

        // Root bypasses Unix DAC permission bits entirely, so under a root
        // test runner the 0o000 directory stays readable and this test's
        // premise doesn't hold. Detect that behaviorally (no libc dependency
        // needed) and skip rather than fail spuriously.
        if fs::read_dir(&restricted).is_ok() {
            eprintln!(
                "skipping test_discover_files_reports_diagnostic_for_permission_denied_subtree: \
                 running as root, which bypasses Unix permission bits"
            );
            fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, diagnostics) = discover_files(&[dir], &[]);

        // Restore permissions so TempDir's Drop can actually remove the directory.
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755)).unwrap();

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Visible.tsx"), "readable file should still be discovered");
        assert!(!names.contains(&"Hidden.tsx"), "unreadable subtree's file must not silently appear");
        assert!(
            diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError),
            "expected an IoError diagnostic for the unreadable subtree, got {:?}",
            diagnostics
        );
    }

    // ── test_discover_files_reports_diagnostic_for_non_utf8_filename ─────────
    //
    // Bug A, second half: a non-UTF8 filename made `Utf8PathBuf::from_path_buf`
    // fail, and the `if let Ok(utf8) = ...` branch had no `else` — the file
    // silently vanished from the discovered set with zero diagnostic.

    #[test]
    #[cfg(unix)]
    fn test_discover_files_reports_diagnostic_for_non_utf8_filename() {
        use std::os::unix::ffi::OsStrExt;

        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");

        // 0xFF is invalid UTF-8 in any position — construct a non-UTF8 filename
        // directly, bypassing Rust's &str API (which can't represent one).
        let bad_name = std::ffi::OsStr::from_bytes(b"Bad\xFF.tsx");

        // ext4 and friends store filenames as opaque bytes, so this write
        // succeeds on Linux. APFS (macOS) and NTFS (Windows) validate
        // UTF-8/UTF-16 at the syscall level and reject it outright — there is
        // no way to get a non-UTF8 path onto disk there at all, Rust API or
        // not. Detect that behaviorally and skip rather than failing on the
        // filesystem's precondition instead of the code under test.
        if let Err(err) = fs::write(tmp.path().join(bad_name), "export const Bad = () => null;") {
            eprintln!(
                "skipping test_discover_files_reports_diagnostic_for_non_utf8_filename: \
                 this filesystem rejects non-UTF8 filenames outright ({err})"
            );
            return;
        }

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"), "valid-UTF8 file should still be discovered");
        assert_eq!(files.len(), 1, "non-UTF8 filename must not silently appear in the discovered set");
        assert!(
            diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError),
            "expected an IoError diagnostic for the non-UTF8 filename, got {:?}",
            diagnostics
        );
    }

    // ── test_extract_empty_src ────────────────────────────────────────────────

    #[test]
    fn test_extract_empty_src() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();

        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        assert!(output.components.is_empty(), "no components from empty dir");
        assert!(output.enums.is_empty(), "no enums from empty dir");
        assert!(output.diagnostics.is_empty(), "no diagnostics from empty dir");
        assert_eq!(output.stats.files_parsed, 0);
        assert_eq!(output.stats.components_extracted, 0);
    }

    // ── test_extract_missing_src_dir ──────────────────────────────────────────

    #[test]
    fn test_extract_missing_src_dir() {
        // A typo'd/stale --src must surface as an Error diagnostic, not silently
        // produce an empty-but-valid-looking result (AGENTS.md non-negotiable #6).
        let tmp = TempDir::new().unwrap();
        let missing = Utf8PathBuf::from_path_buf(tmp.path().join("does-not-exist")).unwrap();

        let options = PipelineOptions {
            src_dirs: vec![missing.clone()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let error = output
            .diagnostics
            .iter()
            .find(|d| matches!(d.severity, DiagnosticSeverity::Error) && d.code == DiagnosticCode::IoError)
            .expect("missing src dir should produce an Error/IoError diagnostic");
        assert!(error.message.contains(missing.as_str()), "message should name the missing path");
    }

    // ── test_extract_empty_src_dirs_produces_diagnostic ───────────────────────
    //
    // Bug B (root-cause-analysis.md): the guard
    // `!options.src_dirs.is_empty() && missing_src_dirs.len() == options.src_dirs.len()`
    // short-circuits to `false` when `src_dirs` itself is empty (`!true` is
    // `false`), so an explicitly empty `src_dirs` bypassed both the "all
    // missing" diagnostic and the per-dir "missing" loop — a silent zero-file,
    // zero-diagnostic run.

    #[test]
    fn test_extract_empty_src_dirs_produces_diagnostic() {
        let tmp = TempDir::new().unwrap();
        let options = PipelineOptions {
            src_dirs: vec![],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        assert!(output.components.is_empty());
        assert_eq!(output.stats.files_parsed, 0);
        let error = output
            .diagnostics
            .iter()
            .find(|d| matches!(d.severity, DiagnosticSeverity::Error) && d.code == DiagnosticCode::IoError)
            .expect("empty src_dirs should produce an Error/IoError diagnostic, not a silent empty run");
        assert!(
            error.message.to_lowercase().contains("no source director"),
            "expected the diagnostic to explain that no source directories were configured, got: {}",
            error.message
        );
    }

    // ── a redeclared prop in a child interface wins over the parent's ─────────

    #[test]
    fn child_interface_redeclaring_a_prop_wins_over_every_ancestor() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp,
            "Comp.tsx",
            r#"
interface Base {
  /**
   * base doc
   * @default "base"
   */
  x: string;
  y: number;
}
interface Mid extends Base {
  /** mid doc */
  x: "a" | "b" | "c";
}
interface Leaf extends Mid {
  /** leaf doc */
  x: "a" | "b";
}
interface Quiet extends Base {
  x: "a" | "b";
}
interface Quieter extends Quiet {
  x: "a";
}
export function FromMid(props: Mid) { return null; }
export function FromLeaf(props: Leaf) { return null; }
export function FromQuiet(props: Quiet) { return null; }
export function FromQuieter(props: Quieter) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };
        let output = extract(&options);

        let literals =
            |members: &[&str]| PropType::Union(members.iter().map(|m| PropType::StringLiteral((*m).into())).collect());
        let x_of = |component: &str| {
            let x = &output.components[component].props["x"];
            (x.prop_type.clone(), x.description.clone(), x.parent.as_ref().map(|p| p.name.clone()))
        };

        assert_eq!(x_of("FromMid"), (literals(&["a", "b", "c"]), "mid doc".to_owned(), Some("Mid".to_owned())));
        assert_eq!(x_of("FromLeaf"), (literals(&["a", "b"]), "leaf doc".to_owned(), Some("Leaf".to_owned())));

        // An undocumented redeclaration inherits the description, transitively, like TS's JSDoc lookup; tags and
        // `@default` stay with the declaration that wrote them.
        assert_eq!(x_of("FromQuiet"), (literals(&["a", "b"]), "base doc".to_owned(), Some("Quiet".to_owned())));
        assert_eq!(
            x_of("FromQuieter"),
            (PropType::StringLiteral("a".into()), "base doc".to_owned(), Some("Quieter".to_owned()))
        );
        let quiet_x = &output.components["FromQuiet"].props["x"];
        assert_eq!((quiet_x.default_value.as_ref(), quiet_x.tags.is_empty()), (None, true));

        let y = &output.components["FromLeaf"].props["y"];
        assert_eq!(y.prop_type, PropType::Number);
        assert_eq!(y.parent.as_ref().map(|p| p.name.as_str()), Some("Base"));
    }

    // ── literal-union alias props: bare members, never pre-quoted ────────────

    #[test]
    fn literal_union_alias_props_keep_bare_members_through_resolution_and_templates() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp,
            "Chip.tsx",
            r#"
type Size = "sm" | "md";
interface ChipProps {
  size?: Size;
  density?: `compact-${Size}`;
}
export function Chip(props: ChipProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);
        let chip = output.components.get("Chip").expect("Chip component not found");

        let size = &chip.props["size"].prop_type;
        assert_eq!(*size, PropType::LiteralUnion { members: vec!["sm".into(), "md".into()], has_default: false });
        assert_eq!(size.raw_string(), r#""sm" | "md""#);

        let density = &chip.props["density"].prop_type;
        assert_eq!(
            *density,
            PropType::LiteralUnion { members: vec!["compact-sm".into(), "compact-md".into()], has_default: false }
        );
    }

    // ── ambient files contribute types, not components ───────────────────────

    /// Full mode merges the real `@types/react` (which declares `class PureComponent<P, S, SS>
    /// extends Component<P, S, SS>`). Its component mappings must not leak into the output.
    fn full_mode_options_for(tmp: &TempDir) -> PipelineOptions {
        PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            html_attributes: HtmlAttributeMode::Full,
            ..Default::default()
        }
    }

    const BUTTON_SOURCE: &str = r#"
import * as React from "react";
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary";
}
export function Button(props: ButtonProps) { return null; }
"#;

    #[test]
    fn full_mode_does_not_emit_components_declared_in_merged_react_types() {
        let tmp = TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        write_file(&tmp, "Button.tsx", BUTTON_SOURCE);

        let output = extract(&full_mode_options_for(&tmp));

        // `TrustedHTML` and friends are a separate, documented same-namespace-resolution gap;
        // the ghost's own symptom is the unresolvable props parameter `P`.
        let ghost_diagnostics: Vec<&Diagnostic> =
            output.diagnostics.iter().filter(|d| d.message.contains("Cannot resolve type 'P'")).collect();
        assert!(ghost_diagnostics.is_empty(), "unexpected diagnostics from the ghost component: {ghost_diagnostics:?}");

        let names: Vec<&str> = output.components.keys().map(String::as_str).collect();
        assert_eq!(names, vec!["Button"]);
    }

    // ── test_html_attribute_mode_full_resolves_real_button_attrs_end_to_end ───

    #[test]
    fn test_html_attribute_mode_full_resolves_real_button_attrs_end_to_end() {
        // Real end-to-end proof, not synthetic GlobalSourceData: Full mode should
        // actually find and parse this repo's real @types/react and merge a real
        // ButtonHTMLAttributes field into a real component's props. Placed inside
        // the crate dir (not a bare system tempdir) so ancestor-walking node_modules
        // resolution reaches this repo's real, installed @types/react.
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "Button.tsx",
            r#"
import * as React from "react";
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary";
}
export function Button(props: ButtonProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            html_attributes: HtmlAttributeMode::Full,
            ..Default::default()
        };

        let output = extract(&options);

        // @types/react itself was found and parsed — this must never fail with
        // "could not be resolved" (that's the graceful fallback for @types/react
        // genuinely not being installed, which isn't the case here).
        let unresolvable_react =
            output.diagnostics.iter().find(|d| d.message.contains("@types/react could not be resolved"));
        assert!(unresolvable_react.is_none(), "expected @types/react to resolve in this repo's real node_modules");

        let button = output.components.get("Button").expect("Button component not found");
        // A handful of individual cross-referenced fields inside @types/react's own
        // interface chain (AriaAttributes referenced bare from within the same
        // `declare namespace React {}` block, not through an explicit `React.`
        // qualifier) don't resolve yet — a narrower, separate gap in same-namespace
        // sibling reference resolution, not a regression of this feature. The load-
        // bearing claim is that the bulk of a real element's real attributes merge
        // in as genuine own props, matching RDT's flat behavior.
        assert!(
            button.props.len() > 200,
            "expected the bulk of ButtonHTMLAttributes' real ~235 fields to resolve, got {} props",
            button.props.len()
        );
        assert!(
            button.props.contains_key("formAction"),
            "expected a real ButtonHTMLAttributes field in Button's own props, got {:?}",
            button.props.keys().collect::<Vec<_>>()
        );
        assert!(button.props.contains_key("variant"), "own prop 'variant' should still be present");
    }

    // ── test_generic_svg_attributes_extends_resolves_to_its_element_in_full_mode ─
    //
    // `IconProps extends React.SVGAttributes<SVGSVGElement>` was resolved as an
    // opaque no-op (react_types::html_element_for("SVGAttributes") returns None
    // — "no single element to pick", true for the *unparameterized* name but
    // not for a real call site, which always supplies a concrete element type
    // argument). classify_extends now derives the element from that argument
    // for the generic SVGAttributes/SVGProps/HTMLProps forms specifically,
    // the same way the concrete `<Element>HTMLAttributes` forms already do
    // from their name alone.

    #[test]
    fn test_generic_svg_attributes_extends_resolves_to_its_element_in_full_mode() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "Icon.tsx",
            r#"
import * as React from "react";
export interface IconProps extends React.SVGAttributes<SVGSVGElement> {
  size?: number;
}
export const Icon = React.forwardRef<SVGSVGElement, IconProps>((props, ref) => <svg ref={ref} {...props} />);
Icon.displayName = "Icon";
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            html_attributes: HtmlAttributeMode::Full,
            ..Default::default()
        };

        let output = extract(&options);

        let icon = output.components.get("Icon").expect("Icon component not found");
        assert!(
            icon.props.contains_key("suppressHydrationWarning"),
            "expected a real SVGAttributes field to resolve as an own prop in Full mode, got {:?}",
            icon.props.keys().collect::<Vec<_>>()
        );
        assert!(icon.props.contains_key("size"), "own prop 'size' should still be present");
    }

    // ── test_unresolvable_intersection_member_records_raw_type_in_composes ────
    //
    // `resolve_base_as_chain`'s final match arm (conditional types, mapped types,
    // indexed access, etc. used directly as a props base) already builds a real
    // diagnostic naming the exact unresolvable expression — but returned a bare
    // `ResolvedChain::default()` instead of `empty_with_compose`, so the props
    // map ends up with zero trace of it. `composes` (`ComponentEntry.composes:
    // Vec<String>`) is the react-docgen-native mechanism for exactly this case
    // ("props come from this type, which we're listing by name/expression
    // instead of flattening") — populating it here needs no new type inference,
    // just recording the raw text this code path was already computing for the
    // diagnostic message.

    #[test]
    fn test_unresolvable_intersection_member_records_raw_type_in_composes() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "Comp.tsx",
            r#"
export type Weird<T> = T extends string ? { a: string } : { b: number };
export type CompProps = Weird<'x'> & { c: boolean };
export function Comp(props: CompProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let comp = output.components.get("Comp").expect("Comp component not found");
        assert!(comp.props.contains_key("c"), "expected the resolvable intersection member's prop to survive");
        assert!(
            !comp.composes.is_empty(),
            "expected the unresolvable Weird<'x'> member to be recorded in `composes` instead of silently vanishing"
        );
        assert!(
            comp.composes[0].contains("extends") && comp.composes[0].contains("string"),
            "expected composes to carry the actual conditional-type expression, got {:?}",
            comp.composes
        );
    }

    // ── test_same_display_name_across_files_with_identical_stem_does_not_collide ─
    //
    // Phase 5's dedup key was `"{name} ({file_stem})"` for the 2nd+ occurrence
    // of a display name — but file_stem alone isn't unique across different
    // src_dirs: two different libraries each shipping a `Button.tsx`/`Button.d.ts`
    // (extremely common — found for real across 5 of this repo's own fixture
    // libraries: chakra, fluentui, mantine, mui, panda, all named literally
    // "Button") produce the identical disambiguation key too, so the 3rd+
    // colliding component silently overwrote the 2nd in the output BTreeMap
    // with no diagnostic at all — a real violation of "never fail silently"
    // for anyone pointing this tool at more than one directory in one call
    // (the default and only way `apps/validate` and any real monorepo build
    // would invoke it).

    #[test]
    fn test_same_display_name_across_files_with_identical_stem_does_not_collide() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let lib_a = TempDir::new_in(manifest_dir).unwrap();
        let lib_b = TempDir::new_in(manifest_dir).unwrap();
        let lib_c = TempDir::new_in(manifest_dir).unwrap();
        write_file(&lib_a, "Button.tsx", "export function Button(props: { a?: string }) { return null; }\n");
        write_file(&lib_b, "Button.tsx", "export function Button(props: { b?: string }) { return null; }\n");
        write_file(&lib_c, "Button.tsx", "export function Button(props: { c?: string }) { return null; }\n");

        let options = PipelineOptions {
            src_dirs: vec![
                Utf8PathBuf::from_path_buf(lib_a.path().to_owned()).unwrap(),
                Utf8PathBuf::from_path_buf(lib_b.path().to_owned()).unwrap(),
                Utf8PathBuf::from_path_buf(lib_c.path().to_owned()).unwrap(),
            ],
            cache_dir: Some(Utf8PathBuf::from_path_buf(lib_a.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        assert_eq!(
            output.components.len(),
            3,
            "expected all 3 same-named, same-stem components to survive under distinct keys, got {:?}",
            output.components.keys().collect::<Vec<_>>()
        );
        let all_props: std::collections::BTreeSet<&str> =
            output.components.values().flat_map(|c| c.props.keys().map(String::as_str)).collect();
        assert_eq!(
            all_props,
            std::collections::BTreeSet::from(["a", "b", "c"]),
            "expected each distinct component's own prop to survive, got {:?}",
            all_props
        );
    }

    // ── test_named_type_only_import_from_react_resolves_to_real_dts ──────────
    //
    // Regression test for: `import type { AriaAttributes } from "react"` used
    // directly as an `extends` target failed for two independent reasons (this
    // uses AriaAttributes rather than HTMLAttributes specifically because
    // "HTMLAttributes" gets special-cased as a synthetic "div" element by
    // `react_types::html_element_for`, which routes through an unrelated,
    // already-working code path — AriaAttributes has no such shortcut, so it
    // actually exercises general named-import resolution):
    //   1. "react" resolved to its own `index.js` (react's package.json has no
    //      "types"/"typings" field and no "types" export condition — its real
    //      type declarations live in the separate `@types/react` package),
    //      instead of falling back to `@types/react`.
    //   2. Even once the right file is found, `@types/react` declares
    //      everything inside `declare namespace React { ... }`, so
    //      `AriaAttributes` is keyed as "React.AriaAttributes" there — a bare
    //      lookup for "AriaAttributes" (the name as actually imported/used)
    //      never matches.

    #[test]
    fn test_named_type_only_import_from_react_resolves_to_real_dts() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "Panel.tsx",
            r#"
import type { AriaAttributes } from "react";
interface PanelProps extends AriaAttributes {
  collapsible?: boolean;
}
export function Panel(props: PanelProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            html_attributes: HtmlAttributeMode::Full,
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable =
            output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'AriaAttributes'"));
        assert!(
            unresolvable.is_none(),
            "expected 'react' to resolve to @types/react's real .d.ts, got diagnostic: {:?}",
            unresolvable
        );

        let panel = output.components.get("Panel").expect("Panel component not found");
        assert!(
            panel.props.contains_key("aria-label"),
            "expected a real AriaAttributes field in Panel's own props, got {:?}",
            panel.props.keys().collect::<Vec<_>>()
        );
        assert!(panel.props.contains_key("collapsible"), "own prop 'collapsible' should still be present");
    }

    // ── test_namespace_import_member_access_resolves_to_real_dts ─────────────
    //
    // Regression test for: `import * as React from "react"` then a plain field
    // reference to `React.DependencyList` failed even in `--html-attributes
    // full` mode, where @types/react genuinely is parsed. Root cause:
    // `resolve_to_canonical` only stripped the "React." prefix for the
    // is_react_builtin check (named.rs step 1) — it never routed a `React.X`
    // member-expression reference through the "react" import at all, since
    // `find_import` only tracks the namespace binding itself ("React", with
    // `exported_name == "*"`), not literal dotted names like
    // "React.DependencyList". It always fell back to a same-file lookup for
    // that literal dotted name, which of course never resolves.

    #[test]
    fn test_namespace_import_member_access_resolves_to_real_dts() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "Table.tsx",
            r#"
import * as React from "react";
interface TableProps {
  cellRendererDependencies?: React.DependencyList;
}
export function Table(props: TableProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            html_attributes: HtmlAttributeMode::Full,
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("DependencyList"));
        assert!(
            unresolvable.is_none(),
            "expected React.DependencyList to resolve via the 'react' namespace import, got diagnostic: {:?}",
            unresolvable
        );

        let table = output.components.get("Table").expect("Table component not found");
        assert!(
            table.props.contains_key("cellRendererDependencies"),
            "expected 'cellRendererDependencies' prop, got: {:?}",
            table.props.keys().collect::<Vec<_>>()
        );
    }

    // ── test_native_js_global_resolves_via_real_typescript_lib_files ─────────
    //
    // Regression test for: `Date` (and any other native/DOM ambient global —
    // never imported, so nothing ever had a reason to resolve it) spuriously
    // triggered "Cannot resolve type 'Date' — it will appear as opaque",
    // hundreds of times on a real date-picker-shaped fixture. Real TypeScript
    // declares these in its own lib.es5.d.ts/lib.dom.d.ts (no export/import at
    // all — ambient script context, not a module) — this test proves the
    // pipeline actually finds and parses this repo's real `typescript` package
    // (not a synthetic stand-in) and resolves `Date` through it, structurally,
    // rather than via a hardcoded name list.

    #[test]
    fn test_native_js_global_resolves_via_real_typescript_lib_files() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "DatePicker.tsx",
            r#"
interface DatePickerProps {
  selected?: Date;
  onSelect?: (date: Date) => void;
}
export function DatePicker(props: DatePickerProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("'Date'"));
        assert!(
            unresolvable.is_none(),
            "expected 'Date' to resolve via this repo's real typescript lib.es5.d.ts, got diagnostic: {:?}",
            unresolvable
        );

        let picker = output.components.get("DatePicker").expect("DatePicker component not found");
        assert!(
            picker.props.contains_key("selected"),
            "expected 'selected' prop, got: {:?}",
            picker.props.keys().collect::<Vec<_>>()
        );
    }

    // ── test_indexed_access_into_ambient_dom_interface_resolves_via_real_lib ──
    //
    // Regression test for: day-picker's `HTMLDivElement["dir"]` (also "nonce",
    // "title", "lang") degraded to Opaque even after `lib.dom.d.ts` became
    // parseable. Two compounding bugs, both now fixed: (1) `lib.dom.d.ts` was
    // silently skipped entirely because `max_bracket_nesting_depth` miscounted
    // JSDoc comments containing unmatched brackets (see
    // `extractor::tests::test_nesting_guard_ignores_brackets_inside_comments`);
    // (2) `dir` isn't declared directly on `HTMLDivElement` — only inherited
    // via `extends HTMLElement` — and `resolve_indexed_access` only searched
    // an interface's own fields, never its extends chain. This test proves
    // both fixes compose correctly against this repo's real installed
    // `typescript` package, not just fabricated in-memory data.

    #[test]
    fn test_indexed_access_into_ambient_dom_interface_resolves_via_real_lib() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(
            &tmp,
            "Foo.tsx",
            r#"
interface FooProps {
  dir?: HTMLDivElement["dir"];
}
export function Foo(props: FooProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("HTMLDivElement"));
        assert!(
            unresolvable.is_none(),
            "expected HTMLDivElement[\"dir\"] to resolve via this repo's real lib.dom.d.ts, got diagnostic: {:?}",
            unresolvable
        );

        let foo = output.components.get("Foo").expect("Foo component not found");
        let dir_prop = foo.props.get("dir").expect("'dir' prop not found");
        assert_eq!(dir_prop.prop_type, PropType::String, "expected String, got {:?}", dir_prop.prop_type);
    }

    // ── test_named_type_reexported_through_a_barrel_file_resolves ────────────
    //
    // `import_map.rs`'s `resolve_reexport_chain`/`wildcard_sources_for` were
    // fully implemented and unit-tested in isolation, but `resolve_to_canonical`
    // never called them — a type imported through a barrel (`export { X } from
    // './x'` or `export * from './x'`, ubiquitous in real component libraries'
    // index.ts files) resolved to the barrel file itself, which doesn't declare
    // the type, so it silently fell through to "Cannot resolve type" instead of
    // following the re-export to where the type actually lives.

    #[test]
    fn test_named_type_reexported_through_a_barrel_file_resolves() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(&tmp, "types.ts", "export interface ButtonProps { label: string; }\n");
        write_file(&tmp, "index.ts", "export type { ButtonProps } from './types';\n");
        write_file(
            &tmp,
            "Button.tsx",
            r#"
import type { ButtonProps } from './index';
export function Button(props: ButtonProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'ButtonProps'"));
        assert!(
            unresolvable.is_none(),
            "expected ButtonProps to resolve through the barrel's named re-export, got diagnostic: {:?}",
            unresolvable
        );

        let button = output.components.get("Button").expect("Button component not found");
        assert!(
            button.props.contains_key("label"),
            "expected 'label' prop from the re-exported ButtonProps, got {:?}",
            button.props.keys().collect::<Vec<_>>()
        );
    }

    // ── test_unresolvable_named_type_reports_barrel_redirected_location ─────
    //
    // resolver/chain.rs's step-6 "Cannot resolve" diagnostic (component prop
    // types) already noted where an import redirected to when it differed from
    // the naive name/consuming-file; resolver/named.rs's step-7 diagnostic
    // (nested/named type-level, not component-prop-level) never got the same
    // treatment. `value` here is imported through a wildcard barrel that never
    // actually declares it — a real broken/incomplete barrel — so it stays
    // unresolvable, but the diagnostic should still say where the import
    // redirected to, not just where it was written.
    #[test]
    fn test_unresolvable_named_type_reports_barrel_redirected_location() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(&tmp, "empty.ts", "export const nothing = 1;\n");
        write_file(&tmp, "barrel.ts", "export * from './empty';\n");
        write_file(
            &tmp,
            "Component.tsx",
            r#"
import type { NeverDeclared } from './barrel';
export function Component(props: { value: Array<NeverDeclared> }) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable =
            output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'NeverDeclared'"));
        let unresolvable = unresolvable.expect("expected an unresolvable-type diagnostic for NeverDeclared");
        assert!(
            unresolvable.message.contains("(resolved to 'NeverDeclared' in")
                && unresolvable.message.contains("barrel.ts"),
            "expected the diagnostic to note the barrel-redirected location, got: {}",
            unresolvable.message
        );
    }

    #[test]
    fn test_named_type_reexported_through_a_wildcard_barrel_file_resolves() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(&tmp, "types.ts", "export interface ButtonProps { label: string; }\n");
        write_file(&tmp, "index.ts", "export * from './types';\n");
        write_file(
            &tmp,
            "Button.tsx",
            r#"
import type { ButtonProps } from './index';
export function Button(props: ButtonProps) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let unresolvable = output.diagnostics.iter().find(|d| d.message.contains("Cannot resolve type 'ButtonProps'"));
        assert!(
            unresolvable.is_none(),
            "expected ButtonProps to resolve through the barrel's wildcard re-export, got diagnostic: {:?}",
            unresolvable
        );

        let button = output.components.get("Button").expect("Button component not found");
        assert!(
            button.props.contains_key("label"),
            "expected 'label' prop from the re-exported ButtonProps, got {:?}",
            button.props.keys().collect::<Vec<_>>()
        );
    }

    // ── test_barrel_resolution_does_not_exponentially_blow_up_on_shared_descendants ──
    //
    // Adversarial-review finding: follow_reexports (resolver/import.rs) had no
    // visited-set, only the MAX_REEXPORT_DEPTH cap. That bounds the RECURSION
    // depth but not the number of CALLS — a barrel graph where multiple parents
    // wildcard-re-export the same shared descendant (ubiquitous in real
    // component libraries: several sub-barrels all wildcarding a common
    // `types.ts`) re-explores that shared subtree once per path into it,
    // multiplying out to branching_factor^depth calls instead of one call per
    // graph node. Reproduced empirically in the review with real distinct
    // files: 7 barrels took 9.7s wall time vs 156ms at 2.
    //
    // Wall-clock isn't a reliable signal for a small, fast-running unit test:
    // oxc_resolver caches path resolution by (from_dir, specifier), and this
    // repo's own per-call work (a couple of hashmap probes) is cheap enough
    // that even a few thousand redundant calls stay under a second on this
    // machine. Assert on the actual call count instead — deterministic, and
    // it's the real mechanism the fix changes.
    //
    // This barrel wildcard-re-exports the same next sibling twice, deep enough
    // to hit MAX_REEXPORT_DEPTH (8), with a name that's never declared
    // anywhere (the unhappy path, which forces full traversal instead of an
    // early exit on first match). Without the visited-set fix this explores
    // 2^8 = 256 calls; with it, exactly 9 (one per graph node, barrel_0..8).
    #[test]
    fn test_barrel_resolution_does_not_exponentially_blow_up_on_shared_descendants() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();

        const DEPTH: usize = 8;
        for level in 0..DEPTH {
            write_file(
                &tmp,
                &format!("barrel_{level}.ts"),
                &format!("export * from './barrel_{}';\nexport * from './barrel_{}';\n", level + 1, level + 1),
            );
        }
        write_file(&tmp, &format!("barrel_{DEPTH}.ts"), "export const unrelated = 1;\n");
        write_file(
            &tmp,
            "Component.tsx",
            r#"
import type { NeverDeclared } from './barrel_0';
export function Component(props: { value: NeverDeclared }) { return null; }
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        crate::resolver::reset_follow_reexports_call_count();
        let output = extract(&options);
        let calls = crate::resolver::follow_reexports_call_count();

        // Linear in graph size (barrel_0..barrel_8 = 9 nodes) with the
        // visited-set; without it this would be 2^8 = 256. A generous bound
        // (well under 256, comfortably above 9) that still fails hard if the
        // fix regresses back to exponential.
        assert!(
            calls < 50,
            "follow_reexports was called {calls} times on a depth-{DEPTH} shared-descendant \
             barrel graph (expected ~9, linear in graph size) — looks like the visited-set \
             regressed back to exploring branching_factor^depth calls"
        );
        // NeverDeclared genuinely doesn't exist anywhere — still expect the normal
        // "cannot resolve" diagnostic, not a crash or a hang.
        assert!(output.components.contains_key("Component"), "expected Component to still be extracted");
    }

    // ── test_static_default_props_assignment_reaches_parsed_prop ─────────────
    //
    // Regression test for: `Button.defaultProps = { size: 'md' }` (deprecated
    // but still real — MUI ships it) was never read at all; only destructured
    // defaults (`function Button({ size = 'md' })`) populated
    // ComponentMapping.param_defaults. Proves the extraction fix reaches all
    // the way through to ExtractionOutput, not just SourceData.

    #[test]
    fn test_static_default_props_assignment_reaches_parsed_prop() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp,
            "Button.tsx",
            r#"
interface ButtonProps {
  size?: string;
}
export function Button(props: ButtonProps) { return null; }
Button.defaultProps = { size: 'md' };
"#,
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };

        let output = extract(&options);

        let button = output.components.get("Button").expect("Button component not found");
        let size_prop = button.props.get("size").expect("'size' prop not found");
        let default = size_prop.default_value.as_ref().expect("expected a default value for 'size'");
        assert_eq!(default.value, "\"md\"");
        assert!(!default.computed);
    }

    // ── test_pipeline_options_default ─────────────────────────────────────────

    #[test]
    fn test_pipeline_options_default() {
        let opts = PipelineOptions::default();
        let fns = &opts.variant_functions;
        assert!(fns.contains(&"cva".to_string()), "cva should be a default variant function");
        assert!(fns.contains(&"tv".to_string()), "tv should be a default variant function");
        assert!(fns.contains(&"defineRecipe".to_string()), "defineRecipe should be a default variant function");
        assert!(fns.contains(&"recipe".to_string()), "recipe should be a default variant function");
    }

    // ── test_reverse_deps_bfs ─────────────────────────────────────────────────

    #[test]
    fn test_reverse_deps_bfs() {
        // Construct a dependency graph:
        //   a.ts  ← b.ts  ← c.ts
        // If a.ts changes, b.ts and c.ts are affected.
        let mut inner: FxHashMap<Utf8PathBuf, Vec<Utf8PathBuf>> = Default::default();
        let a = Utf8PathBuf::from("/project/a.ts");
        let b = Utf8PathBuf::from("/project/b.ts");
        let c = Utf8PathBuf::from("/project/c.ts");

        // b.ts imports a.ts → a is reverse-depended-on by b
        inner.entry(a.clone()).or_default().push(b.clone());
        // c.ts imports b.ts
        inner.entry(b.clone()).or_default().push(c.clone());

        let rev = ReverseDeps { inner };
        let affected = rev.affected(&a);

        assert!(affected.contains(&a), "changed file should be in affected set");
        assert!(affected.contains(&b), "direct importer should be affected");
        assert!(affected.contains(&c), "transitive importer should be affected");
    }

    // ── test_watch_session_update ─────────────────────────────────────────────

    #[test]
    fn test_watch_session_update() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let session = WatchSession::new(options);
        let _ = session.initialize();

        // Modify the file and trigger an incremental update.
        let button_path = Utf8PathBuf::from_path_buf(tmp.path().join("Button.tsx")).unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = ({ label }: { label: string }) => null;");

        let update = session.update_file(&button_path);

        // With the stub resolver, no components are returned (mappings list may be
        // empty for a simple stub file), but the call must complete without panic.
        assert!(update.duration_ms < 5_000, "update should complete quickly");
        // The changed file itself must appear in affected_files. update_file
        // canonicalizes its input (matching discover_files' own symlink-
        // resolved output, e.g. macOS's /var -> /private/var) so compare
        // against the canonicalized form, not necessarily the raw input.
        let canonical_button_path =
            Utf8PathBuf::from_path_buf(std::fs::canonicalize(button_path.as_std_path()).unwrap()).unwrap();
        assert!(
            update.affected_files.contains(&canonical_button_path),
            "changed file should always appear in affected_files"
        );
    }

    #[test]
    fn test_pipeline_plugin_execution() {
        use crate::plugin::{DocgenPlugin, PluginRegistry};
        use crate::types::ComponentEntry;

        struct TestPlugin;
        impl DocgenPlugin for TestPlugin {
            fn name(&self) -> &str {
                "test-plugin"
            }
            fn on_component_resolved(&self, entry: &mut ComponentEntry) {
                entry.composes.push("PluginAdded".into());
            }
        }

        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");

        let mut plugins = PluginRegistry::new();
        plugins.register(TestPlugin);

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            plugins,
            ..Default::default()
        };

        let output = extract(&options);
        let button = output.components.get("Button").expect("Button component not found");
        assert_eq!(button.composes, vec!["PluginAdded"]);
    }

    #[test]
    fn a_panicking_plugin_hook_degrades_to_a_diagnostic_and_other_work_still_completes() {
        use crate::plugin::{DocgenPlugin, PluginRegistry};
        use crate::types::ComponentEntry;

        struct AlwaysPanicsPlugin;
        impl DocgenPlugin for AlwaysPanicsPlugin {
            fn name(&self) -> &str {
                "always-panics"
            }
            fn on_component_resolved(&self, _entry: &mut ComponentEntry) {
                panic!("boom");
            }
        }

        struct WellBehavedPlugin;
        impl DocgenPlugin for WellBehavedPlugin {
            fn name(&self) -> &str {
                "well-behaved"
            }
            fn on_component_resolved(&self, entry: &mut ComponentEntry) {
                entry.composes.push("PluginAdded".into());
            }
        }

        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");
        write_file(&tmp, "Card.tsx", "export function Card(props: { title: string }) { return null; }\n");

        let mut plugins = PluginRegistry::new();
        plugins.register(AlwaysPanicsPlugin);
        plugins.register(WellBehavedPlugin);

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            plugins,
            ..Default::default()
        };

        // Must not panic the test process — that's the whole point.
        let output = extract(&options);

        let panic_diagnostics: Vec<_> =
            output.diagnostics.iter().filter(|d| d.code == DiagnosticCode::InternalPanic).collect();
        assert_eq!(
            panic_diagnostics.len(),
            2,
            "expected one InternalPanic diagnostic per component resolution, got {:?}",
            output.diagnostics
        );
        assert!(
            panic_diagnostics.iter().all(|d| d.message.contains("always-panics")),
            "each diagnostic should name the panicking plugin, got {panic_diagnostics:?}"
        );

        // Both components must still be present — the panicking plugin's hook
        // failure on one entry must not drop that entry, nor prevent the
        // sibling entry from being processed.
        let button = output.components.get("Button").expect("Button component not found");
        let card = output.components.get("Card").expect("Card component not found");

        // The well-behaved plugin, registered after the panicking one, must
        // still have run for both entries.
        assert_eq!(button.composes, vec!["PluginAdded"]);
        assert_eq!(card.composes, vec!["PluginAdded"]);
    }

    // ── test_duplicate_component_declarations_in_one_file_emit_a_collision_diagnostic ─
    //
    // Bug C (root-cause-analysis.md): `components.insert(key, entry)` in Phase
    // 5 discarded the `Option<ComponentEntry>` `BTreeMap::insert` already hands
    // back — a same-key collision (three or more resolutions landing on the
    // identical disambiguated key) silently overwrote the earlier entry with
    // zero diagnostic. `parse_file` never runs `oxc_semantic`'s checker, so
    // three syntactically-duplicate `function Button(...)` declarations in one
    // file parse cleanly into three `ComponentMapping`s sharing both name and
    // file_path (see extractor::visit's `visit_function`) — the 1st occurrence
    // keys as "Button", the 2nd and 3rd both key as "Button (<path>)" (file_path
    // is identical for all three), colliding at insert exactly like an
    // overlapping-src_dirs 3-way repeat would. (Note: a *2-directory* overlap of
    // the same physical file is instead caught upstream by `discover_files`'s
    // post-sort dedup — see `test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing` —
    // since that case never reaches this collision at all.)

    #[test]
    fn test_duplicate_component_declarations_in_one_file_emit_a_collision_diagnostic() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp,
            "Button.tsx",
            "interface ButtonProps { a?: string }\n\
             export function Button(props: ButtonProps) { return null; }\n\
             export function Button(props: ButtonProps) { return null; }\n\
             export function Button(props: ButtonProps) { return null; }\n",
        );

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let collision = output
            .diagnostics
            .iter()
            .find(|d| d.code == DiagnosticCode::ComponentKeyCollision)
            .expect("expected a ComponentKeyCollision diagnostic, got none");
        assert!(
            collision.message.contains("Button.tsx"),
            "expected the diagnostic to name the colliding file path, got: {}",
            collision.message
        );
    }

    // ── test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing ─
    //
    // Corrected finding: the insert-return check above only catches a genuine
    // 3+-way same-key collision. A realistic 2-directory overlap (e.g.
    // `["./src", "./src/components"]` both walking the same physical file)
    // produces exactly 2 duplicate mappings for the identical file that get
    // keyed DIFFERENTLY — the first gets the plain "Button" key, the second
    // gets the disambiguated "Button (<path>)" key — so they never collide at
    // `insert` and both silently appear as separate entries in the output.
    // `discover_files` must dedup identical canonical paths before they ever
    // become two separate component_mappings.

    #[test]
    fn test_overlapping_src_dirs_two_directories_deduplicates_without_a_duplicate_listing() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        let components_dir = tmp.path().join("components");
        fs::create_dir(&components_dir).unwrap();
        fs::write(
            components_dir.join("Button.tsx"),
            "export function Button(props: { a?: string }) { return null; }\n",
        )
        .unwrap();

        let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let nested = Utf8PathBuf::from_path_buf(components_dir).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![root, nested],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let button_keys: Vec<&String> = output.components.keys().filter(|k| k.starts_with("Button")).collect();
        assert_eq!(
            button_keys.len(),
            1,
            "the same physical file discovered via two overlapping src_dirs must produce exactly one \
             component entry, got keys: {button_keys:?}"
        );
        assert_eq!(output.stats.files_parsed, 1, "the overlapping file must be parsed once, not twice");
    }

    // ── SPEC-PIPELINE-001 AC-001: built-in __snapshots__/node_modules
    // substring exclusions.

    #[test]
    fn test_discover_files_excludes_snapshots_and_node_modules() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        fs::create_dir_all(tmp.path().join("__snapshots__")).unwrap();
        write_file(&tmp, "__snapshots__/Button.snap.tsx", "export const X = 1;");
        fs::create_dir_all(tmp.path().join("node_modules/some-lib")).unwrap();
        write_file(&tmp, "node_modules/some-lib/index.tsx", "export const Y = 1;");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"));
        assert!(!files.iter().any(|f| f.as_str().contains("__snapshots__")), "should skip __snapshots__");
        assert!(!files.iter().any(|f| f.as_str().contains("node_modules")), "should skip node_modules by default");
    }

    // ── SPEC-PIPELINE-001 AC-001: .gitignore/.git/info/exclude/.ignore work-
    // tree matrix. Previously zero coverage — the most intricate criterion in
    // the whole spec-drift review (see its own revision_note: 7 gate rounds,
    // verified line-by-line against the `ignore` crate's real source). The
    // only custom logic discover.rs has here is one line —
    // `git_ignore(!dir_is_in_node_modules)` — everything else is the `ignore`
    // crate's own default behavior for `.git/info/exclude` and `.ignore`
    // files, which these tests exercise via real `.git`/`.gitignore`/`.ignore`
    // fixtures rather than trusting the crate's docs alone.

    #[test]
    fn gitignore_excludes_a_matching_file_inside_a_real_git_work_tree() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        write_file(&tmp, ".gitignore", "Ignored.tsx\n");
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, "Ignored.tsx", "export const Skip = () => null;");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"));
        assert!(!names.contains(&"Ignored.tsx"), ".gitignore should exclude the matching file in a real git work tree");
    }

    #[test]
    fn gitignore_has_no_effect_when_the_src_dir_string_contains_node_modules() {
        let tmp = TempDir::new().unwrap();
        let lib_dir = tmp.path().join("node_modules").join("some-lib");
        fs::create_dir_all(&lib_dir).unwrap();
        fs::create_dir_all(lib_dir.join(".git")).unwrap();
        fs::write(lib_dir.join(".gitignore"), "Ignored.tsx\n").unwrap();
        fs::write(lib_dir.join("Button.tsx"), "export const Button = () => null;").unwrap();
        fs::write(lib_dir.join("Ignored.tsx"), "export const Skip = () => null;").unwrap();

        let dir = Utf8PathBuf::from_path_buf(lib_dir).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"));
        assert!(
            names.contains(&"Ignored.tsx"),
            ".gitignore must have NO effect when the src_dir's own configured path string contains \
             'node_modules', even with a real .git directory present, got {names:?}"
        );
    }

    #[test]
    fn ignore_file_excludes_a_matching_file_with_no_exception() {
        // Unlike .gitignore, .ignore excludes unconditionally — test it inside
        // a node_modules-string src_dir specifically, where .gitignore above
        // was proven inert, to prove .ignore really is a separate, unaffected
        // mechanism (not just "also disabled by the same flag").
        let tmp = TempDir::new().unwrap();
        let lib_dir = tmp.path().join("node_modules").join("some-lib");
        fs::create_dir_all(&lib_dir).unwrap();
        fs::write(lib_dir.join(".ignore"), "Ignored.tsx\n").unwrap();
        fs::write(lib_dir.join("Button.tsx"), "export const Button = () => null;").unwrap();
        fs::write(lib_dir.join("Ignored.tsx"), "export const Skip = () => null;").unwrap();

        let dir = Utf8PathBuf::from_path_buf(lib_dir).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"));
        assert!(!names.contains(&"Ignored.tsx"), ".ignore should exclude matching files with no exception");
    }

    #[test]
    fn gitignore_has_no_effect_outside_any_git_work_tree_but_ignore_still_does() {
        let tmp = TempDir::new().unwrap();
        // Deliberately NO .git directory anywhere in this tree.
        write_file(&tmp, ".gitignore", "GitignoredButIncluded.tsx\n");
        write_file(&tmp, ".ignore", "TrulyIgnored.tsx\n");
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, "GitignoredButIncluded.tsx", "export const A = () => null;");
        write_file(&tmp, "TrulyIgnored.tsx", "export const B = () => null;");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"));
        assert!(
            names.contains(&"GitignoredButIncluded.tsx"),
            ".gitignore has no effect outside a git work tree, got {names:?}"
        );
        assert!(!names.contains(&"TrulyIgnored.tsx"), ".ignore still excludes matching files outside a git work tree");
    }

    #[test]
    fn a_src_dir_physically_under_node_modules_but_not_named_that_way_is_not_exempt() {
        // Negative pair: src_dir is physically located inside a node_modules
        // directory on disk, but reached via a symlink whose own literal
        // string does NOT contain "node_modules" — dir_is_in_node_modules is
        // a string check on the CONFIGURED src_dir, not the real filesystem
        // location, so ordinary .gitignore rules (ancestor-.git-gated) must
        // still apply here, unlike the exempted case above.
        let tmp = TempDir::new().unwrap();
        let real_dir = tmp.path().join("vendor").join("node_modules").join("mylib").join("src");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(real_dir.join(".git")).unwrap();
        fs::write(real_dir.join(".gitignore"), "Ignored.tsx\n").unwrap();
        fs::write(real_dir.join("Button.tsx"), "export const Button = () => null;").unwrap();
        fs::write(real_dir.join("Ignored.tsx"), "export const Skip = () => null;").unwrap();

        let link = tmp.path().join("link-without-that-substring");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &link).unwrap();
        #[cfg(not(unix))]
        return; // No portable symlink API on this platform — skip rather than fail.

        let dir = Utf8PathBuf::from_path_buf(link).unwrap();
        assert!(
            !dir.as_str().contains("node_modules"),
            "test fixture invalid: the configured src_dir string must not contain 'node_modules'"
        );
        let (files, _diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"));
        assert!(
            !names.contains(&"Ignored.tsx"),
            ".gitignore must still apply — the src_dir's configured string doesn't contain 'node_modules', \
             even though it's physically located inside a node_modules directory on disk, got {names:?}"
        );

        // Separately, and independent of gitignore: this fixture also proves the
        // pre-canonicalization substring-matching claim. Button.tsx's WALK-
        // EMITTED path (via the symlink, "link-without-that-substring") never
        // contains "node_modules", so the built-in node_modules-substring
        // exclusion never fires for it — but its FINAL, canonicalized file_path
        // (resolved after the exclusion check runs) does contain "node_modules",
        // since the symlink's real target is physically under one. If discover.rs
        // canonicalized before checking, this file would have been wrongly
        // excluded by the same built-in rule that skips real node_modules trees.
        let button = files.iter().find(|f| f.file_name() == Some("Button.tsx")).unwrap();
        assert!(
            button.as_str().contains("node_modules"),
            "test fixture invalid: the resolved file_path should resolve through the real on-disk \
             node_modules directory, got {button}"
        );
    }

    // ── SPEC-PIPELINE-001 AC-001: exclude_prefixes filters components after
    // resolution, silently and by design.

    #[test]
    fn test_exclude_prefixes_filters_matching_components() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");
        write_file(
            &tmp,
            "InternalWidget.tsx",
            "export function InternalWidget(props: { x: string }) { return null; }\n",
        );

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            exclude_prefixes: vec!["Internal".to_string()],
            ..Default::default()
        };
        let output = extract(&options);

        assert!(output.components.contains_key("Button"));
        assert!(!output.components.contains_key("InternalWidget"), "excluded-prefix component must not appear");
        assert!(
            output.diagnostics.is_empty(),
            "exclude_prefixes is a deliberate, silent, opt-in filter — no diagnostic expected, got {:?}",
            output.diagnostics
        );
    }

    // ── SPEC-PIPELINE-001 AC-006: deterministic ordering across two separate
    // extract() calls on the same unchanged source.

    #[test]
    fn test_extract_is_deterministic_across_repeated_calls() {
        let tmp = TempDir::new().unwrap();
        for name in ["Alpha", "Beta", "Gamma", "Delta"] {
            write_file(
                &tmp,
                &format!("{name}.tsx"),
                &format!("export function {name}(props: {{ x: string }}) {{ return null; }}\n"),
            );
        }

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let first = extract(&options);
        let second = extract(&options);

        let first_keys: Vec<&String> = first.components.keys().collect();
        let second_keys: Vec<&String> = second.components.keys().collect();
        assert_eq!(first_keys, second_keys, "component key order must be identical across repeated extract() calls");
    }

    // ── SPEC-PIPELINE-001 AC-010: a triple same-file duplicate declaration
    // produces exactly two entries (bare-name 1st occurrence, disambiguated
    // 2nd/3rd collide, later wins deterministically) plus a collision
    // diagnostic.

    #[test]
    fn test_triple_duplicate_declaration_produces_exactly_two_entries() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp,
            "Button.tsx",
            "export function Button(props: { a: string }) { return null; }\n\
             export function Button(props: { b: string }) { return null; }\n\
             export function Button(props: { c: string }) { return null; }\n",
        );

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };
        let output = extract(&options);

        let button_keys: Vec<&String> = output.components.keys().filter(|k| k.starts_with("Button")).collect();
        assert_eq!(button_keys.len(), 2, "expected exactly 2 entries (bare + disambiguated), got {button_keys:?}");

        // Content, not presence: the bare-name entry (1st occurrence) must carry
        // props {a}, and the disambiguated entry must carry props from the LATER
        // (3rd) occurrence — {c} — not the 2nd's {b}, per AC-010's "iterated
        // later during Resolve/Collect" rule. Checking only the key count/names
        // would pass even if the 2nd occurrence's props won instead of the 3rd's.
        let bare = output.components.get("Button").expect("expected a bare-name entry");
        assert_eq!(
            bare.props.keys().collect::<Vec<_>>(),
            vec!["a"],
            "bare-name entry should keep the 1st occurrence's props, got {:?}",
            bare.props.keys().collect::<Vec<_>>()
        );

        let disambiguated_key =
            button_keys.iter().find(|k| k.as_str() != "Button").expect("expected a disambiguated key");
        let disambiguated = output.components.get(*disambiguated_key).unwrap();
        assert_eq!(
            disambiguated.props.keys().collect::<Vec<_>>(),
            vec!["c"],
            "disambiguated entry should carry the 3rd occurrence's props (the later of the two colliding \
             declarations), not the 2nd's, got {:?}",
            disambiguated.props.keys().collect::<Vec<_>>()
        );

        let collision_diag = output.diagnostics.iter().find(|d| d.code == DiagnosticCode::ComponentKeyCollision);
        assert!(collision_diag.is_some(), "expected a ComponentKeyCollision diagnostic, got {:?}", output.diagnostics);
        assert!(
            collision_diag.unwrap().message.contains("Button.tsx"),
            "collision diagnostic must name the colliding file path, got {:?}",
            collision_diag.unwrap().message
        );
    }

    // ── SPEC-PIPELINE-001 AC-022: exclude_patterns entries containing
    // glob-invalid characters are inert (plain substring match).

    #[test]
    fn test_exclude_patterns_with_glob_invalid_characters_are_inert() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, diagnostics) = discover_files(&[dir], &["[unclosed*".to_string()]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(
            names.contains(&"Button.tsx"),
            "a glob-invalid pattern that doesn't literally match must not exclude anything, got {names:?}"
        );
        assert!(diagnostics.is_empty(), "no error/panic expected for a glob-invalid but harmless pattern");
    }

    // ── SPEC-PIPELINE-001 AC-023: an empty-string exclude_patterns entry
    // excludes every file (str::contains("") is always true).

    #[test]
    fn test_exclude_patterns_empty_string_excludes_everything() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, "Widget.tsx", "export const Widget = () => null;");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, diagnostics) = discover_files(&[dir], &["".to_string()]);

        assert!(files.is_empty(), "an empty-string pattern should exclude every file, got {files:?}");
        assert!(diagnostics.is_empty());
    }
}
