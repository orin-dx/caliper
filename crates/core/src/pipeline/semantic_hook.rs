//! Asking a checker for the component a session was just asked about (ADR 0022).
//!
//! The structural answer is always computed first and is a complete answer on its own. With a [`SemanticProvider`]
//! attached, [`SemanticHook::refine`] asks the checker for exactly that component, waits a bounded time, and merges
//! its props over the structural ones. A checker that is slower than the budget leaves the structural answer in
//! place, and its late answer is kept for the next request of the same, unedited component.

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
    merge_semantic_props, SemanticComponent, SemanticOutcome, SemanticProvider, SemanticTarget,
};

/// What the checker said about one component at one version of its file.
type Answers = Mutex<FxHashMap<(String, u64), Option<Arc<SemanticComponent>>>>;

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
        Self { provider, wait, answers: Arc::new(Mutex::new(FxHashMap::default())), warned: AtomicBool::new(false) }
    }

    /// `answer` with the checker's props, when it has them.
    pub(crate) fn refine(&self, mut answer: ComponentAnswer, mode: HtmlAttributeMode) -> ComponentAnswer {
        let stamp = std::fs::read(&answer.entry.file_path).map(|bytes| {
            let mut hasher = FxHasher::default();
            bytes.hash(&mut hasher);
            hasher.finish()
        });
        let cache_key = stamp.ok().map(|stamp| (answer.key.clone(), stamp));

        if let Some(cached) = cache_key.as_ref().and_then(|key| self.cached(key)) {
            self.apply(&mut answer, cached.as_ref(), mode);
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
                let mut reported = Vec::new();
                let component = self.take_component(&answer, outcome, &mut reported);
                answer.diagnostics.extend(reported);
                if let Some(key) = cache_key {
                    self.store(key, component.clone());
                }
                self.apply(&mut answer, component.as_ref(), mode);
            }
            Err(RecvTimeoutError::Timeout) => {
                self.keep_late_answer(receiver, cache_key);
                let mut pending = Diagnostic::new(
                    DiagnosticSeverity::Info,
                    DiagnosticCode::SemanticBackend,
                    "the TypeScript checker has not answered yet; the structural props are used, and the checker's \
                     replace them on the next request",
                );
                pending.affects = vec![answer.key.clone()];
                answer.diagnostics.push(pending);
            }
            Err(RecvTimeoutError::Disconnected) => {}
        }
        answer
    }

    fn cached(&self, key: &(String, u64)) -> Option<Option<Arc<SemanticComponent>>> {
        self.answers.lock().unwrap_or_else(PoisonError::into_inner).get(key).cloned()
    }

    fn store(&self, key: (String, u64), component: Option<Arc<SemanticComponent>>) {
        self.answers.lock().unwrap_or_else(PoisonError::into_inner).insert(key, component);
    }

    /// Waits for an answer that missed its budget on a thread of its own, so the next request finds it.
    fn keep_late_answer(&self, receiver: Receiver<SemanticOutcome>, cache_key: Option<(String, u64)>) {
        let Some(key) = cache_key else { return };
        let answers = self.answers.clone();
        std::thread::Builder::new()
            .name("caliper-late-answer".to_string())
            .spawn(move || {
                if let Ok(outcome) = receiver.recv() {
                    let component = outcome.components.into_iter().next().map(Arc::new);
                    answers.lock().unwrap_or_else(PoisonError::into_inner).insert(key, component);
                }
            })
            .ok();
    }

    /// The component the checker answered for, with its diagnostics (an unavailable checker's, once; a fallback's
    /// reason) recorded on `diagnostics`.
    fn take_component(
        &self,
        answer: &ComponentAnswer,
        outcome: SemanticOutcome,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<Arc<SemanticComponent>> {
        let SemanticOutcome { components, fallbacks, diagnostics: reported } = outcome;
        for mut diagnostic in reported {
            if diagnostic.severity == DiagnosticSeverity::Warning && self.warned.swap(true, Ordering::Relaxed) {
                continue;
            }
            diagnostic.affects = vec![answer.key.clone()];
            diagnostics.push(diagnostic);
        }
        for fallback in fallbacks {
            let mut info = Diagnostic::new(
                DiagnosticSeverity::Info,
                DiagnosticCode::SemanticBackend,
                format!("1 component(s) kept their structural result: {}", fallback.reason),
            );
            info.affects = vec![answer.key.clone()];
            diagnostics.push(info);
        }
        components
            .into_iter()
            .find(|found| found.file_path == answer.entry.file_path && found.display_name == answer.entry.display_name)
            .map(Arc::new)
    }

    fn apply(&self, answer: &mut ComponentAnswer, component: Option<&Arc<SemanticComponent>>, mode: HtmlAttributeMode) {
        let Some(component) = component else { return };
        let structural = answer.entry.props.clone();
        let used =
            component.props.iter().filter(|prop| checker_prop_used(mode, &prop.name, prop.react_declared, &structural));
        let kept = merge_semantic_props(&mut answer.entry, used, &mut FxHashMap::default());
        // A structural diagnostic about a prop the checker now answers for would be stale.
        answer.diagnostics.retain(|diagnostic| {
            !matches!(
                diagnostic.code,
                DiagnosticCode::OpaqueType | DiagnosticCode::MaxDepthExceeded | DiagnosticCode::CircularReference
            )
        });
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
    }
}
