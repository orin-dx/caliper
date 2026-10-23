//! Prop-type resolver — Phase 3a.
//!
//! Converts `ComponentMapping` (raw collected data) into a fully resolved
//! `ComponentEntry` with `PropType` props.
//!
//! Central design:
//! - `resolve_component` is the entry point (called in parallel per component).
//! - `resolve_collected_type` is the recursive dispatch for `CollectedType → PropType`.
//! - No string parsing happens here — everything is already structured `CollectedType`.
//! - Graceful degradation: every unresolvable type becomes `PropType::Opaque` + a `Diagnostic`.

use std::collections::BTreeMap;
use std::sync::Arc;

#[allow(unused_imports)]
use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use oxc_resolver::{ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::import_map::ImportResolutionMap;
use crate::pipeline::PipelineOptions;
use crate::react_types;
use crate::types::*;

mod alias;
mod chain;
mod collected;
mod deps;
pub(crate) use deps::Dependencies;
mod extends;
mod func;
mod html;
mod import;
mod named;
mod precedence;
mod primitives;
mod react;
mod substitute;
mod template;

#[cfg(test)]
pub(crate) use import::{follow_reexports_call_count, reset_follow_reexports_call_count};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum recursion depth for type resolution.
/// Prevents infinite loops on circular type references.
const MAX_DEPTH: u8 = 20;

/// Shared "max resolution depth exceeded" diagnostic — every recursive
/// resolver entry point (resolve_named, resolve_props_chain,
/// resolve_collected_type) hits this same guard, and previously each built
/// its own slightly different message with only one of the three including
/// the circular-reference hint. `what` should read naturally after
/// "resolving", e.g. "type 'Foo'" or "named type 'Bar'".
pub(super) fn max_depth_diagnostic(what: &str, file: &Utf8Path) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!("Max resolution depth exceeded resolving {what}"),
        file: Some(file.to_string()),
        line: None,
        column: None,
        help: Some("This may indicate a circular type reference.".into()),
        code: DiagnosticCode::MaxDepthExceeded,
        affects: Vec::new(),
    }
}

/// Import resolution may have redirected a name to a different name/file
/// (re-exports, barrel files) — surface that resolved location in an
/// "unresolvable type" diagnostic when it differs from the naive
/// name/consuming-file, since "Cannot resolve X in file A" is confusing if X
/// actually lives in file B. Returns an empty string when nothing to add.
pub(super) fn unresolved_location_note(
    original_name: &str,
    consuming_file: &Utf8Path,
    canonical_file: &Utf8Path,
    canonical_name: &str,
) -> String {
    let resolved_elsewhere = canonical_file != consuming_file || canonical_name != original_name;
    if resolved_elsewhere {
        format!(" (resolved to '{canonical_name}' in '{canonical_file}')")
    } else {
        String::new()
    }
}

/// What a name used in `file` is bound to, for deciding whether it may take a builtin meaning: a project-local
/// `ReactNode` or `Partial`, or one imported from some other package, is that declaration, not React's or TypeScript's.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NameBinding {
    /// Declared in `file`.
    Local,
    /// Imported in `file` (for a qualified name, its namespace is) from this specifier.
    Imported(String),
    /// Neither: a global, such as TypeScript's lib types or React's ambient namespace.
    Unbound,
}

pub(super) fn name_binding(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> NameBinding {
    deps::record(file);
    let head = name.split('.').next().unwrap_or(name);
    if head == name && ctx.named_types.is_declared_at(file, name) {
        return NameBinding::Local;
    }
    match ctx.import_map.find_import(file, head) {
        Some(import) => NameBinding::Imported(import.specifier.clone()),
        None => NameBinding::Unbound,
    }
}

/// Whether `name` in `file` can be React's own type: imported from React, or not bound at all.
pub(super) fn may_be_react_builtin(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> bool {
    match name_binding(name, file, ctx) {
        NameBinding::Local => false,
        NameBinding::Imported(specifier) => {
            specifier == "react" || specifier.starts_with("react/") || specifier == "@types/react"
        }
        NameBinding::Unbound => true,
    }
}

/// Reports that a builtin name bound to something else was given React's meaning because that binding didn't resolve.
pub(super) fn builtin_fallback_diagnostic(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> Diagnostic {
    let bound_to = match name_binding(name, file, ctx) {
        NameBinding::Imported(specifier) => format!("imported from '{specifier}'"),
        NameBinding::Local => "declared in this file".to_owned(),
        NameBinding::Unbound => "unbound".to_owned(),
    };
    Diagnostic::new(
        DiagnosticSeverity::Info,
        DiagnosticCode::UnresolvableImport,
        format!("'{name}' in '{file}' is {bound_to}, which could not be resolved; treated as React's '{name}'"),
    )
    .with_file(file.to_string())
    .with_help("If it is a different type, make its declaration resolvable from this file.")
}

/// Whether `name` in `file` can be the utility `synthesize_utility_alias` would build for it: React's own helpers
/// (`PropsWithChildren`, `PropsWithRef`) need React's binding, TypeScript's (`Omit`, `Partial`, ...) need none.
pub(super) fn may_be_utility(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> bool {
    let bare = name.strip_prefix("React.").unwrap_or(name);
    if matches!(bare, "PropsWithChildren" | "PropsWithRef") {
        may_be_react_builtin(name, file, ctx)
    } else {
        may_be_ts_lib_type(name, file, ctx)
    }
}

/// Whether `name`, as bound in `file`, leads to a declaration the index holds.
pub(super) fn declaration_resolves(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> bool {
    let mut ignored = Vec::new();
    import::resolve_to_canonical(name, file, ctx, &mut ignored).is_some_and(|(canonical_file, canonical_name)| {
        ctx.named_types.is_declared_at(&canonical_file, &canonical_name)
    }) || ctx.named_types.is_declared_at(file, name)
}

/// Whether `name` in `file` can be one of TypeScript's lib types, which are globals and never imported.
pub(super) fn may_be_ts_lib_type(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> bool {
    name_binding(name, file, ctx) == NameBinding::Unbound
}

/// TypeScript's own generic utility types — never prop providers, and expanding
/// them structurally needs a type checker we don't have. `Omit`/`Pick`/`Partial`/
/// `Readonly` get real structural handling as alias shapes (see resolver/alias.rs);
/// this list exists so every OTHER arity/usage of these names (and the ones with
/// no structural handling at all) degrades to a silent no-op instead of a
/// spurious "cannot resolve" warning. Shared by chain.rs (props level) and
/// named.rs (type level) — previously duplicated as two independently-maintained
/// name lists that happened to agree, with no guard against them drifting apart.
pub(super) fn is_ts_utility_type(name: &str) -> bool {
    matches!(
        name,
        "Omit"
            | "Pick"
            | "Partial"
            | "Required"
            | "Readonly"
            | "NonNullable"
            | "ReturnType"
            | "Parameters"
            | "Awaited"
            | "Extract"
            | "Exclude"
            | "Record"
            | "ReadonlyArray"
            | "Array"
            | "Promise"
    )
}

// ─── ResolutionContext ────────────────────────────────────────────────────────

/// Shared, read-only context passed to all resolution functions.
/// Arc'd so it can be cheaply shared across rayon threads.
pub struct ResolutionContext {
    pub global: Arc<GlobalSourceData>,
    pub import_map: Arc<ImportResolutionMap>,
    /// Fast `(file, name)` index over `global.interfaces`/`type_aliases` and
    /// their declared generic type parameters — see `named_type_index.rs`'s
    /// module doc for why this exists alongside (not instead of) those flat
    /// maps.
    pub(crate) named_types: Arc<crate::named_type_index::NamedTypeIndex>,
    pub oxc_resolver: Arc<Resolver>,
    /// Module resolutions by `(directory, specifier)`. Resolution only reads the file system, which a run treats as
    /// fixed, and every component importing from the same directory asks again; a watch revision builds a fresh
    /// context, so nothing outlives one revision.
    pub(crate) module_resolutions:
        dashmap::DashMap<(Box<str>, Box<str>), import::ModuleResolution, rustc_hash::FxBuildHasher>,
    pub extra_builtins: FxHashSet<CompactString>,
    /// Bare name → full scoped key ("file:name") index over `global.enums`,
    /// built once per resolution pass so `resolve_typeof` and
    /// `resolve_cva_variant_props` (VariantProps<typeof X>) can do an O(1)
    /// lookup instead of a linear scan with a `format!()` allocation per
    /// candidate on every reference. Ambiguous bare names (same name in two
    /// files) resolve to whichever key `global.enums` yields first during
    /// this build pass — the same tie-break the old linear scan produced,
    /// since both iterate the same underlying map.
    pub enum_bare_index: FxHashMap<CompactString, CompactString>,
    /// Same as `enum_bare_index`, but over `global.const_arrays` — for
    /// `(typeof X)[number]` on a flat `const X = [...] as const` array.
    pub const_array_bare_index: FxHashMap<CompactString, CompactString>,
    /// How much of an inherited HTML element's attributes to expose.
    pub html_attributes: crate::pipeline::HtmlAttributeMode,
    /// Paths to TypeScript's own `lib.es5.d.ts`/`lib.dom.d.ts`, if resolvable
    /// from this project (empty otherwise — see `resolve_ts_lib_paths`).
    /// Checked as a last-resort fallback when a bare, never-imported name
    /// (a real native/DOM global like `Date`) doesn't resolve via the normal
    /// same-file lookup — see `import::lookup_interface`/`lookup_type_alias`.
    pub ambient_global_files: Vec<Utf8PathBuf>,
    /// Why the configured tsconfig could not be used, if it could not.
    pub(crate) tsconfig_diagnostic: Option<Diagnostic>,
}

impl ResolutionContext {
    #[cfg(test)]
    pub fn new(global: Arc<GlobalSourceData>, options: &PipelineOptions) -> Self {
        let named_types = Arc::new(crate::named_type_index::NamedTypeIndex::build(&global));
        Self::build(global, options, compute_ambient_global_files(options), named_types)
    }

    /// A context over an index and ambient file list the caller already holds. A watch revision patches its
    /// `NamedTypeIndex` per edited file; rebuilding it from the whole merged `global` on every save measurably
    /// regressed watch latency (see `NamedTypeIndex::remove_file`).
    pub fn new_with_cached_ambient_global_files(
        global: Arc<GlobalSourceData>,
        options: &PipelineOptions,
        ambient_global_files: Vec<Utf8PathBuf>,
        named_types: Arc<crate::named_type_index::NamedTypeIndex>,
    ) -> Self {
        Self::build(global, options, ambient_global_files, named_types)
    }

    fn build(
        global: Arc<GlobalSourceData>,
        options: &PipelineOptions,
        ambient_global_files: Vec<Utf8PathBuf>,
        named_types: Arc<crate::named_type_index::NamedTypeIndex>,
    ) -> Self {
        // The resolver reads the tsconfig itself: comments, `extends`, `baseUrl` and `paths` wildcards as TypeScript
        // reads them.
        let resolve_options = ResolveOptions {
            condition_names: vec!["types".into(), "import".into(), "require".into(), "default".into()],
            main_fields: vec!["types".into(), "typings".into(), "module".into(), "main".into()],
            extensions: vec![".ts".into(), ".tsx".into(), ".d.ts".into(), ".js".into()],
            tsconfig: options.tsconfig_path.as_ref().map(|path| {
                TsconfigDiscovery::Manual(TsconfigOptions {
                    config_file: path.as_std_path().to_owned(),
                    references: TsconfigReferences::Auto,
                })
            }),
            ..ResolveOptions::default()
        };
        let oxc_resolver = Resolver::new(resolve_options);
        // A tsconfig that can't be read or parsed would otherwise fail every aliased import one by one.
        let tsconfig_diagnostic = options.tsconfig_path.as_ref().and_then(|path| {
            let error = oxc_resolver.resolve_tsconfig(path.as_std_path()).err()?;
            Some(
                Diagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::IoError,
                    format!("Could not use the tsconfig '{path}': {error}"),
                )
                .with_file(path.to_string())
                .with_help("Its `paths` aliases are not applied; imports that rely on them will not resolve."),
            )
        });

        let mut enum_bare_index: FxHashMap<CompactString, CompactString> = FxHashMap::default();
        // Sorted so a bare name shared by several files binds to the same one regardless of hash order.
        let mut enum_keys: Vec<&String> = global.enums.keys().collect();
        enum_keys.sort();
        for key in enum_keys {
            let bare = key.rsplit_once(':').map(|(_, name)| name).unwrap_or(key.as_str());
            enum_bare_index.entry(CompactString::from(bare)).or_insert_with(|| CompactString::from(key.as_str()));
        }

        let mut const_array_bare_index: FxHashMap<CompactString, CompactString> = FxHashMap::default();
        let mut const_array_keys: Vec<&String> = global.const_arrays.keys().collect();
        const_array_keys.sort();
        for key in const_array_keys {
            let bare = key.rsplit_once(':').map(|(_, name)| name).unwrap_or(key.as_str());
            const_array_bare_index
                .entry(CompactString::from(bare))
                .or_insert_with(|| CompactString::from(key.as_str()));
        }

        Self {
            import_map: Arc::new(ImportResolutionMap::build(&global)),
            named_types,
            global,
            oxc_resolver: Arc::new(oxc_resolver),
            tsconfig_diagnostic,
            extra_builtins: options.extra_builtins.clone(),
            enum_bare_index,
            const_array_bare_index,
            html_attributes: options.html_attributes,
            ambient_global_files,
            module_resolutions: Default::default(),
        }
    }
}

/// Resolve TypeScript's own `lib.es5.d.ts`/`lib.dom.d.ts` paths for the given
/// options' first source directory, if resolvable — see `resolve_ts_lib_paths`.
pub fn compute_ambient_global_files(options: &PipelineOptions) -> Vec<Utf8PathBuf> {
    options
        .src_dirs
        .first()
        .and_then(|dir| std::fs::canonicalize(dir).ok())
        .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
        .map(|from_dir| resolve_ts_lib_paths(&from_dir).paths.into_iter().map(Utf8PathBuf::from).collect())
        .unwrap_or_default()
}

/// Resolve `package_name` (e.g. "react") to its real `.d.ts` file path, for
/// callers outside the resolver — the pipeline's `HtmlAttributeMode::Full`
/// wiring needs this to know which file to parse and merge, before a
/// `ResolutionContext` (which needs the already-merged `GlobalSourceData`) can
/// exist yet.
pub fn resolve_package_dts_path(from_dir: &camino::Utf8Path, package_name: &str) -> Option<String> {
    let resolve_options = ResolveOptions {
        condition_names: vec!["types".into(), "import".into(), "require".into(), "default".into()],
        main_fields: vec!["types".into(), "typings".into(), "module".into(), "main".into()],
        extensions: vec![".ts".into(), ".tsx".into(), ".d.ts".into(), ".js".into()],
        ..ResolveOptions::default()
    };
    let resolver = Resolver::new(resolve_options);
    react::resolve_package_types_file(&resolver, from_dir, package_name)
}

/// Where TypeScript's lib files were found; see [`resolve_ts_lib_paths`].
#[derive(Debug, Default, PartialEq)]
pub struct TsLibs {
    pub paths: Vec<String>,
    /// Set when `typescript` is installed but lacks a lib file. Only the pipeline reports it, not the other callers.
    pub diagnostic: Option<Diagnostic>,
}

const TS_LIB_FILES: [&str; 2] = ["lib.es5.d.ts", "lib.dom.d.ts"];

/// Locate `lib.es5.d.ts`/`lib.dom.d.ts`, which declare the ambient globals (`Date`, `Element`, ...). TS ≤6 keeps them
/// in `typescript/lib`; TS 7 moves them into an optional `@typescript/typescript-<platform>` package whose `exports`
/// hide `lib/`, so they're read off disk. They're identical across platforms, so the first installed package (by name)
/// with a file serves. No `typescript` is silent; one lacking a lib file sets `diagnostic`.
pub fn resolve_ts_lib_paths(from_dir: &camino::Utf8Path) -> TsLibs {
    let resolver = Resolver::new(ResolveOptions::default());
    let Ok(manifest) = resolver.resolve(from_dir.as_std_path(), "typescript/package.json") else {
        return TsLibs::default();
    };
    let Some(ts_dir) = manifest.path().parent() else {
        return TsLibs::default();
    };

    let mut platform_dirs = None;
    let mut paths = Vec::new();
    let mut missing = Vec::new();
    for file in TS_LIB_FILES {
        let found = lib_file_in(ts_dir, file).or_else(|| {
            platform_dirs
                .get_or_insert_with(|| installed_platform_package_dirs(&resolver, ts_dir).unwrap_or_default())
                .iter()
                .find_map(|dir| lib_file_in(dir, file))
        });
        match found {
            Some(path) => paths.push(path.to_string_lossy().into_owned()),
            None => missing.push(file),
        }
    }

    let diagnostic = (!missing.is_empty()).then(|| missing_lib_diagnostic(&missing, ts_dir));
    TsLibs { paths, diagnostic }
}

fn lib_file_in(package_dir: &std::path::Path, file: &str) -> Option<std::path::PathBuf> {
    let path = package_dir.join("lib").join(file);
    path.is_file().then_some(path)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageManifest {
    /// Sorted by name: declaration order would depend on `serde_json`'s `preserve_order` being unified on.
    #[serde(default)]
    optional_dependencies: BTreeMap<String, serde::de::IgnoredAny>,
}

fn installed_platform_package_dirs(resolver: &Resolver, ts_dir: &std::path::Path) -> Option<Vec<std::path::PathBuf>> {
    let manifest: PackageManifest =
        serde_json::from_str(&std::fs::read_to_string(ts_dir.join("package.json")).ok()?).ok()?;
    Some(
        manifest
            .optional_dependencies
            .keys()
            .filter_map(|name| {
                let platform_manifest = resolver.resolve(ts_dir, &format!("{name}/package.json")).ok()?;
                platform_manifest.path().parent().map(std::path::Path::to_path_buf)
            })
            .collect(),
    )
}

fn missing_lib_diagnostic(missing: &[&str], ts_dir: &std::path::Path) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!(
            "TypeScript's {} not found in '{}' or any installed platform package — native and DOM globals (Date, \
             Element, …) will appear as unexpanded named references",
            missing.join(", "),
            ts_dir.display()
        ),
        file: None,
        line: None,
        column: None,
        help: Some(
            "TypeScript 7 ships its lib files in an optional dependency (@typescript/typescript-<os>-<arch>). Install \
             with optional dependencies enabled (no --no-optional / --omit=optional)."
                .into(),
        ),
        code: DiagnosticCode::MissingTypeLibrary,
        affects: Vec::new(),
    }
}

// ─── Entry Point ─────────────────────────────────────────────────────────────

/// `resolve_component`, plus what its resolution depended on. Watch mode re-resolves a component when one of those
/// changes.
pub(crate) fn resolve_component_tracked(
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
) -> (ComponentEntry, Vec<Diagnostic>, Dependencies) {
    let ((entry, diagnostics), mut deps) = deps::recording(|| resolve_component(mapping, ctx));
    deps.files.insert(crate::path_key::PathKey::from(mapping.file_path.as_path()));
    (entry, diagnostics, deps)
}

/// Resolve a `ComponentMapping` to a complete `ComponentEntry`.
///
/// Called in parallel via rayon — must be `Send + Sync` (all data is owned/Arc'd).
pub fn resolve_component(mapping: &ComponentMapping, ctx: &ResolutionContext) -> (ComponentEntry, Vec<Diagnostic>) {
    let mut state = ResolveState::default();

    let chain = chain::resolve_props_chain(
        mapping.props_type_name.as_str(),
        &mapping.props_type_args,
        &mapping.file_path,
        mapping,
        ctx,
        &mut state,
        0,
    );

    // Build props BTreeMap — own props win over inherited props with same name.
    let mut props: BTreeMap<String, ParsedProp> = BTreeMap::new();
    for prop in chain.props {
        props.entry(prop.name.clone()).or_insert(prop);
    }

    // Populate notable_inherited from HTML element layers and non-HTML inherited layers.
    let mut notable_inherited: BTreeMap<String, ParsedProp> = BTreeMap::new();
    for layer in &chain.inheritance {
        if let Some(ref element) = layer.html_element {
            // Curated-mode synthesis only: Full mode already merged the real
            // attributes directly into `props` (see resolver/extends.rs); None
            // mode wants no HTML attrs synthesized at all.
            if ctx.html_attributes != crate::pipeline::HtmlAttributeMode::Curated {
                continue;
            }
            let notable_attrs = react_types::notable_html_attrs(element);
            for attr_name in notable_attrs {
                if props.contains_key(*attr_name) {
                    continue;
                } // own prop wins
                if notable_inherited.contains_key(*attr_name) {
                    continue;
                } // already added

                // Synthesize a minimal prop for display purposes
                let prop_type = html::infer_html_attr_prop_type(attr_name);
                notable_inherited.insert(
                    attr_name.to_string(),
                    ParsedProp::new(
                        attr_name.to_string(),
                        prop_type,
                        false,
                        None,
                        String::new(),
                        Default::default(),
                        Some(PropParent {
                            name: format!("{}HTMLAttributes", html::capitalize_element(element)),
                            file_name: "node_modules/@types/react/index.d.ts".to_string(),
                        }),
                        vec![],
                    ),
                );
            }
        } else {
            // For non-HTML layers: add inherited props from chain.inherited_by_name
            for (name, prop) in &chain.inherited_by_name {
                if !props.contains_key(name) && !notable_inherited.contains_key(name) {
                    notable_inherited.insert(name.clone(), prop.clone());
                }
            }
        }
    }

    // Detect discriminated union if applicable.
    let discriminant_prop = chain.discriminant_prop;

    (
        ComponentEntry {
            display_name: mapping.component_name.clone(),
            file_path: mapping.file_path.clone(),
            description: mapping.description.clone(),
            props,
            inheritance: chain.inheritance,
            notable_inherited,
            discriminant_prop,
            composes: chain.composes,
            tags: mapping.tags.clone(),
            methods: vec![],
            detected_by: mapping.rule,
            exported: mapping.exported,
        },
        state.diagnostics,
    )
}

// ─── Intermediate Resolution Result ──────────────────────────────────────────

/// Result of resolving a props chain (including extends).
struct ResolvedChain {
    /// Resolved props (may include inherited props).
    props: Vec<ParsedProp>,
    /// Inheritance layers, outermost first.
    inheritance: Vec<InheritedLayer>,
    /// Inherited props keyed by name — for notable_inherited population.
    inherited_by_name: FxHashMap<String, ParsedProp>,
    /// Type names that could not be resolved.
    composes: Vec<String>,
    /// Discriminant prop name if this is a discriminated union.
    discriminant_prop: Option<String>,
}

impl ResolvedChain {
    /// The base empty chain. Deliberately not `Default` — every accumulator
    /// site (`resolve_interface_chain`, `resolve_base_as_chain`, ...) that
    /// wants "start empty, then push into" calls this directly instead; the
    /// two *give-up* entry points (`empty_with_compose`, `give_up`) are the
    /// only public-facing "this type contributes nothing" signals. Visible to
    /// `chain`/`alias`/`extends` since they're child modules of `resolver`.
    fn empty() -> Self {
        Self {
            props: Vec::new(),
            inheritance: Vec::new(),
            inherited_by_name: FxHashMap::default(),
            composes: Vec::new(),
            discriminant_prop: None,
        }
    }

    fn empty_with_compose(type_name: String) -> Self {
        Self { composes: vec![type_name], ..Self::empty() }
    }

    /// Give up resolving `type_name` — record it in `composes` (same as
    /// `empty_with_compose`) and, if `diag` is given, push it onto `state`
    /// first. The sanctioned "give up" entry point for every "this type
    /// couldn't be followed further, stop here" path. Replaces the bare
    /// `ResolvedChain::default()` the cycle-detected branch used to return,
    /// which let that one give-up site skip explaining why.
    fn give_up(type_name: String, diag: Option<Diagnostic>, state: &mut ResolveState) -> Self {
        if let Some(d) = diag {
            state.diagnostics.push(d);
        }
        Self::empty_with_compose(type_name)
    }

    /// Merge a parent chain into self — own props already in `self.props` take priority.
    fn merge_parent(&mut self, parent: ResolvedChain) {
        // Collect existing prop names so we can skip duplicates.
        let existing: FxHashSet<String> = self.props.iter().map(|p| p.name.clone()).collect();

        for prop in parent.props {
            if !existing.contains(&prop.name) {
                self.props.push(prop.clone());
            }
            // Always populate inherited_by_name so notable lookup works.
            self.inherited_by_name.entry(prop.name.clone()).or_insert(prop);
        }

        // Prepend parent inheritance layers (parent is further up the chain).
        let mut new_inheritance = parent.inheritance;
        new_inheritance.append(&mut self.inheritance);
        self.inheritance = new_inheritance;

        self.composes.extend(parent.composes);
        for (name, prop) in parent.inherited_by_name {
            self.inherited_by_name.entry(name).or_insert(prop);
        }

        // A discriminated union nested inside an intersection (e.g. `Base & (A | B)`)
        // surfaces its discriminant on the union member's own chain — propagate it
        // up rather than silently dropping it in favor of the fresh, empty chain
        // being merged into.
        if self.discriminant_prop.is_none() {
            self.discriminant_prop = parent.discriminant_prop;
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
