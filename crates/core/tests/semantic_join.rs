#![allow(clippy::unwrap_used, clippy::expect_used)]
//! One question to the checker per component and version of its file: a later caller joins the request already in
//! flight, a background request (warm-up, fill, prefetch) is asked at background priority and waited for by nobody, and a
//! story that joins it raises it. The checker here holds every request until the test answers it, so each step is
//! observed rather than timed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use caliper_docgen_core::pipeline::{ComponentAnswer, PipelineOptions, WatchSession};
use caliper_docgen_core::types::semantic::{
    Delivery, Priority, SemanticComponent, SemanticOutcome, SemanticProp, SemanticProvider, SemanticTarget,
    SharedPriority,
};
use caliper_docgen_core::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity, PropType};
use camino::Utf8PathBuf;

const BUTTON: &str = "export function Button(props: { label: string }) { return null; }";
const CARD: &str = "export function Card(props: { title: string }) { return null; }";
const BADGE: &str = "export function Badge(props: { tone: string }) { return null; }";
const NOT_TOLD: &str = "the TypeScript checker was not told that types.ts changed";
const PENDING: &str = "the TypeScript checker has not answered yet; the structural props are used, and the checker's \
                       replace them on the next request";

/// One request the checker holds: what was asked, at which priority, and where its answer goes.
struct Held {
    target: SemanticTarget,
    priority: SharedPriority,
    delivery: Option<Delivery>,
}

/// Holds every request until the test answers it.
#[derive(Default)]
struct Queued(Mutex<Vec<Held>>);

impl Queued {
    fn requests(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// The key and the priority now of every request, in the order they were asked.
    fn asked(&self) -> Vec<(String, Priority)> {
        self.0.lock().unwrap().iter().map(|held| (held.target.key.clone(), held.priority.get())).collect()
    }

    /// Answers request `n` with `reply` of its target, or drops it unanswered when `reply` is `None`.
    fn answer(&self, n: usize, reply: Option<fn(&SemanticTarget) -> SemanticOutcome>) {
        let (target, delivery) = {
            let mut held = self.0.lock().unwrap();
            (held[n].target.clone(), held[n].delivery.take().unwrap())
        };
        match reply {
            Some(reply) => delivery.deliver(Some(reply(&target))),
            None => drop(delivery),
        }
    }

    fn wait_until(&self, done: impl Fn(&[(String, Priority)]) -> bool) {
        while !done(&self.asked()) {
            std::thread::yield_now();
        }
    }
}

impl SemanticProvider for Queued {
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
        let target = targets.into_iter().next().unwrap();
        self.0.lock().unwrap().push(Held { target, priority, delivery: Some(delivery) });
    }
}

fn outcome(target: &SemanticTarget, names: &[&str], diagnostics: Vec<Diagnostic>) -> SemanticOutcome {
    SemanticOutcome {
        components: vec![SemanticComponent {
            key: target.key.clone(),
            props: names
                .iter()
                .map(|name| {
                    Arc::new(SemanticProp {
                        name: (*name).to_string(),
                        prop_type: PropType::String,
                        required: false,
                        description: String::new(),
                        tags: Default::default(),
                        parent: None,
                        declarations: Vec::new(),
                        react_declared: false,
                    })
                })
                .collect(),
            composes: None,
        }],
        fallbacks: Vec::new(),
        diagnostics,
    }
}

fn finds_size(target: &SemanticTarget) -> SemanticOutcome {
    outcome(target, &["label", "size"], Vec::new())
}

fn finds_tone(target: &SemanticTarget) -> SemanticOutcome {
    outcome(target, &["label", "tone"], Vec::new())
}

fn warns_of_an_unannounced_edit(target: &SemanticTarget) -> SemanticOutcome {
    outcome(
        target,
        &["label", "size"],
        vec![Diagnostic::new(DiagnosticSeverity::Warning, DiagnosticCode::SemanticBackend, NOT_TOLD)],
    )
}

fn down(_: &SemanticTarget) -> SemanticOutcome {
    SemanticOutcome::unavailable("no tsc in this test")
}

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

fn project(files: &[(&str, &str)]) -> Project {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    Project { _dir: dir, root }
}

fn session(project: &Project, checker: &Arc<Queued>, wait: Duration) -> Arc<WatchSession> {
    let session = WatchSession::lazy(PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() })
        .with_semantic(checker.clone(), wait);
    drop(session.initialize());
    Arc::new(session)
}

fn names(answer: &ComponentAnswer) -> Vec<String> {
    answer.entry.props.keys().cloned().collect()
}

fn messages(answer: &ComponentAnswer) -> Vec<&str> {
    answer.diagnostics.iter().map(|d| d.message.as_str()).collect()
}

/// Asks for `key` on a thread of its own, as a story the user opened.
fn open_story(session: &Arc<WatchSession>, key: &'static str) -> std::thread::JoinHandle<ComponentAnswer> {
    let session = session.clone();
    std::thread::spawn(move || session.component(key).unwrap())
}

#[test]
fn background_requests_are_asked_at_background_priority_and_waited_for_by_nobody() {
    let project = project(&[("Button.tsx", BUTTON), ("Card.tsx", CARD), ("Badge.tsx", BADGE)]);
    let checker = Arc::new(Queued::default());
    let session = session(&project, &checker, Duration::from_secs(60));

    let prefetched = session.components_in_background(&["Button".to_string(), "Card".to_string()]);

    // Nothing is answered, and the call returned: it waited for neither.
    let prefetched: Vec<ComponentAnswer> = prefetched.into_iter().map(Option::unwrap).collect();
    assert_eq!(names(&prefetched[0]), ["label"]);
    assert_eq!(names(&prefetched[1]), ["title"]);
    assert_eq!(messages(&prefetched[0]), [PENDING]);
    let story = open_story(&session, "Badge");
    checker.wait_until(|asked| asked.len() == 3);
    assert_eq!(
        checker.asked(),
        [
            ("Button".to_string(), Priority::Background),
            ("Card".to_string(), Priority::Background),
            ("Badge".to_string(), Priority::Interactive),
        ]
    );
    checker.answer(2, Some(finds_tone));
    assert_eq!(names(&story.join().unwrap()), ["label", "tone"]);
}

/// A story asked while its own prefetch is in flight completes when the prefetch does: one request, raised to the
/// story's priority, never a second one queued behind the first.
#[test]
fn a_story_joins_its_prefetch_and_raises_it_to_interactive() {
    let project = project(&[("Button.tsx", BUTTON), ("Card.tsx", CARD)]);
    let checker = Arc::new(Queued::default());
    let session = session(&project, &checker, Duration::from_secs(60));
    drop(session.components_in_background(&["Card".to_string(), "Button".to_string()]));

    let story = open_story(&session, "Button");
    checker.wait_until(|asked| asked[1].1 == Priority::Interactive);
    assert_eq!(
        checker.asked(),
        [("Card".to_string(), Priority::Background), ("Button".to_string(), Priority::Interactive)]
    );
    checker.answer(1, Some(warns_of_an_unannounced_edit));
    let answer = story.join().unwrap();

    assert_eq!(names(&answer), ["label", "size"]);
    assert_eq!(answer.diagnostics.iter().filter(|d| d.message == NOT_TOLD).count(), 1, "{:?}", answer.diagnostics);
    assert_eq!(checker.requests(), 2, "the story asked again");
    let again = session.component("Button").unwrap();
    assert_eq!(again, answer, "the joined answer is kept, with what the checker said");
    assert_eq!(checker.requests(), 2);
}

#[test]
fn a_background_answer_nobody_waited_for_serves_the_next_story_with_what_the_checker_said() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let checker = Arc::new(Queued::default());
    let session = session(&project, &checker, Duration::from_millis(10));
    drop(session.components_in_background(&["Button".to_string()]));

    checker.answer(0, Some(warns_of_an_unannounced_edit));
    let answer = session.component("Button").unwrap();

    assert_eq!(names(&answer), ["label", "size"]);
    assert_eq!(answer.diagnostics.iter().filter(|d| d.message == NOT_TOLD).count(), 1, "{:?}", answer.diagnostics);
    assert_eq!(checker.requests(), 1);
}

/// An answer to a question asked before an edit describes the files before it: a request after the edit asks again,
/// whether the edit gave the component's own file a new hash or changed only a file it imports.
#[test]
fn a_request_after_an_edit_does_not_join_the_request_asked_before_it() {
    const BUTTON_FROM_TYPES: &str =
        "import type { P } from \"./types\";\nexport function Button(props: P) { return null; }";
    for edited in ["Button.tsx", "types.ts"] {
        let project =
            project(&[("Button.tsx", BUTTON_FROM_TYPES), ("types.ts", "export interface P { label: string }")]);
        let checker = Arc::new(Queued::default());
        let session = session(&project, &checker, Duration::from_millis(10));
        drop(session.components_in_background(&["Button".to_string()]));

        let file = project.root.join(edited);
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, format!("{text}\n// edited\n")).unwrap();
        session.update_file(&file);
        let after = session.component("Button").unwrap();

        assert_eq!(checker.requests(), 2, "{edited}: the request after the edit joined the one before it");
        assert_eq!(messages(&after), [PENDING], "{edited}");
        checker.answer(0, Some(finds_size));
        checker.answer(1, Some(finds_tone));
        assert_eq!(names(&session.component("Button").unwrap()), ["label", "tone"], "{edited}: the stale answer won");
        assert_eq!(checker.requests(), 2, "{edited}");
    }
}

#[test]
fn an_unavailable_checker_is_reported_once_to_the_story_that_joined_the_request() {
    let project = project(&[("Button.tsx", BUTTON), ("Card.tsx", CARD)]);
    let checker = Arc::new(Queued::default());
    let session = session(&project, &checker, Duration::from_secs(60));
    drop(session.components_in_background(&["Button".to_string()]));
    let story = open_story(&session, "Button");
    checker.wait_until(|asked| asked[0].1 == Priority::Interactive);

    checker.answer(0, Some(down));
    let joined = story.join().unwrap();
    let card = open_story(&session, "Card");
    checker.wait_until(|asked| asked.len() == 2);
    checker.answer(1, Some(down));
    let card = card.join().unwrap();

    let warnings = |answer: &ComponentAnswer| {
        answer.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Warning).count()
    };
    assert_eq!((warnings(&joined), warnings(&card)), (1, 0));
    assert_eq!(names(&joined), ["label"]);
}

#[test]
fn a_joined_request_the_checker_drops_says_so_and_is_asked_again_next_time() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let checker = Arc::new(Queued::default());
    let session = session(&project, &checker, Duration::from_secs(60));
    drop(session.components_in_background(&["Button".to_string()]));
    let story = open_story(&session, "Button");
    checker.wait_until(|asked| asked[0].1 == Priority::Interactive);

    checker.answer(0, None);
    let answer = story.join().unwrap();

    assert_eq!(names(&answer), ["label"]);
    assert_eq!(messages(&answer), ["the TypeScript checker stopped without answering; the structural props are used"]);
    let next = open_story(&session, "Button");
    checker.wait_until(|asked| asked.len() == 2);
    checker.answer(1, Some(finds_size));
    assert_eq!(names(&next.join().unwrap()), ["label", "size"]);
}

/// An eager session's components are already resolved; asking them in the background still asks the checker at that
/// priority, as a lazy session does.
#[test]
fn an_eager_session_asks_in_the_background_too() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let checker = Arc::new(Queued::default());
    let session = WatchSession::new(PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() })
        .with_semantic(checker.clone(), Duration::from_secs(60));
    drop(session.initialize());

    let prefetched = session.components_in_background(&["Button".to_string()]);

    assert_eq!(names(prefetched[0].as_ref().unwrap()), ["label"]);
    assert_eq!(checker.asked(), [("Button".to_string(), Priority::Background)]);
}
