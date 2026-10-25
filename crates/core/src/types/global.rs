//! Global resolution context — merged source data from all parsed files.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use super::collected::{
    CollectedInterface, CollectedType, CollectedTypeAlias, ComponentMapping, EnumEntry, EnumValue, FunctionResult,
    ImportBinding, LexedExport, RawDefault, SourceData, TypeName,
};
use super::diagnostic::Diagnostic;

// ─── ResolveState ─────────────────────────────────────────────────────────────

/// The id of a cycle-set key, the same for every state in the process. The keys are `file:name<arguments>` strings tens
/// of bytes long; the sets and the read logs of the interface-chain recordings (`resolver::memo`) hold ids instead,
/// so copying a set or a log copies numbers, and no reference count is shared between the threads resolving components.
/// Ids are never reused, so equal ids are equal keys.
fn key_id(key: &str) -> u32 {
    static IDS: std::sync::OnceLock<dashmap::DashMap<Box<str>, u32, rustc_hash::FxBuildHasher>> =
        std::sync::OnceLock::new();
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let ids = IDS.get_or_init(Default::default);
    if let Some(id) = ids.get(key) {
        return *id;
    }
    *ids.entry(key.into()).or_insert_with(|| NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Mutable resolution state threaded through all resolver functions.
/// Bundles the two fields that every recursive resolver function needs,
/// eliminating long argument signatures.
#[derive(Default)]
pub(crate) struct ResolveState {
    /// Cycle-detection set: ids (`key_id`) of "${file}:${type_name}" keys.
    pub(crate) visited: FxHashSet<u32>,
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
    pub(crate) named_in_progress: FxHashSet<u32>,
    /// The most names `named_in_progress` has held since a recording began (`resolver::memo`), to bound how deep a
    /// replayed resolution would go from a deeper state.
    pub(crate) named_high: usize,
    /// The deepest resolution depth checked against the limit since a recording began (`resolver::past_max_depth`), to
    /// bound how deep a replayed resolution would go from a different starting depth.
    pub(crate) depth_high: u8,
    /// Accumulated non-fatal issues.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Diagnostics of replayed or recorded interface chains, shared with the recording instead of copied.
    pub(crate) shared_diagnostics: Vec<Arc<[Diagnostic]>>,
    /// Declared generic type parameter names (`TData`, `T`, `U`, ...) seen so far
    /// while resolving this component — accumulated as interfaces/aliases with
    /// their own type parameters are entered, never removed. A bare reference to
    /// one of these names is an expected, unexpandable generic placeholder, not
    /// a broken/missing type — see `resolver::named::resolve_named`.
    pub(crate) in_scope_type_params: FxHashSet<CompactString>,
    /// Set when an alias is shown by its name because its expansion holds a type the resolver can't evaluate, so an
    /// alias whose expansion contains that one is shown by name too — see `resolver::named::alias_display`.
    pub(crate) named_unevaluated: bool,
    /// Set when `visit` or `enter_named` found its key already in a cycle set: the resolution met a cycle (or a key an
    /// enclosing resolution holds). A resolution that ends with it unset walked an acyclic part of the type graph, which
    /// is what lets `resolver::memo` reuse its recording without checking the cycle sets.
    pub(crate) cut_seen: bool,
    /// Expand every source, collapsing none (ADR 0012): set while resolving `Pick`'s base or a `keyof` operand, whose
    /// props are selected or counted by name. Interface chains are not shared while it is set.
    pub(crate) expand_all: bool,
    /// Only prop names are wanted (a `keyof` operand): members' types are not resolved. Interface chains recorded with
    /// it set answer only states with it set.
    pub(crate) keys_only: bool,
    /// What resolution read from this state while an interface chain is being recorded for reuse, `None` otherwise —
    /// see `resolver::memo`. Every read of the three sets above goes through the methods below so none is missed.
    pub(crate) reads: Option<Box<StateReads>>,
}

/// The reads a recorded resolution made, so another component's state can be checked to give the same answers.
#[derive(Debug, Default, Clone)]
pub(crate) struct StateReads {
    /// Cycle-set keys looked up, each with whether it was already there. Ids, not the long key strings: every enclosing
    /// recording keeps them again, and copying a number needs no reference count shared between threads.
    pub(crate) visited: Vec<(u32, bool)>,
    /// `named_in_progress` keys looked up, each with whether it was already there.
    pub(crate) named: Vec<(u32, bool)>,
    /// In-scope type parameter names looked up, each with whether it was in scope.
    pub(crate) in_scope: Vec<(CompactString, bool)>,
    /// Prop names whose component default was looked up, with the default found.
    pub(crate) defaults: Vec<(CompactString, Option<(std::string::String, bool)>)>,
    /// Cycle-set keys this state itself added (a branch's own additions are dropped with the branch).
    pub(crate) visited_added: Vec<u32>,
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

    /// `append` for an `other` nobody reads again: its entries move instead of being cloned (a refcount bump each) and
    /// dropped (another).
    pub(crate) fn append_owned(&mut self, mut other: StateReads, with_cycle_sets: bool, with_effects: bool) {
        if with_cycle_sets {
            move_all(&mut self.visited, &mut other.visited);
            move_all(&mut self.named, &mut other.named);
        }
        move_all(&mut self.in_scope, &mut other.in_scope);
        move_all(&mut self.defaults, &mut other.defaults);
        if with_effects {
            move_all(&mut self.visited_added, &mut other.visited_added);
            move_all(&mut self.in_scope_added, &mut other.in_scope_added);
        }
    }
}

fn move_all<T>(into: &mut Vec<T>, from: &mut Vec<T>) {
    if into.is_empty() {
        std::mem::swap(into, from);
    } else {
        into.append(from);
    }
}

/// A component's resolution diagnostics: its own, and chunks shared with the interface-chain recordings it replayed.
#[derive(Clone, Default, Debug)]
pub(crate) struct ResolvedDiagnostics {
    pub(crate) own: Vec<Diagnostic>,
    pub(crate) shared: Vec<Arc<[Diagnostic]>>,
}

impl ResolvedDiagnostics {
    /// Every diagnostic, copied out.
    #[cfg(test)]
    pub(crate) fn flatten(&self) -> Vec<Diagnostic> {
        self.own.iter().chain(self.shared.iter().flat_map(|chunk| chunk.iter())).cloned().collect()
    }
}

impl ResolveState {
    /// A state for an independent sibling branch (an extends entry, a union member): same cycle sets and in-scope
    /// parameters, its own diagnostics. Merge it back with `absorb`.
    pub(crate) fn branch(&self) -> ResolveState {
        ResolveState {
            visited: self.visited.clone(),
            named_in_progress: self.named_in_progress.clone(),
            named_high: self.named_in_progress.len(),
            depth_high: 0,
            diagnostics: Vec::new(),
            shared_diagnostics: Vec::new(),
            in_scope_type_params: self.in_scope_type_params.clone(),
            named_unevaluated: false,
            cut_seen: false,
            expand_all: self.expand_all,
            keys_only: self.keys_only,
            reads: self.reads.as_ref().map(|_| Box::default()),
        }
    }

    /// A state that shares only the in-scope parameters: for a separate question asked mid-walk, such as the keys
    /// of a type, whose cycle sets must not be the caller's. Merge it back with `absorb_detached`.
    pub(crate) fn detached(&self) -> ResolveState {
        ResolveState {
            in_scope_type_params: self.in_scope_type_params.clone(),
            expand_all: self.expand_all,
            keys_only: self.keys_only,
            reads: self.reads.as_ref().map(|_| Box::default()),
            ..Self::default()
        }
    }

    /// Take a branch's diagnostics and whether it named an unevaluated alias. Its cycle sets and in-scope additions
    /// stay with the branch.
    pub(crate) fn absorb(&mut self, mut branch: ResolveState) {
        self.absorb_reads(branch.reads.take(), true);
        self.named_high = self.named_high.max(branch.named_high);
        self.depth_high = self.depth_high.max(branch.depth_high);
        self.diagnostics.extend(branch.diagnostics);
        self.shared_diagnostics.extend(branch.shared_diagnostics);
        self.named_unevaluated |= branch.named_unevaluated;
        self.cut_seen |= branch.cut_seen;
    }

    /// `absorb` for a `detached` state.
    pub(crate) fn absorb_detached(&mut self, mut detached: ResolveState) {
        self.absorb_reads(detached.reads.take(), false);
        self.named_high = self.named_high.max(self.named_in_progress.len() + detached.named_high);
        self.depth_high = self.depth_high.max(detached.depth_high);
        self.diagnostics.extend(detached.diagnostics);
        self.shared_diagnostics.extend(detached.shared_diagnostics);
        self.named_unevaluated |= detached.named_unevaluated;
        self.cut_seen |= detached.cut_seen;
    }

    fn absorb_reads(&mut self, theirs: Option<Box<StateReads>>, same_cycle_sets: bool) {
        if let (Some(reads), Some(theirs)) = (self.reads.as_mut(), theirs) {
            reads.append_owned(*theirs, same_cycle_sets, false);
        }
    }

    /// Enter `key` into the cycle set; false if it was already there.
    pub(crate) fn visit(&mut self, key: CompactString) -> bool {
        let id = key_id(&key);
        let inserted = self.visited.insert(id);
        self.cut_seen |= !inserted;
        if let Some(reads) = self.reads.as_mut() {
            if inserted {
                reads.visited_added.push(id);
            }
            reads.visited.push((id, !inserted));
        }
        inserted
    }

    /// Enter `key` into `named_in_progress`; false if it was already there.
    pub(crate) fn enter_named(&mut self, key: &str) -> bool {
        let id = key_id(key);
        let inserted = self.named_in_progress.insert(id);
        self.cut_seen |= !inserted;
        if let Some(reads) = self.reads.as_mut() {
            reads.named.push((id, !inserted));
        }
        self.named_high = self.named_high.max(self.named_in_progress.len());
        inserted
    }

    /// Take `key` out of `named_in_progress`: its resolution has returned.
    pub(crate) fn leave_named(&mut self, key: &str) {
        self.named_in_progress.remove(&key_id(key));
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

    /// Generic functions' result types across all files — see `SourceData::function_results`.
    /// Key: "${absolute_file_path}:${name}"
    pub function_results: FxHashMap<String, FunctionResult>,

    /// Type parameter defaults across all files — see `SourceData::type_param_defaults`.
    /// Key: "${absolute_file_path}:${name}"
    pub type_param_defaults: FxHashMap<String, Vec<Option<CollectedType>>>,
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
        self.function_results.extend(data.function_results);
        self.type_param_defaults.extend(data.type_param_defaults);
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
        self.function_results.retain(|k, _| !k.starts_with(&prefix));
        self.type_param_defaults.retain(|k, _| !k.starts_with(&prefix));
        self.import_map.remove(file_path);
        self.re_export_map.remove(file_path);
        self.component_mappings.retain(|m| m.file_path != file_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reads(tag: &str) -> StateReads {
        StateReads {
            visited: vec![(key_id(&format!("{tag}-visited")), true)],
            named: vec![(key_id(&format!("{tag}-named")), false)],
            in_scope: vec![(CompactString::from(format!("{tag}-param")), true)],
            defaults: vec![(CompactString::from(format!("{tag}-prop")), Some(("1".to_owned(), false)))],
            visited_added: vec![key_id(&format!("{tag}-added"))],
            in_scope_added: vec![CompactString::from(format!("{tag}-param-added"))],
        }
    }

    #[test]
    fn append_owned_takes_what_append_takes_for_every_flag_combination() {
        for with_cycle_sets in [false, true] {
            for with_effects in [false, true] {
                for start_empty in [false, true] {
                    let start = if start_empty { StateReads::default() } else { reads("mine") };
                    let theirs = reads("theirs");
                    let mut cloned = start.clone();
                    cloned.append(&theirs, with_cycle_sets, with_effects);
                    let mut moved = start;
                    moved.append_owned(theirs, with_cycle_sets, with_effects);
                    assert_eq!(
                        format!("{moved:?}"),
                        format!("{cloned:?}"),
                        "with_cycle_sets={with_cycle_sets} with_effects={with_effects} start_empty={start_empty}"
                    );
                }
            }
        }
    }

    #[test]
    fn absorbing_a_branch_moves_its_reads_into_a_recording_parent() {
        let mut parent = ResolveState { reads: Some(Box::default()), ..ResolveState::default() };
        let mut branch = parent.branch();
        assert!(branch.visit(CompactString::from("a.ts:Props")));
        assert!(!branch.has_type_param("T"));
        parent.absorb(branch);
        let reads = parent.reads.as_ref().map(|reads| format!("{reads:?}"));
        let id = key_id("a.ts:Props");
        assert_eq!(
            reads.as_deref(),
            Some(
                format!(
                    "StateReads {{ visited: [({id}, false)], named: [], in_scope: [(\"T\", false)], defaults: [], \
                     visited_added: [], in_scope_added: [] }}"
                )
                .as_str()
            )
        );
    }

    #[test]
    fn a_key_has_one_id_and_two_keys_have_two() {
        let a = key_id("a.ts:A<1>");
        assert_eq!(a, key_id("a.ts:A<1>"));
        assert_ne!(a, key_id("a.ts:A<2>"));
    }
}
