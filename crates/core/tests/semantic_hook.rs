#![allow(clippy::unwrap_used, clippy::expect_used)]
//! A session with a checker attached asks it for the component a request names (ADR 0022): the checker's props when it
//! answers in time, the structural ones when it does not, its late answer on the next request, and one warning when it
//! is unavailable. The checker here is a fake, so the contract is tested without `tsc`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
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
    drop(session.initialize());
    session
}

fn session_in(
    project: &Project,
    mode: HtmlAttributeMode,
    reply: fn(&SemanticTarget) -> SemanticOutcome,
) -> WatchSession {
    let options = PipelineOptions { src_dirs: vec![project.root.clone()], html_attributes: mode, ..Default::default() };
    let session = WatchSession::lazy(options).with_semantic(Fake::new(Duration::ZERO, reply), Duration::from_secs(5));
    drop(session.initialize());
    session
}

fn structural_session(project: &Project, mode: HtmlAttributeMode) -> WatchSession {
    let options = PipelineOptions { src_dirs: vec![project.root.clone()], html_attributes: mode, ..Default::default() };
    let session = WatchSession::lazy(options);
    drop(session.initialize());
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

/// The checker could not read `label` (an import it did not resolve): its answer there is a placeholder.
fn cannot_read_label(target: &SemanticTarget) -> SemanticOutcome {
    SemanticOutcome {
        components: vec![SemanticComponent {
            file_path: target.file_path.clone(),
            display_name: target.display_name.clone(),
            props: vec![Arc::new(SemanticProp {
                name: "label".to_string(),
                prop_type: caliper_docgen_core::types::semantic::unresolved_type("Label"),
                required: true,
                description: String::new(),
                react_declared: false,
            })],
        }],
        ..SemanticOutcome::default()
    }
}

#[test]
fn a_prop_the_checker_could_not_read_keeps_its_structural_type_marked_in_a_session() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let session = session(&project, Fake::new(Duration::ZERO, cannot_read_label), Duration::from_secs(5));

    let answer = session.component("Button").unwrap();

    assert_eq!(answer.entry.props["label"].prop_type, PropType::String);
    assert_eq!(answer.entry.props["label"].degraded_by, ["unresolvableImport"]);
    let unread: Vec<_> = answer.diagnostics.iter().filter(|d| d.message.contains("could not read")).collect();
    assert_eq!(unread.len(), 1, "{:?}", answer.diagnostics);
    assert_eq!(unread[0].affects, ["Button"]);
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
    drop(plain.initialize());

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
        drop(session.initialize());
        let _ = session.component("WithChildren");
        let target = seen.lock().unwrap().take().unwrap();
        target.react_wanted
    };

    assert_eq!(wanted(HtmlAttributeMode::Full), None);
    let curated = wanted(HtmlAttributeMode::Curated).unwrap();
    assert!(curated.contains("children") && curated.contains("label"), "{curated:?}");
    assert!(!curated.contains("className"));
}

const BUTTON_FROM_TYPES: &str =
    "import type { P } from \"./types\";\nexport function Button(props: P) { return null; }";
const TYPES_ONE_PROP: &str = "export interface P { label: string }";
const TYPES_TWO_PROPS: &str = "export interface P { label: string; tone: string }";

/// Stands in for a checker that reads the component's dependency: every prop `types.ts` declares comes back typed
/// `"v<N>"`, where N is how many props `types.ts` declares, so the answer says which version of the file it saw.
fn reads_types_file(target: &SemanticTarget) -> SemanticOutcome {
    let text = std::fs::read_to_string(target.file_path.parent().unwrap().join("types.ts")).unwrap();
    types_file_outcome(target, &text)
}

fn types_file_outcome(target: &SemanticTarget, text: &str) -> SemanticOutcome {
    let names: Vec<String> = text
        .split(['{', '}', ';'])
        .filter_map(|part| part.split_once(':').map(|(name, _)| name.trim().to_string()))
        .collect();
    let version = format!("v{}", names.len());
    SemanticOutcome {
        components: vec![SemanticComponent {
            file_path: target.file_path.clone(),
            display_name: target.display_name.clone(),
            props: names
                .into_iter()
                .map(|name| {
                    Arc::new(SemanticProp {
                        name,
                        prop_type: PropType::StringLiteral(version.clone()),
                        required: false,
                        description: String::new(),
                        react_declared: false,
                    })
                })
                .collect(),
        }],
        ..SemanticOutcome::default()
    }
}

fn label_type(session: &WatchSession) -> PropType {
    session.component("Button").unwrap().entry.props["label"].prop_type.clone()
}

fn edit_types_file(project: &Project) -> Utf8PathBuf {
    let types = project.root.join("types.ts");
    std::fs::write(&types, TYPES_TWO_PROPS).unwrap();
    types
}

#[test]
fn an_edit_to_a_file_the_component_imports_is_a_new_question_for_the_checker() {
    let project = project(&[("Button.tsx", BUTTON_FROM_TYPES), ("types.ts", TYPES_ONE_PROP)]);
    let checker = Fake::new(Duration::ZERO, reads_types_file);
    let session = session(&project, checker.clone(), Duration::from_secs(5));
    assert_eq!(label_type(&session), PropType::StringLiteral("v1".into()));
    assert_eq!(checker.requests(), 1);

    let types = edit_types_file(&project);
    session.update_file(&types);

    assert_eq!(label_type(&session), PropType::StringLiteral("v2".into()), "the checker's type is the old file's");
    assert_eq!(checker.requests(), 2, "Button.tsx is unchanged but its props are not");
    assert_eq!(prop_names(&session, "Button"), ["label", "tone"]);
}

#[test]
fn ensure_fresh_asks_the_checker_again_after_an_imported_file_changed() {
    let project = project(&[("Button.tsx", BUTTON_FROM_TYPES), ("types.ts", TYPES_ONE_PROP)]);
    let checker = Fake::new(Duration::ZERO, reads_types_file);
    let session = session(&project, checker.clone(), Duration::from_secs(5));
    assert_eq!(label_type(&session), PropType::StringLiteral("v1".into()));

    edit_types_file(&project);
    session.ensure_fresh();

    assert_eq!(label_type(&session), PropType::StringLiteral("v2".into()), "the checker's type is the old file's");
    assert_eq!(checker.requests(), 2);
}

/// Holds every request until the test answers it.
struct Gated {
    requests: AtomicUsize,
    held: std::sync::Mutex<Vec<std::sync::mpsc::Sender<SemanticOutcome>>>,
}

impl SemanticProvider for Gated {
    fn request(&self, _: Vec<SemanticTarget>) -> Receiver<SemanticOutcome> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = channel();
        self.held.lock().unwrap().push(sender);
        receiver
    }
}

#[test]
fn an_answer_that_arrives_after_an_edit_is_not_kept_for_the_edited_project() {
    let project = project(&[("Button.tsx", BUTTON_FROM_TYPES), ("types.ts", TYPES_ONE_PROP)]);
    let gated = Arc::new(Gated { requests: AtomicUsize::new(0), held: std::sync::Mutex::default() });
    let session = WatchSession::lazy(PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() })
        .with_semantic(gated.clone(), Duration::from_millis(10));
    drop(session.initialize());
    let early = session.component("Button").unwrap();
    assert_eq!(early.entry.props.keys().cloned().collect::<Vec<_>>(), ["label"], "the structural answer");

    let types = edit_types_file(&project);
    session.update_file(&types);
    // The answer to the question asked before the edit: it describes `types.ts` as it was.
    let target = SemanticTarget {
        file_path: project.root.join("Button.tsx"),
        display_name: "Button".to_string(),
        exported: true,
        described: Default::default(),
        react_wanted: None,
    };
    let stale = types_file_outcome(&target, TYPES_ONE_PROP);
    gated.held.lock().unwrap().pop().unwrap().send(stale).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let _ = session.component("Button");
    assert_eq!(gated.requests.load(Ordering::SeqCst), 2, "the stale answer must not satisfy the next request");
}

const STILL_OPAQUE: &str =
    "export function Button<T>(props: { label: string; item: T extends string ? 1 : 2 }) { return null; }";

fn opaque_type_diagnostics(answer: &caliper_docgen_core::pipeline::ComponentAnswer) -> Vec<Vec<String>> {
    answer.diagnostics.iter().filter(|d| d.code == DiagnosticCode::OpaqueType).map(|d| d.affects.clone()).collect()
}

/// The checker answers for `label` and `item`: nothing the structural pass left opaque is left over.
fn answers_every_prop(target: &SemanticTarget) -> SemanticOutcome {
    let prop = |name: &str, prop_type: PropType| {
        Arc::new(SemanticProp {
            name: name.to_string(),
            prop_type,
            required: true,
            description: String::new(),
            react_declared: false,
        })
    };
    SemanticOutcome {
        components: vec![SemanticComponent {
            file_path: target.file_path.clone(),
            display_name: target.display_name.clone(),
            props: vec![prop("label", PropType::String), prop("item", PropType::StringLiteral("a".into()))],
        }],
        ..SemanticOutcome::default()
    }
}

#[test]
fn a_degrade_diagnostic_stays_while_a_structural_prop_the_checker_did_not_return_is_still_opaque() {
    let project = project(&[("Button.tsx", STILL_OPAQUE)]);
    let structural = structural_session(&project, HtmlAttributeMode::Full).component("Button").unwrap();
    assert_eq!(opaque_type_diagnostics(&structural), [["Button"]], "the structural pass reports the opaque prop");

    // The checker returns `label` and `size`, not `item`: `item` keeps its structural, still opaque, type.
    let checked = session_in(&project, HtmlAttributeMode::Full, finds_size).component("Button").unwrap();

    assert!(matches!(checked.entry.props["item"].prop_type, PropType::Opaque(_)), "item keeps its structural type");
    assert_eq!(opaque_type_diagnostics(&checked), [["Button"]], "the diagnostic that explains `item` must stay");
}

#[test]
fn a_degrade_diagnostic_goes_once_the_checker_has_answered_for_every_prop() {
    let project = project(&[("Button.tsx", STILL_OPAQUE)]);

    let checked = session_in(&project, HtmlAttributeMode::Full, answers_every_prop).component("Button").unwrap();

    assert!(!matches!(checked.entry.props["item"].prop_type, PropType::Opaque(_)));
    assert_eq!(opaque_type_diagnostics(&checked), Vec::<Vec<String>>::new());
}

const DEFAULTED: &str =
    "export function Button({ label = \"ok\" }: { label?: string; legacy?: boolean }) { return null; }";

/// The checker returns `label` (a different type from the source's `string`), a prop of its own (`size`), and one of
/// `@types/react`'s (`className`).
fn label_size_and_class_name(target: &SemanticTarget) -> SemanticOutcome {
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
                prop("label", PropType::StringLiteral("ok".into()), false),
                prop("size", PropType::Number, false),
                prop("className", PropType::String, true),
            ],
        }],
        ..SemanticOutcome::default()
    }
}

/// The CLI merges the checker over a whole extraction; a session merges it over one component. They are two paths to
/// one answer, so what they agree on is pinned here and what they do not is pinned as the difference it is.
#[test]
fn the_whole_project_merge_and_the_session_merge_agree_on_every_prop_the_checker_answered() {
    let project = project(&[("Button.tsx", DEFAULTED)]);
    let options = PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() };

    let mut batch = caliper_docgen_core::extract(&options);
    let target = SemanticTarget {
        file_path: batch.components["Button"].file_path.clone(),
        display_name: "Button".to_string(),
        exported: true,
        described: Default::default(),
        react_wanted: None,
    };
    caliper_docgen_core::types::semantic::merge_semantic(&mut batch, label_size_and_class_name(&target));
    let batch = &batch.components["Button"];

    let session = session_in(&project, HtmlAttributeMode::Full, label_size_and_class_name);
    let on_demand = session.component("Button").unwrap().entry;

    // What both agree on: the checker's type and optionality for a prop it answered, and the structural default.
    for name in ["label", "size"] {
        assert_eq!(batch.props[name], on_demand.props[name], "{name}");
    }
    assert_eq!(batch.props["label"].prop_type, PropType::StringLiteral("ok".into()));
    assert!(batch.props["label"].default_value.is_some(), "the structural default stays");

    // Where they differ, on purpose: the whole-project merge lets the checker decide which props exist and leaves
    // `@types/react`'s to the inheritance layers; the session keeps a structural prop the checker did not return and
    // takes the `@types/react` prop its `htmlAttributes` mode lists.
    assert_eq!(batch.props.keys().cloned().collect::<Vec<_>>(), ["label", "size"]);
    assert_eq!(on_demand.props.keys().cloned().collect::<Vec<_>>(), ["className", "label", "legacy", "size"]);
}

/// A provider that drops its sender without answering, as a checker whose worker died does.
struct Hangup;

impl SemanticProvider for Hangup {
    fn request(&self, _targets: Vec<SemanticTarget>) -> Receiver<SemanticOutcome> {
        channel().1
    }
}

#[test]
fn a_checker_that_disconnects_without_answering_leaves_the_structural_props_and_says_so() {
    let project = project(&[("Button.tsx", BUTTON)]);
    let session = WatchSession::lazy(PipelineOptions { src_dirs: vec![project.root.clone()], ..Default::default() })
        .with_semantic(Arc::new(Hangup), Duration::from_secs(5));
    drop(session.initialize());

    let answer = session.component("Button").unwrap();

    assert_eq!(answer.entry.props.keys().cloned().collect::<Vec<_>>(), ["label"]);
    let notes: Vec<_> = answer.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SemanticBackend).collect();
    assert_eq!(notes.len(), 1, "{:?}", answer.diagnostics);
    assert_eq!(notes[0].severity, DiagnosticSeverity::Warning);
    assert_eq!(notes[0].message, "the TypeScript checker stopped without answering; the structural props are used");
    assert_eq!(notes[0].affects, ["Button"]);
}
