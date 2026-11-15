//! Module and `.d.ts` resolution, through `tsrs_module` and `tsrs_tsoptions`: the algorithm TypeScript itself runs
//! (`exports` conditions, `typesVersions`, the `@types` package, `paths`), and the one the linked checker runs, so
//! the structural answer and the checker's cannot disagree about which file a name comes from.
//!
//! tsrs allocates what it returns in a per-thread arena that is never freed, so each run's resolvers live in a
//! [`Region`] that goes with them: a watch session builds a fresh `ModuleResolvers` per revision (a file edit can
//! change what a specifier resolves to), and each one's caches are freed when it is.

use std::sync::{LazyLock, Mutex, PoisonError};

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::FxHashMap;
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
    resolvers: FxHashMap<Option<Utf8PathBuf>, ConfigResolver>,
    /// The nearest tsconfig of a directory.
    nearest: FxHashMap<Box<str>, Option<Utf8PathBuf>>,
}

struct ConfigResolver {
    resolver: DefaultResolver<'static>,
    /// Why the tsconfig could not be read or parsed, if it could not.
    error: Option<String>,
}

impl ModuleResolvers {
    /// Starts from a fresh view of the disk: the file-system cache of an earlier run is dropped.
    pub(crate) fn new(options: &PipelineOptions) -> Self {
        FILE_SYSTEM.clear_cache();
        ModuleResolvers {
            configured: options.tsconfig_path.clone(),
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

    /// Why the configured or nearest tsconfig of `from_file` could not be used, if it could not.
    pub(crate) fn tsconfig_error(&self, tsconfig: &Utf8Path) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let _scope = self.region.enter();
        state
            .resolvers
            .entry(Some(tsconfig.to_owned()))
            .or_insert_with(|| ConfigResolver::new(Some(tsconfig)))
            .error
            .clone()
    }

    fn resolve_path(&self, specifier: &str, from_file: &Utf8Path) -> Option<Utf8PathBuf> {
        // tsrs resolves absolute paths only.
        let absolute;
        let from_file = if from_file.is_absolute() {
            from_file
        } else {
            absolute = Utf8PathBuf::from_path_buf(std::path::absolute(from_file).ok()?).ok()?;
            &absolute
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        // A region is entered by one thread at a time; the lock above makes that wait explicit.
        let _scope = self.region.enter();
        let tsconfig = match &self.configured {
            Some(path) => Some(path.clone()),
            None => nearest_tsconfig(&mut state.nearest, from_file.parent()?),
        };
        let config = state.resolvers.entry(tsconfig).or_insert_with_key(|key| ConfigResolver::new(key.as_deref()));
        let (module, _) =
            config.resolver.resolve_module_name(specifier, from_file.as_str(), ModuleKind::ES_NEXT, None).ok()?;
        module.is_resolved().then(|| Utf8PathBuf::from(module.resolved_file_name))
    }
}

impl ConfigResolver {
    fn new(tsconfig: Option<&Utf8Path>) -> Self {
        let mut options = CompilerOptions {
            module: ModuleKind::ES_NEXT,
            module_resolution: ModuleResolutionKind::BUNDLER,
            ..CompilerOptions::default()
        };
        let mut error = None;
        if let Some(path) = tsconfig {
            let (parsed, errors) = get_parsed_command_line_of_config_file(path.as_str(), None, None, &HOST, None);
            // Only a file that can't be read or isn't JSON is unusable; the semantic complaints of a config
            // (`include` matching nothing, a misspelled option) leave its resolution options intact.
            let syntax_errors = parsed
                .as_ref()
                .and_then(|parsed| parsed.config_file)
                .map_or(&[][..], |file| file.source_file.diagnostics());
            error = errors.iter().chain(syntax_errors).next().map(|e| e.localize());
            if let Some(config) = parsed.and_then(|parsed| parsed.parsed_config.compiler_options) {
                options.config_file_path = path.to_string();
                options.base_url.clone_from(&config.base_url);
                options.paths.clone_from(&config.paths);
                options.paths_base_path.clone_from(&config.paths_base_path);
                options.custom_conditions.clone_from(&config.custom_conditions);
            } else if error.is_none() {
                error = Some("unreadable".to_owned());
            }
        }
        ConfigResolver { resolver: new_resolver(ResolverOptions::new(&HOST, P::new(options))), error }
    }
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
