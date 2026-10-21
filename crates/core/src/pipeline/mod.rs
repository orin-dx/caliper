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
    /// Cache directory. Defaults to `node_modules/.cache/caliper-docgen`.
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
                    code: DiagnosticCode::MissingTypeLibrary,
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

    let (enums, enum_diagnostics) = collect_public_enums(&global);
    diagnostics.extend(enum_diagnostics);

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
fn canonicalize_first_src_dir(src_dirs: &[Utf8PathBuf]) -> Option<Utf8PathBuf> {
    src_dirs.first().and_then(|dir| std::fs::canonicalize(dir).ok()).and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
}

/// Collect enum entries that are exported from their source files.
fn collect_public_enums(
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
