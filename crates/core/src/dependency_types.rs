//! The dependency type declarations project types reference, loaded by name before resolution
//! (`PipelineOptions::load_dependencies`, ADR 0010).
//!
//! A demand is a name a project file imports from a package and uses in a type. The loader resolves the module the
//! way the resolver will (`resolve_module_with`), finds the file declaring the name by following re-exports
//! (`export *`, `export { a as b } from`, `export * as ns from`, `import x; export { x }`), parses that file, then
//! follows the names its declaration references the same way. Only declaring files enter the layer; re-export stubs are
//! walked once and their answers kept, in the layer and in a per-package index on disk (ADR 0011), so later runs never
//! open them. The resolver consults the layer after the project's own data: its import map and named-type index fall
//! back to it, re-export answers come from it, and dependency components form a value index for `typeof` that is never
//! emitted.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use dashmap::{DashMap, DashSet};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::import_map::ImportResolutionMap;
use crate::named_type_index::NamedTypeIndex;
use crate::path_key::PathKey;
use crate::pipeline::PipelineOptions;
use crate::resolver::{ModuleResolution, ModuleResolvers};
use crate::types::*;

type FxDashMap<K, V> = DashMap<K, V, FxBuildHasher>;
type FxDashSet<K> = DashSet<K, FxBuildHasher>;

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

fn is_typescript(path: &Utf8Path) -> bool {
    matches!(path.extension(), Some("ts" | "tsx" | "mts" | "cts"))
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
        | CollectedType::Tuple(members)
        | CollectedType::TemplateLiteral(members) => members.iter().for_each(|member| type_refs(member, out)),
        CollectedType::Array(inner) | CollectedType::KeyOf(inner) => type_refs(inner, out),
        CollectedType::AtFile { inner, .. } => type_refs(inner, out),
        CollectedType::Object(fields) => fields.iter().for_each(|field| type_refs(&field.collected_type, out)),
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
        | CollectedType::Raw(_) => {}
    }
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

/// The names project file `file` imports from packages (bare specifiers) and uses in a type, sorted.
pub(crate) fn project_demands(file: &Utf8Path, data: &SourceData) -> Vec<Demand> {
    let mut refs = FxHashSet::default();
    referenced_names(data, &mut refs);
    let mut demands: Vec<Demand> = refs
        .iter()
        .filter_map(|reference| imported_demand(file, &data.imports, reference))
        .filter(|demand| !demand.specifier.starts_with('.') && !demand.specifier.starts_with('/'))
        .collect();
    demands.sort();
    demands.dedup();
    demands
}

// ─── The layer ────────────────────────────────────────────────────────────────

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
    /// Falls back to the parent's.
    pub(crate) named_types: Arc<NamedTypeIndex>,
    /// Falls back to the parent's.
    pub(crate) import_map: Arc<ImportResolutionMap>,
    /// `(module, name)` → the declaring `(file, name)`, or `None` when the module's types do not export it.
    exports: FxHashMap<(PathKey, CompactString), Option<(Utf8PathBuf, CompactString)>>,
    /// `(directory, specifier)` → module, for every resolution the loader made.
    resolutions: FxHashMap<(Box<str>, Box<str>), Result<Utf8PathBuf, Unresolved>>,
    files: FxHashSet<PathKey>,
    /// React's files whose declarations' imports were followed; their imports are in `import_map`.
    react_files: FxHashSet<PathKey>,
    demands: FxHashSet<DemandKey>,
    unresolved: FxHashMap<DemandKey, Unresolved>,
    /// The `(file, name)` declarations whose references were followed.
    claimed: FxHashSet<(PathKey, CompactString)>,
    /// Every parsed file's data (stubs included), so an extension reuses them.
    parsed: FxHashMap<PathKey, Option<Arc<SourceData>>>,
}

impl DependencyLayer {
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

    /// The declaring `(file, name)` of `name` exported from `module`, when the loader answered it.
    pub(crate) fn export_of(&self, module: &Utf8Path, name: &str) -> Option<&Option<(Utf8PathBuf, CompactString)>> {
        self.export_entry(&(PathKey::from(module), CompactString::from(name)))
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
        self.chain()
            .find_map(|layer| layer.component_mappings.iter().find(|m| m.file_path == file && m.component_name == name))
    }

    /// Why the loader found no declaration for `name` imported from `specifier` by a file in `dir`, if it didn't.
    pub(crate) fn unresolved_reason(&self, dir: &Utf8Path, specifier: &str, name: &str) -> Option<Unresolved> {
        let key = (Box::from(dir.as_str()), Box::from(specifier), CompactString::from(name));
        self.chain().find_map(|layer| layer.unresolved.get(&key)).copied()
    }

    /// Whether `other`, a layer this one was extended from, resolves every name the way this one does: extending only
    /// adds, so equal counts mean nothing was added.
    pub(crate) fn resolves_like(&self, other: &DependencyLayer) -> bool {
        let (a, b) = (self.totals, other.totals);
        a.files == b.files
            && a.react_files == b.react_files
            && a.exports == b.exports
            && a.resolutions == b.resolutions
            && a.unresolved == b.unresolved
    }

    /// Whether this layer has followed every one of `demands`.
    pub(crate) fn covers(&self, demands: &[Demand]) -> bool {
        demands.iter().all(|d| self.has_demand(&d.key()))
    }

    /// The demands of `demands` this layer has not followed.
    pub(crate) fn missing(&self, demands: &[Demand]) -> Vec<Demand> {
        demands.iter().filter(|d| !self.has_demand(&d.key())).cloned().collect()
    }
}

// ─── Loading ──────────────────────────────────────────────────────────────────

struct Loader<'a> {
    resolvers: &'a ModuleResolvers,
    project_dirs: &'a [Utf8PathBuf],
    /// The layer being extended, consulted before the maps below so an extension holds only its own additions.
    parent: Option<&'a DependencyLayer>,
    parsed: FxDashMap<PathKey, Arc<OnceLock<Option<Arc<SourceData>>>>>,
    resolutions: FxDashMap<(Box<str>, Box<str>), Result<Utf8PathBuf, Unresolved>>,
    exports: FxDashMap<(PathKey, CompactString), Option<(Utf8PathBuf, CompactString)>>,
    claimed: FxDashSet<(PathKey, CompactString)>,
    declaring: FxDashSet<PathKey>,
    react_files: FxDashSet<PathKey>,
    demands: FxDashSet<DemandKey>,
    unresolved: FxDashMap<DemandKey, Unresolved>,
    /// Package directory of each directory asked about, for grouping the index.
    packages: FxDashMap<Box<str>, Option<Box<str>>>,
    /// Something was resolved or answered that the index did not hold.
    learned: AtomicBool,
}

impl Loader<'_> {
    fn parse(&self, file: &Utf8Path) -> Option<Arc<SourceData>> {
        if let Some(found) = self.parent.and_then(|parent| parent.parsed_data(file.as_str())) {
            return found.clone();
        }
        let cell = self.parsed.entry(PathKey::from(file)).or_default().clone();
        cell.get_or_init(|| {
            let source = std::fs::read_to_string(file).ok()?;
            let mut data = crate::extractor::parse_file(file, &source);
            // Problems inside a package's types are not the project's; an unresolved demand is reported instead.
            data.diagnostics.clear();
            Some(Arc::new(data))
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
        self.learn(Utf8Path::new(dir));
        self.resolutions.insert(key, result.clone());
        result
    }

    fn in_project(&self, file: &Utf8Path) -> bool {
        !file.as_str().contains("/node_modules/") && self.project_dirs.iter().any(|dir| file.starts_with(dir))
    }

    /// React's own types stay with the resolver's React handling (builtins, curated HTML attributes).
    fn is_react(file: &Utf8Path) -> bool {
        let path = file.as_str();
        path.contains("/node_modules/@types/react/") || path.contains("/node_modules/react/")
    }

    /// The `(file, name)` declaring `name` as exported from `module`.
    fn find_export(&self, module: &Utf8Path, name: &str) -> Option<(Utf8PathBuf, CompactString)> {
        self.find_export_in(module, name, &mut FxHashSet::default(), &mut false)
    }

    /// `find_export` within one walk. A lookup that met a module already on the walk (a re-export cycle) is `cut`: its
    /// answer depends on where the walk entered the cycle, so it is returned but not kept, and the cut propagates to
    /// every lookup that relied on it. Kept answers are therefore the same in any order and on any thread.
    fn find_export_in(
        &self,
        module: &Utf8Path,
        name: &str,
        visited: &mut FxHashSet<(PathKey, CompactString)>,
        cut: &mut bool,
    ) -> Option<(Utf8PathBuf, CompactString)> {
        let key = (PathKey::from(module), CompactString::from(name));
        if let Some(found) = self.exports.get(&key) {
            return found.clone();
        }
        if let Some(found) = self.parent.and_then(|parent| parent.export_entry(&key)) {
            return found.clone();
        }
        if !visited.insert(key.clone()) {
            *cut = true;
            return None;
        }
        let mut own_cut = false;
        let found = self.find_export_uncached(module, name, visited, &mut own_cut);
        visited.remove(&key);
        if own_cut {
            *cut = true;
        } else {
            if let Some(dir) = module.parent() {
                self.learn(dir);
            }
            self.exports.insert(key, found.clone());
        }
        found
    }

    fn find_export_uncached(
        &self,
        module: &Utf8Path,
        name: &str,
        visited: &mut FxHashSet<(PathKey, CompactString)>,
        cut: &mut bool,
    ) -> Option<(Utf8PathBuf, CompactString)> {
        let data = self.parse(module)?;
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
                    let target = self.resolve(module, source_specifier).ok()?;
                    let next = match rest {
                        Some(rest) => format!("{source_name}.{rest}"),
                        None => source_name.clone(),
                    };
                    return self.find_export_in(&target, &next, visited, cut);
                }
                LexedExport::ReExportNamespace { namespace, source_specifier } if namespace == head => {
                    let target = self.resolve(module, source_specifier).ok()?;
                    return self.find_export_in(&target, rest?, visited, cut);
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
                let target = self.resolve(module, &demand.specifier).ok()?;
                return self.find_export_in(&target, &demand.name, visited, cut);
            }
        }
        if head == "default" {
            return None;
        }
        for export in &data.exports {
            if let LexedExport::ReExportAll { source_specifier, .. } = export {
                let Ok(target) = self.resolve(module, source_specifier) else { continue };
                if let Some(found) = self.find_export_in(&target, name, visited, cut) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn demand<'s>(&'s self, scope: &rayon::Scope<'s>, demand: Demand) {
        let key = demand.key();
        if self.parent.is_some_and(|parent| parent.has_demand(&key)) || !self.demands.insert(key.clone()) {
            return;
        }
        let module = match self.resolve(&demand.from, &demand.specifier) {
            Ok(module) => module,
            Err(reason) => {
                self.unresolved.insert(key, reason);
                return;
            }
        };
        if self.in_project(&module) {
            return;
        }
        if Self::is_react(&module) {
            // `@types/react` declares its exports inside `declare namespace React` (`export = React`).
            let qualified = CompactString::from(format!("React.{}", demand.name));
            if self.parse(&module).is_some_and(|data| declares(&data, &module, &qualified)) {
                self.load_declaration(scope, module, qualified);
            }
            return;
        }
        match self.find_export(&module, &demand.name) {
            Some((file, name)) => self.load_declaration(scope, file, name),
            None => {
                self.unresolved.insert(key, Unresolved::NotExported);
            }
        }
    }

    /// Mark `(file, name)` loaded and follow the names its declaration references: same-file ones in place, imported
    /// ones as new demands. React's own declarations stay with the resolver's React handling (builtins, curated HTML
    /// attributes), so React's files never enter the layer, but what they import does: `CSSProperties` extends
    /// csstype's `Properties`.
    fn load_declaration<'s>(&'s self, scope: &rayon::Scope<'s>, file: Utf8PathBuf, name: CompactString) {
        if self.in_project(&file) {
            return;
        }
        let react = Self::is_react(&file);
        let Some(data) = self.parse(&file) else { return };
        // `Primitive.div` is a member of the value `Primitive`, whose declared type has it.
        let mut stack: Vec<CompactString> =
            name.split_once('.').map(|(head, _)| CompactString::from(head)).into_iter().collect();
        stack.push(name);
        while let Some(name) = stack.pop() {
            let claim = (PathKey::from(file.as_path()), name.clone());
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
                stack.push(value_type);
            }
            let mut refs = FxHashSet::default();
            declaration_refs(&data, &file, &name, &mut refs);
            for reference in refs {
                let qualified = react.then(|| CompactString::from(format!("React.{reference}")));
                if declares(&data, &file, &reference) {
                    stack.push(reference);
                } else if declares(&data, &file, &format!("typeof {reference}")) {
                    // `typeof NODES` names a value that has only its declared type.
                    stack.push(CompactString::from(format!("typeof {reference}")));
                } else if let Some(qualified) = qualified.filter(|q| declares(&data, &file, q)) {
                    stack.push(qualified);
                } else if let Some(demand) = imported_demand(&file, &data.imports, &reference) {
                    scope.spawn(move |scope| self.demand(scope, demand));
                }
            }
        }
    }

    /// Note that an answer about `dir` is new, when `write_index` would keep it. A project file's own imports are
    /// resolved on every run and never reach the index, so they must not make a warm run rewrite it.
    fn learn(&self, dir: &Utf8Path) {
        if dir.as_str().contains("/node_modules/") && self.package_of(dir).is_some() {
            self.learned.store(true, Ordering::Relaxed);
        }
    }

    fn package_of(&self, dir: &Utf8Path) -> Option<Box<str>> {
        if let Some(found) = self.packages.get(dir.as_str()) {
            return found.clone();
        }
        let found = dir
            .ancestors()
            .find(|candidate| candidate.join("package.json").is_file())
            .filter(|candidate| candidate.as_str().contains("/node_modules/"))
            .map(|candidate| Box::from(candidate.as_str()));
        self.packages.insert(Box::from(dir.as_str()), found.clone());
        found
    }
}

/// Whether `file` declares `name` (an interface, type alias, enum, const array or component, possibly
/// namespace-qualified).
fn declares(data: &SourceData, file: &Utf8Path, name: &str) -> bool {
    let key = format!("{file}:{name}");
    data.interfaces.contains_key(&key)
        || data.type_aliases.contains_key(&key)
        || data.enums.contains_key(&key)
        || data.const_arrays.contains_key(&key)
        || data.component_mappings.iter().any(|m| m.component_name == name)
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
    for mapping in data.component_mappings.iter().filter(|m| m.component_name == name) {
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
    let resolvers = ModuleResolvers::new(options);
    let project_dirs: Vec<Utf8PathBuf> = options
        .src_dirs
        .iter()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .filter_map(|dir| Utf8PathBuf::from_path_buf(dir).ok())
        .collect();
    let parent: Option<Arc<DependencyLayer>> = previous.filter(|layer| layer.depth < MAX_CHAIN_DEPTH).cloned();
    let flatten = previous.filter(|_| parent.is_none());
    let loader = Loader {
        resolvers: &resolvers,
        project_dirs: &project_dirs,
        parent: parent.as_deref(),
        parsed: DashMap::default(),
        resolutions: DashMap::default(),
        exports: DashMap::default(),
        claimed: DashSet::default(),
        declaring: DashSet::default(),
        react_files: DashSet::default(),
        demands: DashSet::default(),
        unresolved: DashMap::default(),
        packages: DashMap::default(),
        learned: AtomicBool::new(false),
    };
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
                for (key, value) in &layer.exports {
                    loader.exports.insert(key.clone(), value.clone());
                }
                for (key, data) in &layer.parsed {
                    loader.parsed.insert(key.clone(), Arc::new(OnceLock::from(data.clone())));
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
            }
        }
        (Some(_), None) => {}
    }
    rayon::scope(|scope| {
        for demand in demands {
            let loader = &loader;
            scope.spawn(move |scope| {
                let label = format!("dependency demand {}#{}", demand.specifier, demand.name);
                let _contained = crate::panic_guard::contain_panic(&label, || loader.demand(scope, demand));
            });
        }
    });
    if let (None, Some(path)) = (previous, &index_path) {
        if loader.learned.load(Ordering::Relaxed) {
            write_index(path, &loader);
        }
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
    for file in &files {
        let path = Utf8Path::new(std::borrow::Borrow::<str>::borrow(file));
        if let Some(data) = loader.parse(path) {
            named.merge_file(&data);
            component_mappings.extend(data.component_mappings.iter().cloned());
            imports.import_map.insert(path.to_owned(), data.imports.clone());
            imports.re_export_map.insert(path.to_owned(), data.exports.clone());
        }
    }
    component_mappings.sort_by(|a, b| (&a.file_path, a.span_start).cmp(&(&b.file_path, b.span_start)));
    let named_types = Arc::new(named);
    // React's declarations reference what React imports (`CSS.Properties`), so those imports must resolve too; its
    // declarations stay out of `named_types`.
    let react_files: FxHashSet<PathKey> = loader.react_files.iter().map(|file| file.clone()).collect();
    for file in &react_files {
        let path = Utf8Path::new(std::borrow::Borrow::<str>::borrow(file));
        if let Some(data) = loader.parse(path) {
            imports.import_map.insert(path.to_owned(), data.imports.clone());
            imports.re_export_map.insert(path.to_owned(), Vec::new());
        }
    }
    let import_map = Arc::new(
        ImportResolutionMap::build(&imports).with_fallback(parent.as_ref().map(|layer| layer.import_map.clone())),
    );
    let exports: FxHashMap<_, _> = loader.exports.into_iter().collect();
    let resolutions: FxHashMap<_, _> = loader.resolutions.into_iter().collect();
    let unresolved: FxHashMap<_, _> = loader.unresolved.into_iter().collect();
    let files: FxHashSet<PathKey> = files.into_iter().collect();
    let below = parent.as_ref().map(|layer| layer.totals).unwrap_or_default();
    let totals = Totals {
        files: below.files + files.len(),
        react_files: below.react_files + react_files.len(),
        exports: below.exports + exports.len(),
        resolutions: below.resolutions + resolutions.len(),
        unresolved: below.unresolved + unresolved.len(),
    };
    DependencyLayer {
        depth: parent.as_ref().map_or(0, |layer| layer.depth + 1),
        parent: parent.clone(),
        totals,
        component_mappings,
        named_types,
        import_map,
        exports,
        resolutions,
        files,
        react_files,
        demands: loader.demands.into_iter().collect(),
        unresolved,
        claimed: loader.claimed.into_iter().collect(),
        parsed: loader.parsed.into_iter().map(|(key, cell)| (key, cell.get().cloned().flatten())).collect(),
    }
}

// ─── The persisted index (ADR 0011) ───────────────────────────────────────────

/// `(directory, specifier, module or reason)`.
type ResolutionRow = (String, String, Result<String, Unresolved>);
/// `(module, name, declaring (file, name))`.
type ExportRow = (String, String, Option<(String, String)>);

/// One package's answers: module resolutions from its directories and export lookups on its modules. Valid while its
/// `package.json` keeps its modification time and size.
#[derive(Serialize, Deserialize)]
struct PackageIndex {
    dir: String,
    mtime_ns: u128,
    size: u64,
    resolutions: Vec<ResolutionRow>,
    exports: Vec<ExportRow>,
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

fn package_stamp(dir: &str) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(Utf8Path::new(dir).join("package.json")).ok()?;
    let mtime = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_nanos();
    Some((mtime, meta.len()))
}

/// Seed the loader from the index's packages whose `package.json` is unchanged.
fn read_index((path, fingerprint): &(Utf8PathBuf, u64), loader: &Loader<'_>) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let Ok(index) = rmp_serde::from_slice::<IndexFile>(&bytes) else { return };
    if index.fingerprint != *fingerprint {
        return;
    }
    for package in index.packages {
        if package_stamp(&package.dir) != Some((package.mtime_ns, package.size)) {
            continue;
        }
        for (dir, specifier, result) in package.resolutions {
            loader.resolutions.insert((dir.into(), specifier.into()), result.map(Utf8PathBuf::from));
        }
        for (module, name, found) in package.exports {
            let found = found.map(|(file, name)| (Utf8PathBuf::from(file), CompactString::from(name)));
            loader.exports.insert((PathKey::from(Utf8Path::new(&module)), CompactString::from(name)), found);
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
                PackageIndex { dir: package.to_string(), mtime_ns, size, resolutions: Vec::new(), exports: Vec::new() },
            );
        }
        Some(package)
    };
    let mut resolutions: Vec<(Box<str>, ResolutionRow)> = Vec::new();
    for item in loader.resolutions.iter() {
        let ((dir, specifier), result) = (item.key(), item.value());
        if let Some(package) = entry(Utf8Path::new(dir)) {
            resolutions.push((package, (dir.to_string(), specifier.to_string(), result.clone().map(String::from))));
        }
    }
    let mut exports: Vec<(Box<str>, ExportRow)> = Vec::new();
    for item in loader.exports.iter() {
        let ((module, name), found) = (item.key(), item.value());
        let module: &str = std::borrow::Borrow::borrow(module);
        let Some(dir) = Utf8Path::new(module).parent() else { continue };
        if let Some(package) = entry(dir) {
            let found = found.as_ref().map(|(file, name)| (file.to_string(), name.to_string()));
            exports.push((package, (module.to_owned(), name.to_string(), found)));
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
    }
    let Ok(bytes) = rmp_serde::to_vec(&IndexFile { fingerprint: *fingerprint, packages }) else { return };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&temp, bytes).is_ok() && std::fs::rename(&temp, path).is_err() {
        let _removed = std::fs::remove_file(&temp);
    }
}

#[cfg(test)]
mod tests;
