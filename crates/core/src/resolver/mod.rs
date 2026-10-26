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
use crate::types::*;

mod alias;
mod callee;
mod chain;
mod collected;
mod deps;
mod evaluate;
pub(crate) use deps::Dependencies;
mod extends;
mod func;
mod html;
mod import;
mod memo;
mod named;
mod precedence;
mod primitives;
mod react;
mod substitute;
mod template;

#[cfg(test)]
pub(crate) use import::{follow_reexports_call_count, reset_follow_reexports_call_count};
pub(crate) use import::{resolve_module_with, ModuleResolution};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum recursion depth for type resolution.
/// Prevents infinite loops on circular type references.
const MAX_DEPTH: u8 = 20;

/// Whether `depth` is past the resolution depth limit. The deepest depth checked is kept in `state`, so a recording
/// (`memo`) knows how much deeper a state may start and still resolve the same way.
fn past_max_depth(state: &mut ResolveState, depth: u8) -> bool {
    state.depth_high = state.depth_high.max(depth);
    depth > MAX_DEPTH
}

/// Maximum named types being resolved at once: the declarations a reference passes through to reach another.
const MAX_NAMED_NESTING: usize = 64;

/// The most state reads a named type's recording may hold and still be kept (`memo::Memo::max_reads`).
const NAMED_RESULT_MAX_READS: usize = 24;

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

/// The help for an unresolvable type: why the dependency loader found no declaration for it, when it tried, or
/// `otherwise`.
pub(super) fn unresolved_type_help(
    type_name: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    otherwise: &str,
) -> String {
    let reason = ctx.dependencies.as_ref().and_then(|layer| {
        let (head, rest) = type_name.split_once('.').map_or((type_name, None), |(head, rest)| (head, Some(rest)));
        let binding = ctx.import_map.find_import(file, head)?;
        let name = match (binding.exported_name.as_str(), rest) {
            ("*", Some(rest)) => rest.to_owned(),
            ("*", None) => return None,
            (exported, Some(rest)) => format!("{exported}.{rest}"),
            (exported, None) => exported.to_owned(),
        };
        layer.unresolved_reason(file.parent()?, &binding.specifier, &name)
    });
    match reason {
        Some(reason) => format!("Dependency types are loaded, but for '{type_name}' {}.", reason.describe()),
        None => otherwise.to_owned(),
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

/// Whether `name` in `file` is a type React declares (`UIEventHandler`, `Key`, `AriaRole`, …) that no builtin list names:
/// it is bound to React, or to nothing, and React's own declarations have it. Such a reference is React's type, not an
/// unresolvable one.
pub(super) fn declared_by_react(name: &str, canonical_name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> bool {
    if !may_be_react_builtin(name, file, ctx) {
        return false;
    }
    let react_file = react::resolve_react_types_file(file, ctx);
    let react_file = Utf8Path::new(&react_file);
    let index = ctx.react_index();
    // The name as written, or the one it is imported under (`import { DOMAttributes as ReactDOMAttributes }`).
    [name.strip_prefix("React.").unwrap_or(name), canonical_name.strip_prefix("React.").unwrap_or(canonical_name)]
        .into_iter()
        .any(|bare| {
            index.lookup_interface(react_file, bare).is_some() || index.lookup_type_alias(react_file, bare).is_some()
        })
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
    /// `ModuleResolvers::types_first`.
    pub(crate) types_first_resolver: Arc<Resolver>,
    /// Module resolutions by `(directory, specifier)`. Resolution only reads the file system, which a run treats as
    /// fixed, and every component importing from the same directory asks again; a watch revision builds a fresh
    /// context, so nothing outlives one revision.
    pub(crate) module_resolutions:
        dashmap::DashMap<(Box<str>, Box<str>), import::ModuleResolution, rustc_hash::FxBuildHasher>,
    /// React's declaration file by importing directory, cached as `module_resolutions` is.
    pub(crate) react_types_files: dashmap::DashMap<Box<str>, String, rustc_hash::FxBuildHasher>,
    /// Props already built this run, by structural hash. A design system builds the same prop for hundreds of
    /// components (Chakra: 470,000 props, ~1,200 distinct), and `ComponentEntry` shares them by `Arc` (`share`).
    pub(crate) shared_props: dashmap::DashMap<u64, Vec<Arc<ParsedProp>>, rustc_hash::FxBuildHasher>,
    /// Member reads of a named type (`CssProperties["marker"]`) by `(file, name, key)`, with the files each depended
    /// on: Chakra's style props read the same csstype members once per component (`evaluate::member`).
    pub(crate) member_reads: dashmap::DashMap<evaluate::MemberReadKey, evaluate::MemberRead, rustc_hash::FxBuildHasher>,
    /// Properties read from a named type's props chain (`ComponentProps<typeof Button>["variant"]`) by `(file, type,
    /// key)`, with the files each depended on (`primitives::read_from_props_chain`).
    pub(crate) props_reads:
        dashmap::DashMap<primitives::PropsReadKey, primitives::PropsRead, rustc_hash::FxBuildHasher>,
    pub extra_builtins: FxHashSet<CompactString>,
    /// Bare name → full scoped key ("file:name") index over `global.enums`,
    /// built once per resolution pass so `resolve_typeof` and
    /// `resolve_cva_variant_props` (VariantProps<typeof X>) can do an O(1)
    /// lookup instead of a linear scan with a `format!()` allocation per
    /// candidate on every reference. Ambiguous bare names (same name in two
    /// files) resolve to whichever key `global.enums` yields first during
    /// this build pass — the same tie-break the old linear scan produced,
    /// since both iterate the same underlying map.
    pub enum_bare_index: Arc<FxHashMap<CompactString, CompactString>>,
    /// Same as `enum_bare_index`, but over `global.const_arrays` — for
    /// `(typeof X)[number]` on a flat `const X = [...] as const` array.
    pub const_array_bare_index: Arc<FxHashMap<CompactString, CompactString>>,
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
    /// Interface chains already resolved this run, shared by the components that inherit them.
    pub(crate) interface_chains: memo::InterfaceChains,
    /// Named types resolved once per run (`resolve_named`), shared by every component that names them.
    pub(crate) named_results: memo::NamedResults,
    /// An HTML element's attribute declarations (`html::react_attribute`), read once per element and name.
    pub(crate) attribute_reads: memo::Memo<Option<ParsedProp>>,
    /// Loaded dependency declarations (`PipelineOptions::load_dependencies`, ADR 0010).
    pub(crate) dependencies: Option<Arc<crate::dependency_types::DependencyLayer>>,
    /// React's own declarations in curated mode, which only HTML-attribute lookups read (`react_index`).
    pub(crate) react_types: Option<Arc<crate::named_type_index::NamedTypeIndex>>,
    pub(crate) collapse_inherited_props: crate::pipeline::CollapseInheritedProps,
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
        let resolvers = ModuleResolvers::new(options);
        // A tsconfig that can't be read or parsed would otherwise fail every aliased import one by one.
        let tsconfig_diagnostic = governing_tsconfig(options).and_then(|path| {
            let error = resolvers.main.resolve_tsconfig(path.as_std_path()).err()?;
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
            oxc_resolver: Arc::new(resolvers.main),
            types_first_resolver: Arc::new(resolvers.types_first),
            tsconfig_diagnostic,
            extra_builtins: options.extra_builtins.clone(),
            enum_bare_index: Arc::new(enum_bare_index),
            const_array_bare_index: Arc::new(const_array_bare_index),
            html_attributes: options.html_attributes,
            ambient_global_files,
            module_resolutions: Default::default(),
            react_types_files: Default::default(),
            shared_props: Default::default(),
            member_reads: Default::default(),
            props_reads: Default::default(),
            interface_chains: Default::default(),
            named_results: memo::NamedResults::with_max_reads(NAMED_RESULT_MAX_READS),
            attribute_reads: memo::Memo::with_max_reads(NAMED_RESULT_MAX_READS),
            dependencies: None,
            react_types: None,
            collapse_inherited_props: options.collapse_inherited_props,
        }
    }

    /// Add to this context the memos of `from` that a context over a larger dependency layer would compute again
    /// identically (ADR 0016): file-system answers, shared props, and the interface chains and member reads whose
    /// dependencies `keep` accepts, given the file each was read in, which a layer lacking a name they reached would
    /// otherwise have poisoned.
    pub(crate) fn inherit_memos(&self, from: &ResolutionContext, keep: impl Fn(&Dependencies, &str) -> bool) {
        from.interface_chains.copy_into(&self.interface_chains, &keep);
        for entry in from.member_reads.iter().filter(|entry| keep(entry.value().deps(), &entry.key().0)) {
            self.member_reads.insert(entry.key().clone(), entry.value().clone());
        }
        for entry in from.props_reads.iter().filter(|entry| keep(entry.value().deps(), &entry.key().0)) {
            self.props_reads.insert(entry.key().clone(), entry.value().clone());
        }
        for entry in from.shared_props.iter() {
            self.shared_props.insert(*entry.key(), entry.value().clone());
        }
        for entry in from.module_resolutions.iter() {
            self.module_resolutions.insert(entry.key().clone(), entry.value().clone());
        }
        for entry in from.react_types_files.iter() {
            self.react_types_files.insert(entry.key().clone(), entry.value().clone());
        }
    }

    /// The key in `global.enums` of the variants (`cva`, `tv`, recipes) `name` stands for as `consuming_file` writes
    /// it: its own declaration, else the one it imports, else the first file by path that declares the bare name. Two
    /// files can each declare an `itemVariants` (shadcn keeps a copy of every component per registry), and the bare
    /// name alone would give both the same one.
    pub(crate) fn enum_key_for(&self, name: &str, consuming_file: &Utf8Path) -> Option<String> {
        let own = format!("{consuming_file}:{name}");
        if self.global.enums.contains_key(&own) {
            return Some(own);
        }
        if let Some((file, canonical)) = import::resolve_to_canonical(name, consuming_file, self, &mut Vec::new()) {
            let imported = format!("{file}:{canonical}");
            if self.global.enums.contains_key(&imported) {
                return Some(imported);
            }
        }
        self.enum_bare_index.get(name).map(|key| key.to_string())
    }

    /// `prop` as the run's one shared allocation of it: an equal prop built earlier, or `prop` itself.
    pub(crate) fn share(&self, prop: ParsedProp) -> Arc<ParsedProp> {
        use std::hash::Hasher;
        let mut hasher = rustc_hash::FxHasher::default();
        prop.structural_hash(&mut hasher);
        let key = hasher.finish();
        // Nearly every call finds the prop already built, so look under the shard's shared lock first.
        if let Some(bucket) = self.shared_props.get(&key) {
            if let Some(found) = bucket.iter().find(|shared| ***shared == prop) {
                return Arc::clone(found);
            }
        }
        let mut bucket = self.shared_props.entry(key).or_default();
        // Another thread may have added it between the two locks.
        if let Some(found) = bucket.iter().find(|shared| ***shared == prop) {
            return Arc::clone(found);
        }
        let shared = Arc::new(prop);
        bucket.push(Arc::clone(&shared));
        shared
    }

    /// This context with React's own declarations indexed apart from the project's, for an HTML layer's attribute
    /// lookups only (`html::react_attribute`, ADR 0010's amendment). Full mode merges them into `named_types` instead.
    pub(crate) fn with_react_types(
        mut self,
        react_types: Option<Arc<crate::named_type_index::NamedTypeIndex>>,
    ) -> Self {
        self.react_types = react_types;
        self
    }

    /// The index holding React's declarations, when this run has them.
    pub(super) fn react_index(&self) -> &crate::named_type_index::NamedTypeIndex {
        self.react_types.as_deref().unwrap_or(&self.named_types)
    }

    /// This context over a loaded dependency layer: imports and re-exports of dependency files come from it. The
    /// caller sets the same layer as its `NamedTypeIndex` fallback.
    pub(crate) fn with_dependencies(mut self, layer: Option<Arc<crate::dependency_types::DependencyLayer>>) -> Self {
        if let Some(layer) = &layer {
            // Built just above and not yet shared, so this never copies the map.
            if let Some(import_map) = Arc::get_mut(&mut self.import_map) {
                import_map.fallback = Some(layer.import_map.clone());
            }
        }
        self.dependencies = layer;
        self
    }

    /// A context for the same project over a larger dependency `layer`, with empty memos. Everything about the project
    /// that a layer cannot change is shared with this one rather than built again: the merged source data, the import
    /// map and `named_types` declarations, the module resolvers (and their file-system caches), the tsconfig check, and
    /// the bare-name indexes. `named_types` is the project's index with `layer` as its fallback.
    pub(crate) fn over_layer(
        &self,
        named_types: Arc<crate::named_type_index::NamedTypeIndex>,
        layer: Option<Arc<crate::dependency_types::DependencyLayer>>,
    ) -> Self {
        let import_map = self.import_map.with_fallback(layer.as_ref().map(|layer| layer.import_map.clone()));
        Self {
            global: self.global.clone(),
            import_map: Arc::new(import_map),
            named_types,
            oxc_resolver: self.oxc_resolver.clone(),
            types_first_resolver: self.types_first_resolver.clone(),
            module_resolutions: Default::default(),
            react_types_files: Default::default(),
            shared_props: Default::default(),
            member_reads: Default::default(),
            props_reads: Default::default(),
            extra_builtins: self.extra_builtins.clone(),
            enum_bare_index: self.enum_bare_index.clone(),
            const_array_bare_index: self.const_array_bare_index.clone(),
            html_attributes: self.html_attributes,
            ambient_global_files: self.ambient_global_files.clone(),
            tsconfig_diagnostic: self.tsconfig_diagnostic.clone(),
            interface_chains: Default::default(),
            named_results: memo::NamedResults::with_max_reads(NAMED_RESULT_MAX_READS),
            attribute_reads: memo::Memo::with_max_reads(NAMED_RESULT_MAX_READS),
            dependencies: layer,
            react_types: self.react_types.clone(),
            collapse_inherited_props: self.collapse_inherited_props,
        }
    }

    /// Whether `file` is a loaded dependency file.
    pub(crate) fn is_dependency_file(&self, file: &Utf8Path) -> bool {
        self.dependencies.as_ref().is_some_and(|layer| layer.is_dependency_file(file))
    }

    /// Whether this run can collapse any source; `expand_all` is set only then, since it splits shared chains.
    pub(crate) fn can_collapse(&self) -> bool {
        use crate::pipeline::CollapseInheritedProps;
        match self.collapse_inherited_props {
            CollapseInheritedProps::Dependencies => self.dependencies.is_some(),
            CollapseInheritedProps::All => true,
            CollapseInheritedProps::None => false,
        }
    }

    /// Whether a source declaring `declared` props in `file`, reached at `depth`, is recorded as a layer instead of
    /// expanded (ADR 0012). Depth 0 is the component's own props type. React's and TypeScript's own declarations stay
    /// with `html_attributes` even under `All`.
    pub(crate) fn collapses(&self, file: &Utf8Path, declared: usize, depth: u8, state: &ResolveState) -> bool {
        use crate::pipeline::CollapseInheritedProps;
        depth > 0
            && declared > COLLAPSE_THRESHOLD
            && !state.expand_all
            && match self.collapse_inherited_props {
                CollapseInheritedProps::Dependencies => self.is_dependency_file(file),
                CollapseInheritedProps::All => {
                    self.is_dependency_file(file) || !file.as_str().contains("/node_modules/")
                }
                CollapseInheritedProps::None => false,
            }
    }
}

/// Storybook's react-component-meta hides a dependency source contributing more than this many props.
pub(crate) const COLLAPSE_THRESHOLD: usize = 30;

/// Changes whenever the options could change what a specifier resolves to: the resolver configuration, the governing
/// tsconfig and its modification time, and the build. Keys the dependency loader's persisted index (ADR 0011).
pub(crate) fn resolution_fingerprint(options: &PipelineOptions) -> u64 {
    let tsconfig = governing_tsconfig(options);
    let tsconfig_mtime = tsconfig
        .as_ref()
        .and_then(|path| std::fs::metadata(path).ok()?.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| age.as_nanos());
    let text = format!(
        "{:?}|{:?}|{:?}|{}",
        resolve_options(options),
        tsconfig,
        tsconfig_mtime,
        env!("CALIPER_SOURCE_FINGERPRINT"),
    );
    text.bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
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
pub fn resolve_package_dts_path(
    from_dir: &camino::Utf8Path,
    package_name: &str,
    options: &PipelineOptions,
) -> Option<String> {
    let resolvers = ModuleResolvers::new(options);
    // Resolved as if imported by a file in `from_dir`: the declaration resolver walks up from a file's directory.
    let importing_file = from_dir.join("index.ts");
    react::resolve_package_types_file(&resolvers.main, &resolvers.types_first, &importing_file, package_name)
}

/// The two resolvers of a run, configured the way TypeScript resolves this project's modules (`resolve_options`).
pub(crate) struct ModuleResolvers {
    pub(crate) main: Resolver,
    /// Only `types` and the custom conditions: `resolve_dts` takes the first `exports` key that matches any condition,
    /// so a package listing `"import"` before `"types"` resolves to JavaScript; this one reaches its declarations.
    pub(crate) types_first: Resolver,
}

impl ModuleResolvers {
    pub(crate) fn new(options: &PipelineOptions) -> Self {
        let main = resolve_options(options);
        let custom = custom_conditions(options);
        let types_first =
            ResolveOptions { condition_names: std::iter::once("types".into()).chain(custom).collect(), ..main.clone() };
        ModuleResolvers { main: Resolver::new(main), types_first: Resolver::new(types_first) }
    }
}

/// Module resolution as TypeScript does it for this project. The resolver reads the tsconfig itself (comments,
/// `extends`, `baseUrl`, `paths` wildcards): the configured one, or else the nearest one above each importing file.
/// The governing tsconfig's `customConditions` apply, ESM and CommonJS declaration files resolve, and an import written
/// with a JavaScript extension (`./button.js`) finds its TypeScript source.
fn resolve_options(options: &PipelineOptions) -> ResolveOptions {
    let condition_names = ["types", "import", "require", "default"]
        .into_iter()
        .map(String::from)
        .chain(custom_conditions(options))
        .collect();
    let tsconfig = match &options.tsconfig_path {
        Some(path) => TsconfigDiscovery::Manual(TsconfigOptions {
            config_file: path.as_std_path().to_owned(),
            references: TsconfigReferences::Auto,
        }),
        None => TsconfigDiscovery::Auto,
    };
    let aliases = |js: &str, ts: &[&str]| (js.to_owned(), ts.iter().map(|ext| (*ext).to_owned()).collect());
    ResolveOptions {
        condition_names,
        main_fields: vec!["types".into(), "typings".into(), "module".into(), "main".into()],
        extensions: [".ts", ".tsx", ".d.ts", ".d.mts", ".d.cts", ".js"].map(String::from).to_vec(),
        extension_alias: vec![
            aliases(".js", &[".ts", ".tsx", ".d.ts", ".js"]),
            aliases(".jsx", &[".tsx", ".jsx"]),
            aliases(".mjs", &[".mts", ".d.mts", ".mjs"]),
            aliases(".cjs", &[".cts", ".d.cts", ".cjs"]),
        ],
        tsconfig: Some(tsconfig),
        ..ResolveOptions::default()
    }
}

/// The tsconfig that governs the run: the configured one, or the nearest `tsconfig.json` above the first source
/// directory.
fn governing_tsconfig(options: &PipelineOptions) -> Option<Utf8PathBuf> {
    if let Some(path) = &options.tsconfig_path {
        return Some(path.clone());
    }
    let first = Utf8PathBuf::from_path_buf(std::fs::canonicalize(options.src_dirs.first()?).ok()?).ok()?;
    first.ancestors().map(|dir| dir.join("tsconfig.json")).find(|candidate| candidate.is_file())
}

/// `compilerOptions.customConditions` of the governing tsconfig, or of the nearest config it extends that sets them.
/// `oxc_resolver` does not read them, and workspaces such as Chakra's and React Spectrum's point their packages'
/// `exports` at TypeScript sources only through them.
fn custom_conditions(options: &PipelineOptions) -> Vec<String> {
    let mut next = governing_tsconfig(options);
    // Bounds an `extends` cycle.
    for _ in 0..16 {
        let Some(path) = next.take() else { break };
        let Some(config) = read_tsconfig_json(&path) else { break };
        if let Some(conditions) = config.pointer("/compilerOptions/customConditions").and_then(|v| v.as_array()) {
            return conditions.iter().filter_map(|c| c.as_str().map(str::to_owned)).collect();
        }
        // TypeScript applies an array of bases in order, so the last one that sets an option wins.
        let extends: Vec<&str> = match config.get("extends") {
            Some(serde_json::Value::String(base)) => vec![base.as_str()],
            Some(serde_json::Value::Array(bases)) => bases.iter().rev().filter_map(|b| b.as_str()).collect(),
            _ => Vec::new(),
        };
        let dir = path.parent().map(Utf8Path::to_owned).unwrap_or_default();
        next = extends.into_iter().find_map(|base| tsconfig_base_path(&dir, base));
    }
    Vec::new()
}

/// Where an `extends` entry points: a relative path, with `.json` implied, or a package's tsconfig.
fn tsconfig_base_path(dir: &Utf8Path, base: &str) -> Option<Utf8PathBuf> {
    if base.starts_with('.') || base.starts_with('/') {
        let path = dir.join(base);
        let path = if path.extension() == Some("json") { path } else { Utf8PathBuf::from(format!("{path}.json")) };
        return path.is_file().then_some(path);
    }
    let found = Resolver::new(ResolveOptions::default()).resolve(dir.as_std_path(), base).ok()?;
    Utf8PathBuf::from_path_buf(found.into_path_buf()).ok()
}

/// A tsconfig as JSON: comments and trailing commas removed the way `oxc_resolver` removes them.
fn read_tsconfig_json(path: &Utf8Path) -> Option<serde_json::Value> {
    let mut bytes = std::fs::read(path).ok()?;
    json_strip_comments::strip_slice(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Where TypeScript's lib files were found; see [`resolve_ts_lib_paths`].
#[derive(Debug, Default, PartialEq)]
pub struct TsLibs {
    pub paths: Vec<String>,
    /// Set when `typescript` is installed but lacks a lib file. Only the pipeline reports it, not the other callers.
    pub diagnostic: Option<Diagnostic>,
}

const TS_LIB_FILES: [&str; 2] = ["lib.es5.d.ts", "lib.dom.d.ts"];

/// The ES2015 collections and iteration (`Iterable`, `Set`, `Map`), which props name (`items?: Iterable<T>`). An install
/// without one is not reported: they are not what a project's `lib` setting always includes.
const TS_OPTIONAL_LIB_FILES: [&str; 3] =
    ["lib.es2015.iterable.d.ts", "lib.es2015.collection.d.ts", "lib.es2015.promise.d.ts"];

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
    let mut find = |file: &str| {
        lib_file_in(ts_dir, file).or_else(|| {
            platform_dirs
                .get_or_insert_with(|| installed_platform_package_dirs(&resolver, ts_dir).unwrap_or_default())
                .iter()
                .find_map(|dir| lib_file_in(dir, file))
        })
    };
    for file in TS_LIB_FILES {
        match find(file) {
            Some(path) => paths.push(path.to_string_lossy().into_owned()),
            None => missing.push(file),
        }
    }
    paths.extend(TS_OPTIONAL_LIB_FILES.iter().filter_map(|file| find(file)).map(|p| p.to_string_lossy().into_owned()));

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
) -> (ComponentEntry, ResolvedDiagnostics, Dependencies) {
    let ((entry, diagnostics), mut deps) = deps::recording(|| resolve_component_shared(mapping, ctx));
    deps.files.insert(crate::path_key::PathKey::from(mapping.file_path.as_path()));
    (entry, diagnostics, deps)
}

/// Resolve a `ComponentMapping` to a complete `ComponentEntry`.
///
/// Called in parallel via rayon — must be `Send + Sync` (all data is owned/Arc'd).
#[cfg(test)]
pub fn resolve_component(mapping: &ComponentMapping, ctx: &ResolutionContext) -> (ComponentEntry, Vec<Diagnostic>) {
    let (entry, diagnostics) = resolve_component_shared(mapping, ctx);
    (entry, diagnostics.flatten())
}

/// `resolve_component`, with the diagnostics of replayed interface chains shared rather than copied.
pub(crate) fn resolve_component_shared(
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
) -> (ComponentEntry, ResolvedDiagnostics) {
    let mut state = ResolveState::default();
    state.add_type_params(mapping.type_params.iter());

    // A component a generic call builds has the props its callee's result type declares; the props type the extractor
    // selected is for a callee whose result isn't known.
    let chain = callee::result_chain(mapping, ctx, &mut state).unwrap_or_else(|| {
        chain::resolve_props_chain(
            mapping.props_type_name.as_str(),
            &mapping.props_type_args,
            &mapping.file_path,
            mapping,
            ctx,
            &mut state,
            0,
        )
    });

    // Build props BTreeMap — own props win over inherited props with same name.
    let mut props: BTreeMap<String, Arc<ParsedProp>> = BTreeMap::new();
    for prop in chain.props {
        props.entry(prop.name.clone()).or_insert(prop);
    }
    apply_param_defaults(mapping, &mut props, ctx);

    // Populate notable_inherited from HTML element layers and non-HTML inherited layers. Curated mode lists the
    // curated attributes there; Full mode lists all of them in `props`, except for a layer whose attributes the
    // interface chain already merged (`total_props`); None mode lists none.
    let mut notable_inherited: BTreeMap<String, Arc<ParsedProp>> = BTreeMap::new();
    for layer in &chain.inheritance {
        if layer.html_element.is_none() {
            continue;
        }
        match ctx.html_attributes {
            crate::pipeline::HtmlAttributeMode::None => {}
            crate::pipeline::HtmlAttributeMode::Full => {
                if layer.total_props == 0 {
                    let added =
                        html::layer_attributes(layer, true, |name| props.contains_key(name), ctx, &mut state, 0);
                    for prop in added {
                        props.entry(prop.name.clone()).or_insert(prop);
                    }
                }
            }
            crate::pipeline::HtmlAttributeMode::Curated => {
                let added = html::layer_attributes(
                    layer,
                    false,
                    |name| props.contains_key(name) || notable_inherited.contains_key(name),
                    ctx,
                    &mut state,
                    0,
                );
                for prop in added {
                    notable_inherited.entry(prop.name.clone()).or_insert(prop);
                }
            }
        }
    }
    // A `forwardRef` component takes `ref` whatever its props type declares. The element type isn't recorded, so it is
    // `Ref<unknown>` unless the props or an HTML layer gave the real declaration.
    let takes_ref = matches!(mapping.rule, DetectionRule::ForwardRef | DetectionRule::ForwardRefExoticDecl);
    if takes_ref && ctx.html_attributes != crate::pipeline::HtmlAttributeMode::None {
        let target = if ctx.html_attributes == crate::pipeline::HtmlAttributeMode::Full {
            &mut props
        } else {
            &mut notable_inherited
        };
        if !target.contains_key("ref") {
            let ref_type = PropType::Named {
                name: "Ref".into(),
                args: vec![PropType::Named { name: "unknown".into(), args: vec![] }],
            };
            // The component takes it, so no interface is its parent, and a consumer that hides React's inherited props
            // (the addon's `collapseInheritedProps`) still shows it.
            let parent = None;
            target.insert(
                "ref".to_owned(),
                ctx.share(ParsedProp::new(
                    "ref".to_owned(),
                    ref_type,
                    false,
                    None,
                    String::new(),
                    Default::default(),
                    parent,
                    vec![],
                )),
            );
        }
    }
    // A collapsed source's notable props (ADR 0012); the only non-HTML layers.
    for prop in &chain.notable {
        if !props.contains_key(&prop.name) && !notable_inherited.contains_key(&prop.name) {
            notable_inherited.insert(prop.name.clone(), prop.clone());
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
        ResolvedDiagnostics { own: state.diagnostics, shared: state.shared_diagnostics },
    )
}

/// A destructuring default is the component's default for that prop wherever the prop's type was declared, as
/// TypeScript's docgen reads it. The chain applied it only to props an interface or an object alias declares, so a prop
/// that arrives through `Pick<ComponentProps<typeof Button>, "variant">` or another resolved type has none yet.
fn apply_param_defaults(
    mapping: &ComponentMapping,
    props: &mut BTreeMap<String, Arc<ParsedProp>>,
    ctx: &ResolutionContext,
) {
    for (name, raw) in &mapping.param_defaults {
        let Some(prop) = props.get_mut(name) else { continue };
        if prop.default_value.is_some() {
            continue;
        }
        let default = DefaultValue { value: raw.value.clone(), computed: raw.computed };
        *prop = ctx.share(prop.with_default_value(default));
    }
}

// ─── Intermediate Resolution Result ──────────────────────────────────────────

/// Result of resolving a props chain (including extends). Props are shared: a parent's props move into every chain
/// that inherits them, and into the component, without being copied.
#[derive(Clone)]
pub(crate) struct ResolvedChain {
    /// Resolved props (may include inherited props).
    props: Vec<Arc<ParsedProp>>,
    /// Inheritance layers, outermost first.
    inheritance: Vec<InheritedLayer>,
    /// Inherited props keyed by name — for notable_inherited population.
    inherited_by_name: FxHashMap<String, Arc<ParsedProp>>,
    /// The notable props of collapsed sources, which are not in `props`.
    notable: Vec<Arc<ParsedProp>>,
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
            notable: Vec::new(),
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
                self.props.push(Arc::clone(&prop));
            }
            // Always populate inherited_by_name so notable lookup works.
            self.inherited_by_name.entry(prop.name.clone()).or_insert(prop);
        }

        // Prepend parent inheritance layers (parent is further up the chain).
        let mut new_inheritance = parent.inheritance;
        new_inheritance.append(&mut self.inheritance);
        self.inheritance = new_inheritance;

        self.composes.extend(parent.composes);
        self.notable.extend(parent.notable);
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

    /// Merge the next member of an intersection into self. Unlike an extended interface's, a prop both declare keeps
    /// both declarations, as TypeScript intersects them.
    fn intersect_member(&mut self, mut member: ResolvedChain) {
        if !self.props.is_empty() && !member.props.is_empty() {
            let theirs: FxHashMap<&str, usize> =
                member.props.iter().enumerate().map(|(i, prop)| (prop.name.as_str(), i)).collect();
            let mut merged = vec![false; member.props.len()];
            for prop in &mut self.props {
                if let Some(&i) = theirs.get(prop.name.as_str()) {
                    *prop = Arc::new(intersect_declarations(prop, &member.props[i]));
                    merged[i] = true;
                }
            }
            let mut merged = merged.into_iter();
            member.props.retain(|_| !merged.next().unwrap_or(false));
        }
        self.merge_parent(member);
    }
}

/// One prop two intersection members declare, as TypeScript sees it: both types, each once; required when either
/// requires it; documented by the first declaration that has documentation.
fn intersect_declarations(first: &ParsedProp, second: &ParsedProp) -> ParsedProp {
    let members = |prop_type: &PropType| match prop_type {
        PropType::Intersection(members) => members.clone(),
        other => vec![other.clone()],
    };
    let mut types = members(&first.prop_type);
    for member in members(&second.prop_type) {
        if !types.contains(&member) {
            types.push(member);
        }
    }
    // `any` absorbs an intersection, as in TypeScript.
    let prop_type = if types.contains(&PropType::Any) {
        PropType::Any
    } else {
        match <[PropType; 1]>::try_from(types) {
            Ok([only]) => only,
            Err(types) => common_literals(&types).unwrap_or(PropType::Intersection(types)),
        }
    };
    let documented = if first.description.is_empty() && first.tags.is_empty() { second } else { first };
    let mut declarations = first.declarations.clone();
    for declaration in &second.declarations {
        if !declarations.contains(declaration) {
            declarations.push(declaration.clone());
        }
    }
    ParsedProp::new(
        first.name.clone(),
        prop_type,
        first.required || second.required,
        first.default_value.clone().or_else(|| second.default_value.clone()),
        documented.description.clone(),
        documented.tags.clone(),
        first.parent.clone().or_else(|| second.parent.clone()),
        declarations,
    )
}

/// The intersection of types made only of literals, as TypeScript reduces it: the values every one of them has
/// (`("a" | "b" | null) & ("a" | "c")` is `"a"`). `None` when any is not such a type.
fn common_literals(types: &[PropType]) -> Option<PropType> {
    let (first, rest) = types.split_first()?;
    let rest: Vec<Vec<PropType>> = rest.iter().map(literal_values).collect::<Option<_>>()?;
    let values = literal_values(first)?;
    let total = values.len();
    let mut common: Vec<PropType> =
        values.into_iter().filter(|value| rest.iter().all(|values| values.contains(value))).collect();
    Some(match common.len() {
        0 => PropType::Never,
        1 => common.remove(0),
        // Spelled as the first declaration spells it when the others remove nothing.
        len if len == total => first.clone(),
        _ => PropType::Union(common),
    })
}

/// The values of a type made only of literals, or `None`.
fn literal_values(prop_type: &PropType) -> Option<Vec<PropType>> {
    match prop_type {
        PropType::StringLiteral(_) | PropType::NumberLiteral(_) | PropType::BoolLiteral(_) | PropType::Null => {
            Some(vec![prop_type.clone()])
        }
        PropType::Boolean => Some(vec![PropType::BoolLiteral(false), PropType::BoolLiteral(true)]),
        PropType::LiteralUnion { members, .. } => {
            Some(members.iter().map(|member| PropType::StringLiteral(member.clone())).collect())
        }
        PropType::Union(members) => {
            members.iter().map(literal_values).collect::<Option<Vec<_>>>().map(|values| values.concat())
        }
        _ => None,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
