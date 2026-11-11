//! Options for one extraction run.
//!
//! These live in `types/` so the resolver and the dependency loader can read them without importing `pipeline`;
//! `pipeline` re-exports them, so `caliper_docgen_core::pipeline::PipelineOptions` and the others keep their paths.

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

/// How much of an inherited HTML element's attribute surface to expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HtmlAttributeMode {
    /// A compact subset per element: `ref`, `aria-*`, every event handler but the `*Capture` twins, the global
    /// attributes and the element's own notable ones, kept in `notableInherited`. Opt-in; `Full` is the default.
    Curated,
    /// Actually resolve `@types/react`'s real `HTMLAttributes`/`AriaAttributes`/
    /// `DOMAttributes`/`<Element>HTMLAttributes` interface chain, matching RDT's
    /// full ~250-300 attributes per element — the default.
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

/// Which inherited sources are recorded as an `InheritedLayer` of names instead of expanded prop by prop (collapse).
/// A component's own props type is always expanded. A dependency source collapses when it is ambient (DOM and ARIA
/// attributes, style systems) and declares more than 30 props; a dependency's own API is listed unless it declares more than 200 props.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollapseInheritedProps {
    /// Large ambient sources declared in a loaded dependency (`load_dependencies`).
    #[default]
    Dependencies,
    /// Those, and the project's own sources of more than 30 props.
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
    /// Extra gitignore-style glob patterns to exclude, relative to each source directory (on top of built-in: stories,
    /// tests, snapshots).
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
    /// Cache directory. Defaults to `node_modules/.cache/caliper-docgen` under the nearest ancestor of the first source
    /// directory that has a `node_modules` or a `package.json` (a directory in the OS temp directory when there is
    /// none), so the CLI, the Vite plugin and the Storybook addon share one cache whatever directory they run from.
    pub cache_dir: Option<Utf8PathBuf>,
    /// Read and write the on-disk caches (the declaration parse cache and the dependency index) under `cache_dir`.
    /// Off, a run reads and writes nothing there and parses everything; its output is the same. On by default.
    pub use_cache: bool,
    /// Load the dependency declarations project types reference, by name, before resolving (ADR 0010). On by default
    /// (`docs/performance.md`); off, a type imported from a package resolves only when it is a builtin or a known
    /// shortcut. A project with no installed packages loads nothing either way.
    pub load_dependencies: bool,
    /// Which large inherited sources to record as a layer instead of expanding.
    pub collapse_inherited_props: CollapseInheritedProps,
    /// Ask the TypeScript 7.1 checker for each component's props and merge them over the structural result
    /// (`docs/type-checker-integration.md`). Off by default and experimental. `extract` ignores it: the checker
    /// backend lives in `caliper-docgen-semantic`, which depends on this crate, so the embedding (the CLI) runs it and
    /// calls [`crate::types::semantic::merge_semantic`].
    pub experimental_semantic_types: bool,
    /// The `tsc` binary the semantic backend spawns. When unset it finds `node_modules/typescript/bin/tsc` above the
    /// first source directory.
    pub semantic_tsc_path: Option<Utf8PathBuf>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            src_dirs: vec![Utf8PathBuf::from("./src")],
            exclude_patterns: vec![],
            exclude_prefixes: vec![],
            html_attributes: HtmlAttributeMode::Full,
            tsconfig_path: None,
            extra_builtins: Default::default(),
            cache_dir: None,
            use_cache: true,
            load_dependencies: true,
            collapse_inherited_props: CollapseInheritedProps::default(),
            experimental_semantic_types: false,
            semantic_tsc_path: None,
        }
    }
}

impl PipelineOptions {
    /// Where the parse and dependency caches live: `cache_dir` when set, else `node_modules/.cache/caliper-docgen`
    /// under the nearest ancestor of the first source directory that has a `node_modules` or a `package.json`, so
    /// running from another directory neither misses the project's cache nor creates a `node_modules` there. A source
    /// directory in no project gets a directory in the OS temp directory.
    pub(crate) fn effective_cache_dir(&self) -> Utf8PathBuf {
        if let Some(dir) = &self.cache_dir {
            return dir.clone();
        }
        const CACHE: &str = "node_modules/.cache/caliper-docgen";
        let root = canonicalize_first_src_dir(&self.src_dirs).and_then(|src| {
            src.ancestors()
                .find(|dir| dir.join("node_modules").is_dir() || dir.join("package.json").is_file())
                .map(Utf8Path::to_owned)
        });
        match root {
            Some(root) => root.join(CACHE),
            None => {
                let temp = std::env::temp_dir();
                Utf8PathBuf::from_path_buf(temp).unwrap_or_else(|_| Utf8PathBuf::from(".")).join("caliper-docgen-cache")
            }
        }
    }
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
pub(crate) fn canonicalize_first_src_dir(src_dirs: &[Utf8PathBuf]) -> Option<Utf8PathBuf> {
    src_dirs.first().and_then(|dir| crate::canonical_path::canonicalize(dir).ok())
}
