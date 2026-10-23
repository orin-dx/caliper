//! Global resolution context — merged source data from all parsed files.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use super::collected::{
    CollectedInterface, CollectedTypeAlias, ComponentMapping, EnumEntry, EnumValue, ImportBinding, LexedExport,
    RawDefault, SourceData, TypeName,
};
use super::diagnostic::Diagnostic;

// ─── ResolveState ─────────────────────────────────────────────────────────────

/// Mutable resolution state threaded through all resolver functions.
/// Bundles the two fields that every recursive resolver function needs,
/// eliminating long argument signatures.
#[derive(Default)]
pub(crate) struct ResolveState {
    /// Cycle-detection set: "${file}:${type_name}" keys.
    pub(crate) visited: FxHashSet<CompactString>,
    /// `resolve_named`'s own cycle-detection set — deliberately separate from
    /// `visited` above. `visited` is chain.rs's, permanent for the rest of a
    /// given branch's resolution (with per-sibling-branch cloning to avoid
    /// diamond-inheritance false positives — see `resolve_interface_chain`).
    /// `resolve_named` is called repeatedly and independently for every
    /// unrelated field's type across a whole component, not once per
    /// interface, so a permanent set would falsely flag two different,
    /// non-cyclic fields that happen to share a type (`{ a: Shared; b: Shared
    /// }`) as circular. This set instead tracks only what's currently on the
    /// live call stack — inserted on entry to `resolve_named`, removed before
    /// it returns — the textbook DFS "on this path" set, which catches real
    /// self-reference while never flagging legitimate repeated reuse.
    pub(crate) named_in_progress: FxHashSet<CompactString>,
    /// Accumulated non-fatal issues.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Declared generic type parameter names (`TData`, `T`, `U`, ...) seen so far
    /// while resolving this component — accumulated as interfaces/aliases with
    /// their own type parameters are entered, never removed. A bare reference to
    /// one of these names is an expected, unexpandable generic placeholder, not
    /// a broken/missing type — see `resolver::named::resolve_named`.
    pub(crate) in_scope_type_params: FxHashSet<CompactString>,
    /// Set when an alias is shown by its name because its expansion holds a type the resolver can't evaluate, so an
    /// alias whose expansion contains that one is shown by name too — see `resolver::named::alias_display`.
    pub(crate) named_unevaluated: bool,
    /// What resolution read from this state while an interface chain is being recorded for reuse, `None` otherwise —
    /// see `resolver::memo`. Every read of the three sets above goes through the methods below so none is missed.
    pub(crate) reads: Option<Box<StateReads>>,
}

/// The reads a recorded resolution made, so another component's state can be checked to give the same answers.
#[derive(Debug, Default, Clone)]
pub(crate) struct StateReads {
    /// Cycle-set keys looked up, each with whether it was already there. The keys are long and every enclosing
    /// recording keeps them again, so they are shared.
    pub(crate) visited: Vec<(Arc<str>, bool)>,
    /// `named_in_progress` keys looked up, each with whether it was already there.
    pub(crate) named: Vec<(Arc<str>, bool)>,
    /// In-scope type parameter names looked up, each with whether it was in scope.
    pub(crate) in_scope: Vec<(CompactString, bool)>,
    /// Prop names whose component default was looked up, with the default found.
    pub(crate) defaults: Vec<(CompactString, Option<(std::string::String, bool)>)>,
    /// Cycle-set keys this state itself added (a branch's own additions are dropped with the branch).
    pub(crate) visited_added: Vec<Arc<str>>,
    /// In-scope type parameters this state itself added.
    pub(crate) in_scope_added: Vec<CompactString>,
}

impl StateReads {
    /// Take `other`'s lookups. `with_effects` also takes its additions, for reads made on this same state; a branch's
    /// additions die with it. `with_cycle_sets` is false for a detached state, whose cycle sets started empty instead of
    /// from this one, so its lookups say nothing about this state.
    pub(crate) fn append(&mut self, other: &StateReads, with_cycle_sets: bool, with_effects: bool) {
        if with_cycle_sets {
            self.visited.extend(other.visited.iter().cloned());
            self.named.extend(other.named.iter().cloned());
        }
        self.in_scope.extend(other.in_scope.iter().cloned());
        self.defaults.extend(other.defaults.iter().cloned());
        if with_effects {
            self.visited_added.extend(other.visited_added.iter().cloned());
            self.in_scope_added.extend(other.in_scope_added.iter().cloned());
        }
    }
}

impl ResolveState {
    /// A state for an independent sibling branch (an extends entry, a union member): same cycle sets and in-scope
    /// parameters, its own diagnostics. Merge it back with `absorb`.
    pub(crate) fn branch(&self) -> ResolveState {
        ResolveState {
            visited: self.visited.clone(),
            named_in_progress: self.named_in_progress.clone(),
            diagnostics: Vec::new(),
            in_scope_type_params: self.in_scope_type_params.clone(),
            named_unevaluated: false,
            reads: self.reads.as_ref().map(|_| Box::default()),
        }
    }

    /// A state that shares only the in-scope parameters: for a separate question asked mid-walk, such as the keys
    /// of a type, whose cycle sets must not be the caller's. Merge it back with `absorb_detached`.
    pub(crate) fn detached(&self) -> ResolveState {
        ResolveState {
            in_scope_type_params: self.in_scope_type_params.clone(),
            reads: self.reads.as_ref().map(|_| Box::default()),
            ..Self::default()
        }
    }

    /// Take a branch's diagnostics and whether it named an unevaluated alias. Its cycle sets and in-scope additions
    /// stay with the branch.
    pub(crate) fn absorb(&mut self, branch: ResolveState) {
        self.absorb_reads(&branch, true);
        self.diagnostics.extend(branch.diagnostics);
        self.named_unevaluated |= branch.named_unevaluated;
    }

    /// `absorb` for a `detached` state.
    pub(crate) fn absorb_detached(&mut self, detached: ResolveState) {
        self.absorb_reads(&detached, false);
        self.diagnostics.extend(detached.diagnostics);
        self.named_unevaluated |= detached.named_unevaluated;
    }

    fn absorb_reads(&mut self, other: &ResolveState, same_cycle_sets: bool) {
        if let (Some(reads), Some(theirs)) = (self.reads.as_mut(), other.reads.as_ref()) {
            reads.append(theirs, same_cycle_sets, false);
        }
    }

    /// Enter `key` into the cycle set; false if it was already there.
    pub(crate) fn visit(&mut self, key: CompactString) -> bool {
        let Some(reads) = self.reads.as_mut() else { return self.visited.insert(key) };
        let shared: Arc<str> = Arc::from(key.as_str());
        let inserted = self.visited.insert(key);
        if inserted {
            reads.visited_added.push(Arc::clone(&shared));
        }
        reads.visited.push((shared, !inserted));
        inserted
    }

    /// Enter `key` into `named_in_progress`; false if it was already there.
    pub(crate) fn enter_named(&mut self, key: CompactString) -> bool {
        let Some(reads) = self.reads.as_mut() else { return self.named_in_progress.insert(key) };
        let shared: Arc<str> = Arc::from(key.as_str());
        let inserted = self.named_in_progress.insert(key);
        reads.named.push((shared, !inserted));
        inserted
    }

    /// Whether `name` is a type parameter in scope.
    pub(crate) fn has_type_param(&mut self, name: &str) -> bool {
        let found = self.in_scope_type_params.contains(name);
        if let Some(reads) = self.reads.as_mut() {
            reads.in_scope.push((CompactString::from(name), found));
        }
        found
    }

    /// Bring type parameters into scope.
    pub(crate) fn add_type_params<'p>(&mut self, names: impl IntoIterator<Item = &'p CompactString>) {
        for name in names {
            if self.in_scope_type_params.insert(name.clone()) {
                if let Some(reads) = self.reads.as_mut() {
                    reads.in_scope_added.push(name.clone());
                }
            }
        }
    }

    /// The component's default for prop `name`.
    pub(crate) fn param_default<'m>(&mut self, mapping: &'m ComponentMapping, name: &str) -> Option<&'m RawDefault> {
        let found = mapping.param_defaults.get(name);
        if let Some(reads) = self.reads.as_mut() {
            reads.defaults.push((CompactString::from(name), found.map(|d| (d.value.clone(), d.computed))));
        }
        found
    }
}

// ─── GlobalSourceData ─────────────────────────────────────────────────────────

/// Merged source data from all files — the shared resolution context.
/// Built once, then read by all parallel resolution workers.
/// Uses Arc in pipeline — clone is cheap.
#[derive(Debug, Default, Clone)]
pub struct GlobalSourceData {
    /// All interfaces across all files.
    /// Key: "${absolute_file_path}:${name}" — always scoped, never bare
    pub interfaces: FxHashMap<String, CollectedInterface>,

    /// All type aliases across all files.
    /// Key: "${absolute_file_path}:${name}"
    pub type_aliases: FxHashMap<String, CollectedTypeAlias>,

    /// Declared type parameter names for generic type alias declarations — see
    /// `SourceData::type_alias_params`. Key: "${absolute_file_path}:${name}"
    pub type_alias_params: FxHashMap<String, Vec<TypeName>>,

    /// Declared type parameter names for generic interface declarations — see
    /// `SourceData::interface_type_params`. Key: "${absolute_file_path}:${name}"
    pub interface_type_params: FxHashMap<String, Vec<TypeName>>,

    /// All enum-like definitions across all files.
    /// Key: "${absolute_file_path}:${name}"
    pub enums: FxHashMap<String, Vec<EnumEntry>>,

    /// All `const X = [...] as const` array literals across all files — see
    /// `SourceData::const_arrays`. Key: "${absolute_file_path}:${name}"
    pub const_arrays: FxHashMap<String, Vec<EnumValue>>,

    /// Import resolution map: file → [ImportBinding]
    pub import_map: FxHashMap<Utf8PathBuf, Vec<ImportBinding>>,

    /// Re-export map: file → [LexedExport]
    pub re_export_map: FxHashMap<Utf8PathBuf, Vec<LexedExport>>,

    /// All component mappings discovered
    pub component_mappings: Vec<ComponentMapping>,
}

impl GlobalSourceData {
    /// Merge a single file's SourceData into the global data.
    pub fn merge(&mut self, file_path: &Utf8Path, data: SourceData) {
        for (key, iface) in data.interfaces {
            match self.interfaces.entry(key) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    // Declaration merging: combine props and extends
                    let existing = e.get_mut();
                    existing.props.extend(iface.props);
                    existing.extends.extend(iface.extends);
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(iface);
                }
            }
        }
        self.type_aliases.extend(data.type_aliases);
        self.type_alias_params.extend(data.type_alias_params);
        self.interface_type_params.extend(data.interface_type_params);
        self.enums.extend(data.enums);
        self.const_arrays.extend(data.const_arrays);
        self.import_map.insert(file_path.to_owned(), data.imports);
        self.re_export_map.insert(file_path.to_owned(), data.exports);
        self.component_mappings.extend(data.component_mappings);
    }

    /// Remove all entries contributed by `file_path`. Called before re-merging an updated file.
    pub fn remove_file(&mut self, file_path: &Utf8Path) {
        let prefix = format!("{}:", file_path);
        self.interfaces.retain(|k, _| !k.starts_with(&prefix));
        self.type_aliases.retain(|k, _| !k.starts_with(&prefix));
        self.type_alias_params.retain(|k, _| !k.starts_with(&prefix));
        self.interface_type_params.retain(|k, _| !k.starts_with(&prefix));
        self.enums.retain(|k, _| !k.starts_with(&prefix));
        self.const_arrays.retain(|k, _| !k.starts_with(&prefix));
        self.import_map.remove(file_path);
        self.re_export_map.remove(file_path);
        self.component_mappings.retain(|m| m.file_path != file_path);
    }
}
