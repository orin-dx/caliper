//! Asking a checker for the component a session was just asked about (ADR 0022).
//!
//! The structural answer is always computed first and is a complete answer on its own. With a [`SemanticProvider`]
//! attached, [`SemanticHook::refine`] asks the checker for exactly that component, waits a bounded time, and merges
//! its props over the structural ones. A checker that is slower than the budget leaves the structural answer in
//! place, and its late answer is kept for the next request of the same, unedited component.
//!
//! An answer describes the component's props as the checker read them, which depends on every file the component
//! imports, not only its own. So the session drops every answer when any source file changes
//! ([`SemanticHook::invalidate`]), and an answer that was still on its way then is not kept. The same call tells the
//! checker which files changed, since a warm one answers from what it read before (ADR 0026 S6).

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHasher};

use super::ComponentAnswer;
use crate::pipeline::HtmlAttributeMode;
use crate::react_types::{checker_prop_used, react_props_wanted};
use crate::types::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use crate::types::semantic::{
    MergedProps, SemanticComponent, SemanticOutcome, SemanticProp, SemanticProvider, SemanticTarget, SourceChange,
    merge_semantic_props, unread_diagnostic,
};

/// What one request of the checker made of a component: the component it found, if any, and what the checker said
/// about the request itself (an edit it was not told about, why a component kept its structural result). Both are the
/// answer, so a late answer and every request the cache serves it to carry both.
#[derive(Clone, Default)]
struct Answered {
    component: Option<Arc<SemanticComponent>>,
    diagnostics: Vec<Diagnostic>,
}

/// What the checker said about one component at one version of its file, and the generation they belong to.
#[derive(Default)]
struct AnswerCache {
    /// Raised by every [`SemanticHook::invalidate`]; an answer asked for under an earlier one is not stored.
    generation: u64,
    answers: FxHashMap<(String, u64), Answered>,
}

type Answers = Mutex<AnswerCache>;

pub(crate) struct SemanticHook {
    provider: Arc<dyn SemanticProvider>,
    /// How long a request waits for the checker before it takes the structural answer.
    wait: Duration,
    answers: Arc<Answers>,
    /// Set once the checker has been reported unavailable, so a session says so once, not per component.
    warned: AtomicBool,
}

impl SemanticHook {
    pub(crate) fn new(provider: Arc<dyn SemanticProvider>, wait: Duration) -> Self {
        Self { provider, wait, answers: Arc::new(Mutex::new(AnswerCache::default())), warned: AtomicBool::new(false) }
    }

    /// Forget every answer, refuse the ones still being waited for, and tell the checker what changed: a source file
    /// changed, and the checker's props for any component can depend on it. The next request of each component asks
    /// the checker again, after it has been told.
    pub(crate) fn invalidate(&self, change: SourceChange) {
        let mut cache = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        cache.generation += 1;
        cache.answers.clear();
        self.provider.changed(change);
    }

    /// `answer` with the checker's props, when it has them.
    pub(crate) fn refine(&self, mut answer: ComponentAnswer, mode: HtmlAttributeMode) -> ComponentAnswer {
        let stamp = std::fs::read(&answer.entry.file_path).map(|bytes| {
            let mut hasher = FxHasher::default();
            bytes.hash(&mut hasher);
            hasher.finish()
        });
        let cache_key = stamp.ok().map(|stamp| (answer.key.clone(), stamp));
        let generation = self.generation();

        if let Some(cached) = cache_key.as_ref().and_then(|key| self.cached(key)) {
            answer.diagnostics.extend(cached.diagnostics);
            self.apply(&mut answer, cached.component.as_ref(), mode);
            return answer;
        }

        let target = SemanticTarget {
            file_path: answer.entry.file_path.clone(),
            display_name: answer.entry.display_name.clone(),
            exported: answer.entry.exported,
            described: answer
                .entry
                .props
                .values()
                .filter(|prop| !prop.description.is_empty())
                .map(|prop| prop.name.clone())
                .collect(),
            react_wanted: react_props_wanted(mode, &answer.entry.props),
        };
        let receiver = self.provider.request(vec![target]);
        match receiver.recv_timeout(self.wait) {
            Ok(outcome) => {
                let Read { answered, once } = read_outcome(&Asked::from(&answer), outcome);
                for diagnostic in once {
                    if !self.warned.swap(true, Ordering::Relaxed) {
                        answer.diagnostics.push(diagnostic);
                    }
                }
                answer.diagnostics.extend(answered.diagnostics.iter().cloned());
                let component = answered.component.clone();
                if let Some(key) = cache_key {
                    self.store(key, generation, answered);
                }
                self.apply(&mut answer, component.as_ref(), mode);
            }
            Err(RecvTimeoutError::Timeout) => {
                self.keep_late_answer(receiver, &answer, cache_key, generation);
                let mut pending = Diagnostic::new(
                    DiagnosticSeverity::Info,
                    DiagnosticCode::SemanticBackend,
                    "the TypeScript checker has not answered yet; the structural props are used, and the checker's \
                     replace them on the next request",
                );
                pending.affects = vec![answer.key.clone()];
                answer.diagnostics.push(pending);
            }
            Err(RecvTimeoutError::Disconnected) => {
                let mut stopped = Diagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::SemanticBackend,
                    "the TypeScript checker stopped without answering; the structural props are used",
                );
                stopped.affects = vec![answer.key.clone()];
                answer.diagnostics.push(stopped);
            }
        }
        answer
    }

    fn generation(&self) -> u64 {
        self.answers.lock().unwrap_or_else(PoisonError::into_inner).generation
    }

    fn cached(&self, key: &(String, u64)) -> Option<Answered> {
        self.answers.lock().unwrap_or_else(PoisonError::into_inner).answers.get(key).cloned()
    }

    fn store(&self, key: (String, u64), generation: u64, answered: Answered) {
        store(&self.answers, key, generation, answered);
    }

    /// Waits for an answer that missed its budget on a thread of its own, so the next request finds it.
    fn keep_late_answer(
        &self,
        receiver: Receiver<SemanticOutcome>,
        answer: &ComponentAnswer,
        cache_key: Option<(String, u64)>,
        generation: u64,
    ) {
        let Some(key) = cache_key else { return };
        let answers = self.answers.clone();
        let asked = Asked::from(answer);
        std::thread::Builder::new()
            .name("caliper-late-answer".to_string())
            .spawn(move || {
                if let Ok(outcome) = receiver.recv() {
                    store(&answers, key, generation, read_outcome(&asked, outcome).answered);
                }
            })
            .ok();
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
        let MergedProps { kept, unread } =
            merge_semantic_props(&mut answer.entry, used.into_iter(), &mut FxHashMap::default());
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
        if unread > 0 {
            answer.diagnostics.push(unread_diagnostic(unread, vec![answer.key.clone()]));
        }
    }
}

/// Keeps `answered` for `key` unless the cache was invalidated since the question was asked.
fn store(answers: &Answers, key: (String, u64), generation: u64, answered: Answered) {
    let mut cache = answers.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.generation == generation {
        cache.answers.insert(key, answered);
    }
}

/// The component a request asked about, as a late answer's thread, which has no [`ComponentAnswer`], knows it.
struct Asked {
    key: String,
    file_path: camino::Utf8PathBuf,
    display_name: String,
}

impl From<&ComponentAnswer> for Asked {
    fn from(answer: &ComponentAnswer) -> Self {
        Self {
            key: answer.key.clone(),
            file_path: answer.entry.file_path.clone(),
            display_name: answer.entry.display_name.clone(),
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

/// The component the checker answered for, and the diagnostics the request produced: a warning that came with an
/// answer (an edit the checker was not told about) is about this request, and a fallback's reason says why the
/// component kept its structural result.
fn read_outcome(asked: &Asked, outcome: SemanticOutcome) -> Read {
    let SemanticOutcome { components, fallbacks, diagnostics: reported } = outcome;
    // Only an outcome with no answer at all says the checker is unavailable.
    let unavailable = components.is_empty() && fallbacks.is_empty();
    let mut read = Read { answered: Answered::default(), once: Vec::new() };
    for mut diagnostic in reported {
        diagnostic.affects = vec![asked.key.clone()];
        if unavailable && diagnostic.severity == DiagnosticSeverity::Warning {
            read.once.push(diagnostic);
        } else {
            read.answered.diagnostics.push(diagnostic);
        }
    }
    for fallback in fallbacks {
        let mut info = Diagnostic::new(
            DiagnosticSeverity::Info,
            DiagnosticCode::SemanticBackend,
            format!("1 component(s) kept their structural result: {}", fallback.reason),
        );
        info.affects = vec![asked.key.clone()];
        read.answered.diagnostics.push(info);
    }
    read.answered.component = components
        .into_iter()
        .find(|found| found.file_path == asked.file_path && found.display_name == asked.display_name)
        .map(Arc::new);
    read
}
