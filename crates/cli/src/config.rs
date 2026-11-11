use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;

/// The JSON-facing shape of `docgen.config.ts`'s default export.
///
/// `deny_unknown_fields` means a config naming an option this tool does not have (or a typo'd key) gets a clear error
/// naming the field, not a silently-ignored setting — same reasoning as this module's existing "found but
/// unsupported" hard error above.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DocgenConfigSchema {
    src_dirs: Option<Vec<String>>,
    exclude_patterns: Option<Vec<String>>,
    exclude_prefixes: Option<Vec<String>>,
    html_attributes: Option<String>,
    tsconfig_path: Option<String>,
    cache_dir: Option<String>,
    cache: Option<bool>,
    extra_builtins: Option<Vec<String>>,
    load_dependencies: Option<bool>,
    collapse_inherited_props: Option<String>,
    experimental_semantic_types: Option<bool>,
    tsc_path: Option<String>,
}

/// The keys `DocgenConfigSchema` accepts, for the error help; a test holds this list to the struct's own field list.
const CONFIG_KEYS: [&str; 12] = [
    "srcDirs",
    "excludePatterns",
    "excludePrefixes",
    "htmlAttributes",
    "tsconfigPath",
    "cacheDir",
    "cache",
    "extraBuiltins",
    "loadDependencies",
    "collapseInheritedProps",
    "experimentalSemanticTypes",
    "tscPath",
];

impl DocgenConfigSchema {
    /// `Err` names the bad `htmlAttributes` value — a typo must not silently fall back to a default
    /// (non-negotiable #6: never fail silently).
    fn into_pipeline_options(self) -> Result<PipelineOptions, String> {
        let mut opts = PipelineOptions::default();
        if let Some(dirs) = self.src_dirs {
            opts.src_dirs = dirs.into_iter().map(Into::into).collect();
        }
        if let Some(patterns) = self.exclude_patterns {
            opts.exclude_patterns = patterns;
        }
        if let Some(prefixes) = self.exclude_prefixes {
            opts.exclude_prefixes = prefixes;
        }
        if let Some(mode) = self.html_attributes.as_deref() {
            opts.html_attributes = HtmlAttributeMode::parse(mode)
                .map_err(|bad| format!("htmlAttributes is '{bad}', expected \"curated\", \"full\" or \"none\""))?;
        }
        if let Some(path) = self.tsconfig_path {
            opts.tsconfig_path = Some(path.into());
        }
        if let Some(dir) = self.cache_dir {
            opts.cache_dir = Some(dir.into());
        }
        if let Some(cache) = self.cache {
            opts.use_cache = cache;
        }
        if let Some(names) = self.extra_builtins {
            opts.extra_builtins = names.into_iter().map(Into::into).collect();
        }
        if let Some(load) = self.load_dependencies {
            opts.load_dependencies = load;
        }
        if let Some(mode) = self.collapse_inherited_props.as_deref() {
            opts.collapse_inherited_props = CollapseInheritedProps::parse(mode).map_err(|bad| {
                format!("collapseInheritedProps is '{bad}', expected \"dependencies\", \"all\" or \"none\"")
            })?;
        }
        if let Some(on) = self.experimental_semantic_types {
            opts.experimental_semantic_types = on;
        }
        if let Some(path) = self.tsc_path {
            opts.semantic_tsc_path = Some(path.into());
        }
        Ok(opts)
    }
}

/// The `--config` path names no file.
#[derive(Debug, miette::Diagnostic)]
#[diagnostic(help(
    "Check the path passed to --config, or drop the flag to look for docgen.config.ts upward from here."
))]
pub struct ConfigNotFound {
    pub path: std::path::PathBuf,
}

impl std::fmt::Display for ConfigNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config file {} does not exist", self.path.display())
    }
}

impl std::error::Error for ConfigNotFound {}

/// Find and load docgen.config.ts, walking up from start_dir to workspace root.
///
/// Returns `Ok(None)` when no config file exists anywhere between `start_dir` and the
/// workspace root — callers fall back to [`PipelineOptions::default`]. Returns `Err` when
/// a config file IS found but can't be honored; see [`try_load_config`].
pub fn load_config_file(start_dir: &std::path::Path) -> Result<Option<PipelineOptions>> {
    let mut dir = start_dir;
    loop {
        let candidate = dir.join("docgen.config.ts");
        if candidate.exists() {
            return try_load_config(&candidate);
        }
        // Stop at workspace root signals
        for signal in &["pnpm-workspace.yaml", "turbo.json", ".moon/workspace.yml"] {
            if dir.join(signal).exists() {
                return Ok(None);
            }
        }
        let Some(parent) = dir.parent() else { return Ok(None) };
        dir = parent;
    }
}

/// Execute `docgen.config.ts` at `path` and map its default export onto
/// [`PipelineOptions`].
///
/// A config file that fails to evaluate, or evaluates to something that doesn't
/// match [`DocgenConfigSchema`] (a typo'd key, wrong value type, or a field this
/// module doesn't support yet), is a hard error rather than silently falling back
/// to defaults — AGENTS.md non-negotiable #6: never fail silently. A
/// config a user explicitly wrote getting quietly ignored is a plausible-looking
/// but wrong result forever.
pub fn try_load_config(path: &std::path::Path) -> Result<Option<PipelineOptions>> {
    use std::io::Write;

    // Canonicalize before passing to node — command.current_dir(dir) below
    // changes the working directory to the config's parent, and a *relative*
    // __DOCGEN_CONFIG_PATH would then resolve against that new cwd instead of
    // the cwd this path was originally relative to (e.g. `--config
    // plain/docgen.config.ts` would fail with ERR_MODULE_NOT_FOUND even though
    // the file exists). A path that does not exist is reported first, naming
    // the path the user actually typed.
    if !path.exists() {
        return Err(ConfigNotFound { path: path.to_path_buf() }.into());
    }
    let path = &path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Pass the config path via environment variable to avoid any path content
    // being interpreted as JavaScript (command injection via crafted filenames).
    //
    // The config is the default export, or the module itself without one. A function is called and a promise
    // awaited, so `export default defineConfig(() => ...)` and `export default fetchConfig()` work. In a package
    // without `"type": "module"` tsx compiles the file to CommonJS, and importing that wraps the exports in a
    // `default` of their own, which `__esModule` marks for unwrapping.
    let script = "import { pathToFileURL } from 'node:url';\
        const p = process.env.__DOCGEN_CONFIG_PATH;\
        const m = await import(pathToFileURL(p).href);\
        let config = m.default ?? m;\
        if (config && typeof config === 'object' && config.__esModule && 'default' in config) config = config.default;\
        if (typeof config === 'function') config = config();\
        config = await config;\
        if (config === null || typeof config !== 'object' || Array.isArray(config)) {\
          const kind = config === null ? 'null' : Array.isArray(config) ? 'an array' : config === undefined ? 'undefined' : 'a ' + typeof config;\
          console.error('docgen.config.ts must export a config object (or a function or promise resolving to one), but its default export is ' + kind);\
          process.exit(1);\
        }\
        process.stdout.write(JSON.stringify(config));";

    let mut command = std::process::Command::new("node");
    command
        .args(["--input-type=module"])
        .env("NODE_OPTIONS", "--import=tsx/esm")
        .env("__DOCGEN_CONFIG_PATH", path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Resolve `tsx` (and anything the config file itself imports) relative to the
    // config file's own directory, not wherever the CLI happened to be invoked
    // from — matters for `--config ../other-project/docgen.config.ts` pointing
    // outside the current project, and for a project whose `tsx` devDependency
    // isn't hoisted to whatever the caller's own cwd happens to be.
    if let Some(dir) = path.parent() {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to spawn node to evaluate docgen.config.ts — is node installed and on PATH?")?;

    // drop(stdin) below signals EOF so node's read finishes.
    let mut stdin =
        child.stdin.take().ok_or_else(|| miette::miette!("failed to open stdin for the node subprocess"))?;
    stdin.write_all(script.as_bytes()).into_diagnostic()?;
    drop(stdin);

    let output = child.wait_with_output().into_diagnostic()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(miette::miette!(
            help = "Check docgen.config.ts for syntax errors, or remove it to use defaults.",
            "docgen.config.ts at {} failed to evaluate ({}):\n{}",
            path.display(),
            output.status,
            stderr.trim(),
        ));
    }

    let schema: DocgenConfigSchema = serde_json::from_slice(&output.stdout).map_err(|e| {
        miette::miette!(
            help = format!(
                "Check docgen.config.ts's exported fields against the supported schema ({}).",
                CONFIG_KEYS.join(", ")
            ),
            "docgen.config.ts at {} doesn't match the expected shape: {}",
            path.display(),
            e,
        )
    })?;

    let names_src_dirs = schema.src_dirs.is_some();
    let mut opts =
        schema.into_pipeline_options().map_err(|e| miette::miette!("docgen.config.ts at {}: {}", path.display(), e))?;
    // A path the config names is relative to the config, not to wherever the CLI runs from.
    if let Some(dir) = path.parent().and_then(camino::Utf8Path::from_path) {
        for src_dir in opts.src_dirs.iter_mut().filter(|src_dir| names_src_dirs && src_dir.is_relative()) {
            *src_dir = dir.join(&*src_dir);
        }
        if let Some(tsconfig) = opts.tsconfig_path.as_mut().filter(|tsconfig| tsconfig.is_relative()) {
            *tsconfig = dir.join(&*tsconfig);
        }
        if let Some(cache_dir) = opts.cache_dir.as_mut().filter(|cache_dir| cache_dir.is_relative()) {
            *cache_dir = dir.join(&*cache_dir);
        }
        if let Some(tsc) = opts.semantic_tsc_path.as_mut().filter(|tsc| tsc.is_relative()) {
            *tsc = dir.join(&*tsc);
        }
    }
    Ok(Some(opts))
}

/// CLI-flag overrides for [`build_options`], bundled to stay under this crate's
/// `too-many-arguments-threshold` of 6 (see `.clippy.toml`).
pub struct BuildOptionsArgs<'a> {
    pub src: &'a [String],
    pub cache_dir: Option<&'a str>,
    /// `--no-cache`: turns the on-disk caches off over the config; absent leaves the config's choice.
    pub no_cache: bool,
    pub html_attributes: Option<&'a str>,
    pub config_path: Option<&'a str>,
    pub extra_builtins: &'a [String],
    /// `--load-dependencies`: turns loading on over the config; absent leaves the config's choice.
    pub load_dependencies: bool,
    /// `--no-load-dependencies`: turns loading off over the config; absent leaves the config's choice.
    pub no_load_dependencies: bool,
    /// `--collapse-inherited-props`; absent leaves the config's choice.
    pub collapse_inherited_props: Option<CollapseInheritedProps>,
}

pub fn build_options(args: BuildOptionsArgs) -> Result<PipelineOptions> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let config_override = match args.config_path {
        Some(p) => try_load_config(std::path::Path::new(p))?,
        None => load_config_file(&cwd)?,
    };

    let mut opts = config_override.unwrap_or_default();

    if !args.src.is_empty() {
        opts.src_dirs = args.src.iter().map(|s| s.into()).collect();
    }
    if let Some(dir) = args.cache_dir {
        opts.cache_dir = Some(dir.into());
    }
    if args.no_cache {
        opts.use_cache = false;
    }
    if let Some(mode) = args.html_attributes {
        opts.html_attributes = HtmlAttributeMode::parse(mode).map_err(|bad| {
            miette::miette!(
                help = "Expected \"curated\", \"full\" or \"none\".",
                "--html-attributes is '{}', which isn't a recognized value",
                bad
            )
        })?;
    }
    if !args.extra_builtins.is_empty() {
        opts.extra_builtins = args.extra_builtins.iter().map(Into::into).collect();
    }
    if args.load_dependencies {
        opts.load_dependencies = true;
    }
    if args.no_load_dependencies {
        opts.load_dependencies = false;
    }
    if let Some(mode) = args.collapse_inherited_props {
        opts.collapse_inherited_props = mode;
    }

    Ok(opts)
}

/// The note for a command that never runs the TypeScript checker (everything but `extract`) when the config switches
/// it on anyway: the setting is read, and then has no effect, which a config the user wrote should not do quietly.
/// `None` when the config leaves the checker alone.
pub fn ignored_semantic_config_note(command: &str, options: &PipelineOptions) -> Option<String> {
    let keys: Vec<&str> = [
        (options.experimental_semantic_types, "experimentalSemanticTypes"),
        (options.semantic_tsc_path.is_some(), "tscPath"),
    ]
    .into_iter()
    .filter_map(|(set, key)| set.then_some(key))
    .collect();
    if keys.is_empty() {
        return None;
    }
    Some(format!(
        "warning: the config sets {}, but `{command}` never runs the TypeScript checker, so its output is the \
         structural result. Only `extract` runs the checker.",
        keys.join(" and ")
    ))
}

/// Writes [`ignored_semantic_config_note`] to stderr, unless `quiet`.
pub fn warn_ignored_semantic_config(command: &str, options: &PipelineOptions, quiet: bool) {
    use std::io::Write;

    if quiet {
        return;
    }
    if let Some(note) = ignored_semantic_config_note(command, options) {
        // A closed stderr isn't worth failing a run over.
        writeln!(std::io::stderr().lock(), "{note}").ok();
    }
}

#[cfg(test)]
mod tests;
