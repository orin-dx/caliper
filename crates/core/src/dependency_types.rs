//! The dependency type declarations project types reference, loaded by name before resolution
//! (`PipelineOptions::load_dependencies`, ADR 0010).
//!
//! A demand is a name a project file imports from a package and uses in a type. The loader resolves the module the
//! way the resolver will (`resolve_module_with`), finds the file declaring the name by following re-exports
//! (`export *`, `export { a as b } from`, `export * as ns from`, `import x; export { x }`), parses that file, then
//! follows the names its declaration references the same way. Only declaring files enter the layer; re-export stubs are
//! walked once and their answers kept, in the layer and in a per-package index on disk, so later runs never
//! open them. The resolver consults the layer after the project's own data: its import map and named-type index fall
//! back to it, re-export answers come from it, and dependency components form a value index for `typeof` that is never
//! emitted.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use dashmap::{DashMap, DashSet};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::import_map::ImportResolutionMap;
use crate::named_type_index::NamedTypeIndex;
use crate::path_key::PathKey;
use crate::resolver::{ModuleResolution, ModuleResolvers};
use crate::types::options::PipelineOptions;
use crate::types::*;

mod ledger;
pub(crate) use ledger::{Clock, DEGRADED_RETRY, Edits, Footprint, LoadIds, Moment, Retry, Stamp, UnreadWalks};
use ledger::{LedgerBuilder, Node, ReadLedger};

type FxDashMap<K, V> = DashMap<K, V, FxBuildHasher>;
type FxDashSet<K> = DashSet<K, FxBuildHasher>;
/// The files that augment React which the module a `(scope, specifier)` resolved to leads to, parsed, once the first
/// demand of the scope, whose key it holds, has searched them; incomplete when the search could not read a file, for
/// every demand of the scope.
type ModuleCell = Arc<OnceLock<(DemandKey, Partial<Vec<(Utf8PathBuf, Arc<SourceData>)>>)>>;

/// A file's data, read and parsed once per load whoever asks first, a failed read included (`Loader::parse_source`).
type ParseCell = Arc<OnceLock<Result<Option<Arc<SourceData>>, ReadFailed>>>;

/// Whether a file merges declarations into React, and the specifiers it re-exports, in order.
type Scan = (bool, Arc<[String]>);

/// A [`Scan`] of a file, read once per load (`Loader::augmentation_scan`).
type ScanCell = Arc<OnceLock<Result<Option<Scan>, ReadFailed>>>;

/// A dependency file that exists but could not be read (a permission, too many open files, an install racing the run):
/// what it declares is unknown, not absent. `Loader::read` and `Loader::parse` return it for the one file; every walk
/// or search over many files turns it into an incomplete [`Partial`] and goes on with the files it can read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadFailed(PathKey);

/// What a walk or search found in the files it could read, and the files on it that it could not ([`ReadFailed`]).
/// An incomplete answer is used by the load that made it, as the best it has, and kept nowhere: not in the loader's
/// maps, the layer or the index. The demand it served is marked unread in the layer's [`ReadLedger`], which is what
/// makes the next load follow it again and a lazy session not memoize a component that reached it.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Partial<T> {
    value: T,
    /// The files this walk passed and could not read; empty when it is complete.
    unread: Vec<PathKey>,
}

impl<T> Partial<T> {
    fn complete(value: T) -> Self {
        Partial { value, unread: Vec::new() }
    }

    fn incomplete(&self) -> bool {
        !self.unread.is_empty()
    }
}

impl Partial<()> {
    const DONE: Partial<()> = Partial { value: (), unread: Vec::new() };

    /// Nothing, because the one file needed could not be read.
    fn unread_at(ReadFailed(file): ReadFailed) -> Partial<()> {
        Partial { value: (), unread: vec![file] }
    }

    fn and(mut self, mut other: Partial<()>) -> Partial<()> {
        self.unread.append(&mut other.unread);
        self
    }
}

/// Why an export lookup's answer may not be kept, gathered over one walk.
#[derive(Debug, Default, Clone)]
struct Unkept {
    /// The walk met a module already on it (a re-export cycle), so the answer depends on where it entered.
    cycle: bool,
    /// The modules the walk passed and could not read, any of which may have declared the name.
    unread: Vec<PathKey>,
}

impl Unkept {
    fn absorb(&mut self, other: Unkept) {
        self.cycle |= other.cycle;
        self.unread.extend(other.unread);
    }
}

/// What one probe of a directory found, once it has run (`probe_once`).
type ProbeCell = Arc<OnceLock<bool>>;

/// `(directory of the importing file, specifier, exported name path)`.
type DemandKey = (Box<str>, Box<str>, CompactString);

/// A name a file uses from another module: `specifier` as written in `from`, and the exported name path
/// (`Accordion.Root` for `RadixPrimitive.Accordion.Root` through `import * as RadixPrimitive`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Demand {
    pub(crate) from: Utf8PathBuf,
    pub(crate) specifier: Box<str>,
    pub(crate) name: CompactString,
}

impl Demand {
    fn key(&self) -> DemandKey {
        let dir = self.from.parent().map_or("", Utf8Path::as_str);
        (Box::from(dir), self.specifier.clone(), self.name.clone())
    }
}

/// Why a demand found no declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Unresolved {
    /// The specifier resolves to nothing.
    Unresolvable,
    /// The module resolves, but to JavaScript with no declaration file.
    NoTypes,
    /// The module has types, but none of its exports is the name.
    NotExported,
}

impl Unresolved {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Unresolved::Unresolvable => "its module could not be resolved",
            Unresolved::NoTypes => "its package ships no type declarations",
            Unresolved::NotExported => "its package's types do not export it",
        }
    }
}

/// A demanded name that panics the loader under `#[cfg(test)]`, to drive the containment path.
#[cfg(test)]
const DEMAND_PANIC_TEST_NAME: &str = "__DEPENDENCY_PANIC_TEST__";

fn is_typescript(path: &Utf8Path) -> bool {
    crate::extractor::is_typescript(path.as_std_path())
}

// ─── Demands ──────────────────────────────────────────────────────────────────

/// Every name `data` uses in a type: references, `typeof` queries, heritage and type arguments, in its interfaces,
/// type aliases and component props.
fn referenced_names(data: &SourceData, out: &mut FxHashSet<CompactString>) {
    for iface in data.interfaces.values() {
        interface_refs(iface, out);
    }
    for alias in data.type_aliases.values() {
        alias_refs(alias, out);
    }
    for mapping in &data.component_mappings {
        mapping_refs(mapping, out);
    }
}

fn interface_refs(iface: &CollectedInterface, out: &mut FxHashSet<CompactString>) {
    for prop in &iface.props {
        type_refs(&prop.collected_type, out);
    }
    for extends in &iface.extends {
        let (name, args) = match extends {
            ExtendsRef::SameFile { name, type_args } => (name, type_args),
            ExtendsRef::Imported { local_name, type_args, .. } => (local_name, type_args),
            ExtendsRef::Builtin { name, type_args, .. } => (name, type_args),
        };
        out.insert(CompactString::from(name.as_str()));
        args.iter().for_each(|arg| type_refs(arg, out));
    }
}

fn alias_refs(alias: &CollectedTypeAlias, out: &mut FxHashSet<CompactString>) {
    match alias {
        CollectedTypeAlias::Omit { base, keys, .. } | CollectedTypeAlias::Pick { base, keys, .. } => {
            type_refs(base, out);
            type_refs(keys, out);
        }
        CollectedTypeAlias::Partial { base, .. } | CollectedTypeAlias::Required { base, .. } => type_refs(base, out),
        CollectedTypeAlias::Union { members, .. } | CollectedTypeAlias::Intersection { members, .. } => {
            members.iter().for_each(|member| type_refs(member, out));
        }
        CollectedTypeAlias::Passthrough { target, .. } => type_refs(target, out),
        CollectedTypeAlias::LiteralUnion { .. } => {}
    }
}

fn mapping_refs(mapping: &ComponentMapping, out: &mut FxHashSet<CompactString>) {
    out.insert(CompactString::from(mapping.props_type_name.as_str()));
    mapping.props_type_args.iter().for_each(|arg| type_refs(arg, out));
    // A factory call's callback annotation (`make('div', (props: ExternalProps) => ...)`) names types only here.
    if let Some(factory_props) = &mapping.factory_props {
        type_refs(factory_props, out);
    }
}

fn type_refs(ty: &CollectedType, out: &mut FxHashSet<CompactString>) {
    match ty {
        CollectedType::Named { name, args } => {
            out.insert(name.clone());
            args.iter().for_each(|arg| type_refs(arg, out));
        }
        CollectedType::TypeOf(name) => {
            out.insert(name.clone());
        }
        CollectedType::Union(members)
        | CollectedType::Intersection(members)
        | CollectedType::TemplateLiteral(members) => members.iter().for_each(|member| type_refs(member, out)),
        CollectedType::Tuple(elements) => elements.iter().for_each(|element| type_refs(&element.element_type, out)),
        CollectedType::Array(inner) | CollectedType::Readonly(inner) | CollectedType::KeyOf(inner) => {
            type_refs(inner, out)
        }
        CollectedType::AtFile { inner, .. } => type_refs(inner, out),
        CollectedType::Object(fields) => field_refs(fields, out),
        CollectedType::Callable { signatures, fields } => {
            for signature in signatures {
                let mut inner = FxHashSet::default();
                signature.params.iter().for_each(|param| type_refs(param, &mut inner));
                type_refs(&signature.return_type, &mut inner);
                out.extend(inner.into_iter().filter(|name| !signature.type_params.contains(name)));
            }
            field_refs(fields, out);
        }
        CollectedType::IndexedAccess { obj, key } => {
            type_refs(obj, out);
            type_refs(key, out);
        }
        CollectedType::Function { type_params, params, return_type, .. } => {
            let mut inner = FxHashSet::default();
            params.iter().for_each(|param| type_refs(param, &mut inner));
            type_refs(return_type, &mut inner);
            out.extend(inner.into_iter().filter(|name| !type_params.contains(name)));
        }
        CollectedType::Conditional { check, extends_type, true_type, false_type } => {
            for part in [check, extends_type, true_type, false_type] {
                type_refs(part, out);
            }
        }
        CollectedType::Mapped { param, key_type, value_type, .. } => {
            type_refs(key_type, out);
            let mut inner = FxHashSet::default();
            type_refs(value_type, &mut inner);
            out.extend(inner.into_iter().filter(|name| name != param));
        }
        CollectedType::String
        | CollectedType::Number
        | CollectedType::Boolean
        | CollectedType::Null
        | CollectedType::Undefined
        | CollectedType::Any
        | CollectedType::Never
        | CollectedType::Unknown
        | CollectedType::Void
        | CollectedType::BigInt
        | CollectedType::Symbol
        | CollectedType::StringLiteral(_)
        | CollectedType::NumberLiteral(_)
        | CollectedType::BoolLiteral(_)
        | CollectedType::BigIntLiteral(_)
        | CollectedType::UniqueSymbol
        | CollectedType::Raw(_) => {}
    }
}

fn field_refs(fields: &[crate::types::CollectedObjectField], out: &mut FxHashSet<CompactString>) {
    fields.iter().for_each(|field| {
        type_refs(&field.collected_type, out);
        if let Some(key) = &field.index_key {
            type_refs(key, out);
        }
    })
}

/// The module and exported name path `reference` (as written in `file`) reaches through one of `imports`, if it is
/// imported.
fn imported_demand(file: &Utf8Path, imports: &[ImportBinding], reference: &str) -> Option<Demand> {
    let (head, rest) = match reference.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (reference, None),
    };
    let Some(binding) = imports.iter().find(|binding| binding.local_name == head) else {
        // `@types/react`'s UMD global (`export as namespace React`): `React.CSSProperties` without an import.
        let name = rest.filter(|_| head == "React")?;
        return Some(Demand { from: file.to_owned(), specifier: Box::from("react"), name: CompactString::from(name) });
    };
    let name = match (binding.exported_name.as_str(), rest) {
        ("*", Some(rest)) => CompactString::from(rest),
        ("*", None) => return None,
        (exported, Some(rest)) => CompactString::from(format!("{exported}.{rest}")),
        (exported, None) => CompactString::from(exported),
    };
    Some(Demand { from: file.to_owned(), specifier: Box::from(binding.specifier.as_str()), name })
}

/// What project file `file` takes from packages (bare specifiers), sorted: each name it imports and uses in a type, and
/// each module it imports at all (a [`Demand`] with an empty name). TypeScript reads a module's React augmentations
/// whenever the program reaches the module, whether or not the file uses a name from it.
pub(crate) fn project_demands(file: &Utf8Path, data: &SourceData) -> Vec<Demand> {
    let mut refs = FxHashSet::default();
    referenced_names(data, &mut refs);
    let bare = |specifier: &str| !specifier.starts_with('.') && !specifier.starts_with('/');
    let mut demands: Vec<Demand> = refs
        .iter()
        .filter_map(|reference| imported_demand(file, &data.imports, reference))
        .filter(|demand| bare(&demand.specifier))
        .collect();
    demands.extend(data.imports.iter().filter(|binding| bare(&binding.specifier)).map(|binding| Demand {
        from: file.to_owned(),
        specifier: Box::from(binding.specifier.as_str()),
        name: CompactString::default(),
    }));
    // Every demand is from `file`, so the derived order's first key, a path of a hundred bytes, only ever ties.
    demands.sort_unstable_by(|a, b| (&a.specifier, &a.name).cmp(&(&b.specifier, &b.name)));
    demands.dedup();
    demands
}

// ─── The layer ────────────────────────────────────────────────────────────────

/// How many files, the module included, a search for React augmentations reads through one module's re-exports.
const AUGMENTATION_SCAN_FILES: usize = 64;

/// How many layers a chain holds before the next extension flattens it into one.
///
/// An extension adds only the files it loaded, as a layer over the one it extends, so a growth costs what it loads and
/// not what the session already holds. Lookups walk the chain, so it is kept short: a flatten copies everything once
/// and the chain starts again.
const MAX_CHAIN_DEPTH: usize = 8;

/// The sizes of a layer and its parents together, for [`DependencyLayer::resolves_like`] and `file_count`.
#[derive(Clone, Copy, Default)]
struct Totals {
    files: usize,
    react_files: usize,
    exports: usize,
    /// Partial mappings a layer added or changed from the chain's ([`DependencyLayer::partial_exports`]): the resolver
    /// reads them, so a load that only adds one is a change.
    partial_exports: usize,
    resolutions: usize,
    unresolved: usize,
}

/// The dependency declarations a run loaded. Immutable once built; a watch revision or a lazy session that needs more
/// builds a new one over it, holding only what the extension added.
pub(crate) struct DependencyLayer {
    /// The layer this one extends. Everything below is this layer's own additions; the parent's are consulted through
    /// the accessors, `named_types` and `import_map`.
    parent: Option<Arc<DependencyLayer>>,
    /// Layers below this one.
    depth: usize,
    totals: Totals,
    /// Declaring files' component mappings: the value index. Nothing here is emitted.
    component_mappings: Vec<ComponentMapping>,
    /// Positions in `component_mappings` by component name, ascending: `ComponentProps<typeof X>` asks `component` for a
    /// name that is usually not here, and comparing every mapping's path to find that out dominated a design system's run.
    component_positions: FxHashMap<CompactString, Vec<usize>>,
    /// Falls back to the parent's.
    pub(crate) named_types: Arc<NamedTypeIndex>,
    /// Falls back to the parent's.
    pub(crate) import_map: Arc<ImportResolutionMap>,
    /// `(module, name)` → the declaring `(file, name)`, or `None` when the module's types do not export it.
    exports: dashmap::ReadOnlyView<(PathKey, CompactString), Option<(Utf8PathBuf, CompactString)>, FxBuildHasher>,
    /// `(module, name)` → the declaring `(file, name)` a walk found past a file it could not read: the resolver's
    /// mapping of a barrel to its declaration while that walk is unread (the ledger marks it). Never in `exports`,
    /// the loader's own lookups or the index; a complete answer in `exports` wins.
    partial_exports: FxHashMap<(PathKey, CompactString), (Utf8PathBuf, CompactString)>,
    /// `(directory, specifier)` → module, for every resolution the loader made.
    resolutions: FxHashMap<(Box<str>, Box<str>), Result<Utf8PathBuf, Unresolved>>,
    files: FxHashSet<PathKey>,
    /// React's files whose declarations' imports were followed; their imports are in `import_map`.
    react_files: FxHashSet<PathKey>,
    /// Each module a demand reached, with the files its re-exports lead to that augment React.
    augmenters: FxHashMap<PathKey, Vec<Utf8PathBuf>>,
    demands: FxHashSet<DemandKey>,
    unresolved: FxHashMap<DemandKey, Unresolved>,
    /// The `(file, name)` declarations whose references were followed.
    claimed: FxHashSet<(PathKey, CompactString)>,
    /// Every parsed file's data (stubs included), so an extension reuses them.
    parsed: FxHashMap<PathKey, Option<Arc<SourceData>>>,
    /// The panics this layer's own demands hit, contained; the demands they covered stay unanswered.
    panics: Vec<Diagnostic>,
    /// The dependency files this layer's load could not read or read again, and what those reads left unfollowed.
    ledger: ReadLedger,
    /// What the chain's ledger leaves unread, once asked ([`DependencyLayer::unread_walks`]).
    unread_walks: OnceLock<Arc<UnreadWalks>>,
    #[cfg(test)]
    unread_walk_builds: std::sync::atomic::AtomicUsize,
    /// How many times this layer's load read a dependency file, for tests.
    #[cfg(test)]
    reads: usize,
}

impl DependencyLayer {
    /// The contained panics of this layer and the ones it extends, and a warning for each file whose latest read in
    /// the chain failed ([`DependencyLayer::read_warnings`]), in message order: a session whose next load read the file
    /// answers as a fresh one.
    pub(crate) fn panic_diagnostics(&self) -> Vec<Diagnostic> {
        let mut all: Vec<Diagnostic> = self.chain().flat_map(|layer| layer.panics.iter().cloned()).collect();
        all.extend(self.read_warnings());
        all.sort_by(|a, b| a.message.cmp(&b.message));
        all
    }

    /// This layer and then each one it extends.
    fn chain(&self) -> impl Iterator<Item = &DependencyLayer> {
        std::iter::successors(Some(self), |layer| layer.parent.as_deref())
    }

    /// How many declaration files this layer holds, with its parents'.
    pub(crate) fn file_count(&self) -> usize {
        self.totals.files
    }

    pub(crate) fn is_dependency_file(&self, file: &Utf8Path) -> bool {
        self.chain().any(|layer| layer.files.contains(file.as_str()))
    }

    /// The declaring `(file, name)` of `name` exported from `module`, when a walk that read every file answered it:
    /// what the index writes. The resolver reads [`Self::export_for_resolver`], which a partial walk's answer also
    /// reaches.
    #[cfg(test)]
    pub(crate) fn export_of_complete(
        &self,
        module: &Utf8Path,
        name: &str,
    ) -> Option<&Option<(Utf8PathBuf, CompactString)>> {
        self.export_entry(&(PathKey::from(module), CompactString::from(name)))
    }

    /// [`Self::export_of_complete`] for the resolver: a complete answer, else one a walk found past a file it could not
    /// read (`partial_exports`), which holds for this layer while that walk is unread.
    pub(crate) fn export_for_resolver(
        &self,
        module: &Utf8Path,
        name: &str,
    ) -> Option<Option<(Utf8PathBuf, CompactString)>> {
        let key = (PathKey::from(module), CompactString::from(name));
        if let Some(found) = self.export_entry(&key) {
            return Some(found.clone());
        }
        self.chain().find_map(|layer| layer.partial_exports.get(&key)).map(|found| Some(found.clone()))
    }

    fn export_entry(&self, key: &(PathKey, CompactString)) -> Option<&Option<(Utf8PathBuf, CompactString)>> {
        self.chain().find_map(|layer| layer.exports.get(key))
    }

    fn resolution(&self, key: &(Box<str>, Box<str>)) -> Option<&Result<Utf8PathBuf, Unresolved>> {
        self.chain().find_map(|layer| layer.resolutions.get(key))
    }

    fn parsed_data(&self, file: &str) -> Option<&Option<Arc<SourceData>>> {
        self.chain().find_map(|layer| layer.parsed.get(file))
    }

    fn augmenters_of(&self, module: &PathKey) -> Option<&Vec<Utf8PathBuf>> {
        self.chain().find_map(|layer| layer.augmenters.get(module))
    }

    fn has_react_file(&self, file: &str) -> bool {
        self.chain().any(|layer| layer.react_files.contains(file))
    }

    fn has_demand(&self, key: &DemandKey) -> bool {
        self.chain().any(|layer| layer.demands.contains(key))
    }

    fn has_claimed(&self, key: &(PathKey, CompactString)) -> bool {
        self.chain().any(|layer| layer.claimed.contains(key))
    }

    /// A dependency component by its declaring `(file, name)`.
    pub(crate) fn component(&self, file: &Utf8Path, name: &str) -> Option<&ComponentMapping> {
        self.chain().find_map(|layer| {
            layer
                .component_positions
                .get(name)?
                .iter()
                .filter_map(|&position| layer.component_mappings.get(position))
                .find(|mapping| mapping.file_path == file)
        })
    }

    /// Why the loader found no declaration for `name` imported from `specifier` by a file in `dir`, if it didn't.
    pub(crate) fn unresolved_reason(&self, dir: &Utf8Path, specifier: &str, name: &str) -> Option<Unresolved> {
        let key = (Box::from(dir.as_str()), Box::from(specifier), CompactString::from(name));
        self.chain().find_map(|layer| layer.unresolved.get(&key)).copied()
    }

    /// Whether `other`, a layer this one was extended from, resolves every name the way this one does: extending only
    /// adds, so equal counts mean nothing was added. A partial mapping counts only where it differs from the chain's.
    pub(crate) fn resolves_like(&self, other: &DependencyLayer) -> bool {
        let (a, b) = (self.totals, other.totals);
        a.files == b.files
            && a.react_files == b.react_files
            && a.exports == b.exports
            && a.partial_exports == b.partial_exports
            && a.resolutions == b.resolutions
            && a.unresolved == b.unresolved
    }

    /// Whether this layer has followed every one of `demands`, none of them past a file it could not read.
    pub(crate) fn covers(&self, demands: &[Demand]) -> bool {
        self.missing(demands).is_empty()
    }

    /// The demands of `demands` a load over this layer follows: those it has not followed, and those whose walk passed
    /// a file it could not read ([`DependencyLayer::unread`] gives the ones they lead to as well).
    pub(crate) fn missing(&self, demands: &[Demand]) -> Vec<Demand> {
        self.missing_at(demands, Retry::Always)
    }

    /// [`Self::missing`], following again only the unread walks that are due under `retry`.
    pub(crate) fn missing_at(&self, demands: &[Demand], retry: Retry) -> Vec<Demand> {
        demands
            .iter()
            .filter(|d| {
                let key = d.key();
                !self.has_demand(&key) || self.awaits_retry(&key, retry)
            })
            .cloned()
            .collect()
    }
}

// ─── Loading ──────────────────────────────────────────────────────────────────

struct Loader<'a> {
    resolvers: &'a ModuleResolvers,
    project_dirs: &'a [Utf8PathBuf],
    /// The layer being extended, consulted before the maps below so an extension holds only its own additions.
    parent: Option<&'a DependencyLayer>,
    parsed: FxDashMap<PathKey, ParseCell>,
    resolutions: FxDashMap<(Box<str>, Box<str>), Result<Utf8PathBuf, Unresolved>>,
    exports: FxDashMap<(PathKey, CompactString), Option<(Utf8PathBuf, CompactString)>>,
    partial_exports: FxDashMap<(PathKey, CompactString), (Utf8PathBuf, CompactString)>,
    claimed: FxDashSet<(PathKey, CompactString)>,
    declaring: FxDashSet<PathKey>,
    react_files: FxDashSet<PathKey>,
    augmenters: FxDashMap<PathKey, Vec<Utf8PathBuf>>,
    /// For each module in `augmenters`, the stamps of the other packages its search read files from.
    augmenter_crossed: FxDashMap<PathKey, Vec<PackageStampRow>>,
    demands: FxDashSet<DemandKey>,
    /// What this load could not read or read again, and the walk that met it.
    ledger: LedgerBuilder<'a>,
    unresolved: FxDashMap<DemandKey, Unresolved>,
    /// Whether each directory `package_of` passed has a `package.json` file.
    package_manifests: FxDashMap<Box<str>, ProbeCell>,
    /// What `package_of` answered for each directory it was asked about, so a repeat is one lookup, not a walk.
    packages: FxDashMap<Box<str>, Option<Box<str>>>,
    /// Whether each directory `resolution_scope` passed has one of its marks.
    scope_marks: FxDashMap<Box<str>, ProbeCell>,
    /// What `resolution_scope` answered for each directory it was asked about.
    scopes: FxDashMap<Box<str>, Box<str>>,
    /// What each `(scope, specifier)` of a module-only demand resolved to, when outside the project and not React's.
    module_demands: FxDashMap<(Box<str>, Box<str>), ModuleCell>,
    /// What an augmentation search found in each file it passed through that was not parsed already.
    scans: FxDashMap<PathKey, ScanCell>,
    /// Something was resolved or answered that the index did not hold.
    learned: AtomicBool,
    /// The diagnostics of demands that panicked.
    panics: Mutex<Vec<Diagnostic>>,
    /// How many times each file was read, and how long each read waits first, so a test can make readers meet.
    #[cfg(test)]
    reads: FxDashMap<PathKey, usize>,
    #[cfg(test)]
    read_pause: std::time::Duration,
}

impl<'a> Loader<'a> {
    fn new(
        resolvers: &'a ModuleResolvers,
        project_dirs: &'a [Utf8PathBuf],
        parent: Option<&'a DependencyLayer>,
    ) -> Self {
        Loader {
            resolvers,
            project_dirs,
            parent,
            parsed: DashMap::default(),
            resolutions: DashMap::default(),
            exports: DashMap::default(),
            partial_exports: DashMap::default(),
            claimed: DashSet::default(),
            declaring: DashSet::default(),
            react_files: DashSet::default(),
            augmenters: DashMap::default(),
            augmenter_crossed: DashMap::default(),
            demands: DashSet::default(),
            ledger: LedgerBuilder::new(parent),
            unresolved: DashMap::default(),
            package_manifests: DashMap::default(),
            packages: DashMap::default(),
            scope_marks: DashMap::default(),
            scopes: DashMap::default(),
            module_demands: DashMap::default(),
            scans: DashMap::default(),
            learned: AtomicBool::new(false),
            panics: Mutex::new(Vec::new()),
            #[cfg(test)]
            reads: DashMap::default(),
            #[cfg(test)]
            read_pause: std::time::Duration::ZERO,
        }
    }
}

impl Loader<'_> {
    /// The text of a dependency file. One that is missing is skipped quietly (`Ok(None)`), since resolution found it a
    /// moment ago and a dangling link or a racing uninstall declares nothing. One that exists but cannot be read is
    /// [`ReadFailed`], recorded in the ledger, which reports it.
    fn read(&self, file: &Utf8Path) -> Result<Option<String>, ReadFailed> {
        #[cfg(test)]
        {
            *self.reads.entry(PathKey::from(file)).or_default() += 1;
            std::thread::sleep(self.read_pause);
        }
        match std::fs::read_to_string(file) {
            Ok(source) => {
                self.ledger.read_done(file);
                Ok(Some(source))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.ledger.read_done(file);
                Ok(None)
            }
            Err(error) => {
                let mut diagnostic = Diagnostic::io_read_error(file, &error);
                // The package's types are not the project's, and the run still produces its output.
                diagnostic.severity = crate::types::DiagnosticSeverity::Warning;
                self.ledger.read_failed(file, diagnostic);
                Err(ReadFailed(PathKey::from(file)))
            }
        }
    }

    /// The data of a dependency file, `None` when it is missing.
    fn parse(&self, file: &Utf8Path) -> Result<Option<Arc<SourceData>>, ReadFailed> {
        self.parse_source(file, None)
    }

    /// `parse`, with the text of `file` when the caller has already read it. The file is read once per load: callers
    /// that ask at once wait for the first, and every caller gets its result, a failed read included, so one load sees
    /// one state of the file and opens it once. The layer keeps only files that were read (`load`), so the next load
    /// reads a failed one again.
    fn parse_source(&self, file: &Utf8Path, source: Option<String>) -> Result<Option<Arc<SourceData>>, ReadFailed> {
        if let Some(found) = self.parent.and_then(|parent| parent.parsed_data(file.as_str())) {
            return Ok(found.clone());
        }
        let cell = self.parsed.entry(PathKey::from(file)).or_default().clone();
        cell.get_or_init(|| {
            let source = match source {
                Some(source) => source,
                None => match self.read(file)? {
                    Some(source) => source,
                    None => return Ok(None),
                },
            };
            let mut data = crate::extractor::parse_file(file, &source);
            // Problems inside a package's types are not the project's; an unresolved demand is reported instead.
            data.diagnostics.clear();
            Ok(Some(Arc::new(data)))
        })
        .clone()
    }

    /// Resolve `specifier` as imported by `from_file`, the way the resolver will.
    fn resolve(&self, from_file: &Utf8Path, specifier: &str) -> Result<Utf8PathBuf, Unresolved> {
        let dir = from_file.parent().map_or("", Utf8Path::as_str);
        let key = (Box::from(dir), Box::from(specifier));
        if let Some(found) = self.resolutions.get(&key) {
            return found.clone();
        }
        if let Some(found) = self.parent.and_then(|parent| parent.resolution(&key)) {
            return found.clone();
        }
        let result = match crate::resolver::resolve_module_with(
            &self.resolvers.main,
            &self.resolvers.types_first,
            specifier,
            from_file,
        ) {
            ModuleResolution::PackageTypes(path) => Ok(path),
            ModuleResolution::File(path) if is_typescript(&path) => Ok(path),
            ModuleResolution::File(_) => Err(Unresolved::NoTypes),
            ModuleResolution::Unresolved(_) => Err(Unresolved::Unresolvable),
        };
        // An unresolved import is not kept in the index, so asking again is not new knowledge.
        if result.is_ok() {
            self.learn(Utf8Path::new(dir));
        }
        self.resolutions.insert(key, result.clone());
        result
    }

    fn in_project(&self, file: &Utf8Path) -> bool {
        !in_node_modules(file.as_str()) && self.project_dirs.iter().any(|dir| file.starts_with(dir))
    }

    /// React's own types stay with the resolver's React handling (builtins, curated HTML attributes).
    fn is_react(file: &Utf8Path) -> bool {
        crate::origin::declaration_origin(file.as_str()) == crate::origin::Origin::React
    }

    /// The `(file, name)` declaring `name` as exported from `module`.
    /// Incomplete when the walk passed a module it could not read: what it found then is the best this load has.
    fn find_export(&self, module: &Utf8Path, name: &str) -> Partial<Option<(Utf8PathBuf, CompactString)>> {
        let mut unkept = Unkept::default();
        let value = self.find_export_in(module, name, &mut FxHashSet::default(), &mut unkept);
        Partial { value, unread: unkept.unread }
    }

    /// `find_export` within one walk. A lookup that met a module already on the walk (a re-export cycle) or passed one
    /// it could not read is returned but not kept, and `unkept` says why to every lookup that relied on it: a cycle's
    /// answer depends on where the walk entered it, and an unread module may have declared the name. Kept answers are
    /// therefore the same in any order, on any thread, and the same as a run that read every file. The walk goes on
    /// past a module it cannot read, as it does past one that does not export the name.
    fn find_export_in(
        &self,
        module: &Utf8Path,
        name: &str,
        visited: &mut FxHashSet<(PathKey, CompactString)>,
        unkept: &mut Unkept,
    ) -> Option<(Utf8PathBuf, CompactString)> {
        let key = (PathKey::from(module), CompactString::from(name));
        if let Some(found) = self.exports.get(&key) {
            return found.clone();
        }
        if let Some(found) = self.parent.and_then(|parent| parent.export_entry(&key)) {
            return found.clone();
        }
        if !visited.insert(key.clone()) {
            unkept.cycle = true;
            return None;
        }
        let mut own = Unkept::default();
        let found = self.find_export_uncached(module, name, visited, &mut own);
        visited.remove(&key);
        if !own.cycle && own.unread.is_empty() {
            if let Some(dir) = module.parent() {
                self.learn(dir);
            }
            self.exports.insert(key, found.clone());
        }
        unkept.absorb(own);
        found
    }

    fn find_export_uncached(
        &self,
        module: &Utf8Path,
        name: &str,
        visited: &mut FxHashSet<(PathKey, CompactString)>,
        unkept: &mut Unkept,
    ) -> Option<(Utf8PathBuf, CompactString)> {
        let data = match self.parse(module) {
            Ok(data) => data?,
            Err(ReadFailed(file)) => {
                unkept.unread.push(file);
                return None;
            }
        };
        if declares(&data, module, name) {
            return Some((module.to_owned(), CompactString::from(name)));
        }
        // A member of a value the module declares (`Primitive.div`): its declaration is the value's.
        if name.split_once('.').is_some_and(|(head, _)| declares(&data, module, head)) {
            return Some((module.to_owned(), CompactString::from(name)));
        }
        let (head, rest) = match name.split_once('.') {
            Some((head, rest)) => (head, Some(rest)),
            None => (name, None),
        };
        for export in &data.exports {
            match export {
                LexedExport::ReExportNamed { local_name, source_name, source_specifier, .. } if local_name == head => {
                    // An explicit export shadows a wildcard one, so a hop that cannot be resolved leaves the name unanswered
                    // rather than to a later `export *`; `unresolved_cause` names the hop.
                    let Ok(target) = self.resolve(module, source_specifier) else { return None };
                    let next = match rest {
                        Some(rest) => format!("{source_name}.{rest}"),
                        None => source_name.clone(),
                    };
                    return self.find_export_in(&target, &next, visited, unkept);
                }
                LexedExport::ReExportNamespace { namespace, source_specifier } if namespace == head => {
                    let (Ok(target), Some(rest)) = (self.resolve(module, source_specifier), rest) else {
                        return None;
                    };
                    return self.find_export_in(&target, rest, visited, unkept);
                }
                // `export { DialogContent as Content }`, the way bundled typings (Radix's) name their parts.
                LexedExport::LocalDeclaration { name: exported, declared_as: Some(local), .. }
                    if exported == head && declares(&data, module, local) =>
                {
                    let name = match rest {
                        Some(rest) => format!("{local}.{rest}"),
                        None => local.clone(),
                    };
                    return Some((module.to_owned(), CompactString::from(name)));
                }
                _ => {}
            }
        }
        // A local export can name an imported binding (`import Image from './Image'; export default Image`).
        if let Some(local) = data.exports.iter().find_map(|export| match export {
            LexedExport::LocalDeclaration { name: exported, declared_as, .. } if exported == head => {
                Some(declared_as.as_deref().unwrap_or(head))
            }
            LexedExport::LocalDeclaration { .. }
            | LexedExport::ReExportNamed { .. }
            | LexedExport::ReExportAll { .. }
            | LexedExport::ReExportNamespace { .. } => None,
        }) {
            let reference = rest.map_or_else(|| local.to_owned(), |rest| format!("{local}.{rest}"));
            if let Some(demand) = imported_demand(module, &data.imports, &reference) {
                let Ok(target) = self.resolve(module, &demand.specifier) else { return None };
                return self.find_export_in(&target, &demand.name, visited, unkept);
            }
        }
        if head == "default" {
            return None;
        }
        // A target that cannot be read is passed, marked in `unkept`, so a later target can still answer.
        for export in &data.exports {
            if let LexedExport::ReExportAll { source_specifier, .. } = export {
                let Ok(target) = self.resolve(module, source_specifier) else { continue };
                if let Some(found) = self.find_export_in(&target, name, visited, unkept) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Runs `demand` on the scope, recording a panic as a diagnostic instead of letting it end the whole load.
    fn spawn_demand<'s>(&'s self, scope: &rayon::Scope<'s>, demand: Demand) {
        scope.spawn(move |scope| {
            let label = format!("dependency demand {}#{}", demand.specifier, demand.name);
            if let Err(panic) = crate::panic_guard::contain_panic(&label, || self.demand(scope, demand)) {
                self.panics.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(panic);
            }
        });
    }

    fn demand<'s>(&'s self, scope: &rayon::Scope<'s>, demand: Demand) {
        #[cfg(test)]
        assert!(&*demand.name != DEMAND_PANIC_TEST_NAME, "injected panic");
        let key = demand.key();
        let known = self.parent.is_some_and(|parent| parent.has_demand(&key)) || !self.demands.insert(key.clone());
        // A demand whose last walk passed a file it could not read is followed again, once per load.
        if known && !self.ledger.retries(&key) {
            return;
        }
        let followed = self.follow(scope, &demand, &key);
        if followed.incomplete() {
            self.ledger.unread_demand(&demand, followed.unread);
        }
    }

    /// Follows `demand`, whose key is `key`: what it names is loaded, or why not is in `unresolved`. Incomplete when a
    /// file it needed could not be read: what the readable files gave is loaded for this load, and no reason is kept,
    /// since the name is unknown, not absent.
    fn follow<'s>(&'s self, scope: &rayon::Scope<'s>, demand: &Demand, key: &DemandKey) -> Partial<()> {
        let node = Node::Demand(key.clone());
        if demand.name.is_empty() {
            return self.module_demand(scope, demand, &node);
        }
        let module = match self.resolve(&demand.from, &demand.specifier) {
            Ok(module) => module,
            Err(reason) => {
                self.unresolved.insert(key.clone(), reason);
                return Partial::DONE;
            }
        };
        if self.in_project(&module) {
            return Partial::DONE;
        }
        if Self::is_react(&module) {
            // `@types/react` declares its exports inside `declare namespace React` (`export = React`).
            let qualified = CompactString::from(format!("React.{}", demand.name));
            return match self.parse(&module) {
                Ok(Some(data)) if declares(&data, &module, &qualified) => {
                    self.load_declaration(scope, module, qualified, &node)
                }
                Ok(_) => Partial::DONE,
                Err(failed) => Partial::unread_at(failed),
            };
        }
        let found = self.find_export(&module, &demand.name);
        if let (Some(declaring), true) = (&found.value, found.incomplete()) {
            self.partial_exports.insert((PathKey::from(module.as_path()), demand.name.clone()), declaring.clone());
        }
        let loaded = match found.value {
            Some((file, name)) => self.load_declaration(scope, file, name, &node),
            None if !found.unread.is_empty() => Partial::DONE,
            None => {
                let cause = self.unresolved_cause(&module, &demand.name);
                if cause.incomplete() {
                    return Partial { value: (), unread: cause.unread };
                }
                self.unresolved.insert(key.clone(), cause.value.unwrap_or(Unresolved::NotExported));
                Partial::DONE
            }
        };
        loaded.and(Partial { value: (), unread: found.unread })
    }

    /// Why a name `module` seems not to export was not found: a re-export hop on the way to it that cannot be
    /// resolved. A named or namespace hop for the name is the cause outright; an unresolvable `export *` only could
    /// have held it, so it is reported when nothing else explains the miss. `None` means every hop resolved;
    /// incomplete when a module on the way could not be read, which may have held the cause.
    fn unresolved_cause(&self, module: &Utf8Path, name: &str) -> Partial<Option<Unresolved>> {
        let mut visited = FxHashSet::default();
        let (mut star, mut unread) = (None, Vec::new());
        let cause = self.unresolved_cause_in(module, name, &mut visited, &mut star, &mut unread);
        Partial { value: cause.or(star), unread }
    }

    fn unresolved_cause_in(
        &self,
        module: &Utf8Path,
        name: &str,
        visited: &mut FxHashSet<(PathKey, CompactString)>,
        star: &mut Option<Unresolved>,
        unread: &mut Vec<PathKey>,
    ) -> Option<Unresolved> {
        if !visited.insert((PathKey::from(module), CompactString::from(name))) {
            return None;
        }
        let data = match self.parse(module) {
            Ok(data) => data?,
            Err(ReadFailed(file)) => {
                unread.push(file);
                return None;
            }
        };
        let (head, rest) = match name.split_once('.') {
            Some((head, rest)) => (head, Some(rest)),
            None => (name, None),
        };
        // The order `find_export_uncached` reads them in: an explicit export answers for its name and shadows every
        // wildcard, so a wildcard is looked into only when no explicit export names it.
        for star_pass in [false, true] {
            for export in &data.exports {
                let (specifier, next) = match export {
                    LexedExport::ReExportNamed { local_name, source_name, source_specifier, .. }
                        if !star_pass && local_name == head =>
                    {
                        let next = rest.map_or_else(|| source_name.clone(), |rest| format!("{source_name}.{rest}"));
                        (source_specifier, next)
                    }
                    LexedExport::ReExportNamespace { namespace, source_specifier }
                        if !star_pass && namespace == head =>
                    {
                        (source_specifier, rest?.to_owned())
                    }
                    // `export *` never carries `default`, as `find_export_uncached` enforces.
                    LexedExport::ReExportAll { source_specifier, .. } if star_pass && head != "default" => {
                        (source_specifier, name.to_owned())
                    }
                    _ => continue,
                };
                match self.resolve(module, specifier) {
                    Err(cause) if star_pass => {
                        star.get_or_insert(cause);
                    }
                    Err(cause) => return Some(cause),
                    Ok(target) => {
                        let cause = self.unresolved_cause_in(&target, &next, visited, star, unread);
                        if cause.is_some() || !star_pass {
                            return cause;
                        }
                    }
                }
            }
        }
        None
    }

    /// A module imported for its own sake: only what it merges into React is read.
    ///
    /// Directories with the same resolution scope resolve a specifier to the same module, so the first of them to ask
    /// resolves it, finds and reads the files that augment React, and loads their declarations; the rest wait for the
    /// files and ask nothing, and reach what the first reached. A file the search could not read leaves every demand of
    /// the scope incomplete alike, and the augmentations of the files it did read are loaded all the same.
    fn module_demand<'s>(&'s self, scope: &rayon::Scope<'s>, demand: &Demand, node: &Node) -> Partial<()> {
        let dir = demand.from.parent().map_or("", Utf8Path::as_str);
        let key = (self.resolution_scope(dir), demand.specifier.clone());
        let cell = self.module_demands.entry(key).or_default().clone();
        let mut first = false;
        let (searched_by, augmenting) = cell.get_or_init(|| {
            first = true;
            (demand.key(), self.search_module(demand))
        });
        let loaded = match first {
            true => self.load_augmentations(scope, &augmenting.value, node),
            false => {
                self.ledger.reached(node, Node::Demand(searched_by.clone()));
                Partial::DONE
            }
        };
        loaded.and(Partial { value: (), unread: augmenting.unread.clone() })
    }

    /// The files that augment React which the module `demand` resolves to leads to, parsed.
    fn search_module(&self, demand: &Demand) -> Partial<Vec<(Utf8PathBuf, Arc<SourceData>)>> {
        let Ok(module) = self.resolve(&demand.from, &demand.specifier) else {
            return Partial::complete(Vec::new());
        };
        if self.in_project(&module) || Self::is_react(&module) {
            return Partial::complete(Vec::new());
        }
        let Partial { value: files, mut unread } = self.augmenting_files(&module);
        let mut parsed = Vec::new();
        for file in files {
            match self.parse(&file) {
                Ok(Some(data)) => parsed.push((file, data)),
                Ok(None) => {}
                Err(ReadFailed(file)) => unread.push(file),
            }
        }
        Partial { value: parsed, unread }
    }

    /// The nearest directory at or above `dir` where module resolution can start to differ: one with a `package.json`
    /// (its `imports`), a `tsconfig.json` or `jsconfig.json` (its `paths`) or a `node_modules`. A bare specifier
    /// resolves alike from every directory with the same scope. A directory with no such ancestor is its own.
    fn resolution_scope(&self, dir: &str) -> Box<str> {
        if let Some(found) = self.scopes.get(dir) {
            return found.clone();
        }
        let marks = ["package.json", "tsconfig.json", "jsconfig.json", "node_modules"];
        let has_mark = |candidate: &Utf8Path| marks.iter().any(|mark| candidate.join(mark).exists());
        let scope = Utf8Path::new(dir).ancestors().find(|candidate| probe_once(&self.scope_marks, candidate, has_mark));
        let scope = Box::<str>::from(scope.map_or(dir, Utf8Path::as_str));
        self.scopes.insert(Box::from(dir), scope.clone());
        scope
    }

    /// The files `module` leads to through its re-exports that merge declarations into React
    /// (`declare module "react" { ... }`): TypeScript reads them whenever the program reaches the file, whether or not
    /// a name from them is used. The search is bounded ([`AUGMENTATION_SCAN_FILES`]), breadth first in source order,
    /// so the answer for a module is the same on any thread; it is kept, in the layer and in the index on disk, unless
    /// the search could not read a file: then it holds the files found among those read, for this load.
    fn augmenting_files(&self, module: &Utf8Path) -> Partial<Vec<Utf8PathBuf>> {
        let key = PathKey::from(module);
        let known = self.augmenters.get(&key).map(|files| files.clone());
        if let Some(files) = known.or_else(|| self.parent.and_then(|parent| parent.augmenters_of(&key)).cloned()) {
            return Partial::complete(files);
        }
        let Partial { value: (files, crossed), unread } = self.find_augmenting_files(module);
        if unread.is_empty() {
            self.augmenters.insert(key.clone(), files.clone());
            self.augmenter_crossed.insert(key, crossed);
            // An empty answer is persisted too, or the next run would scan the module's re-exports again.
            module.parent().inspect(|dir| self.learn(dir));
        }
        Partial { value: files, unread }
    }

    /// Loads the `React.`-qualified declarations of `augmenting`, the parsed files that augment React.
    fn load_augmentations<'s>(
        &'s self,
        scope: &rayon::Scope<'s>,
        augmenting: &[(Utf8PathBuf, Arc<SourceData>)],
        from: &Node,
    ) -> Partial<()> {
        let mut loaded = Partial::DONE;
        for (file, data) in augmenting {
            let mut names: Vec<&String> = data.interfaces.keys().collect();
            names.sort();
            for key in names {
                if let Some(name) = key.strip_prefix(file.as_str()).and_then(|rest| rest.strip_prefix(':'))
                    && name.starts_with("React.")
                {
                    loaded = loaded.and(self.load_declaration(scope, file.clone(), CompactString::from(name), from));
                }
            }
        }
        loaded
    }

    /// Whether `file` merges declarations into React, and the specifiers it re-exports, in order. A file that names
    /// neither `React` nor the module `"react"` cannot declare into the React namespace, so one not already parsed is
    /// only scanned for its re-exports and keeps nothing else: most files an augmentation search passes through
    /// declare nothing the run needs. Read once per load, as `parse_source` reads; a file both scanned and parsed by
    /// callers that meet may be read by each.
    fn augmentation_scan(&self, file: &Utf8Path, entry: bool) -> Result<Option<Scan>, ReadFailed> {
        let parsed = self.parent.and_then(|parent| parent.parsed_data(file.as_str())).cloned().map(Ok).or_else(|| {
            let cell = self.parsed.get(file.as_str())?;
            cell.get().cloned()
        });
        if let Some(parsed) = parsed {
            return Ok(parsed?.map(|data| scan_of(&data, file)));
        }
        let cell = self.scans.entry(PathKey::from(file)).or_default().clone();
        cell.get_or_init(|| {
            let Some(source) = self.read(file)? else { return Ok(None) };
            if entry || may_declare_into_react(&source) {
                return Ok(self.parse_source(file, Some(source))?.map(|data| scan_of(&data, file)));
            }
            let re_exports = match may_re_export(&source) {
                true => Arc::from(crate::extractor::re_export_specifiers(file, &source)),
                false => Arc::from([]),
            };
            Ok(Some((false, re_exports)))
        })
        .clone()
    }

    /// The files `module`'s re-exports lead to that augment React, and the stamps of the other packages the search read
    /// files from: the answer holds only while those packages do not change, as well as the module's own.
    /// Incomplete when it could not read a file on the way, which may have been one or led to one; the search goes on
    /// with the rest.
    fn find_augmenting_files(&self, module: &Utf8Path) -> Partial<(Vec<Utf8PathBuf>, Vec<PackageStampRow>)> {
        let mut found = Vec::new();
        let mut unread = Vec::new();
        let mut seen: FxHashSet<Utf8PathBuf> = FxHashSet::default();
        let mut queue = std::collections::VecDeque::from([module.to_owned()]);
        while let Some(file) = queue.pop_front() {
            if seen.len() >= AUGMENTATION_SCAN_FILES {
                break;
            }
            if !seen.insert(file.clone()) || self.in_project(&file) || Self::is_react(&file) {
                continue;
            }
            let (augments, re_exports) = match self.augmentation_scan(&file, file == module) {
                Ok(Some(scan)) => scan,
                Ok(None) => continue,
                Err(ReadFailed(file)) => {
                    unread.push(file);
                    continue;
                }
            };
            if augments {
                found.push(file.clone());
            }
            for specifier in re_exports.iter() {
                if let Ok(target) = self.resolve(&file, specifier) {
                    queue.push_back(target);
                }
            }
        }
        found.sort();
        let own = module.parent().and_then(|dir| self.package_of(dir));
        let mut crossed: Vec<PackageStampRow> = seen
            .iter()
            .filter_map(|file| self.package_of(file.parent()?))
            .filter(|package| Some(package) != own.as_ref())
            .collect::<FxHashSet<_>>()
            .into_iter()
            .filter_map(|package| {
                let (mtime_ns, size) = package_stamp(&package)?;
                Some((package.to_string(), mtime_ns, size))
            })
            .collect();
        crossed.sort();
        Partial { value: (found, crossed), unread }
    }

    /// Mark `(file, name)` loaded and follow the names its declaration references: same-file ones in place, imported
    /// ones as new demands. React's own declarations stay with the resolver's React handling (builtins, curated HTML
    /// attributes), so React's files never enter the layer, but what they import does: `CSSProperties` extends
    /// csstype's `Properties`. Incomplete, claiming nothing, when `file` could not be read (an export the index
    /// answered, whose declaring file this load had not read); the demands it spawns answer for themselves, and the
    /// ledger records that `from` reached each declaration and each declaration the demands it spawned.
    fn load_declaration<'s>(
        &'s self,
        scope: &rayon::Scope<'s>,
        file: Utf8PathBuf,
        name: CompactString,
        from: &Node,
    ) -> Partial<()> {
        if self.in_project(&file) {
            return Partial::DONE;
        }
        let react = Self::is_react(&file);
        let data = match self.parse(&file) {
            Ok(Some(data)) => data,
            Ok(None) => return Partial::DONE,
            Err(failed) => return Partial::unread_at(failed),
        };
        // `Primitive.div` is a member of the value `Primitive`, whose declared type has it. Each name goes with the
        // node that reached it.
        let mut stack: Vec<(CompactString, Node)> =
            name.split_once('.').map(|(head, _)| (CompactString::from(head), from.clone())).into_iter().collect();
        stack.push((name, from.clone()));
        while let Some((name, reached_from)) = stack.pop() {
            let claim = (PathKey::from(file.as_path()), name.clone());
            let node = Node::Claim(claim.clone());
            self.ledger.reached(&reached_from, node.clone());
            if self.parent.is_some_and(|parent| parent.has_claimed(&claim)) || !self.claimed.insert(claim) {
                continue;
            }
            if react {
                if !self.parent.is_some_and(|parent| parent.has_react_file(file.as_str())) {
                    self.react_files.insert(PathKey::from(file.as_path()));
                }
            } else if !self.parent.is_some_and(|parent| parent.is_dependency_file(&file)) {
                self.declaring.insert(PathKey::from(file.as_path()));
            }
            // The type a value is declared with (`declare const Primitive: Primitives`) goes with the value.
            let value_type = CompactString::from(format!("typeof {name}"));
            if declares(&data, &file, &value_type) {
                stack.push((value_type, node.clone()));
            }
            let mut refs = FxHashSet::default();
            declaration_refs(&data, &file, &name, &mut refs);
            for reference in refs {
                let qualified = react.then(|| CompactString::from(format!("React.{reference}")));
                if declares(&data, &file, &reference) {
                    stack.push((reference, node.clone()));
                } else if declares(&data, &file, &format!("typeof {reference}")) {
                    // `typeof NODES` names a value that has only its declared type.
                    stack.push((CompactString::from(format!("typeof {reference}")), node.clone()));
                } else if let Some(qualified) = qualified.filter(|q| declares(&data, &file, q)) {
                    stack.push((qualified, node.clone()));
                } else if let Some(demand) = imported_demand(&file, &data.imports, &reference) {
                    self.ledger.reached(&node, Node::Demand(demand.key()));
                    self.spawn_demand(scope, demand);
                }
            }
        }
        Partial::DONE
    }

    /// Note that an answer about `dir` is new. A project file's own imports are resolved on every run and never reach
    /// the index, so they must not make a warm run rewrite it. Export answers of a package that `write_index` drops
    /// still count here, since only the written bytes tell; `write_index` leaves the file alone when they are unchanged.
    fn learn(&self, dir: &Utf8Path) {
        if in_node_modules(dir.as_str()) && self.package_of(dir).is_some() {
            self.learned.store(true, Ordering::Relaxed);
        }
    }

    /// The nearest directory at or above `dir` with a `package.json`, if it is inside `node_modules`.
    fn package_of(&self, dir: &Utf8Path) -> Option<Box<str>> {
        if let Some(found) = self.packages.get(dir.as_str()) {
            return found.clone();
        }
        let has_manifest = |candidate: &Utf8Path| candidate.join("package.json").is_file();
        let found = dir
            .ancestors()
            .find(|candidate| probe_once(&self.package_manifests, candidate, has_manifest))
            .filter(|candidate| in_node_modules(candidate.as_str()))
            .map(|candidate| Box::from(candidate.as_str()));
        self.packages.insert(Box::from(dir.as_str()), found.clone());
        found
    }
}

#[cfg(test)]
thread_local! {
    /// How many ancestor cells this thread looked up in `probe_once`.
    static CELL_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `probe(dir)`, asked of the disk once per load: the files of one tree share their ancestors, and a load's demands
/// start together, so the threads that ask about one directory at once wait for one probe. A probe never waits on
/// another directory's, and the callers walk the lexical ancestors in a loop, so a deep tree cannot deadlock or
/// exhaust the stack.
fn probe_once(memo: &FxDashMap<Box<str>, ProbeCell>, dir: &Utf8Path, probe: impl FnOnce(&Utf8Path) -> bool) -> bool {
    #[cfg(test)]
    CELL_LOOKUPS.with(|lookups| lookups.set(lookups.get() + 1));
    let cell = match memo.get(dir.as_str()) {
        Some(cell) => cell.clone(),
        None => memo.entry(Box::from(dir.as_str())).or_default().clone(),
    };
    *cell.get_or_init(|| probe(dir))
}

/// Whether `source` could declare into the React namespace: `declare module "react"` or `namespace React`, which are
/// the only ways the extractor keys an interface as `React.`-qualified. A word that may be one of those, or follows a
/// comment, counts; importing from "react" does not.
fn may_declare_into_react(source: &str) -> bool {
    ["React", "\"react\"", "'react'"].iter().any(|name| {
        source.match_indices(name).any(|(at, _)| {
            let before = source[..at].trim_end();
            before.ends_with("module") || before.ends_with("namespace") || before.ends_with('/')
        })
    })
}

/// Whether `source` could re-export anything: every spelling puts `{`, `*` or `default` after `export` (`export type {`
/// too), and a comment there counts. A file that does not has nothing to parse for.
fn may_re_export(source: &str) -> bool {
    source.match_indices("export").any(|(at, word)| {
        let rest = source[at + word.len()..].trim_start();
        let rest = rest.strip_prefix("type").map_or(rest, str::trim_start);
        rest.starts_with(['{', '*', '/']) || rest.starts_with("default")
    })
}

/// Whether `data`, the parse of `file`, declares into the React namespace, and the specifiers it re-exports, in order.
fn scan_of(data: &SourceData, file: &Utf8Path) -> Scan {
    let react_prefix = format!("{file}:React.");
    let augments = data.interfaces.keys().any(|key| key.starts_with(&react_prefix));
    let re_exports = data
        .exports
        .iter()
        .filter_map(|export| match export {
            LexedExport::ReExportNamed { source_specifier, .. }
            | LexedExport::ReExportAll { source_specifier, .. }
            | LexedExport::ReExportNamespace { source_specifier, .. } => Some(source_specifier.clone()),
            LexedExport::LocalDeclaration { .. } => None,
        })
        .collect();
    (augments, re_exports)
}

/// Whether `file` declares `name` (an interface, type alias, enum, const array or component, possibly
/// namespace-qualified).
fn declares(data: &SourceData, file: &Utf8Path, name: &str) -> bool {
    let key = format!("{file}:{name}");
    data.interfaces.contains_key(&key)
        || data.type_aliases.contains_key(&key)
        || data.enums.contains_key(&key)
        || data.const_arrays.contains_key(&key)
        || data.component_mappings.iter().any(|m| m.binding_name() == name)
        // A constant declared by its type alone (`declare const Primitive: Primitives`).
        || data.type_aliases.contains_key(&format!("{file}:typeof {name}"))
}

/// The names `(file, name)`'s declaration references, without its own type parameters.
fn declaration_refs(data: &SourceData, file: &Utf8Path, name: &str, out: &mut FxHashSet<CompactString>) {
    let key = format!("{file}:{name}");
    if let Some(iface) = data.interfaces.get(&key) {
        interface_refs(iface, out);
    }
    if let Some(alias) = data.type_aliases.get(&key) {
        alias_refs(alias, out);
    }
    for mapping in data.component_mappings.iter().filter(|m| m.binding_name() == name) {
        mapping_refs(mapping, out);
    }
    // A reference that omits an argument reads the default (`DOMAttributes<T = FocusableElement>`).
    for default in data.type_param_defaults.get(&key).into_iter().flatten().flatten() {
        type_refs(default, out);
    }
    for params in [data.interface_type_params.get(&key), data.type_alias_params.get(&key)].into_iter().flatten() {
        for param in params {
            out.remove(param.as_str());
        }
    }
}

/// Load what `demands` reference: from scratch for a cold run, or on top of `previous` when a watch revision or a lazy
/// session found names it hasn't loaded.
///
/// An extension is a layer over `previous` holding only what it added, until the chain reaches [`MAX_CHAIN_DEPTH`]; then
/// it takes everything the chain holds into one layer, and the chain starts again.
pub(crate) fn load(
    demands: Vec<Demand>,
    options: &PipelineOptions,
    previous: Option<&Arc<DependencyLayer>>,
) -> DependencyLayer {
    load_at(demands, options, previous, Retry::Always, Stamp::unsessioned())
}

/// [`load`], following again only the walks `retry` makes due, and recording its failed reads with `stamp`.
pub(crate) fn load_at(
    demands: Vec<Demand>,
    options: &PipelineOptions,
    previous: Option<&Arc<DependencyLayer>>,
    retry: Retry,
    stamp: Stamp,
) -> DependencyLayer {
    load_timed(demands, options, previous, retry, stamp, Arc::new(Instant::now))
}

/// [`load_at`], each failed read timed by `clock` when it fails: a session's own clock, so a slow load does not make
/// a failure near its end due early.
pub(crate) fn load_timed(
    demands: Vec<Demand>,
    options: &PipelineOptions,
    previous: Option<&Arc<DependencyLayer>>,
    retry: Retry,
    stamp: Stamp,
    clock: Clock,
) -> DependencyLayer {
    // Its own resolvers, never another consumer's: `oxc_resolver` keeps a config reached through another's
    // `references` without that config's own references, so a shared cache would make what this load resolves depend
    // on what was resolved first.
    let resolvers = &ModuleResolvers::new(options);
    let project_dirs: Vec<Utf8PathBuf> =
        options.src_dirs.iter().filter_map(|dir| crate::canonical_path::canonicalize(dir).ok()).collect();
    let parent: Option<Arc<DependencyLayer>> = previous.filter(|layer| layer.depth < MAX_CHAIN_DEPTH).cloned();
    let flatten = previous.filter(|_| parent.is_none());
    let mut loader = Loader::new(resolvers, &project_dirs, parent.as_deref());
    loader.ledger.retry = retry;
    loader.ledger.stamp = stamp;
    loader.ledger.clock = clock;
    // With the cache off the index is neither read nor written, and the run learns everything again.
    let index_path = options.use_cache.then(|| index_path(options));
    match (previous, flatten) {
        (None, _) => {
            if let Some(path) = &index_path {
                read_index(path, &loader);
            }
        }
        // A chain at its depth limit: take everything it holds, so this layer stands alone.
        (Some(_), Some(full)) => {
            let mut chain: Vec<_> = full.chain().collect();
            chain.reverse();
            for layer in chain {
                for (key, value) in &layer.resolutions {
                    loader.resolutions.insert(key.clone(), value.clone());
                }
                for (key, value) in layer.exports.iter() {
                    loader.exports.insert(key.clone(), value.clone());
                }
                for (key, value) in &layer.partial_exports {
                    loader.partial_exports.insert(key.clone(), value.clone());
                }
                for (key, data) in &layer.parsed {
                    loader.parsed.insert(key.clone(), Arc::new(OnceLock::from(Ok(data.clone()))));
                }
                for key in &layer.demands {
                    loader.demands.insert(key.clone());
                }
                for key in &layer.claimed {
                    loader.claimed.insert(key.clone());
                }
                for (key, reason) in &layer.unresolved {
                    loader.unresolved.insert(key.clone(), *reason);
                }
                for file in &layer.files {
                    loader.declaring.insert(file.clone());
                }
                for file in &layer.react_files {
                    loader.react_files.insert(file.clone());
                }
                for (module, files) in &layer.augmenters {
                    loader.augmenters.insert(module.clone(), files.clone());
                }
                loader.ledger.inherit(&layer.ledger);
                // The demands a panic covered stay unanswered in the flattened layer, so its report stays too.
                loader
                    .panics
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(layer.panics.iter().cloned());
            }
        }
        (Some(_), None) => {}
    }
    rayon::scope(|scope| {
        for demand in demands {
            let loader = &loader;
            loader.spawn_demand(scope, demand);
        }
    });
    // Only a cold load writes the index. A walk that passed a file it could not read put nothing in the maps it writes
    // (an incomplete [`Partial`] is kept nowhere), so a failed read costs only its own walks' entries.
    if let (None, Some(path)) = (previous, &index_path)
        && loader.learned.load(Ordering::Relaxed)
    {
        write_index(path, &loader);
    }

    let mut files: Vec<PathKey> = loader.declaring.iter().map(|f| f.clone()).collect();
    files.sort();
    // Each file's declarations go straight into the index, and only what the import map reads (imports and
    // re-exports) and the value index (component mappings) is kept apart; a merged copy of every file's data would
    // hold the same declarations twice. An extension indexes only its own files; the parent's answer for the rest.
    let mut named = NamedTypeIndex::default();
    named.fallback = parent.as_ref().map(|layer| layer.named_types.clone());
    let mut component_mappings: Vec<ComponentMapping> = Vec::new();
    let mut imports = GlobalSourceData::default();
    // A file is declaring or React's only once its parse was claimed, so these read the parse this load kept; a failed
    // read claimed nothing.
    for file in &files {
        let path = Utf8Path::new(std::borrow::Borrow::<str>::borrow(file));
        if let Ok(Some(data)) = loader.parse(path) {
            named.merge_file(&data);
            component_mappings.extend(data.component_mappings.iter().cloned());
            imports.import_map.insert(path.to_owned(), data.imports.clone());
            imports.re_export_map.insert(path.to_owned(), data.exports.clone());
        }
    }
    component_mappings.sort_by(|a, b| (&a.file_path, a.span_start).cmp(&(&b.file_path, b.span_start)));
    let mut component_positions: FxHashMap<CompactString, Vec<usize>> = FxHashMap::default();
    for (position, mapping) in component_mappings.iter().enumerate() {
        component_positions.entry(CompactString::from(mapping.binding_name())).or_default().push(position);
    }
    let named_types = Arc::new(named);
    // React's declarations reference what React imports (`CSS.Properties`), so those imports must resolve too; its
    // declarations stay out of `named_types`.
    let react_files: FxHashSet<PathKey> = loader.react_files.iter().map(|file| file.clone()).collect();
    for file in &react_files {
        let path = Utf8Path::new(std::borrow::Borrow::<str>::borrow(file));
        if let Ok(Some(data)) = loader.parse(path) {
            imports.import_map.insert(path.to_owned(), data.imports.clone());
            imports.re_export_map.insert(path.to_owned(), Vec::new());
        }
    }
    let import_map = Arc::new(
        ImportResolutionMap::build(&imports).with_fallback(parent.as_ref().map(|layer| layer.import_map.clone())),
    );
    let exports = loader.exports.into_read_only();
    let resolutions: FxHashMap<_, _> = loader.resolutions.into_iter().collect();
    let unresolved: FxHashMap<_, _> = loader.unresolved.into_iter().collect();
    let demands: FxHashSet<DemandKey> = loader.demands.into_iter().collect();
    let files: FxHashSet<PathKey> = files.into_iter().collect();
    // A walk that found the mapping the chain already holds adds nothing the resolver reads.
    let partial_exports: FxHashMap<_, _> = loader
        .partial_exports
        .into_iter()
        .filter(|(key, found)| {
            parent.as_ref().and_then(|layer| layer.chain().find_map(|layer| layer.partial_exports.get(key)))
                != Some(found)
        })
        .collect();
    let below = parent.as_ref().map(|layer| layer.totals).unwrap_or_default();
    let totals = Totals {
        files: below.files + files.len(),
        react_files: below.react_files + react_files.len(),
        exports: below.exports + exports.len(),
        partial_exports: below.partial_exports + partial_exports.len(),
        resolutions: below.resolutions + resolutions.len(),
        unresolved: below.unresolved + unresolved.len(),
    };
    DependencyLayer {
        depth: parent.as_ref().map_or(0, |layer| layer.depth + 1),
        parent: parent.clone(),
        totals,
        component_mappings,
        component_positions,
        named_types,
        import_map,
        exports,
        resolutions,
        files,
        react_files,
        augmenters: loader.augmenters.into_iter().collect(),
        partial_exports,
        // A demand that met a file it could not read is here too; the ledger marks it, so a load over this layer
        // follows it again.
        demands,
        unresolved,
        claimed: loader.claimed.into_iter().collect(),
        // Only files that were read: the next load reads a failed one again.
        parsed: loader.parsed.into_iter().filter_map(|(key, cell)| Some((key, cell.get()?.clone().ok()?))).collect(),
        panics: loader.panics.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner),
        ledger: loader.ledger.finish(),
        unread_walks: OnceLock::new(),
        #[cfg(test)]
        unread_walk_builds: std::sync::atomic::AtomicUsize::new(0),
        #[cfg(test)]
        reads: loader.reads.iter().map(|entry| *entry.value()).sum(),
    }
}

// ─── The persisted index ───────────────────────────────────────────

/// `(directory, specifier, module or reason)`.
type ResolutionRow = (String, String, Result<String, Unresolved>);
/// `(module, name, declaring (file, name))`.
type ExportRow = (String, String, Option<(String, String)>);
/// `(module, the files its re-exports lead to that augment React)`.
/// `(package directory, package.json modification time, size)`.
type PackageStampRow = (String, u128, u64);
type AugmenterRow = (String, Vec<String>, Vec<PackageStampRow>);

/// One package's answers: module resolutions from its directories and export lookups on its modules. Valid while its
/// `package.json` keeps its modification time and size.
#[derive(Serialize, Deserialize)]
struct PackageIndex {
    dir: String,
    mtime_ns: u128,
    size: u64,
    resolutions: Vec<ResolutionRow>,
    exports: Vec<ExportRow>,
    /// `(module, the files its re-exports lead to that augment React, the other packages that search read files from)`.
    /// A row is dropped on read when any of those packages' `package.json` changed, as well as this package's.
    #[serde(default)]
    augmenters: Vec<AugmenterRow>,
    /// The other packages the resolutions and exports above lead into. They hold only while those packages'
    /// `package.json` files do, and while the packages that are themselves in the index do.
    #[serde(default)]
    depends_on: Vec<PackageStampRow>,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    fingerprint: u64,
    packages: Vec<PackageIndex>,
}

/// Next to the parse cache, named by the resolution fingerprint so a changed configuration or build never reads
/// another's answers.
fn index_path(options: &PipelineOptions) -> (Utf8PathBuf, u64) {
    let dir = options.effective_cache_dir();
    let fingerprint = crate::resolver::resolution_fingerprint(options);
    (dir.join(format!("dependency-index-{fingerprint:016x}.msgpack")), fingerprint)
}

/// Whether `path` lies inside a `node_modules` directory, spelled with either separator (a Windows path has
/// backslashes once the verbatim prefix is gone).
pub(crate) use crate::types::options::in_node_modules;

fn package_stamp(dir: &str) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(Utf8Path::new(dir).join("package.json")).ok()?;
    let mtime = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_nanos();
    Some((mtime, meta.len()))
}

/// The directories of the index's packages whose answers still hold: the package's own `package.json` and those of the
/// packages it leads into are unchanged, and so are the answers of those that are themselves in the index (a re-export
/// chain is only as current as its newest link).
fn current_packages(packages: &[PackageIndex]) -> FxHashSet<&str> {
    let unchanged = |dir: &str, mtime_ns: u128, size: u64| package_stamp(dir) == Some((mtime_ns, size));
    let mut current: FxHashSet<&str> = packages
        .iter()
        .filter(|package| unchanged(&package.dir, package.mtime_ns, package.size))
        .filter(|package| package.depends_on.iter().all(|(dir, mtime_ns, size)| unchanged(dir, *mtime_ns, *size)))
        .map(|package| package.dir.as_str())
        .collect();
    let by_dir: FxHashMap<&str, &PackageIndex> =
        packages.iter().map(|package| (package.dir.as_str(), package)).collect();
    loop {
        let stale: Vec<&str> = current
            .iter()
            .copied()
            .filter(|dir| {
                by_dir[dir]
                    .depends_on
                    .iter()
                    .any(|(dep, ..)| by_dir.contains_key(dep.as_str()) && !current.contains(dep.as_str()))
            })
            .collect();
        if stale.is_empty() {
            return current;
        }
        for dir in stale {
            current.remove(dir);
        }
    }
}

/// Seed the loader from the index's packages whose answers still hold.
fn read_index((path, fingerprint): &(Utf8PathBuf, u64), loader: &Loader<'_>) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let Ok(index) = rmp_serde::from_slice::<IndexFile>(&bytes) else { return };
    if index.fingerprint != *fingerprint {
        return;
    }
    let current: FxHashSet<String> = current_packages(&index.packages).into_iter().map(str::to_owned).collect();
    for package in index.packages {
        if !current.contains(&package.dir) {
            continue;
        }
        for (dir, specifier, result) in package.resolutions {
            loader.resolutions.insert((dir.into(), specifier.into()), result.map(Utf8PathBuf::from));
        }
        for (module, name, found) in package.exports {
            let found = found.map(|(file, name)| (Utf8PathBuf::from(file), CompactString::from(name)));
            loader.exports.insert((PathKey::from(Utf8Path::new(&module)), CompactString::from(name)), found);
        }
        for (module, files, crossed) in package.augmenters {
            // The search read files of other packages too, so an upgrade of any of them voids the answer.
            if crossed.iter().any(|(dir, mtime_ns, size)| package_stamp(dir) != Some((*mtime_ns, *size))) {
                continue;
            }
            let key = PathKey::from(Utf8Path::new(&module));
            loader.augmenters.insert(key.clone(), files.into_iter().map(Utf8PathBuf::from).collect());
            loader.augmenter_crossed.insert(key, crossed);
        }
    }
}

/// Write every answer that came from inside a package, grouped by package. A write that fails leaves the next run
/// to walk the stubs again; the output does not depend on the index.
fn write_index((path, fingerprint): &(Utf8PathBuf, u64), loader: &Loader<'_>) {
    let mut packages: FxHashMap<Box<str>, PackageIndex> = FxHashMap::default();
    let mut entry = |dir: &Utf8Path| -> Option<Box<str>> {
        let package = loader.package_of(dir)?;
        if !packages.contains_key(&package) {
            let (mtime_ns, size) = package_stamp(&package)?;
            packages.insert(
                package.clone(),
                PackageIndex {
                    dir: package.to_string(),
                    mtime_ns,
                    size,
                    resolutions: Vec::new(),
                    exports: Vec::new(),
                    augmenters: Vec::new(),
                    depends_on: Vec::new(),
                },
            );
        }
        Some(package)
    };
    // The package a file belongs to, when it is another one than `from`.
    let other_package = |file: &Utf8Path, from: &str| -> Option<Box<str>> {
        loader.package_of(file.parent()?).filter(|package| &**package != from)
    };
    let mut depends_on: FxHashMap<Box<str>, FxHashSet<Box<str>>> = FxHashMap::default();
    // A package with an unresolved import, or one leading into such a package, has export lookups that walked past
    // it; installing the missing package changes them, and the index could not say so, so none of its exports are kept.
    let mut with_unresolved: FxHashSet<Box<str>> = FxHashSet::default();
    let mut resolutions: Vec<(Box<str>, ResolutionRow)> = Vec::new();
    for item in loader.resolutions.iter() {
        let ((dir, specifier), result) = (item.key(), item.value());
        let Some(package) = entry(Utf8Path::new(dir)) else { continue };
        match result {
            Ok(target) => {
                if let Some(other) = other_package(target, &package) {
                    depends_on.entry(package.clone()).or_default().insert(other);
                }
                resolutions.push((package, (dir.to_string(), specifier.to_string(), Ok(target.to_string()))));
            }
            Err(_) => {
                with_unresolved.insert(package);
            }
        }
    }
    let mut exports: Vec<(Box<str>, ExportRow)> = Vec::new();
    for item in loader.exports.iter() {
        let ((module, name), found) = (item.key(), item.value());
        let module: &str = std::borrow::Borrow::borrow(module);
        let Some(dir) = Utf8Path::new(module).parent() else { continue };
        if let Some(package) = entry(dir) {
            if let Some(other) = found.as_ref().and_then(|(file, _)| other_package(file, &package)) {
                depends_on.entry(package.clone()).or_default().insert(other);
            }
            let found = found.as_ref().map(|(file, name)| (file.to_string(), name.to_string()));
            exports.push((package, (module.to_owned(), name.to_string(), found)));
        }
    }
    // A package that leads into one with an unresolved import has export lookups that walked past it too.
    loop {
        let reaching: Vec<Box<str>> = depends_on
            .iter()
            .filter(|(package, others)| {
                !with_unresolved.contains(*package) && others.iter().any(|other| with_unresolved.contains(other))
            })
            .map(|(package, _)| package.clone())
            .collect();
        if reaching.is_empty() {
            break;
        }
        with_unresolved.extend(reaching);
    }
    exports.retain(|(package, _)| !with_unresolved.contains(package));
    let mut augmenters: Vec<(Box<str>, AugmenterRow)> = Vec::new();
    for item in loader.augmenters.iter() {
        let (module, files): (&str, _) = (std::borrow::Borrow::borrow(item.key()), item.value());
        let Some(dir) = Utf8Path::new(module).parent() else { continue };
        if let Some(package) = entry(dir) {
            let crossed = loader.augmenter_crossed.get(item.key()).map(|crossed| crossed.clone()).unwrap_or_default();
            augmenters.push((package, (module.to_owned(), files.iter().map(ToString::to_string).collect(), crossed)));
        }
    }
    for (package, others) in depends_on {
        // A dependency whose stamp cannot be read gets one that never matches, so the rows it supports are dropped.
        let stamps = others.iter().map(|other| {
            let (mtime_ns, size) = package_stamp(other).unwrap_or_default();
            (other.to_string(), mtime_ns, size)
        });
        if let Some(index) = packages.get_mut(&package) {
            index.depends_on.extend(stamps);
            index.depends_on.sort();
        }
    }
    for (package, row) in augmenters {
        if let Some(index) = packages.get_mut(&package) {
            index.augmenters.push(row);
        }
    }
    for (package, row) in resolutions {
        if let Some(index) = packages.get_mut(&package) {
            index.resolutions.push(row);
        }
    }
    for (package, row) in exports {
        if let Some(index) = packages.get_mut(&package) {
            index.exports.push(row);
        }
    }
    let mut packages: Vec<PackageIndex> = packages.into_values().collect();
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));
    for package in &mut packages {
        package.resolutions.sort();
        package.exports.sort();
        package.augmenters.sort();
    }
    let Ok(bytes) = rmp_serde::to_vec(&IndexFile { fingerprint: *fingerprint, packages }) else { return };
    // Answers the index does not keep (those of a package with an unresolved import) make every run learn again; the
    // file is left alone when it would not change.
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return;
    }
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let temp = temp_path(path);
    if std::fs::write(&temp, bytes).is_ok() && std::fs::rename(&temp, path).is_err() {
        let _removed = std::fs::remove_file(&temp);
    }
}

/// A name no other write of `path` shares, within this process or another: writers racing on one name would rename
/// each other's half-written files into place.
fn temp_path(path: &Utf8Path) -> Utf8PathBuf {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("tmp{}-{write}", std::process::id()))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod augmentation_tests;
