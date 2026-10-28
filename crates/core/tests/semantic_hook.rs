#![allow(clippy::unwrap_used, clippy::expect_used)]
//! A session with a checker attached asks it for the component a request names (ADR 0022): the checker's props when it
//! answers in time, the structural ones when it does not, its late answer on the next request, and one warning when it
//! is unavailable. The checker here is a fake, so the contract is tested without `tsc`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession};
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
        Arc::new(SemanticProp {
            name: name.to_string(),
            prop_type,
            required: false,
            description: String::new(),
            react_declared: false,
        })
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

/// The checker reports the component's `label` and, as `tsc` does without a filter, everything `@types/react` declares:
/// `children` as a string (so the checker's type is visible), `className`, `id`, and the `key` and `ref` React adds.
fn reports_react_props(target: &SemanticTarget) -> SemanticOutcome {
    let prop = |name: &str, prop_type: PropType, react_declared: bool| {
        Arc::new(SemanticProp {
            name: name.to_string(),
            prop_type,
            required: false,
            description: String::new(),
            react_declared,
        })
    };
    SemanticOutcome {
        components: vec![SemanticComponent {
            file_path: target.file_path.clone(),
            display_name: target.display_name.clone(),
            props: vec![
                prop("label", PropType::String, false),
                prop("children", PropType::String, true),
                prop("className", PropType::String, true),
                prop("id", PropType::String, true),
                prop("key", PropType::String, true),
                prop("ref", PropType::String, true),
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

fn session_in(
    project: &Project,
    mode: HtmlAttributeMode,
    reply: fn(&SemanticTarget) -> SemanticOutcome,
) -> WatchSession {
    let options = PipelineOptions { src_dirs: vec![project.root.clone()], html_attributes: mode, ..Default::default() };
    let session = WatchSession::lazy(options).with_semantic(Fake::new(Duration::ZERO, reply), Duration::from_secs(5));
    let _ = session.initialize();
    session
}

fn structural_session(project: &Project, mode: HtmlAttributeMode) -> WatchSession {
    let options = PipelineOptions { src_dirs: vec![project.root.clone()], html_attributes: mode, ..Default::default() };
    let session = WatchSession::lazy(options);
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
    // The checker's fake reply does not know `tone`; the structural pass does, and a prop the checker does not return stays.
    assert_eq!(edited.entry.props.keys().cloned().collect::<Vec<_>>(), ["label", "size", "tone"]);
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
fn a_structural_prop_the_checker_does_not_return_stays_with_its_structural_type() {
    let project = project(&[(
        "Button.tsx",
        "interface Base { id?: string }\nexport function Button(props: Base & { label: string }) { return null; }",
    )]);
    let checker = Fake::new(Duration::ZERO, finds_size);
    let session = session(&project, checker, Duration::from_secs(5));

    let answer = session.component("Button").unwrap();

    assert_eq!(answer.entry.props.keys().cloned().collect::<Vec<_>>(), ["id", "label", "size"]);
    assert_eq!(answer.entry.props["id"].parent.as_ref().map(|parent| parent.name.as_str()), Some("Base"));
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

const REACT_PROP_COMPONENTS: [(&str, &str); 3] = [
    (
        "WithChildren.tsx",
        "import type { PropsWithChildren } from \"react\";\n\
         export function WithChildren(props: PropsWithChildren<{ label: string }>) { return null; }",
    ),
    (
        "OwnChildren.tsx",
        "import type { ReactNode } from \"react\";\n\
         export function OwnChildren(props: { label: string; children?: ReactNode }) { return null; }",
    ),
    (
        "Div.tsx",
        "import type { HTMLAttributes } from \"react\";\n\
         export interface DivProps extends HTMLAttributes<HTMLDivElement> { label: string }\n\
         export function Div(props: DivProps) { return null; }",
    ),
];

fn names(session: &WatchSession, key: &str) -> Vec<String> {
    prop_names(session, key)
}

#[test]
fn curated_and_none_take_the_checkers_react_props_only_where_the_structural_pass_lists_them() {
    let project = project(&REACT_PROP_COMPONENTS);
    for mode in [HtmlAttributeMode::Curated, HtmlAttributeMode::None] {
        let structural = structural_session(&project, mode);
        let checked = session_in(&project, mode, reports_react_props);
        for key in ["WithChildren", "OwnChildren", "Div"] {
            let before = names(&structural, key);
            let after = names(&checked, key);
            assert_eq!(after, before, "{key} under {mode:?}: the checker adds no name of its own here");
        }
        // Where `children` is a prop of the component, the checker's type wins.
        for key in ["WithChildren", "OwnChildren"] {
            let children = &checked.component(key).unwrap().entry.props["children"];
            assert_eq!(children.prop_type, PropType::String, "{key} under {mode:?}");
        }
    }
}

#[test]
fn full_takes_everything_the_checker_reports_except_key() {
    let project = project(&REACT_PROP_COMPONENTS);
    let checked = session_in(&project, HtmlAttributeMode::Full, reports_react_props);
    for key in ["WithChildren", "OwnChildren", "Div"] {
        let got = names(&checked, key);
        for wanted in ["label", "children", "className", "id", "ref"] {
            assert!(got.iter().any(|name| name == wanted), "{key} lacks {wanted}: {got:?}");
        }
        assert!(!got.iter().any(|name| name == "key"), "{key} lists key: {got:?}");
    }
}

#[test]
fn the_request_asks_only_for_the_react_props_the_mode_can_use() {
    let wanted = |mode| {
        let seen = Arc::new(std::sync::Mutex::new(None));
        struct Spy(Arc<std::sync::Mutex<Option<SemanticTarget>>>);
        impl SemanticProvider for Spy {
            fn request(&self, mut targets: Vec<SemanticTarget>) -> Receiver<SemanticOutcome> {
                *self.0.lock().unwrap() = targets.pop();
                channel().1
            }
        }
        let project = project(&REACT_PROP_COMPONENTS);
        let options =
            PipelineOptions { src_dirs: vec![project.root.clone()], html_attributes: mode, ..Default::default() };
        let session = WatchSession::lazy(options).with_semantic(Arc::new(Spy(seen.clone())), Duration::from_millis(1));
        let _ = session.initialize();
        let _ = session.component("WithChildren");
        let target = seen.lock().unwrap().take().unwrap();
        target.react_wanted
    };

    assert_eq!(wanted(HtmlAttributeMode::Full), None);
    let curated = wanted(HtmlAttributeMode::Curated).unwrap();
    assert!(curated.contains("children") && curated.contains("label"), "{curated:?}");
    assert!(!curated.contains("className"));
}
