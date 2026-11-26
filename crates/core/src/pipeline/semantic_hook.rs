//! Asking a checker for the component a session was just asked about (ADR 0022).
//!
//! The structural answer is always computed first and is a complete answer on its own. With a [`SemanticProvider`]
//! attached, [`SemanticHook::refine`] asks the checker for exactly that component, waits a bounded time, and merges
//! its props over the structural ones. A checker that is slower than the budget leaves the structural answer in
//! place, and its late answer is kept for the next request of the same, unedited component.
//!
//! There is one request in flight per component and version of its file: a caller that finds one waits on it instead
//! of asking again, and a caller raises a request it joins to its own class, so a story asked while its own prefetch
//! runs is answered when the prefetch is.
//!
//! A request nobody waits for is cancelled when its answer would not be read: an edit superseded the revision it was
//! asked at ([`SemanticHook::invalidate`]), or the client no longer shows its component ([`SemanticHook::cancel`]).
//! A checker drops it before converting it; one already converting it finishes.
//!
//! An answer describes the component's props as the checker read them, which depends on every file the component
//! imports, not only its own. So the session drops every answer when any source file changes
//! ([`SemanticHook::invalidate`]), and an answer that was still on its way then is neither kept nor joined. The same
//! call tells the checker which files changed, since a warm one answers from what it read before (ADR 0026 S6).

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

use super::ComponentAnswer;
use crate::pipeline::HtmlAttributeMode;
use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use crate::types::options::CollapseContext;
use crate::types::semantic::{
    Delivery, Layering, MergedProps, Priority, PropInterner, SemanticComponent, SemanticFallback, SemanticOutcome,
    SemanticProp, SemanticProvider, SemanticTarget, SharedPriority, SourceChange, Superseding, UnavailableScope,
    drop_superseded, fallback_severity, merge_semantic_props, unread_diagnostic,
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
    /// The checker answered for the component: kept for the next request.
    Answered(Answered),
    /// The checker answered nothing for the component: it was unavailable, timed out or crashed
    /// ([`SemanticOutcome::unavailable`]), or failed on this component ([`SemanticFallback::transient`]). The
    /// structural props stand, with what the checker said. Never kept as the answer: it is held as a refusal
    /// ([`AnswerCache::degraded`]) until [`DEGRADED_RETRY`] passes or a file changes, and the request after that asks
    /// the checker again.
    Degraded(Vec<Diagnostic>),
    /// The backend cannot serve any request ([`UnavailableScope::Backend`]: missing, unsupported, could not start,
    /// down for the session): what it said. Held for every component at once ([`AnswerCache::outage`]), so a batch of
    /// components asks it once, not once each.
    Unavailable(Vec<Diagnostic>),
    /// The checker stopped without answering. Held as a refusal like [`Settled::Degraded`], with the warning that says
    /// so ([`stopped_warning`]): a checker that keeps stopping would otherwise be asked by every request.
    Stopped,
    /// The request was cancelled before the checker converted it ([`SharedPriority::cancel`]): not an answer, and not
    /// a checker that stopped, so a caller reads it as one still on its way.
    Cancelled,
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
    /// The requests the checker last answered with nothing ([`Settled::Degraded`]), when, and what it said. Not an
    /// answer: a request within [`DEGRADED_RETRY`] of it gets the structural props and these diagnostics without asking
    /// the checker, which would spend a restart or a timeout to fail again; one after it asks again.
    degraded: FxHashMap<(String, u64), (Instant, Vec<Diagnostic>)>,
    /// The size of `degraded` at which the expired refusals are dropped next.
    prune_at: usize,
    /// When the backend last could not serve any request ([`Settled::Unavailable`]), and what it said: until
    /// [`DEGRADED_RETRY`] passes or a file changes, every request without a kept answer is refused with it.
    outage: Option<(Instant, Vec<Diagnostic>)>,
    /// The one copy of each prop the kept answers hold, for this generation.
    props: PropInterner,
}

/// The size of [`AnswerCache::degraded`] at which its expired refusals are first dropped. A key asked once and never
/// again would otherwise stay for the session.
const DEGRADED_HELD: usize = 256;

impl AnswerCache {
    /// Hold `key`'s refusal from `at`. Once the map reaches `prune_at`, the refusals past [`DEGRADED_RETRY`] are
    /// dropped and the next prune waits until the map doubles. A refusal is never dropped before its retry: the map
    /// holds at most twice the refusals of one retry interval, and a batch larger than [`DEGRADED_HELD`] repeated
    /// within it asks nothing.
    fn hold_degraded(&mut self, key: (String, u64), at: Instant, said: Vec<Diagnostic>) {
        if self.degraded.len() >= self.prune_at.max(DEGRADED_HELD) {
            self.degraded.retain(|_, (held, _)| at.saturating_duration_since(*held) < DEGRADED_RETRY);
            self.prune_at = 2 * self.degraded.len();
        }
        self.degraded.insert(key, (at, said));
    }
}

impl AnswerCache {
    /// `component` with each prop the one copy the kept answers share. A design system's components repeat the same
    /// props (Chakra's about a thousand style props each), and an answer kept with its own copies held about 6.7 MiB
    /// per Chakra component, so a session that answered a few hundred stories held gigabytes.
    fn shared(&self, component: Arc<SemanticComponent>) -> Arc<SemanticComponent> {
        let SemanticComponent { key, props, composes, whole } = Arc::unwrap_or_clone(component);
        let props = props.into_iter().map(|prop| self.props.share_arc(prop)).collect();
        Arc::new(SemanticComponent { key, props, composes, whole })
    }
}

type Answers = Mutex<AnswerCache>;

/// Where a request finds its answer.
enum Found {
    /// An answer kept from an earlier request.
    Cached(Answered),
    /// A request already in flight, to wait on.
    Joined(Arc<Pending>),
    /// The checker answered nothing for it a moment ago ([`AnswerCache::degraded`]): what it said then.
    Degraded(Vec<Diagnostic>),
    /// Nothing: this caller asks, and others join it.
    Asking(Arc<Pending>),
}

/// How long a request for a component the structural pass lists no props for waits for the checker, unless the session
/// says otherwise ([`SemanticHook::wait_empty`]): an empty answer is not worth showing early, and Chakra's `Button` takes
/// 0.4 to 0.8 s.
pub(crate) const DEFAULT_WAIT_EMPTY: Duration = Duration::from_secs(2);

/// How long a request the checker answered with nothing ([`Settled::Degraded`], [`Settled::Stopped`]) stands before the
/// component is asked again, unless a file changes first: the interval a walk past an unreadable dependency file waits
/// too, one constant for both.
pub(crate) use crate::dependency_types::DEGRADED_RETRY;

/// The time now, as the session reads it; a test sets its own.
type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// What an answer says when the checker had not answered within the wait and the structural props stand in for its.
const CHECKER_PENDING: &str = "the TypeScript checker has not answered yet; the structural props are used, and the \
                               checker's replace them on the next request";

/// Whether `answer`'s props are the structural ones because the checker had not answered within the wait: asked again
/// once the checker has answered, and before any source file changes, the session answers with the checker's props.
/// The one place that decides it, for hosts that show an answer and replace it later.
pub fn awaits_checker(answer: &ComponentAnswer) -> bool {
    answer.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::SemanticBackend
            && diagnostic.severity == DiagnosticSeverity::Info
            && diagnostic.message == CHECKER_PENDING
    })
}

pub(crate) struct SemanticHook {
    provider: Arc<dyn SemanticProvider>,
    /// How long a request waits for the checker before it takes the structural answer.
    wait: Duration,
    /// How long it waits instead when the structural answer has no props; never less than `wait`.
    wait_empty: Duration,
    answers: Arc<Answers>,
    now: Clock,
}

impl SemanticHook {
    pub(crate) fn new(provider: Arc<dyn SemanticProvider>, wait: Duration) -> Self {
        Self {
            provider,
            wait,
            wait_empty: DEFAULT_WAIT_EMPTY,
            answers: Arc::new(Mutex::new(AnswerCache::default())),
            now: Arc::new(Instant::now),
        }
    }

    /// Sets how long a request for a component with no structural props waits for the checker. A wait at most `wait`
    /// gives such a component the same wait as every other.
    pub(crate) fn wait_empty(&mut self, wait_empty: Duration) {
        self.wait_empty = wait_empty;
    }

    /// The wait for `answer`. Nobody waits for a neighbour or background request: its answer is kept for the request
    /// that follows.
    /// Someone does for an interactive one, and longer when the structural pass found no props, since there is nothing
    /// to show before the checker answers. Any prop, even a degraded one, is an answer worth showing at the short wait.
    fn wait_for(&self, answer: &ComponentAnswer, priority: Priority) -> Duration {
        match (priority, answer.entry.props.is_empty()) {
            (Priority::Neighbour | Priority::Background, _) => Duration::ZERO,
            (Priority::Interactive, true) => self.wait_empty.max(self.wait),
            (Priority::Interactive, false) => self.wait,
        }
    }

    /// Forget every answer, refuse the ones still being waited for, and tell the checker what changed: a source file
    /// changed, and the checker's props for any component can depend on it. The next request of each component asks
    /// the checker again, after it has been told, rather than joining a request asked before the change. A request
    /// nobody waits for is cancelled: its answer, of the superseded revision, would not be kept, so a checker that has
    /// not started it drops it. One someone waits for is still answered, after the change.
    pub(crate) fn invalidate(&self, change: SourceChange) {
        let mut cache = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        cache.generation += 1;
        cache.answers.clear();
        cache.degraded.clear();
        cache.outage = None;
        for pending in cache.pending.values() {
            pending.priority.cancel();
        }
        cache.pending.clear();
        cache.props = PropInterner::default();
        self.provider.changed(change);
    }

    /// Cancels the requests in flight for `keys` that nobody waits for: the client no longer shows those components,
    /// so a checker that has not started them drops them. A later request asks again rather than joining a cancelled
    /// one; a request someone waits for, or one the checker is already converting, stays registered, so a later request
    /// joins it, and is answered (and kept) as before.
    pub(crate) fn cancel(&self, keys: &[String]) {
        let keys: FxHashSet<&str> = keys.iter().map(String::as_str).collect();
        let mut cache = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        cache.pending.retain(|(key, _), pending| !(keys.contains(key.as_str()) && pending.priority.cancel()));
    }

    /// `answer` with the checker's props, when it has them within the wait `priority` gives it.
    pub(crate) fn refine(
        &self,
        mut answer: ComponentAnswer,
        mode: HtmlAttributeMode,
        cx: CollapseContext,
        priority: Priority,
    ) -> ComponentAnswer {
        let mode = crate::react_types::react_attribute_mode(mode, crate::react_types::expanded(&answer.entry, cx.mode));
        // The checker converts only the props this merge keeps, decided from the structural answer and `cx`: an answer
        // is kept and joined for those too, so another structural answer is never merged with props left unconverted.
        let layering = Layering::of(&answer.entry, cx);
        let stamp = std::fs::read(&answer.entry.file_path).map(|bytes| {
            let mut hasher = FxHasher::default();
            bytes.hash(&mut hasher);
            layering.hash(&mut hasher);
            hasher.finish()
        });
        let cache_key = stamp.ok().map(|stamp| (answer.key.clone(), stamp));

        let (found, generation) = self.find(cache_key.as_ref(), priority);
        let pending = match found {
            Found::Cached(cached) => {
                self.apply(&mut answer, cached, mode, cx);
                return answer;
            }
            Found::Joined(pending) => pending,
            Found::Degraded(diagnostics) => {
                answer.diagnostics.extend(
                    diagnostics
                        .into_iter()
                        .map(|diagnostic| Diagnostic { affects: vec![answer.key.clone()], ..diagnostic }),
                );
                return answer;
            }
            Found::Asking(pending) => {
                self.ask(target_of(&answer, mode, layering), &pending, cache_key, generation);
                pending
            }
        };

        match pending.wait(self.wait_for(&answer, priority)) {
            Some(Settled::Answered(answered)) => self.apply(&mut answer, answered, mode, cx),
            Some(Settled::Degraded(diagnostics) | Settled::Unavailable(diagnostics)) => {
                answer.diagnostics.extend(diagnostics)
            }
            Some(Settled::Stopped) => answer.diagnostics.push(stopped_warning(&answer.key)),
            Some(Settled::Cancelled) | None => {
                let mut pending =
                    Diagnostic::new(DiagnosticSeverity::Info, DiagnosticCode::SemanticBackend, CHECKER_PENDING);
                pending.affects = vec![answer.key.clone()];
                answer.diagnostics.push(pending);
            }
        }
        answer
    }

    /// The kept answer for `key`, else the request in flight for it (raised to `priority` when that goes first), else
    /// what the checker said when it last answered nothing for it within [`DEGRADED_RETRY`], else a new request this
    /// caller makes, registered so the next caller joins it. One lock, so two callers never both ask. A file that could
    /// not be read has no key: its request is neither kept nor joined.
    fn find(&self, key: Option<&(String, u64)>, priority: Priority) -> (Found, u64) {
        let now = (self.now)();
        let mut cache = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        let generation = cache.generation;
        if let Some(key) = key {
            if let Some(cached) = cache.answers.get(key) {
                return (Found::Cached(cached.clone()), generation);
            }
            match cache.degraded.get(key) {
                Some((at, said)) if now.saturating_duration_since(*at) < DEGRADED_RETRY => {
                    return (Found::Degraded(said.clone()), generation);
                }
                Some(_) => drop(cache.degraded.remove(key)),
                None => {}
            }
            match &cache.outage {
                Some((at, said)) if now.saturating_duration_since(*at) < DEGRADED_RETRY => {
                    return (Found::Degraded(said.clone()), generation);
                }
                Some(_) => cache.outage = None,
                None => {}
            }
            if let Some(pending) = cache.pending.get(key) {
                pending.priority.raise_to(priority);
                return (Found::Joined(pending.clone()), generation);
            }
        }
        let pending = Arc::new(Pending::new(priority));
        if let Some(key) = key {
            cache.pending.insert(key.clone(), pending.clone());
        }
        (Found::Asking(pending), generation)
    }

    /// Asks the checker about `target`. Its outcome settles `pending` for every caller waiting on it and, when it
    /// answers for the component, is kept for the next request, unless the cache was invalidated since `generation`;
    /// when it answers nothing, that is held as a refusal for [`DEGRADED_RETRY`] instead. Called without the cache's
    /// lock, since a provider may deliver at once.
    fn ask(&self, target: SemanticTarget, pending: &Arc<Pending>, key: Option<(String, u64)>, generation: u64) {
        let answers = self.answers.clone();
        let asked = target.key.clone();
        let settling = pending.clone();
        let now = self.now.clone();
        let delivery = Delivery::new(move |outcome| {
            let mut settled = match outcome {
                Some(outcome) => read_outcome(&asked, outcome),
                None if settling.priority.is_cancelled() => Settled::Cancelled,
                None => Settled::Stopped,
            };
            if let Some(key) = key {
                let mut cache = answers.lock().unwrap_or_else(PoisonError::into_inner);
                if cache.pending.get(&key).is_some_and(|registered| Arc::ptr_eq(registered, &settling)) {
                    cache.pending.remove(&key);
                }
                if cache.generation == generation {
                    match &mut settled {
                        Settled::Answered(answered) => {
                            answered.component = answered.component.take().map(|component| cache.shared(component));
                            cache.answers.insert(key, answered.clone());
                        }
                        Settled::Degraded(said) => cache.hold_degraded(key, now(), said.clone()),
                        Settled::Unavailable(said) => cache.outage = Some((now(), said.clone())),
                        Settled::Stopped => cache.hold_degraded(key, now(), vec![stopped_warning(&asked)]),
                        // Not an answer, nor one the checker failed to give: the next request asks.
                        Settled::Cancelled => {}
                    }
                }
            }
            settling.settle(settled);
        });
        self.provider.request_at(vec![target], pending.priority.clone(), delivery);
    }

    /// Merges what the checker made of the component into `answer`: its props, if it found the component, then what it
    /// said about the request. The structural diagnostics its props supersede go first, so the checker's are never
    /// read as structural.
    fn apply(&self, answer: &mut ComponentAnswer, answered: Answered, mode: HtmlAttributeMode, cx: CollapseContext) {
        let Answered { component, diagnostics } = answered;
        let Some(component) = component else {
            answer.diagnostics.extend(diagnostics);
            return;
        };
        let structural = answer.entry.props.clone();
        let visible = crate::react_types::visible_rows(&answer.entry);
        let used: Vec<&Arc<SemanticProp>> = component
            .props
            .iter()
            .filter(|prop| crate::react_types::checker_prop_kept(mode, &prop.name, prop.react_declared, &visible))
            .collect();
        // The props of a collapsed source are its layer's, not listed again.
        let (used, notable) =
            crate::types::semantic::layer_checker_props(&mut answer.entry, used, &component.whole, cx);
        // A structural prop the checker did not return keeps its structural type, opaque or not, and a structural
        // diagnostic naming the component may be about it.
        let structural_left = structural
            .iter()
            .any(|(name, prop)| prop.prop_type.holds_opaque() && !used.iter().any(|answered| answered.name == *name));
        let MergedProps { kept, unread, diagnostics: syntax } = merge_semantic_props(
            &mut answer.entry,
            used.into_iter(),
            component.composes.as_deref(),
            &mut FxHashMap::default(),
        );
        crate::types::semantic::move_notable(&mut answer.entry, &notable);
        crate::types::semantic::listed_props_leave_layers(&mut answer.entry, true);
        crate::relevance::narrow(&mut answer.entry);
        let superseding = Superseding::new(&answer.key, &answer.entry, structural_left);
        drop_superseded(&mut answer.diagnostics, &[superseding]);
        answer.diagnostics.extend(diagnostics);
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
        answer.diagnostics.extend(syntax);
        if unread > 0 {
            answer.diagnostics.push(unread_diagnostic(unread, vec![answer.key.clone()]));
        }
    }
}

/// What an answer for `key` says when the checker stopped without answering for it.
fn stopped_warning(key: &str) -> Diagnostic {
    let mut stopped = Diagnostic::new(
        DiagnosticSeverity::Warning,
        DiagnosticCode::SemanticBackend,
        "the TypeScript checker stopped without answering; the structural props are used",
    );
    stopped.affects = vec![key.to_owned()];
    stopped
}

/// What the checker is asked about `answer`'s component: its props under `mode`, converted only where the merge with
/// `layering` keeps them.
fn target_of(answer: &ComponentAnswer, mode: HtmlAttributeMode, layering: Layering) -> SemanticTarget {
    SemanticTarget {
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
        react_wanted: crate::react_types::react_props_wanted_for(mode, &answer.entry),
        layering: Some(layering),
    }
}

/// The outcome read for the component whose output key is `asked`, with every diagnostic it carries: a warning that
/// came with an answer (an edit the checker was not told about) is about this request, and a fallback's reason says why
/// the component kept its structural result. An outcome marked unavailable ([`SemanticOutcome::unavailable`]), or with
/// a transient fallback for `asked` (the checker failed on it), answers nothing: it is [`Settled::Degraded`]. Decided
/// from those marks alone: an outcome with no entry for `asked` (a target the pass skipped, a run past its heap budget)
/// is the checker's answer, kept like any other. An answer or fallback for another key is not this component's.
/// The mark's scope, decided where the failure was, says whom the refusal holds for: every component when the backend
/// cannot serve any request, the asked one alone when only this request failed.
fn read_outcome(asked: &str, outcome: SemanticOutcome) -> Settled {
    let SemanticOutcome { components, fallbacks, diagnostics: reported, unavailable } = outcome;
    let mut diagnostics: Vec<Diagnostic> = reported
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.affects = vec![asked.to_owned()];
            diagnostic
        })
        .collect();
    if let Some(unasked) = unavailable.filter(|unasked| unasked.covers(asked)) {
        return match unasked.scope() {
            UnavailableScope::Backend => Settled::Unavailable(diagnostics),
            UnavailableScope::Request => Settled::Degraded(diagnostics),
        };
    }
    let component = components.into_iter().find(|found| found.key == asked).map(Arc::new);
    let mut failed = false;
    for SemanticFallback { key, reason, transient } in fallbacks {
        if key != asked {
            continue;
        }
        failed |= transient;
        let mut kept = Diagnostic::new(
            fallback_severity(transient),
            DiagnosticCode::SemanticBackend,
            format!("1 component(s) kept their structural result: {reason}"),
        );
        kept.affects = vec![key];
        diagnostics.push(kept);
    }
    if failed {
        return Settled::Degraded(diagnostics);
    }
    Settled::Answered(Answered { component, diagnostics })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropType;

    fn component(key: &str, names: &[&str]) -> Arc<SemanticComponent> {
        let props = names
            .iter()
            .map(|name| {
                Arc::new(SemanticProp {
                    name: (*name).to_owned(),
                    prop_type: PropType::String,
                    required: false,
                    description: String::new(),
                    tags: Default::default(),
                    parent: None,
                    declarations: Vec::new(),
                    react_declared: false,
                })
            })
            .collect();
        Arc::new(SemanticComponent { key: key.to_owned(), props, composes: None, whole: Vec::new() })
    }

    #[test]
    fn kept_answers_share_one_copy_of_an_equal_prop_until_the_cache_is_invalidated() {
        let mut cache = AnswerCache::default();
        let first = cache.shared(component("A", &["color", "size"]));
        let second = cache.shared(component("B", &["color", "label"]));

        assert!(Arc::ptr_eq(&first.props[0], &second.props[0]), "both answers hold the one `color`");
        assert!(!Arc::ptr_eq(&first.props[1], &second.props[1]));
        assert_eq!(second.key, "B");
        assert_eq!(second.props.iter().map(|prop| prop.name.as_str()).collect::<Vec<_>>(), ["color", "label"]);

        // What `invalidate` does to the cache: a later answer no longer reaches the props of the dropped ones.
        cache.props = PropInterner::default();
        let third = cache.shared(component("C", &["color"]));
        assert!(!Arc::ptr_eq(&first.props[0], &third.props[0]));
    }

    /// A checker that holds every request with its priority; the test answers one or reads its priority.
    #[derive(Default)]
    struct Holding(Mutex<Vec<(String, SharedPriority, Option<Delivery>)>>);

    impl Holding {
        /// The key, priority now and whether it was cancelled, of every request in the order asked.
        fn asked(&self) -> Vec<(String, Priority, bool)> {
            let held = self.0.lock().unwrap();
            held.iter().map(|(key, priority, _)| (key.clone(), priority.get(), priority.is_cancelled())).collect()
        }

        /// What the worker does when it takes request `n` to convert it.
        fn start(&self, n: usize) {
            assert!(self.0.lock().unwrap()[n].1.start());
        }

        fn answer(&self, n: usize) {
            let (key, delivery) = {
                let mut held = self.0.lock().unwrap();
                (held[n].0.clone(), held[n].2.take().unwrap())
            };
            let outcome =
                SemanticOutcome { components: vec![(*component(&key, &["label"])).clone()], ..Default::default() };
            delivery.deliver(Some(outcome));
        }
    }

    impl SemanticProvider for Holding {
        fn request(&self, targets: Vec<SemanticTarget>) -> std::sync::mpsc::Receiver<SemanticOutcome> {
            let (sender, receiver) = std::sync::mpsc::channel();
            let delivery = Delivery::new(move |outcome| {
                if let Some(outcome) = outcome {
                    sender.send(outcome).ok();
                }
            });
            self.request_at(targets, SharedPriority::new(Priority::Interactive), delivery);
            receiver
        }

        fn request_at(&self, targets: Vec<SemanticTarget>, priority: SharedPriority, delivery: Delivery) {
            let key = targets.into_iter().next().unwrap().key;
            self.0.lock().unwrap().push((key, priority, Some(delivery)));
        }
    }

    fn session(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<crate::pipeline::WatchSession>, Arc<Holding>) {
        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        for (name, source) in files {
            std::fs::write(root.join(name), source).unwrap();
        }
        let checker = Arc::new(Holding::default());
        let options = crate::pipeline::PipelineOptions { src_dirs: vec![root], ..Default::default() };
        let session =
            crate::pipeline::WatchSession::lazy(options).with_semantic(checker.clone(), Duration::from_secs(60));
        drop(session.initialize());
        (dir, Arc::new(session), checker)
    }

    const BUTTON: &str = "export function Button(props: { label: string }) { return null; }";
    const CARD: &str = "export function Card(props: { title: string }) { return null; }";

    /// A prefetch of a story the client navigated away from is cancelled, so the checker drops it before converting it;
    /// the next request of that component asks again rather than joining it. A prefetch a story joined is waited for
    /// and is not cancelled.
    #[test]
    fn a_request_for_a_component_nobody_shows_is_dropped_before_conversion() {
        let (_dir, session, checker) = session(&[("Button.tsx", BUTTON), ("Card.tsx", CARD)]);
        drop(session.components_as_neighbours(&["Button".to_string(), "Card".to_string()]));
        let story = {
            let session = session.clone();
            std::thread::spawn(move || session.component("Card").unwrap())
        };
        while checker.asked()[1].1 != Priority::Interactive {
            std::thread::yield_now();
        }

        session.cancel_components(&["Button".to_string(), "Card".to_string()]);

        assert_eq!(
            checker.asked(),
            [("Button".to_string(), Priority::Neighbour, true), ("Card".to_string(), Priority::Interactive, false)]
        );
        checker.answer(1);
        assert!(story.join().unwrap().entry.props.contains_key("label"));
        drop(session.components_as_neighbours(&["Button".to_string()]));
        assert_eq!(checker.asked().len(), 3, "a cancelled request is not joined");
        assert_eq!(checker.asked()[2], ("Button".to_string(), Priority::Neighbour, false));
    }

    /// A neighbour request the checker already took is converted whatever the client shows now: a cancel leaves it
    /// registered, so a request for the component joins it, and its one delivery settles both callers.
    #[test]
    fn a_request_being_converted_survives_a_cancel_and_is_joined() {
        let (_dir, session, checker) = session(&[("Button.tsx", BUTTON)]);
        drop(session.components_as_neighbours(&["Button".to_string()]));
        checker.start(0);

        session.cancel_components(&["Button".to_string()]);
        let story = {
            let session = session.clone();
            std::thread::spawn(move || session.component("Button").unwrap())
        };
        while checker.asked()[0].1 != Priority::Interactive {
            std::thread::yield_now();
        }

        assert_eq!(checker.asked(), [("Button".to_string(), Priority::Interactive, false)]);
        checker.answer(0);
        assert!(story.join().unwrap().entry.props.contains_key("label"));
        assert_eq!(checker.asked().len(), 1, "one conversion served both");
    }

    /// A checker that drops each request as cancelled the moment it is asked: a cancel that lands between a caller
    /// registering its request and reading it.
    struct CancelsAtOnce;

    impl SemanticProvider for CancelsAtOnce {
        fn request(&self, _targets: Vec<SemanticTarget>) -> std::sync::mpsc::Receiver<SemanticOutcome> {
            std::sync::mpsc::channel().1
        }

        fn request_at(&self, _targets: Vec<SemanticTarget>, priority: SharedPriority, delivery: Delivery) {
            assert!(priority.cancel());
            drop(delivery);
        }
    }

    /// A cancelled request is not a checker that stopped: its caller is told the answer is still to come, with no
    /// warning.
    #[test]
    fn a_cancelled_request_is_not_reported_as_a_stopped_checker() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Button.tsx"), BUTTON).unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let options = crate::pipeline::PipelineOptions { src_dirs: vec![root], ..Default::default() };
        let session = crate::pipeline::WatchSession::lazy(options)
            .with_semantic(Arc::new(CancelsAtOnce), Duration::from_secs(60));
        drop(session.initialize());

        let answer = session.components_as_neighbours(&["Button".to_string()]).remove(0).unwrap();

        let messages: Vec<&str> = answer.diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages, [CHECKER_PENDING]);
    }

    /// Refusals for keys never asked again do not pile up: once the map reaches its prune size the expired ones go, and
    /// a refusal is never dropped before its retry, however many keys were refused within it.
    #[test]
    fn refusals_for_keys_never_asked_again_are_bounded() {
        let mut cache = AnswerCache::default();
        let start = Instant::now();
        let key = |n: usize| (format!("C{n}"), 0_u64);
        for n in 0..DEGRADED_HELD {
            cache.hold_degraded(key(n), start, Vec::new());
        }
        assert_eq!(cache.degraded.len(), DEGRADED_HELD);
        let later = start + DEGRADED_RETRY;
        cache.hold_degraded(key(DEGRADED_HELD), later, Vec::new());
        assert_eq!(cache.degraded.len(), 1, "every expired refusal went");
        for n in 1..=DEGRADED_HELD {
            cache.hold_degraded(key(DEGRADED_HELD + n), later + Duration::from_millis(n as u64), Vec::new());
        }
        assert_eq!(cache.degraded.len(), DEGRADED_HELD + 1, "no fresh refusal is dropped before its retry");
        let expired = later + DEGRADED_RETRY * 2;
        for n in 0..DEGRADED_HELD {
            cache.hold_degraded(key(3 * DEGRADED_HELD + n), expired, Vec::new());
        }
        assert_eq!(cache.degraded.len(), DEGRADED_HELD, "at twice the size the expired ones went, the fresh stayed");
        assert!((0..DEGRADED_HELD).all(|n| cache.degraded.contains_key(&key(3 * DEGRADED_HELD + n))));
    }

    /// A batch of 300 components while the backend cannot serve any request asks it once. While each request times out
    /// or the checker rejects each component, it asks once per component: a request that failed refuses its own
    /// component alone. The same batch again within the retry asks nothing.
    #[test]
    fn a_batch_larger_than_the_prune_size_repeated_within_the_retry_asks_nothing() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        std::fs::write(root.join("Button.tsx"), BUTTON).unwrap();
        let options = crate::pipeline::PipelineOptions { src_dirs: vec![root], ..Default::default() };
        let structural = crate::pipeline::WatchSession::lazy(options.clone());
        drop(structural.initialize());
        let answer = structural.component("Button").unwrap();
        let cx = CollapseContext::of(&options);
        for (reply, first) in [(Reply::Down, 1), (Reply::TimedOut, 300), (Reply::Rejected, 300)] {
            let checker = Arc::new(Replies { reply, recovered: false.into(), asked: 0.into() });
            let mut hook = SemanticHook::new(checker.clone(), Duration::from_secs(60));
            let start = Instant::now();
            hook.now = Arc::new(move || start);
            let batch = || {
                for n in 0..300 {
                    let asked = ComponentAnswer { key: format!("C{n}"), ..answer.clone() };
                    let refined = hook.refine(asked, options.html_attributes, cx, Priority::Background);
                    assert_eq!(refined.diagnostics.iter().filter(|d| d.affects == [format!("C{n}")]).count(), 1);
                }
            };
            batch();
            assert_eq!(checker.asked.load(SeqCst), first, "{reply:?}");
            batch();
            assert_eq!(checker.asked.load(SeqCst), first, "{reply:?}: the repeat asks nothing");
        }
    }

    /// One component's request fails (its program timed out); an unrelated component asked right after is answered by
    /// the checker, not refused.
    #[test]
    fn a_failed_request_refuses_only_its_own_component() {
        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        std::fs::write(root.join("Button.tsx"), BUTTON).unwrap();
        std::fs::write(root.join("Card.tsx"), CARD).unwrap();
        let options = crate::pipeline::PipelineOptions { src_dirs: vec![root], ..Default::default() };
        let structural = crate::pipeline::WatchSession::lazy(options.clone());
        drop(structural.initialize());
        let cx = CollapseContext::of(&options);
        let checker = Arc::new(Holding::default());
        let hook = SemanticHook::new(checker.clone(), Duration::ZERO);
        let ask = |key: &str| {
            hook.refine(structural.component(key).unwrap(), options.html_attributes, cx, Priority::Background)
        };
        drop(ask("Button"));
        let delivery = checker.0.lock().unwrap()[0].2.take().unwrap();
        delivery.deliver(Some(SemanticOutcome::request_failed("the checker stopped answering: timed out")));
        drop(ask("Card"));
        let asked: Vec<String> = checker.asked().into_iter().map(|(key, _, _)| key).collect();
        assert_eq!(asked, ["Button", "Card"], "the unrelated component is asked");
        drop(ask("Button"));
        assert_eq!(checker.asked().len(), 2, "the failed one is held for its retry");
    }

    /// What [`Replies`] answers while the checker has not recovered.
    #[derive(Clone, Copy, Debug)]
    enum Reply {
        /// `SemanticOutcome::unavailable`: the backend cannot serve any request.
        Down,
        /// `SemanticOutcome::request_failed`: this request timed out.
        TimedOut,
        /// A transient fallback for the component: the checker rejected a request (it panicked or stopped).
        Rejected,
        /// A fallback that is the checker's verdict about the component.
        Verdict,
        /// An outcome with no entry for the component and a warning, which marks it (and another) as not asked: a run
        /// past its heap budget.
        OverBudget,
        /// An outcome with a warning and no entry or mark for the component: the checker's answer is the structural
        /// one.
        Absent,
        /// No outcome at all: the checker stopped mid-request (the delivery is dropped, not cancelled).
        Stops,
    }

    /// A checker that answers every request at once with `reply`, or with `label` and `size` once it has `recovered`,
    /// and counts the requests.
    struct Replies {
        reply: Reply,
        recovered: std::sync::atomic::AtomicBool,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl SemanticProvider for Replies {
        fn request(&self, _targets: Vec<SemanticTarget>) -> std::sync::mpsc::Receiver<SemanticOutcome> {
            std::sync::mpsc::channel().1
        }

        fn request_at(&self, targets: Vec<SemanticTarget>, _priority: SharedPriority, delivery: Delivery) {
            use std::sync::atomic::Ordering::SeqCst;
            self.asked.fetch_add(1, SeqCst);
            let key = targets.into_iter().next().unwrap().key;
            let fallback = |transient| SemanticFallback { key: key.clone(), reason: "why".to_string(), transient };
            let warning = Diagnostic::new(DiagnosticSeverity::Warning, DiagnosticCode::SemanticBackend, "over budget");
            let outcome = match (self.recovered.load(SeqCst), self.reply) {
                (true, _) => SemanticOutcome {
                    components: vec![(*component(&key, &["label", "size"])).clone()],
                    ..Default::default()
                },
                (false, Reply::Down) => SemanticOutcome::unavailable("no tsc in this test"),
                (false, Reply::TimedOut) => SemanticOutcome::request_failed("the checker stopped answering: timed out"),
                (false, Reply::Rejected) => SemanticOutcome { fallbacks: vec![fallback(true)], ..Default::default() },
                (false, Reply::Verdict) => SemanticOutcome { fallbacks: vec![fallback(false)], ..Default::default() },
                (false, Reply::OverBudget) => {
                    let mut outcome = SemanticOutcome { diagnostics: vec![warning], ..Default::default() };
                    outcome.leave_unasked([key.clone(), "Other".to_owned()]);
                    outcome
                }
                (false, Reply::Absent) => {
                    let mut outcome = SemanticOutcome { diagnostics: vec![warning], ..Default::default() };
                    outcome.leave_unasked(["Other".to_owned()]);
                    outcome
                }
                (false, Reply::Stops) => {
                    drop(delivery);
                    return;
                }
            };
            delivery.deliver(Some(outcome));
        }
    }

    /// A request the checker answered with nothing (down, or failed on the component) is refused again without asking
    /// it until `DEGRADED_RETRY` passes or a file changes, each answer still carrying what it said; then it is asked
    /// again, and a recovered checker's answer replaces the structural one; a run that left the component unasked once
    /// its heap passed the budget is refused the same way. A verdict, or an outcome with no entry and no mark for the
    /// component, is the checker's answer and is kept, even past the retry.
    #[test]
    fn a_checker_that_answered_nothing_is_asked_again_only_after_the_retry_or_an_edit() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        std::fs::write(root.join("Button.tsx"), BUTTON).unwrap();
        let options = crate::pipeline::PipelineOptions { src_dirs: vec![root.clone()], ..Default::default() };
        let structural = crate::pipeline::WatchSession::lazy(options.clone());
        drop(structural.initialize());
        let answer = structural.component("Button").unwrap();
        let cx = CollapseContext::of(&options);

        for reply in [
            Reply::Down,
            Reply::TimedOut,
            Reply::Rejected,
            Reply::OverBudget,
            Reply::Stops,
            Reply::Verdict,
            Reply::Absent,
        ] {
            let degraded = match reply {
                Reply::Down | Reply::TimedOut | Reply::Rejected | Reply::OverBudget | Reply::Stops => true,
                Reply::Verdict | Reply::Absent => false,
            };
            let checker = Arc::new(Replies { reply, recovered: false.into(), asked: 0.into() });
            let mut hook = SemanticHook::new(checker.clone(), Duration::from_secs(60));
            let start = Instant::now();
            let clock = Arc::new(Mutex::new(start));
            hook.now = {
                let clock = clock.clone();
                Arc::new(move || *clock.lock().unwrap())
            };
            let at = |after: Duration| *clock.lock().unwrap() = start + after;
            let ask = || hook.refine(answer.clone(), options.html_attributes, cx, Priority::Interactive);
            let props = |answer: &ComponentAnswer| answer.entry.props.keys().cloned().collect::<Vec<_>>();
            let said = |answer: &ComponentAnswer| {
                answer.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SemanticBackend).count()
            };

            let first: Vec<ComponentAnswer> = (0..5).map(|_| ask()).collect();
            for answer in &first {
                assert_eq!(props(answer), ["label"], "{reply:?}");
                assert_eq!(said(answer), 1, "{reply:?}: every answer says why: {:?}", answer.diagnostics);
                assert!(!awaits_checker(answer), "{reply:?}");
                // What a host counts as degrading the answer (Storybook's `degradedBy`): a warning or an error.
                let warns = answer.diagnostics.iter().any(|d| d.severity != DiagnosticSeverity::Info);
                assert!(!degraded || warns, "{reply:?}: a degraded answer warns: {:?}", answer.diagnostics);
            }
            assert_eq!(checker.asked.load(SeqCst), 1, "{reply:?}: five requests ask once");
            at(DEGRADED_RETRY - Duration::from_millis(1));
            ask();
            assert_eq!(checker.asked.load(SeqCst), 1, "{reply:?}: not asked within the retry");
            at(DEGRADED_RETRY);
            ask();
            assert_eq!(checker.asked.load(SeqCst), if degraded { 2 } else { 1 }, "{reply:?}: asked after the retry");
            hook.invalidate(SourceChange::Files(vec![root.join("Button.tsx")]));
            ask();
            assert_eq!(checker.asked.load(SeqCst), if degraded { 3 } else { 2 }, "{reply:?}: asked after an edit");

            checker.recovered.store(true, SeqCst);
            at(DEGRADED_RETRY * 2);
            let recovered = ask();
            let expected: &[&str] = if degraded { &["label", "size"] } else { &["label"] };
            assert_eq!(props(&recovered), expected, "{reply:?}: a recovered checker's answer replaces it");
            assert_eq!(props(&ask()), expected, "{reply:?}");
            assert_eq!(checker.asked.load(SeqCst), if degraded { 4 } else { 2 }, "{reply:?}: then kept");
        }
    }

    /// An edit cancels the requests of the revision before it that nobody waits for: their answers would not be kept.
    #[test]
    fn a_request_of_a_superseded_revision_is_dropped() {
        let (dir, session, checker) = session(&[("Button.tsx", BUTTON), ("Card.tsx", CARD)]);
        drop(session.components_in_background(&["Button".to_string()]));
        drop(session.components_as_neighbours(&["Card".to_string()]));

        let file = camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        std::fs::write(file.join("Card.tsx"), CARD.replace("title", "heading")).unwrap();
        session.update_file(&file.join("Card.tsx"));

        assert_eq!(
            checker.asked(),
            [("Button".to_string(), Priority::Background, true), ("Card".to_string(), Priority::Neighbour, true)]
        );
    }
}
