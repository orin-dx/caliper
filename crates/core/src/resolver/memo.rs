//! Interface chains and named types resolved once per run and shared by every component that reaches them.
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
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::ThreadId;
use std::time::Duration;

use compact_str::CompactString;
use rustc_hash::FxHashSet;

use crate::types::{ComponentMapping, Diagnostic, ResolveState, StateReads};

use super::deps::Dependencies;
use super::{ResolvedChain, MAX_NAMED_NESTING};
use crate::types::PropType;

/// Recordings kept per interface and depth. A component whose state differs from all of them resolves the chain itself.
const RECORDINGS_PER_KEY: usize = 4;

/// The recordings of one run, by key and depth. A watch revision builds a new context and so starts empty.
pub(crate) struct Memo<T> {
    recordings: dashmap::DashMap<(CompactString, u8), Vec<Arc<Recording<T>>>, rustc_hash::FxBuildHasher>,
    /// Recordings that read more than this are not kept: replaying one appends all of its reads to every enclosing
    /// recording, which costs more than resolving a small type again.
    max_reads: usize,
    /// The keys being resolved right now, and by which thread, so that threads that need the same one wait for it
    /// instead of each resolving it (Chakra's components all start from the same few thousand style props).
    flights: dashmap::DashMap<(CompactString, u8), Arc<Flight>, rustc_hash::FxBuildHasher>,
    /// Which thread each waiting thread waits for, to refuse a wait that would close a cycle.
    waiting: dashmap::DashMap<ThreadId, ThreadId, rustc_hash::FxBuildHasher>,
}

/// One thread's resolution of a key, which others wait on.
struct Flight {
    leader: ThreadId,
    done: Mutex<bool>,
    finished: Condvar,
}

/// The longest a thread waits for another's resolution. Mutually recursive types can make two threads wait on each
/// other past what the waits-for check sees; the waiter then resolves the key itself.
const MAX_WAIT: Duration = Duration::from_millis(250);

impl Flight {
    fn wait(&self) {
        let mut done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        while !*done {
            let (guard, timeout) = self.finished.wait_timeout(done, MAX_WAIT).unwrap_or_else(PoisonError::into_inner);
            done = guard;
            if timeout.timed_out() {
                break;
            }
        }
    }
}

/// Ends the leader's flight when the resolution returns, or unwinds.
struct FlightGuard<'a, T> {
    memo: &'a Memo<T>,
    key: (CompactString, u8),
    flight: Arc<Flight>,
}

impl<T> Drop for FlightGuard<'_, T> {
    fn drop(&mut self) {
        self.memo.flights.remove_if(&self.key, |_, flight| Arc::ptr_eq(flight, &self.flight));
        *self.flight.done.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.flight.finished.notify_all();
    }
}

impl<T> Default for Memo<T> {
    fn default() -> Self {
        Self::with_max_reads(usize::MAX)
    }
}

impl<T> Memo<T> {
    pub(crate) fn with_max_reads(max_reads: usize) -> Self {
        Self { recordings: Default::default(), max_reads, flights: Default::default(), waiting: Default::default() }
    }
}

/// The chains of interfaces, by scoped key.
pub(crate) type InterfaceChains = Memo<ResolvedChain>;

/// The types of named references, by consuming file, name and arguments.
pub(crate) type NamedResults = Memo<PropType>;

/// One resolution and everything needed to decide whether another state would repeat it.
struct Recording<T> {
    value: T,
    /// How many more named types the resolution had in progress at its deepest than when it started: a state already
    /// that deep would have hit the nesting limit where this one did not.
    reach: usize,
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

impl<T: Clone> Recording<T> {
    /// Whether `state` and `mapping` give the same answer to every read the recorded resolution made.
    fn applies_to(&self, state: &ResolveState, mapping: Option<&ComponentMapping>) -> bool {
        let reads = &self.reads;
        state.named_in_progress.len() + self.reach <= MAX_NAMED_NESTING
            && self.expand_all == state.expand_all
            && self.keys_only == state.keys_only
            && reads.visited.iter().all(|(key, present)| state.visited.contains(&**key) == *present)
            && reads.named.iter().all(|(key, present)| state.named_in_progress.contains(&**key) == *present)
            && reads.in_scope.iter().all(|(name, present)| state.in_scope_type_params.contains(name) == *present)
            && reads.defaults.iter().all(|(name, found)| {
                mapping.is_some_and(|mapping| {
                    mapping.param_defaults.get(name.as_str()).map(|d| (d.value.as_str(), d.computed))
                        == found.as_ref().map(|(value, computed)| (value.as_str(), *computed))
                })
            })
    }

    /// Apply the recorded resolution's effects to `state` and return its value.
    fn replay(&self, state: &mut ResolveState) -> T {
        state.named_high = state.named_high.max(state.named_in_progress.len() + self.reach);
        if let Some(reads) = state.reads.as_mut() {
            reads.append(&self.reads, true, true);
        }
        state.visited.extend(self.reads.visited_added.iter().map(|key| CompactString::from(&**key)));
        state.in_scope_type_params.extend(self.reads.in_scope_added.iter().cloned());
        state.shared_diagnostics.extend(self.shared.iter().cloned());
        state.named_unevaluated |= self.named_unevaluated;
        super::deps::replay(&self.deps);
        self.value.clone()
    }
}

impl<T: Clone> Memo<T> {
    /// The value for `key` reached at `depth`: replayed from a recording that applies to `state`, or resolved by
    /// `resolve` and recorded.
    pub(super) fn get_or_resolve(
        &self,
        key: &str,
        depth: u8,
        state: &mut ResolveState,
        mapping: Option<&ComponentMapping>,
        resolve: impl FnOnce(&mut ResolveState) -> T,
    ) -> T {
        // The modes never answer each other, so they must not compete for one key's recordings.
        let mode = depth | (u8::from(state.expand_all) << 6) | (u8::from(state.keys_only) << 7);
        let map_key = (CompactString::from(key), mode);
        let find = |state: &ResolveState| {
            self.recordings
                .get(&map_key)
                .and_then(|recordings| recordings.iter().find(|r| r.applies_to(state, mapping)).cloned())
        };
        if let Some(recording) = find(state) {
            return recording.replay(state);
        }

        // Another thread already resolving this key finishes it for everyone; wait for it, then look again.
        let me = std::thread::current().id();
        let _flight = match self.flights.entry(map_key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(entry) => {
                let flight = Arc::clone(entry.get());
                drop(entry);
                if flight.leader != me && !self.would_wait_in_a_cycle(me, flight.leader) {
                    self.waiting.insert(me, flight.leader);
                    flight.wait();
                    self.waiting.remove(&me);
                    if let Some(recording) = find(state) {
                        return recording.replay(state);
                    }
                }
                None
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let flight = Arc::new(Flight { leader: me, done: Mutex::new(false), finished: Condvar::new() });
                entry.insert(Arc::clone(&flight));
                Some(FlightGuard { memo: self, key: map_key.clone(), flight })
            }
        };

        // Record: a fresh read log for this resolution, merged into the enclosing one afterwards.
        let entry_nesting = state.named_in_progress.len();
        let enclosing_high = std::mem::replace(&mut state.named_high, entry_nesting);
        let enclosing = state.reads.replace(Box::default());
        let entry_unevaluated = std::mem::take(&mut state.named_unevaluated);
        let diagnostics_before = state.diagnostics.len();
        let shared_before = state.shared_diagnostics.len();
        let (value, deps) = super::deps::capturing(|| resolve(state));
        let high = state.named_high;
        state.named_high = enclosing_high.max(high);
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
        let read_count = reads.visited.len()
            + reads.named.len()
            + reads.in_scope.len()
            + reads.defaults.len()
            + reads.visited_added.len()
            + reads.in_scope_added.len();
        let recording = Recording {
            value: value.clone(),
            reach: high.saturating_sub(entry_nesting),
            shared,
            named_unevaluated,
            deps,
            reads,
            expand_all: state.expand_all,
            keys_only: state.keys_only,
        };
        let mut recordings = self.recordings.entry(map_key).or_default();
        let same = |r: &Recording<T>| {
            r.expand_all == recording.expand_all
                && r.keys_only == recording.keys_only
                && r.reads.visited == recording.reads.visited
                && r.reads.named == recording.reads.named
                && r.reads.in_scope == recording.reads.in_scope
                && r.reads.defaults == recording.reads.defaults
        };
        // Components resolving the same chain concurrently record it several times; one copy is enough.
        let duplicate = recordings.iter().any(|r| same(r));
        // A resolution that reached the nesting limit answers only a state as deep as the one that recorded it.
        if !duplicate
            && recordings.len() < RECORDINGS_PER_KEY
            && high <= MAX_NAMED_NESTING
            && read_count <= self.max_reads
        {
            recordings.push(Arc::new(recording));
        }
        value
    }
}

impl<T> Memo<T> {
    /// Whether waiting for `leader` would wait, through other threads' waits, for `me`.
    fn would_wait_in_a_cycle(&self, me: ThreadId, leader: ThreadId) -> bool {
        let mut thread = leader;
        for _ in 0..64 {
            let Some(next) = self.waiting.get(&thread).map(|next| *next) else { return false };
            if next == me {
                return true;
            }
            thread = next;
        }
        true
    }
}

/// The number of recordings kept for each interface, over every depth, for tests.
#[cfg(test)]
pub(crate) fn recording_counts<T>(chains: &Memo<T>) -> rustc_hash::FxHashMap<String, usize> {
    let mut counts = rustc_hash::FxHashMap::default();
    for entry in chains.recordings.iter() {
        *counts.entry(entry.key().0.to_string()).or_default() += entry.value().len();
    }
    counts
}
