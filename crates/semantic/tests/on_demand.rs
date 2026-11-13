#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The warm checker: one session of the built-in checker answers requests for one component at a time.

use std::time::Duration;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession, extract};
use caliper_docgen_core::types::semantic::{
    Delivery, Priority, SemanticOutcome, SemanticProvider, SemanticTarget, SharedPriority,
};
use caliper_docgen_core::types::{DiagnosticSeverity, ExtractionOutput, PropType};
use caliper_docgen_semantic::{OnDemand, OnDemandConfig, targets_of};
use camino::Utf8PathBuf;

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// The fixture project in a scratch directory.
fn project() -> Project {
    let dir = tempfile::Builder::new().prefix("OnDemand").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["Button.tsx", "tsconfig.json"] {
        std::fs::copy(fixtures.join("project").join(name), root.join(name)).unwrap();
    }
    Project { _dir: dir, root }
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

fn checker(root: &Utf8PathBuf) -> OnDemand {
    let mut config = OnDemandConfig::new(root.clone());
    config.request_timeout = Duration::from_secs(60);
    OnDemand::new(config)
}

fn target(output: &ExtractionOutput, name: &str) -> SemanticTarget {
    targets_of(output).into_iter().find(|target| target.display_name == name).unwrap()
}

fn ask(checker: &OnDemand, target: SemanticTarget) -> SemanticOutcome {
    checker.request(vec![target]).recv_timeout(Duration::from_secs(60)).expect("the checker answers")
}

fn ask_in_background(checker: &OnDemand, target: SemanticTarget) -> SemanticOutcome {
    let (sender, receiver) = std::sync::mpsc::channel();
    let delivery = Delivery::new(move |outcome| {
        sender.send(outcome).ok();
    });
    checker.request_at(vec![target], SharedPriority::new(Priority::Background), delivery);
    receiver.recv_timeout(Duration::from_secs(60)).expect("a delivery").expect("the checker answers")
}

/// The built-in checker answers in this process, for a project with no `typescript` of its own.
#[test]
fn one_warm_checker_answers_the_component_it_is_asked_about_and_then_the_next() {
    let project = project();
    let output = structural(&project);
    let checker = checker(&project.root);
    checker.warm_up();

    let button = ask(&checker, target(&output, "Button"));
    let other = ask(&checker, target(&output, "Other"));

    assert_eq!(button.diagnostics, []);
    assert_eq!(button.components.len(), 1);
    let names: Vec<_> = button.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    // Declaration order; the merge into the output sorts them.
    assert_eq!(names, ["label", "size", "disabled", "onClick", "tags", "pair", "box"]);
    assert_eq!(other.components.len(), 1);
    let names: Vec<_> = other.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["a"]);
}

/// The checker is never replaced: a fresh session would parse every file again while this process keeps most of what
/// the old one held, so it would hold more, not less. One session keeps its snapshot through 66 requests, whatever
/// priority each was asked at.
#[test]
fn the_checker_is_not_replaced_however_many_programs_it_builds() {
    let project = project();
    let output = structural(&project);
    let checker = checker(&project.root);

    for round in 0..66 {
        let answered = match round % 2 {
            0 => ask(&checker, target(&output, "Button")),
            _ => ask_in_background(&checker, target(&output, "Button")),
        };
        assert_eq!((answered.diagnostics.len(), answered.components.len()), (0, 1), "request {round}");
    }

    assert_eq!(checker.snapshots().created, 1, "one session kept its snapshot throughout");
}

/// What the checker holds is bounded: past the bound, it stays down, and every later request keeps its structural
/// answer with a warning that says why. Its arenas hold the TypeScript lib at least, so one program passes a bound of
/// zero, and the first check is after the first program.
#[test]
fn a_checker_past_its_memory_bound_stays_down_and_says_why() {
    let project = project();
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.request_timeout = Duration::from_secs(60);
    config.max_rss_mib = 0;
    let checker = OnDemand::new(config);

    let first = ask(&checker, target(&output, "Button"));
    let later: Vec<SemanticOutcome> = ["Button", "Other"].map(|name| ask(&checker, target(&output, name))).into();

    // The answer the bound was read after is the checker's: a correct answer, which cost more than the bound.
    assert_eq!((first.diagnostics.len(), first.components.len()), (0, 1));
    for outcome in later {
        assert!(outcome.components.is_empty());
        assert_eq!(outcome.diagnostics.len(), 1);
        let diagnostic = &outcome.diagnostics[0];
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
        let message = diagnostic.message.as_str();
        let held = message.strip_prefix("TypeScript checker backend unavailable: the built-in checker held ").and_then(
            |rest| {
                rest.strip_suffix(
                    " MiB, more than its 0 MiB bound (`max_rss_mib`), and this process keeps that memory, so \
                     components keep their structural answer for the rest of the session",
                )
            },
        );
        assert!(held.is_some_and(|mib| mib.parse::<u64>().is_ok_and(|mib| mib > 0)), "{message}");
    }
}

#[test]
fn dropping_the_checker_stops_its_thread() {
    let project = project();
    let output = structural(&project);
    let checker = checker(&project.root);
    let answered = ask(&checker, target(&output, "Button"));
    assert_eq!(answered.components.len(), 1);

    // Drop joins the worker, which stops the checker first; a hang here is the failure.
    drop(checker);
}

/// The checker escapes a name that starts with two underscores (`__size` is `___size` inside it); the answer carries
/// the name the source writes.
#[test]
fn the_checker_names_props_as_the_source_writes() {
    let dir = tempfile::Builder::new().prefix("Markers").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markers");
    for name in ["Markers.tsx", "tsconfig.json"] {
        std::fs::copy(fixtures.join(name), root.join(name)).unwrap();
    }
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let checker = checker(&root);

    let answer = ask(&checker, target(&output, "Markers"));

    assert_eq!(answer.diagnostics, []);
    let names: Vec<_> = answer.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["_single", "__size", "___triple", "plain"]);
}

/// The checker prints `(percent?: number, ...rest: string[])` and says which signatures have a rest parameter; the
/// answer keeps both markers, and a parameter typed `string | undefined` is not optional.
#[test]
fn the_checker_keeps_optional_and_rest_parameters() {
    let dir = tempfile::Builder::new().prefix("Markers").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markers");
    for name in ["Markers.tsx", "tsconfig.json"] {
        std::fs::copy(fixtures.join(name), root.join(name)).unwrap();
    }
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let checker = checker(&root);

    let answer = ask(&checker, target(&output, "Callbacks"));

    assert_eq!(answer.diagnostics, []);
    let markers =
        |name: &str| match &answer.components[0].props.iter().find(|prop| prop.name == name).unwrap().prop_type {
            PropType::Function { params, .. } => params.iter().map(|p| (p.optional, p.rest)).collect::<Vec<_>>(),
            other => panic!("{name} is not a function: {other:?}"),
        };
    assert_eq!(markers("format"), [(true, false), (false, true)]);
    assert_eq!(markers("close"), [(false, true)]);
    assert_eq!(markers("onClick"), [(true, false)]);
    assert_eq!(markers("plain"), [(false, false), (true, false)]);
    assert_eq!(markers("explicit"), [(false, false), (false, false)]);
}

/// With the real `@types/react` (the repository's own, via `node_modules/@types/react`), a component's prop names are
/// the same with the checker as without it, in every `htmlAttributes` mode: the checker improves types and
/// optionality, and `htmlAttributes` means one thing in both engines.
#[test]
fn the_checker_lists_the_same_prop_names_as_the_structural_pass_in_every_html_attributes_mode() {
    let react = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/@types/react");
    if !react.exists() {
        eprintln!("skipped: {react} is not installed");
        return;
    }
    let react = Utf8PathBuf::from_path_buf(std::fs::canonicalize(&react).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::create_dir_all(root.join("node_modules/@types")).unwrap();
    std::os::unix::fs::symlink(&react, root.join("node_modules/@types/react")).unwrap();
    std::fs::copy(
        Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project/tsconfig.json"),
        root.join("tsconfig.json"),
    )
    .unwrap();
    let sources = [
        (
            "WithChildren.tsx",
            "import type { PropsWithChildren } from \"react\";\nexport function WithChildren(props: PropsWithChildren<{ label: string }>) { return null; }\n",
        ),
        (
            "OwnChildren.tsx",
            "import type { ReactNode } from \"react\";\nexport function OwnChildren(props: { label: string; children?: ReactNode }) { return null; }\n",
        ),
        (
            "Div.tsx",
            "import type { HTMLAttributes } from \"react\";\nexport interface DivProps extends HTMLAttributes<HTMLDivElement> { label: string }\nexport function Div(props: DivProps) { return null; }\n",
        ),
    ];
    for (name, source) in sources {
        std::fs::write(root.join(name), source).unwrap();
    }

    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated, HtmlAttributeMode::None] {
        let options =
            || PipelineOptions { src_dirs: vec![root.clone()], html_attributes: mode, ..PipelineOptions::default() };
        let structural = WatchSession::lazy(options());
        drop(structural.initialize());
        let checked =
            WatchSession::lazy(options()).with_semantic(std::sync::Arc::new(checker(&root)), Duration::from_secs(60));
        drop(checked.initialize());

        for key in ["WithChildren", "OwnChildren", "Div"] {
            let before = structural.component(key).unwrap();
            let after = checked.component(key).unwrap();
            assert!(
                after.diagnostics.iter().any(|d| d.message.contains("resolved by the TypeScript checker")),
                "{key} under {mode:?} was not answered by the checker: {:?}",
                after.diagnostics
            );
            let names = |answer: &caliper_docgen_core::pipeline::ComponentAnswer| -> Vec<String> {
                answer.entry.props.keys().cloned().collect()
            };
            assert_eq!(names(&after), names(&before), "{key} under {mode:?}");
            assert_eq!(
                after.entry.notable_inherited.keys().collect::<Vec<_>>(),
                before.entry.notable_inherited.keys().collect::<Vec<_>>(),
                "{key} under {mode:?}: the curated attributes are the structural pass's"
            );
            if mode == HtmlAttributeMode::Full && key == "Div" {
                assert!(names(&before).iter().any(|name| name == "className"), "Full lists the real attributes");
            }
        }
    }
}

/// Creating the checker and `warm_up` return at once: the checker starts on the worker thread.
#[test]
fn creating_and_warming_a_checker_does_not_block() {
    let project = project();
    let output = structural(&project);
    let started = std::time::Instant::now();
    let checker = checker(&project.root);
    checker.warm_up();
    assert!(started.elapsed() < Duration::from_millis(500), "creating and warming took {:?}", started.elapsed());

    let answered = ask(&checker, target(&output, "Button"));
    assert_eq!((answered.diagnostics.len(), answered.components.len()), (0, 1));
}
