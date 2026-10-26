#![allow(clippy::unwrap_used, clippy::expect_used)]
//! A session with a checker attached asks it for the component a request names (ADR 0022): the checker's props when it
//! answers in time, the structural ones when it does not, its late answer on the next request, and one warning when it
//! is unavailable. The checker here is a fake, so the contract is tested without `tsc`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, WatchSession};
use caliper_docgen_core::types::semantic::{
    SemanticComponent, SemanticOutcome, SemanticProp, SemanticProvider, SemanticTarget,
};
use caliper_docgen_core::types::{DiagnosticCode, DiagnosticSeverity, PropType};
use camino::Utf8PathBuf;

const BUTTON: &str = "export function Button(props: { label: string }) { return null; }";
const CARD: &str = "export function Card(props: { title: string }) { return null; }";

/// Answers every request with `reply(target)`, after `delay`, and counts the requests.
struct Fake {
    requests: AtomicUsize,
    delay: Duration,
    reply: fn(&SemanticTarget) -> SemanticOutcome,
}

impl Fake {
    fn new(delay: Duration, reply: fn(&SemanticTarget) -> SemanticOutcome) -> Arc<Self> {
        Arc::new(Self { requests: AtomicUsize::new(0), delay, reply })
    }
    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl SemanticProvider for Fake {
    fn request(&self, targets: Vec<SemanticTarget>) -> Receiver<SemanticOutcome> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = channel();
        let (delay, reply) = (self.delay, self.reply);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            sender.send(reply(&targets[0])).ok();
        });
        receiver
    }
}

/// The checker finds a prop the structural pass cannot see: `size`, typed as a union of two literals.
fn finds_size(target: &SemanticTarget) -> SemanticOutcome {
    let prop = |name: &str, prop_type: PropType| {
        Arc::new(SemanticProp { name: name.to_string(), prop_type, required: false, description: String::new() })
    };
    SemanticOutcome {
        components: vec![SemanticComponent {
            file_path: target.file_path.clone(),
            display_name: target.display_name.clone(),
            props: vec![
                prop("label", PropType::String),
                prop(
                    "size",
                    PropType::Union(vec![PropType::StringLiteral("sm".into()), PropType::StringLiteral("lg".into())]),
                ),
            ],
        }],
        ..SemanticOutcome::default()
    }
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

fn session(project: &Project, checker: Arc<Fake>, wait: Duration) -> WatchSession {
    let session = WatchSession::lazy(PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() })
        .with_semantic(checker, wait);
    let _ = session.initialize();
    session
}

fn prop_names(session: &WatchSession, key: &str) -> Vec<String> {
    session.component(key).unwrap().entry.props.keys().cloned().collect()
}

#[test]
fn a_checker_that_answers_in_time_replaces_the_structural_props_and_says_so() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let checker = Fake::new(Duration::ZERO, finds_size);
    let session = session(&project, checker.clone(), Duration::from_secs(5));

    let answer = session.component("Button").unwrap();

    assert_eq!(answer.entry.props.keys().cloned().collect::<Vec<_>>(), ["label", "size"]);
    assert_eq!(answer.entry.props["size"].prop_type.raw_string(), "\"sm\" | \"lg\"");
    let provenance: Vec<_> = answer.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SemanticBackend).collect();
    assert_eq!(provenance.len(), 1);
    assert_eq!(provenance[0].severity, DiagnosticSeverity::Info);
    assert_eq!(provenance[0].affects, ["Button"]);
}

#[test]
fn the_checker_is_asked_once_per_version_of_the_file() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let checker = Fake::new(Duration::ZERO, finds_size);
    let session = session(&project, checker.clone(), Duration::from_secs(5));

    let first = session.component("Button").unwrap();
    let second = session.component("Button").unwrap();
    assert_eq!(first, second);
    assert_eq!(checker.requests(), 1);

    std::fs::write(
        project.root.join("Button.tsx"),
        "export function Button(props: { label: string; tone?: string }) { return null; }",
    )
    .unwrap();
    session.update_file(&project.root.join("Button.tsx"));
    let edited = session.component("Button").unwrap();

    assert_eq!(checker.requests(), 2, "an edited file is a new question");
    assert_eq!(edited.entry.props.keys().cloned().collect::<Vec<_>>(), ["label", "size"]);
}

#[test]
fn a_slow_checker_leaves_the_structural_props_and_its_late_answer_serves_the_next_request() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let checker = Fake::new(Duration::from_millis(300), finds_size);
    let session = session(&project, checker.clone(), Duration::from_millis(10));

    let early = session.component("Button").unwrap();
    assert_eq!(early.entry.props.keys().cloned().collect::<Vec<_>>(), ["label"]);
    let pending: Vec<_> = early.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SemanticBackend).collect();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].message.contains("has not answered yet"), "{}", pending[0].message);

    std::thread::sleep(Duration::from_millis(900));

    assert_eq!(prop_names(&session, "Button"), ["label", "size"]);
    assert_eq!(checker.requests(), 1, "the late answer was kept, not asked for again");
}

#[test]
fn an_unavailable_checker_warns_once_per_session_and_never_fails_a_request() {
    let project = project(&[("Button.tsx", BUTTON), ("Card.tsx", CARD)]);
    let checker = Fake::new(Duration::ZERO, down);
    let session = session(&project, checker, Duration::from_secs(5));

    let button = session.component("Button").unwrap();
    let card = session.component("Card").unwrap();

    assert_eq!(button.entry.props.keys().cloned().collect::<Vec<_>>(), ["label"]);
    assert_eq!(card.entry.props.keys().cloned().collect::<Vec<_>>(), ["title"]);
    let warnings = [&button, &card]
        .iter()
        .flat_map(|answer| answer.diagnostics.iter())
        .filter(|d| d.severity == DiagnosticSeverity::Warning && d.code == DiagnosticCode::SemanticBackend)
        .count();
    assert_eq!(warnings, 1);
}

#[test]
fn a_session_without_a_checker_answers_as_before() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let plain = WatchSession::lazy(PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() });
    let _ = plain.initialize();

    let answer = plain.component("Button").unwrap();

    assert_eq!(answer.entry.props.keys().cloned().collect::<Vec<_>>(), ["label"]);
    assert!(answer.diagnostics.iter().all(|d| d.code != DiagnosticCode::SemanticBackend));
}
