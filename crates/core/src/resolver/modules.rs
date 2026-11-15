//! Module and `.d.ts` resolution, through `tsrs_module` and `tsrs_tsoptions`: the algorithm TypeScript itself runs
//! (`exports` conditions, `typesVersions`, the `@types` package, `paths`), and the one the linked checker runs, so
//! the structural answer and the checker's cannot disagree about which file a name comes from.
//!
//! tsrs allocates what it returns in a per-thread arena that is never freed, so each run's resolvers live in a
//! [`Region`] that goes with them: a watch session builds a fresh `ModuleResolvers` per revision (a file edit can
//! change what a specifier resolves to), and each one's caches are freed when it is.

use std::sync::{LazyLock, Mutex, PoisonError};

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::arena::Region;
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, P};
use tsrs_module::{DefaultResolver, ResolutionHost, ResolverOptions, new_resolver};
use tsrs_tsoptions::tsconfigparsing::{ParseConfigHost, get_parsed_command_line_of_config_file};
use tsrs_vfs::cachedvfs;

use super::ModuleResolution;
use crate::types::options::PipelineOptions;

/// The file system every resolution reads: tsrs's cached view of the disk. Uncached, resolution stats every candidate
/// and is three to four times slower on a project with installed packages.
static FILE_SYSTEM: LazyLock<cachedvfs::FS<&'static dyn tsrs_vfs::FS>> =
    LazyLock::new(|| cachedvfs::from(tsrs_vfs::osvfs::fs()));

struct Host;

impl ResolutionHost for Host {
    fn fs(&self) -> &dyn tsrs_vfs::FS {
        &*FILE_SYSTEM
    }

    fn get_current_directory(&self) -> &str {
        "/"
    }
}

impl ParseConfigHost for Host {
    fn fs(&self) -> &dyn tsrs_vfs::FS {
        &*FILE_SYSTEM
    }

    fn get_current_directory(&self) -> &str {
        "/"
    }
}

static HOST: Host = Host;

/// The resolvers of a run, one per governing tsconfig, configured the way TypeScript resolves this project's modules:
/// `bundler` resolution, the governing tsconfig's `extends`, `baseUrl`, `paths` and `customConditions`, and an import
/// written with a JavaScript extension (`./button.js`) finding its TypeScript source. The governing tsconfig is the
/// configured one, or else the nearest one above each importing file.
pub(crate) struct ModuleResolvers {
    configured: Option<Utf8PathBuf>,
    state: Mutex<State>,
    /// Last: it frees what `state` holds.
    region: Region,
}

#[derive(Default)]
struct State {
    resolvers: FxHashMap<Option<Utf8PathBuf>, DefaultResolver<'static>>,
    configs: FxHashMap<Utf8PathBuf, ParsedConfig>,
    /// The nearest tsconfig of a directory.
    nearest: FxHashMap<Box<str>, Option<Utf8PathBuf>>,
    /// The tsconfig governing a file.
    governing: FxHashMap<Box<str>, Option<Utf8PathBuf>>,
}

/// A tsconfig as `tsrs_tsoptions` reads it.
struct ParsedConfig {
    options: Option<P<CompilerOptions>>,
    /// Why the tsconfig could not be read or parsed, if it could not.
    error: Option<String>,
    /// The files its `files`/`include` select: which project of a solution-style config a file belongs to.
    files: FxHashSet<String>,
    references: Vec<Utf8PathBuf>,
    /// The config files it extends, directly and through the chain.
    extended: Vec<Utf8PathBuf>,
}

/// `path` made absolute against the working directory: tsrs resolves absolute paths only, and takes a relative one
/// as relative to `/`.
fn absolute(path: &Utf8Path) -> Utf8PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }
    std::path::absolute(path)
        .ok()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .unwrap_or_else(|| path.to_owned())
}

/// What a module resolution cache keyed by directory must also key on: a `.cts` importer resolves with `require`
/// conditions where every other importer resolves with `import`, so it gets a key of its own.
pub(crate) fn resolution_scope(from_file: &Utf8Path) -> &str {
    if is_commonjs_importer(from_file) { from_file.as_str() } else { from_file.parent().map_or("", Utf8Path::as_str) }
}

fn is_commonjs_importer(file: &Utf8Path) -> bool {
    file.as_str().ends_with(".cts")
}

impl ModuleResolvers {
    /// Starts from a fresh view of the disk: the file-system cache of an earlier run is dropped.
    pub(crate) fn new(options: &PipelineOptions) -> Self {
        FILE_SYSTEM.clear_cache();
        Self::without_invalidation(options)
    }

    /// `new` minus the cache clear, to show in a test that the clear is what makes a revision see an edit.
    #[cfg(test)]
    pub(crate) fn keeping_file_system_cache(options: &PipelineOptions) -> Self {
        Self::without_invalidation(options)
    }

    fn without_invalidation(options: &PipelineOptions) -> Self {
        ModuleResolvers {
            configured: options.tsconfig_path.as_deref().map(absolute),
            state: Mutex::default(),
            region: Region::new_scratch(1 << 20),
        }
    }

    /// Resolve `specifier` as imported by `from_file`. A bare specifier that resolves to a declaration file is
    /// [`ModuleResolution::PackageTypes`]: TypeScript prefers a package's declarations over its JavaScript entry point
    /// and falls back to its `@types` package, so `react` lands on `@types/react`, not on `index.js`.
    pub(crate) fn resolve(&self, specifier: &str, from_file: &Utf8Path) -> ModuleResolution {
        let Some(path) = self.resolve_path(specifier, from_file) else {
            return ModuleResolution::Unresolved(format!("Cannot find module '{specifier}'"));
        };
        let bare = !specifier.starts_with('.') && !specifier.starts_with('/');
        if bare && is_declaration_file(&path) {
            ModuleResolution::PackageTypes(path)
        } else {
            ModuleResolution::File(path)
        }
    }

    /// The declaration file `package_name` resolves to from `importing_file`, if it resolves to one.
    pub(crate) fn package_declaration(&self, importing_file: &Utf8Path, package_name: &str) -> Option<String> {
        self.resolve_path(package_name, importing_file).filter(|path| is_declaration_file(path)).map(String::from)
    }

    /// Why `tsconfig` could not be used, if it could not.
    pub(crate) fn tsconfig_error(&self, tsconfig: &Utf8Path) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let _scope = self.region.enter();
        parsed_config(&mut state.configs, &absolute(tsconfig)).error.clone()
    }

    fn resolve_path(&self, specifier: &str, from_file: &Utf8Path) -> Option<Utf8PathBuf> {
        let from_file = absolute(from_file);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        // A region is entered by one thread at a time; the lock above makes that wait explicit.
        let _scope = self.region.enter();
        let tsconfig = governing_tsconfig(&mut state, self.configured.as_deref(), &from_file);
        let base_url = tsconfig
            .as_deref()
            .and_then(|path| parsed_config(&mut state.configs, path).options)
            .map(|options| options.base_url.clone())
            .unwrap_or_default();
        let resolver = match state.resolvers.get(&tsconfig) {
            Some(resolver) => resolver,
            None => {
                let config = tsconfig.as_deref().map(|path| parsed_config(&mut state.configs, path));
                let options = config_options(tsconfig.as_deref(), config);
                state
                    .resolvers
                    .entry(tsconfig)
                    .or_insert_with(|| new_resolver(ResolverOptions::new(&HOST, P::new(options))))
            }
        };
        // `import` conditions, as TypeScript's `bundler` resolution applies them to the `import` statements declaration
        // files are made of; a package that only declares `require` types is found by the second attempt, as it was
        // when the conditions were listed together.
        let (first, second) = if is_commonjs_importer(&from_file) {
            (ModuleKind::COMMON_JS, ModuleKind::ES_NEXT)
        } else {
            (ModuleKind::ES_NEXT, ModuleKind::COMMON_JS)
        };
        let attempt = |specifier: &str| {
            [first, second].into_iter().find_map(|mode| {
                let (module, _) = resolver.resolve_module_name(specifier, from_file.as_str(), mode, None).ok()?;
                module.is_resolved().then(|| Utf8PathBuf::from(module.resolved_file_name))
            })
        };
        let mut found = attempt(specifier);
        // TypeScript 7 no longer resolves a bare specifier against `baseUrl` (only `paths` read it), which TypeScript 6
        // and the resolver this replaced did, after `paths` and before `node_modules`: `lib/index` with `baseUrl: "."`
        // is `./lib/index`. A specifier `paths` mapped is a project file, and one `node_modules` answered is not.
        let bare = !specifier.starts_with('.') && !specifier.starts_with('/');
        if bare
            && !base_url.is_empty()
            && found.as_ref().is_none_or(|path| crate::dependency_types::in_node_modules(path.as_str()))
        {
            found = attempt(&format!("{}/{specifier}", base_url.trim_end_matches('/'))).or(found);
        }
        // tsrs keeps a symlink in the path it returns, and writes it with forward slashes; discovery, the declaration
        // index and every comparison between files use the real path.
        let found = found?;
        Some(crate::canonical_path::canonicalize(&found).unwrap_or(found))
    }
}

/// The tsconfig that governs `file`: the configured or nearest one, or, when that is a solution-style config that
/// references other projects and does not include the file itself, the first referenced project that does.
fn governing_tsconfig(state: &mut State, configured: Option<&Utf8Path>, file: &Utf8Path) -> Option<Utf8PathBuf> {
    if let Some(known) = state.governing.get(file.as_str()) {
        return known.clone();
    }
    let mut found = match configured {
        Some(path) => Some(path.to_owned()),
        None => nearest_tsconfig(&mut state.nearest, file.parent()?),
    };
    // Bounds a chain of references.
    for _ in 0..8 {
        let Some(current) = found.clone() else { break };
        let config = parsed_config(&mut state.configs, &current);
        if config.files.contains(file.as_str()) {
            break;
        }
        let references = config.references.clone();
        let Some(next) = references
            .into_iter()
            .find(|reference| parsed_config(&mut state.configs, reference).files.contains(file.as_str()))
        else {
            break;
        };
        found = Some(next);
    }
    state.governing.insert(Box::from(file.as_str()), found.clone());
    found
}

fn parsed_config<'a>(configs: &'a mut FxHashMap<Utf8PathBuf, ParsedConfig>, path: &Utf8Path) -> &'a ParsedConfig {
    configs.entry(path.to_owned()).or_insert_with(|| ParsedConfig::read(path))
}

impl ParsedConfig {
    fn read(path: &Utf8Path) -> Self {
        let (parsed, errors) = get_parsed_command_line_of_config_file(path.as_str(), None, None, &HOST, None);
        // Only a file that can't be read or isn't JSON is unusable; the semantic complaints of a config
        // (`include` matching nothing, a misspelled option) leave its resolution options intact.
        let syntax_errors = parsed
            .as_ref()
            .and_then(|parsed| parsed.config_file)
            .map_or(&[][..], |file| file.source_file.diagnostics());
        let mut error = errors.iter().chain(syntax_errors).next().map(|e| e.localize());
        let Some(parsed) = parsed else {
            return ParsedConfig {
                options: None,
                error: error.or_else(|| Some("unreadable".to_owned())),
                files: FxHashSet::default(),
                references: Vec::new(),
                extended: Vec::new(),
            };
        };
        let options = parsed.parsed_config.compiler_options;
        if options.is_none() && error.is_none() {
            error = Some("unreadable".to_owned());
        }
        let references = parsed
            .parsed_config
            .project_references
            .iter()
            .map(|reference| {
                let reference = Utf8PathBuf::from(&reference.path);
                if reference.extension() == Some("json") { reference } else { reference.join("tsconfig.json") }
            })
            .collect();
        let extended = parsed.config_file.map_or_else(Vec::new, |file| {
            let files = file.extended_source_files.lock().unwrap_or_else(PoisonError::into_inner);
            files.iter().map(Utf8PathBuf::from).collect()
        });
        ParsedConfig {
            options,
            error,
            files: parsed.parsed_config.file_names.iter().cloned().collect(),
            references,
            extended,
        }
    }
}

/// What resolution reads from the tsconfig `root` governs, as text: every config file of its `extends` chain and
/// `references` (path, modification time, size) and the resolution-relevant compiler options after `extends` of each.
/// It names the answers a persisted cache may keep: any change to one of them changes the text. The root file's own
/// modification time says nothing about the files it extends.
pub(crate) fn effective_inputs(root: Option<&Utf8Path>) -> String {
    let Some(root) = root else { return "no tsconfig".to_owned() };
    FILE_SYSTEM.clear_cache();
    // Declared before `configs`, so it is freed after the options that live in it.
    let region = Region::new_scratch(1 << 20);
    let _scope = region.enter();
    let mut configs = FxHashMap::default();
    let mut seen: std::collections::BTreeSet<Utf8PathBuf> = std::collections::BTreeSet::new();
    let mut pending = vec![absolute(root)];
    let mut text = String::new();
    // Bounds a cycle of references.
    while let Some(path) = pending.pop() {
        if seen.len() >= 64 || !seen.insert(path.clone()) {
            continue;
        }
        let config = parsed_config(&mut configs, &path);
        pending.extend(config.extended.iter().cloned());
        pending.extend(config.references.iter().cloned());
        let stamp = std::fs::metadata(&path).ok().map(|meta| {
            let mtime = meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
            (mtime.map(|age| age.as_nanos()), meta.len())
        });
        let options = config.options.map(|options| {
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                options.custom_conditions,
                options.paths,
                options.base_url,
                options.paths_base_path,
                options.root_dirs,
                options.module_suffixes,
                options.module_resolution,
                options.resolve_json_module,
                options.resolve_package_json_exports,
                options.resolve_package_json_imports,
            )
        });
        text.push_str(&format!("{path}|{stamp:?}|{options:?}|{:?}\n", config.error));
    }
    // Sorted by path, so the text does not depend on the order the chain was walked in.
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    format!("{}|tsrs_module {}", lines.join("\n"), TSRS_PIN)
}

/// The `tsrs_*` version the resolver is built from (the workspace pins it exactly).
const TSRS_PIN: &str = "0.24.0";

/// `bundler` resolution with what a tsconfig contributes to resolving: `baseUrl`, `paths` and `customConditions`.
fn config_options(path: Option<&Utf8Path>, config: Option<&ParsedConfig>) -> CompilerOptions {
    let mut options = CompilerOptions {
        module: ModuleKind::ES_NEXT,
        module_resolution: ModuleResolutionKind::BUNDLER,
        ..CompilerOptions::default()
    };
    if let Some(config) = config.and_then(|config| config.options) {
        options.config_file_path = path.map(Utf8Path::to_string).unwrap_or_default();
        options.base_url.clone_from(&config.base_url);
        options.paths.clone_from(&config.paths);
        options.paths_base_path.clone_from(&config.paths_base_path);
        options.custom_conditions.clone_from(&config.custom_conditions);
    }
    options
}

/// The nearest `tsconfig.json` in `dir` or above it. Every file of a directory shares one.
fn nearest_tsconfig(known: &mut FxHashMap<Box<str>, Option<Utf8PathBuf>>, dir: &Utf8Path) -> Option<Utf8PathBuf> {
    if let Some(found) = known.get(dir.as_str()) {
        return found.clone();
    }
    let found = dir
        .ancestors()
        .map(|ancestor| ancestor.join("tsconfig.json"))
        .find(|candidate| tsrs_vfs::FS::file_exists(&*FILE_SYSTEM, candidate.as_str()));
    known.insert(Box::from(dir.as_str()), found.clone());
    found
}

fn is_declaration_file(path: &Utf8Path) -> bool {
    [".d.ts", ".d.mts", ".d.cts"].iter().any(|ext| path.as_str().ends_with(ext))
}

#[cfg(test)]
mod tests;
