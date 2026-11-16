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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

use super::ComponentAnswer;
use crate::pipeline::HtmlAttributeMode;
use crate::react_types::{checker_prop_used, react_props_wanted};
use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use crate::types::options::CollapseContext;
use crate::types::semantic::{
    Delivery, Layering, MergedProps, Priority, PropInterner, SemanticComponent, SemanticFallback, SemanticOutcome,
    SemanticProp, SemanticProvider, SemanticTarget, SharedPriority, SourceChange, Superseding, drop_superseded,
    merge_semantic_props, unread_diagnostic,
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
    /// The one copy of each prop the kept answers hold, for this generation.
    props: PropInterner,
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
    /// Nothing: this caller asks, and others join it.
    Asking(Arc<Pending>),
}

/// How long a request for a component the structural pass lists no props for waits for the checker, unless the session
/// says otherwise ([`SemanticHook::wait_empty`]): an empty answer is not worth showing early, and Chakra's `Button` takes
/// 0.4 to 0.8 s.
pub(crate) const DEFAULT_WAIT_EMPTY: Duration = Duration::from_secs(2);

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
            Found::Asking(pending) => {
                self.ask(target_of(&answer, mode, layering), &pending, cache_key, generation);
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
                self.apply(&mut answer, answered, mode, cx);
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
            Some(Settled::Cancelled) | None => {
                let mut pending =
                    Diagnostic::new(DiagnosticSeverity::Info, DiagnosticCode::SemanticBackend, CHECKER_PENDING);
                pending.affects = vec![answer.key.clone()];
                answer.diagnostics.push(pending);
            }
        }
        answer
    }

    /// The kept answer for `key`, else the request in flight for it (raised to `priority` when that goes first), else a
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

    /// Asks the checker about `target`. Its outcome settles `pending` for every caller waiting on it and is
    /// kept for the next request, unless the cache was invalidated since `generation`. Called without the cache's lock,
    /// since a provider may deliver at once.
    fn ask(&self, target: SemanticTarget, pending: &Arc<Pending>, key: Option<(String, u64)>, generation: u64) {
        let answers = self.answers.clone();
        let asked = target.key.clone();
        let settling = pending.clone();
        let delivery = Delivery::new(move |outcome| {
            let mut settled = match outcome {
                Some(outcome) => {
                    let Read { answered, once } = read_outcome(&asked, outcome);
                    Settled::Answered { answered, once }
                }
                None if settling.priority.is_cancelled() => Settled::Cancelled,
                None => Settled::Stopped,
            };
            if let Some(key) = key {
                let mut cache = answers.lock().unwrap_or_else(PoisonError::into_inner);
                if cache.pending.get(&key).is_some_and(|registered| Arc::ptr_eq(registered, &settling)) {
                    cache.pending.remove(&key);
                }
                if let Settled::Answered { answered, .. } = &mut settled
                    && cache.generation == generation
                {
                    answered.component = answered.component.take().map(|component| cache.shared(component));
                    cache.answers.insert(key, answered.clone());
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
        let used: Vec<&Arc<SemanticProp>> = component
            .props
            .iter()
            .filter(|prop| checker_prop_used(mode, &prop.name, prop.react_declared, &structural))
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
        react_wanted: react_props_wanted(mode, &answer.entry.props),
        layering: Some(layering),
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
    /// and is not cancelled. On the base nothing could cancel a request.
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
    /// registered, so a request for the component joins it, and its one delivery settles both callers. On the base the
    /// cancel removed the entry and the second request started a second conversion.
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
