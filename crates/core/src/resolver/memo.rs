//! Interface chains resolved once per run and shared by every component that inherits them.
//!
//! A non-generic interface's chain depends on more than the interface: on the component's cycle sets (an ancestor
//! already being resolved cuts a branch short), on the type parameters in scope, on the component's own parameter
//! defaults, and on the depth it is reached at. So a recording keeps every read the resolution made of those
//! (`StateReads`), and another component reuses it only when its own state gives the same answer to each one. Then
//! the resolution would take exactly the same steps, so the recording replays its effects instead: the chain itself,
//! with its props shared rather than copied, its diagnostics, the cycle-set and in-scope additions, whether it showed
//! an unevaluated alias by name, and the files watch mode must re-resolve the component for.
//!
//! Only the first read of a key says what the state held on entry: a later one may see what the resolution itself
//! added, which a replay adds again.
//!
//! A recording keeps each distinct diagnostic once. Resolution reports one each time it meets the cause, the output
//! merges identical diagnostics, and nothing in resolution reads them back, so only the first of each matters; keeping
//! every copy, again in each recording that encloses it, took gigabytes on Chakra with dependencies loaded.

use std::hash::Hash;
use std::sync::Arc;

use compact_str::CompactString;
use rustc_hash::FxHashSet;

use crate::types::{ComponentMapping, Diagnostic, ResolveState, StateReads};

use super::deps::Dependencies;
use super::ResolvedChain;

/// Recordings kept per interface and depth. A component whose state differs from all of them resolves the chain itself.
const RECORDINGS_PER_KEY: usize = 4;

/// The recordings of one run, by interface scoped key and depth. A watch revision builds a new context and so starts
/// empty.
#[derive(Default)]
pub(crate) struct InterfaceChains {
    recordings: dashmap::DashMap<(CompactString, u8), Vec<Arc<Recording>>, rustc_hash::FxBuildHasher>,
}

/// One resolution of an interface chain and everything needed to decide whether another state would repeat it.
struct Recording {
    chain: ResolvedChain,
    /// Chunks of diagnostics the resolution reported, its own distinct ones first; replays share them.
    shared: Vec<Arc<[Diagnostic]>>,
    /// The first read of each key, which is what the state held on entry, and the state's own additions.
    reads: StateReads,
    named_unevaluated: bool,
    deps: Dependencies,
    /// `ResolveState::expand_all` while recording: a chain with collapsed sources must not answer an expanding state.
    expand_all: bool,
    keys_only: bool,
}

/// Keep the first read of each key: what the state held on entry. A later read may see the resolution's own
/// additions, and an enclosing recording only needs the first one either.
fn first_reads<K: Clone + Eq + Hash, V>(reads: Vec<(K, V)>) -> Vec<(K, V)> {
    let mut seen: FxHashSet<K> = FxHashSet::default();
    reads.into_iter().filter(|(key, _)| seen.insert(key.clone())).collect()
}

/// Each distinct diagnostic once, in the order first reported.
fn distinct(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut seen: FxHashSet<&Diagnostic> = FxHashSet::default();
    diagnostics.iter().filter(|diagnostic| seen.insert(diagnostic)).cloned().collect()
}

impl Recording {
    /// Whether `state` and `mapping` give the same answer to every read the recorded resolution made.
    fn applies_to(&self, state: &ResolveState, mapping: &ComponentMapping) -> bool {
        let reads = &self.reads;
        self.expand_all == state.expand_all
            && self.keys_only == state.keys_only
            && reads.visited.iter().all(|(key, present)| state.visited.contains(&**key) == *present)
            && reads.named.iter().all(|(key, present)| state.named_in_progress.contains(&**key) == *present)
            && reads.in_scope.iter().all(|(name, present)| state.in_scope_type_params.contains(name) == *present)
            && reads.defaults.iter().all(|(name, found)| {
                mapping.param_defaults.get(name.as_str()).map(|d| (d.value.as_str(), d.computed))
                    == found.as_ref().map(|(value, computed)| (value.as_str(), *computed))
            })
    }

    /// Apply the recorded resolution's effects to `state` and return its chain.
    fn replay(&self, state: &mut ResolveState) -> ResolvedChain {
        if let Some(reads) = state.reads.as_mut() {
            reads.append(&self.reads, true, true);
        }
        state.visited.extend(self.reads.visited_added.iter().map(|key| CompactString::from(&**key)));
        state.in_scope_type_params.extend(self.reads.in_scope_added.iter().cloned());
        state.shared_diagnostics.extend(self.shared.iter().cloned());
        state.named_unevaluated |= self.named_unevaluated;
        super::deps::replay(&self.deps);
        self.chain.clone()
    }
}

impl InterfaceChains {
    /// The chain of the interface `key` reached at `depth`: replayed from a recording that applies to `state`, or
    /// resolved by `resolve` and recorded.
    pub(super) fn get_or_resolve(
        &self,
        key: &str,
        depth: u8,
        state: &mut ResolveState,
        mapping: &ComponentMapping,
        resolve: impl FnOnce(&mut ResolveState) -> ResolvedChain,
    ) -> ResolvedChain {
        // The modes never answer each other, so they must not compete for one key's recordings.
        let mode = depth | (u8::from(state.expand_all) << 6) | (u8::from(state.keys_only) << 7);
        let map_key = (CompactString::from(key), mode);
        let found = self
            .recordings
            .get(&map_key)
            .and_then(|recordings| recordings.iter().find(|r| r.applies_to(state, mapping)).cloned());
        if let Some(recording) = found {
            return recording.replay(state);
        }

        // Record: a fresh read log for this resolution, merged into the enclosing one afterwards.
        let enclosing = state.reads.replace(Box::default());
        let entry_unevaluated = std::mem::take(&mut state.named_unevaluated);
        let diagnostics_before = state.diagnostics.len();
        let shared_before = state.shared_diagnostics.len();
        let (chain, deps) = super::deps::capturing(|| resolve(state));
        let mut reads = *state.reads.take().unwrap_or_default();
        reads.visited = first_reads(reads.visited);
        reads.named = first_reads(reads.named);
        reads.in_scope = first_reads(reads.in_scope);
        reads.defaults = first_reads(reads.defaults);
        let named_unevaluated = state.named_unevaluated;
        state.named_unevaluated = entry_unevaluated || named_unevaluated;
        state.reads = enclosing;
        if let Some(enclosing) = state.reads.as_mut() {
            enclosing.append(&reads, true, true);
        }

        // This state shares the recording's chunks too, so the first component holds no copy of its own.
        let own: Arc<[Diagnostic]> = distinct(&state.diagnostics[diagnostics_before..]).into();
        state.diagnostics.truncate(diagnostics_before);
        let mut seen: FxHashSet<*const Diagnostic> = FxHashSet::default();
        let inner: Vec<Arc<[Diagnostic]>> = state.shared_diagnostics.drain(shared_before..).collect();
        let shared: Vec<Arc<[Diagnostic]>> = std::iter::once(own)
            .chain(inner)
            .filter(|chunk| !chunk.is_empty() && seen.insert(chunk.as_ptr()))
            .collect();
        state.shared_diagnostics.extend(shared.iter().cloned());
        let recording = Recording {
            chain: chain.clone(),
            shared,
            named_unevaluated,
            deps,
            reads,
            expand_all: state.expand_all,
            keys_only: state.keys_only,
        };
        let mut recordings = self.recordings.entry(map_key).or_default();
        let same = |r: &Recording| {
            r.expand_all == recording.expand_all
                && r.keys_only == recording.keys_only
                && r.reads.visited == recording.reads.visited
                && r.reads.named == recording.reads.named
                && r.reads.in_scope == recording.reads.in_scope
                && r.reads.defaults == recording.reads.defaults
        };
        // Components resolving the same chain concurrently record it several times; one copy is enough.
        let duplicate = recordings.iter().any(|r| same(r));
        if !duplicate && recordings.len() < RECORDINGS_PER_KEY {
            recordings.push(Arc::new(recording));
        }
        chain
    }
}

/// The number of recordings kept for each interface, over every depth, for tests.
#[cfg(test)]
pub(crate) fn recording_counts(chains: &InterfaceChains) -> rustc_hash::FxHashMap<String, usize> {
    let mut counts = rustc_hash::FxHashMap::default();
    for entry in chains.recordings.iter() {
        *counts.entry(entry.key().0.to_string()).or_default() += entry.value().len();
    }
    counts
}
