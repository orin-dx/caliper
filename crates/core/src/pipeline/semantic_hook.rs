//! Asking a checker for the component a session was just asked about (ADR 0022).
//!
//! The structural answer is always computed first and is a complete answer on its own. With a [`SemanticProvider`]
//! attached, [`SemanticHook::refine`] asks the checker for exactly that component, waits a bounded time, and merges
//! its props over the structural ones. A checker that is slower than the budget leaves the structural answer in
//! place, and its late answer is kept for the next request of the same, unedited component.
//!
//! There is one request in flight per component and version of its file: a caller that finds one waits on it instead
//! of asking again, and a caller someone is waiting for raises a background request it joins to interactive, so a
//! story asked while its own prefetch runs is answered when the prefetch is.
//!
//! An answer describes the component's props as the checker read them, which depends on every file the component
//! imports, not only its own. So the session drops every answer when any source file changes
//! ([`SemanticHook::invalidate`]), and an answer that was still on its way then is neither kept nor joined. The same
//! call tells the checker which files changed, since a warm one answers from what it read before (ADR 0026 S6).

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHasher};

use super::ComponentAnswer;
use crate::pipeline::HtmlAttributeMode;
use crate::react_types::{checker_prop_used, react_props_wanted};
use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use crate::types::semantic::{
    Delivery, MergedProps, Priority, SemanticComponent, SemanticFallback, SemanticOutcome, SemanticProp,
    SemanticProvider, SemanticTarget, SharedPriority, SourceChange, merge_semantic_props, unread_diagnostic,
};

/// What one request of the checker made of a component: the component it found, if any, and what the checker said
/// about the request itself (an edit it was not told about, why a component kept its structural result). Both are the
/// answer, so a late answer, a joined one and every request the cache serves it to carry both.
#[derive(Clone, Default)]
struct Answered {
    component: Option<Arc<SemanticComponent>>,
    diagnostics: Vec<Diagnostic>,
}

/// What became of one request to the checker, for every caller that waited on it.
#[derive(Clone)]
enum Settled {
    /// The checker answered: what is kept for the component, and what an unavailable checker says once per session.
    Answered { answered: Answered, once: Vec<Diagnostic> },
    /// The checker stopped without answering.
    Stopped,
}

/// A request in flight. Every caller for the same component and version of its file waits on it instead of asking the
/// checker again.
struct Pending {
    /// What the checker answers it at; a caller someone is waiting for raises it on joining.
    priority: SharedPriority,
    settled: Mutex<Option<Settled>>,
    ready: Condvar,
}

impl Pending {
    fn new(priority: Priority) -> Self {
        Self { priority: SharedPriority::new(priority), settled: Mutex::new(None), ready: Condvar::new() }
    }

    fn settle(&self, settled: Settled) {
        *self.settled.lock().unwrap_or_else(PoisonError::into_inner) = Some(settled);
        self.ready.notify_all();
    }

    /// What became of the request, if that is known within `wait`.
    fn wait(&self, wait: Duration) -> Option<Settled> {
        let settled = self.settled.lock().unwrap_or_else(PoisonError::into_inner);
        let (settled, _) = self
            .ready
            .wait_timeout_while(settled, wait, |settled| settled.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        settled.clone()
    }
}

/// What the checker said about one component at one version of its file, what it is still being asked, and the
/// generation they belong to.
#[derive(Default)]
struct AnswerCache {
    /// Raised by every [`SemanticHook::invalidate`]; an answer asked for under an earlier one is not stored.
    generation: u64,
    answers: FxHashMap<(String, u64), Answered>,
    /// Requests in flight under the current generation, by the same key as `answers`.
    pending: FxHashMap<(String, u64), Arc<Pending>>,
}

type Answers = Mutex<AnswerCache>;

/// Where a request finds its answer.
enum Found {
    /// An answer kept from an earlier request.
    Cached(Answered),
    /// A request already in flight, to wait on.
    Joined(Arc<Pending>),
    /// Nothing: this caller asks, and others join it.
    Asking(Arc<Pending>),
}

/// How long a request for a component the structural pass lists no props for waits for the checker, unless the session
/// says otherwise ([`SemanticHook::wait_empty`]): an empty answer is not worth showing early, and Chakra's `Button` takes
/// 0.4 to 0.8 s.
pub(crate) const DEFAULT_WAIT_EMPTY: Duration = Duration::from_secs(2);

pub(crate) struct SemanticHook {
    provider: Arc<dyn SemanticProvider>,
    /// How long a request waits for the checker before it takes the structural answer.
    wait: Duration,
    /// How long it waits instead when the structural answer has no props; never less than `wait`.
    wait_empty: Duration,
    answers: Arc<Answers>,
    /// Set once the checker has been reported unavailable, so a session says so once, not per component.
    warned: AtomicBool,
}

impl SemanticHook {
    pub(crate) fn new(provider: Arc<dyn SemanticProvider>, wait: Duration) -> Self {
        Self {
            provider,
            wait,
            wait_empty: DEFAULT_WAIT_EMPTY,
            answers: Arc::new(Mutex::new(AnswerCache::default())),
            warned: AtomicBool::new(false),
        }
    }

    /// Sets how long a request for a component with no structural props waits for the checker. A wait at most `wait`
    /// gives such a component the same wait as every other.
    pub(crate) fn wait_empty(&mut self, wait_empty: Duration) {
        self.wait_empty = wait_empty;
    }

    /// The wait for `answer`. Nobody waits for a background request: its answer is kept for the request that follows.
    /// Someone does for an interactive one, and longer when the structural pass found no props, since there is nothing
    /// to show before the checker answers. Any prop, even a degraded one, is an answer worth showing at the short wait.
    fn wait_for(&self, answer: &ComponentAnswer, priority: Priority) -> Duration {
        match (priority, answer.entry.props.is_empty()) {
            (Priority::Background, _) => Duration::ZERO,
            (Priority::Interactive, true) => self.wait_empty.max(self.wait),
            (Priority::Interactive, false) => self.wait,
        }
    }

    /// Forget every answer, refuse the ones still being waited for, and tell the checker what changed: a source file
    /// changed, and the checker's props for any component can depend on it. The next request of each component asks
    /// the checker again, after it has been told, rather than joining a request asked before the change.
    pub(crate) fn invalidate(&self, change: SourceChange) {
        let mut cache = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        cache.generation += 1;
        cache.answers.clear();
        cache.pending.clear();
        self.provider.changed(change);
    }

    /// `answer` with the checker's props, when it has them within the wait `priority` gives it.
    pub(crate) fn refine(
        &self,
        mut answer: ComponentAnswer,
        mode: HtmlAttributeMode,
        priority: Priority,
    ) -> ComponentAnswer {
        let stamp = std::fs::read(&answer.entry.file_path).map(|bytes| {
            let mut hasher = FxHasher::default();
            bytes.hash(&mut hasher);
            hasher.finish()
        });
        let cache_key = stamp.ok().map(|stamp| (answer.key.clone(), stamp));

        let (found, generation) = self.find(cache_key.as_ref(), priority);
        let pending = match found {
            Found::Cached(cached) => {
                answer.diagnostics.extend(cached.diagnostics);
                self.apply(&mut answer, cached.component.as_ref(), mode);
                return answer;
            }
            Found::Joined(pending) => pending,
            Found::Asking(pending) => {
                self.ask(&answer, mode, &pending, cache_key, generation);
                pending
            }
        };

        match pending.wait(self.wait_for(&answer, priority)) {
            Some(Settled::Answered { answered, once }) => {
                for diagnostic in once {
                    if !self.warned.swap(true, Ordering::Relaxed) {
                        answer.diagnostics.push(diagnostic);
                    }
                }
                answer.diagnostics.extend(answered.diagnostics);
                self.apply(&mut answer, answered.component.as_ref(), mode);
            }
            Some(Settled::Stopped) => {
                let mut stopped = Diagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::SemanticBackend,
                    "the TypeScript checker stopped without answering; the structural props are used",
                );
                stopped.affects = vec![answer.key.clone()];
                answer.diagnostics.push(stopped);
            }
            None => {
                let mut pending = Diagnostic::new(
                    DiagnosticSeverity::Info,
                    DiagnosticCode::SemanticBackend,
                    "the TypeScript checker has not answered yet; the structural props are used, and the checker's \
                     replace them on the next request",
                );
                pending.affects = vec![answer.key.clone()];
                answer.diagnostics.push(pending);
            }
        }
        answer
    }

    /// The kept answer for `key`, else the request in flight for it (raised when someone waits for this one), else a
    /// new request this caller makes, registered so the next caller joins it. One lock, so two callers never both ask.
    /// A file that could not be read has no key: its request is neither kept nor joined.
    fn find(&self, key: Option<&(String, u64)>, priority: Priority) -> (Found, u64) {
        let mut cache = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        let generation = cache.generation;
        if let Some(key) = key {
            if let Some(cached) = cache.answers.get(key) {
                return (Found::Cached(cached.clone()), generation);
            }
            if let Some(pending) = cache.pending.get(key) {
                match priority {
                    Priority::Interactive => pending.priority.raise(),
                    Priority::Background => {}
                }
                return (Found::Joined(pending.clone()), generation);
            }
        }
        let pending = Arc::new(Pending::new(priority));
        if let Some(key) = key {
            cache.pending.insert(key.clone(), pending.clone());
        }
        (Found::Asking(pending), generation)
    }

    /// Asks the checker for `answer`'s component. Its outcome settles `pending` for every caller waiting on it and is
    /// kept for the next request, unless the cache was invalidated since `generation`. Called without the cache's lock,
    /// since a provider may deliver at once.
    fn ask(
        &self,
        answer: &ComponentAnswer,
        mode: HtmlAttributeMode,
        pending: &Arc<Pending>,
        key: Option<(String, u64)>,
        generation: u64,
    ) {
        let target = SemanticTarget {
            key: answer.key.clone(),
            file_path: answer.entry.file_path.clone(),
            display_name: answer.entry.display_name.clone(),
            exports: answer.entry.exports.clone(),
            described: answer
                .entry
                .props
                .values()
                .filter(|prop| !prop.description.is_empty())
                .map(|prop| prop.name.clone())
                .collect(),
            react_wanted: react_props_wanted(mode, &answer.entry.props),
        };
        let answers = self.answers.clone();
        let asked = answer.key.clone();
        let settling = pending.clone();
        let delivery = Delivery::new(move |outcome| {
            let settled = match outcome {
                Some(outcome) => {
                    let Read { answered, once } = read_outcome(&asked, outcome);
                    Settled::Answered { answered, once }
                }
                None => Settled::Stopped,
            };
            if let Some(key) = key {
                let mut cache = answers.lock().unwrap_or_else(PoisonError::into_inner);
                if cache.pending.get(&key).is_some_and(|registered| Arc::ptr_eq(registered, &settling)) {
                    cache.pending.remove(&key);
                }
                if let Settled::Answered { answered, .. } = &settled
                    && cache.generation == generation
                {
                    cache.answers.insert(key, answered.clone());
                }
            }
            settling.settle(settled);
        });
        self.provider.request_at(vec![target], pending.priority.clone(), delivery);
    }

    fn apply(&self, answer: &mut ComponentAnswer, component: Option<&Arc<SemanticComponent>>, mode: HtmlAttributeMode) {
        let Some(component) = component else { return };
        let structural = answer.entry.props.clone();
        let used: Vec<&Arc<SemanticProp>> = component
            .props
            .iter()
            .filter(|prop| checker_prop_used(mode, &prop.name, prop.react_declared, &structural))
            .collect();
        // A degrade diagnostic names the component, not the prop, so it can only go once no structural prop is left
        // that it could be about: one the checker did not return keeps its structural type, opaque or not.
        let still_degraded = structural
            .iter()
            .any(|(name, prop)| prop.prop_type.holds_opaque() && !used.iter().any(|answered| answered.name == *name));
        let MergedProps { kept, unread, diagnostics } = merge_semantic_props(
            &mut answer.entry,
            used.into_iter(),
            component.composes.as_deref(),
            &mut FxHashMap::default(),
        );
        if !still_degraded {
            // A structural diagnostic about a prop the checker now answers for would be stale.
            answer.diagnostics.retain(|diagnostic| {
                !matches!(
                    diagnostic.code,
                    DiagnosticCode::OpaqueType | DiagnosticCode::MaxDepthExceeded | DiagnosticCode::CircularReference
                )
            });
        }
        let mut provenance = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            match kept {
                0 => "1 component(s) resolved by the TypeScript checker".to_string(),
                kept => format!(
                    "1 component(s) resolved by the TypeScript checker; {kept} prop(s) it did not report keep their \
                     structural type"
                ),
            },
        );
        provenance.affects = vec![answer.key.clone()];
        answer.diagnostics.push(provenance);
        answer.diagnostics.extend(diagnostics);
        if unread > 0 {
            answer.diagnostics.push(unread_diagnostic(unread, vec![answer.key.clone()]));
        }
    }
}

/// An outcome read for the component asked about.
struct Read {
    /// What is kept for the next request: the component and what the checker said about the request, each diagnostic
    /// naming the component.
    answered: Answered,
    /// What an unavailable checker says, which a session says once and not per component, so it is not kept.
    once: Vec<Diagnostic>,
}

/// The answer for the component whose output key is `asked`, and the diagnostics the request produced: a warning that
/// came with an answer (an edit the checker was not told about) is about this request, and its fallback's reason says
/// why the component kept its structural result. An answer or fallback for another key is not this component's.
fn read_outcome(asked: &str, outcome: SemanticOutcome) -> Read {
    let SemanticOutcome { components, fallbacks, diagnostics: reported } = outcome;
    // Only an outcome with no answer at all says the checker is unavailable.
    let unavailable = components.is_empty() && fallbacks.is_empty();
    let mut read = Read { answered: Answered::default(), once: Vec::new() };
    for mut diagnostic in reported {
        diagnostic.affects = vec![asked.to_owned()];
        if unavailable && diagnostic.severity == DiagnosticSeverity::Warning {
            read.once.push(diagnostic);
        } else {
            read.answered.diagnostics.push(diagnostic);
        }
    }
    for SemanticFallback { key, reason } in fallbacks {
        if key != asked {
            continue;
        }
        let mut info = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            format!("1 component(s) kept their structural result: {reason}"),
        );
        info.affects = vec![key];
        read.answered.diagnostics.push(info);
    }
    read.answered.component = components.into_iter().find(|found| found.key == asked).map(Arc::new);
    read
}
