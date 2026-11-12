#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The warm checker against `replay-tsc` (the recording of the real `7.1.0-beta`): one process answers requests for
//! one component at a time, and a checker that cannot start says so and never fails a request.

use std::time::Duration;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession, extract};
use caliper_docgen_core::types::semantic::{
    Delivery, Priority, SemanticOutcome, SemanticProvider, SemanticTarget, SharedPriority,
};
use caliper_docgen_core::types::{DiagnosticSeverity, ExtractionOutput, PropType};
use caliper_docgen_semantic::{OnDemand, OnDemandConfig, Recycles, targets_of};
use camino::Utf8PathBuf;

const FAKE_VERSION: &str = "7.1.0-test";

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// The fixture project in a scratch directory whose name has an upper-case letter (see `replay_tsc.rs`), with
/// `replay-tsc` installed as `node_modules/typescript/bin/tsc` when `with_tsc`.
fn project(with_tsc: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Replay").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["Button.tsx", "tsconfig.json"] {
        std::fs::copy(fixtures.join("project").join(name), root.join(name)).unwrap();
    }
    if with_tsc {
        let package = root.join("node_modules/typescript");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), format!(r#"{{"version": "{FAKE_VERSION}"}}"#)).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_replay-tsc"), package.join("bin/tsc")).unwrap();
    }
    Project { _dir: dir, root }
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

fn checker(project: &Project) -> OnDemand {
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
    config.expected_versions = &[FAKE_VERSION];
    config.request_timeout = Duration::from_secs(20);
    config
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/recording.json", env!("CARGO_MANIFEST_DIR"))));
    OnDemand::new(config)
}

fn target(output: &ExtractionOutput, name: &str) -> SemanticTarget {
    targets_of(output).into_iter().find(|target| target.display_name == name).unwrap()
}

fn ask(checker: &OnDemand, target: SemanticTarget) -> SemanticOutcome {
    checker.request(vec![target]).recv_timeout(Duration::from_secs(30)).expect("the checker answers")
}

fn ask_in_background(checker: &OnDemand, target: SemanticTarget) -> SemanticOutcome {
    let (sender, receiver) = std::sync::mpsc::channel();
    let delivery = Delivery::new(move |outcome| {
        sender.send(outcome).ok();
    });
    checker.request_at(vec![target], SharedPriority::new(Priority::Background), delivery);
    receiver.recv_timeout(Duration::from_secs(30)).expect("a delivery").expect("the checker answers")
}

/// A story asked while background requests wait is answered before them. `REPLAY_HOLD` keeps the checker starting
/// until every request is queued; the answers arrive in the order the one worker gives them.
#[test]
fn a_story_asked_behind_queued_background_requests_is_answered_before_them() {
    let project = project(true);
    let output = structural(&project);
    let hold = project.root.join("hold");
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
    config.expected_versions = &[FAKE_VERSION];
    config.request_timeout = Duration::from_secs(20);
    config
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/recording.json", env!("CARGO_MANIFEST_DIR"))));
    config.extra_env.push(("REPLAY_HOLD".to_string(), hold.to_string()));
    let checker = OnDemand::new(config);
    checker.warm_up();
    let (sender, answered) = std::sync::mpsc::channel();
    let queue = |name: &'static str, priority: Priority| {
        let sender = sender.clone();
        let delivery = Delivery::new(move |outcome| {
            sender.send((name, outcome)).ok();
        });
        checker.request_at(vec![target(&output, name)], SharedPriority::new(priority), delivery);
    };
    for _ in 0..3 {
        queue("Button", Priority::Background);
    }
    queue("Other", Priority::Interactive);

    std::fs::write(&hold, "").unwrap();
    let order: Vec<&str> = (0..4)
        .map(|_| {
            let (name, outcome) = answered.recv_timeout(Duration::from_secs(30)).expect("an answer");
            let outcome = outcome.expect("the checker answered");
            assert_eq!(outcome.components.len(), 1, "{name}: {:?}", outcome.diagnostics);
            name
        })
        .collect();

    assert_eq!(order, ["Other", "Button", "Button", "Button"]);
}

#[test]
fn one_warm_checker_answers_the_component_it_is_asked_about_and_then_the_next() {
    let project = project(true);
    let output = structural(&project);
    let checker = checker(&project);
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

#[test]
fn a_tsc_path_with_no_binary_answers_every_request_with_a_warning_and_no_components() {
    let project = project(false);
    let output = structural(&project);
    let checker = checker(&project);
    let missing = project.root.join("node_modules/typescript/bin/tsc");

    let first = ask(&checker, target(&output, "Button"));
    let second = ask(&checker, target(&output, "Other"));

    for outcome in [first, second] {
        assert!(outcome.components.is_empty());
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(
            outcome.diagnostics[0].message,
            format!(
                "TypeScript checker backend unavailable: no tsc binary at {missing}; pass --tsc-path (tscPath) a `tsc` \
                 of typescript@7.1 or a `tsrs` binary, or leave it out to use the built-in checker"
            )
        );
    }
}

/// Without a `tsc_path` the built-in checker answers, in this process: no process id, and a project with no
/// `typescript` of its own.
#[test]
fn without_a_tsc_path_the_built_in_checker_answers_one_component_and_then_the_next() {
    let project = project(false);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.request_timeout = Duration::from_secs(60);
    let checker = OnDemand::new(config);
    checker.warm_up();

    let button = ask(&checker, target(&output, "Button"));
    let other = ask(&checker, target(&output, "Other"));

    assert_eq!(button.diagnostics, []);
    let names: Vec<_> = button.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["label", "size", "disabled", "onClick", "tags", "pair", "box"]);
    let names: Vec<_> = other.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["a"]);
    assert_eq!(checker.process_id(), None);
}

/// The built-in checker is replaced by a fresh session after its 64 programs, as a process is; the memory bound, which
/// would count this whole process, is not applied to it.
#[test]
fn the_built_in_checker_is_replaced_after_its_programs_and_never_for_memory() {
    let project = project(false);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.request_timeout = Duration::from_secs(60);
    config.max_rss_mib = 0;
    let checker = OnDemand::new(config);

    for round in 0..65 {
        let answered = ask(&checker, target(&output, "Button"));
        assert_eq!((answered.diagnostics.len(), answered.components.len()), (0, 1), "request {round}");
    }

    assert_eq!(checker.recycles(), Recycles { worn: 1, bloated: 0 });
}

#[test]
fn dropping_the_checker_stops_its_process_and_thread() {
    let project = project(true);
    let output = structural(&project);
    let checker = checker(&project);
    let answered = ask(&checker, target(&output, "Button"));
    assert_eq!(answered.components.len(), 1);

    // Drop joins the worker, which shuts `tsc` down first; a hang here is the failure.
    drop(checker);
}

/// The real checker the tests below run against: the Go `typescript@7.1` as a process when `CALIPER_TEST_TSC` names its
/// `bin/tsc`, else the built-in one.
fn real_tsc() -> Option<Utf8PathBuf> {
    std::env::var("CALIPER_TEST_TSC").ok().filter(|tsc| !tsc.is_empty()).map(Utf8PathBuf::from)
}

/// Against a real checker (see [`real_tsc`]).
#[test]
fn the_real_checker_answers_the_requested_component() {
    let tsc = real_tsc();
    let project = project(false);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = tsc.clone();
    config.request_timeout = Duration::from_secs(60);
    let checker = OnDemand::new(config);

    let first = ask(&checker, target(&output, "Button"));
    let second = ask(&checker, target(&output, "Button"));

    assert_eq!(first.diagnostics, []);
    let names: Vec<_> = first.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["label", "size", "disabled", "onClick", "tags", "pair", "box"]);
    assert_eq!(second.components[0].props.len(), 7, "the same warm process answers again");
}

/// The real checker escapes a name that starts with two underscores (`__size` is `___size` inside it); the answer
/// carries the name the source writes.
#[test]
fn the_real_checker_names_props_as_the_source_writes() {
    let tsc = real_tsc();
    let dir = tempfile::Builder::new().prefix("Real").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markers");
    for name in ["Markers.tsx", "tsconfig.json"] {
        std::fs::copy(fixtures.join(name), root.join(name)).unwrap();
    }
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut config = OnDemandConfig::new(root.clone());
    config.tsc_path = tsc.clone();
    config.request_timeout = Duration::from_secs(60);
    let checker = OnDemand::new(config);

    let answer = ask(&checker, target(&output, "Markers"));

    assert_eq!(answer.diagnostics, []);
    let names: Vec<_> = answer.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["_single", "__size", "___triple", "plain"]);
}

/// The real checker prints `(percent?: number, ...rest: string[])` and says which signatures have a rest parameter; the
/// answer keeps both markers, and a parameter typed `string | undefined` is not optional.
#[test]
fn the_real_checker_keeps_optional_and_rest_parameters() {
    let tsc = real_tsc();
    let dir = tempfile::Builder::new().prefix("Real").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markers");
    for name in ["Markers.tsx", "tsconfig.json"] {
        std::fs::copy(fixtures.join(name), root.join(name)).unwrap();
    }
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut config = OnDemandConfig::new(root.clone());
    config.tsc_path = tsc.clone();
    config.request_timeout = Duration::from_secs(60);
    let checker = OnDemand::new(config);

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

/// With the real checker and the real `@types/react` (the repository's own, via `node_modules/@types/react`), a
/// component's prop names are the same with the checker as without it, in every `htmlAttributes` mode: the checker
/// improves types and optionality, and `htmlAttributes` means one thing in both engines.
#[test]
fn the_real_checker_lists_the_same_prop_names_as_the_structural_pass_in_every_html_attributes_mode() {
    let tsc = real_tsc();
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
        let mut config = OnDemandConfig::new(root.clone());
        config.tsc_path = tsc.clone();
        config.request_timeout = Duration::from_secs(60);
        let checked = WatchSession::lazy(options())
            .with_semantic(std::sync::Arc::new(OnDemand::new(config)), Duration::from_secs(60));
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

#[test]
fn a_checker_past_its_memory_bound_is_replaced_between_requests_and_keeps_answering() {
    let project = project(true);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
    config.expected_versions = &[FAKE_VERSION];
    config.request_timeout = Duration::from_secs(20);
    // Any running process is past a bound of zero, so every fourth request replaces `tsc`.
    config.max_rss_mib = 0;
    config
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/recording.json", env!("CARGO_MANIFEST_DIR"))));
    let checker = OnDemand::new(config);

    let mut processes = Vec::new();
    for round in 0..9 {
        let answered = ask(&checker, target(&output, "Button"));
        assert_eq!(answered.diagnostics, [], "request {round}");
        assert_eq!(answered.components.len(), 1, "request {round}");
        processes.push(checker.process_id().expect("a request was answered"));
    }

    // The checks fall after the fourth and eighth replies, so requests 0 to 3, 4 to 7 and 8 each have a process.
    assert_eq!(checker.recycles(), Recycles { worn: 0, bloated: 2 });
    assert_eq!(processes[..4], [processes[0]; 4]);
    assert_eq!(processes[4..8], [processes[4]; 4]);
    assert!(processes[0] != processes[4] && processes[4] != processes[8], "{processes:?}");
}

#[test]
fn a_checker_that_has_built_its_programs_is_replaced_and_counted_as_worn() {
    let project = project(true);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
    config.expected_versions = &[FAKE_VERSION];
    config.request_timeout = Duration::from_secs(20);
    config.max_rss_mib = u64::MAX;
    config
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/recording.json", env!("CARGO_MANIFEST_DIR"))));
    let checker = OnDemand::new(config);
    assert_eq!((checker.recycles(), checker.process_id()), (Recycles::default(), None));

    // 64 programs is one process's allowance, so the 65th request is the first answered by a second process, whatever
    // priority each was asked at.
    let mut last = None;
    for round in 0..65 {
        let answered = match round % 2 {
            0 => ask(&checker, target(&output, "Button")),
            _ => ask_in_background(&checker, target(&output, "Button")),
        };
        assert_eq!(answered.components.len(), 1, "request {round}");
        let process = checker.process_id();
        assert_eq!(process != last, round == 0 || round == 64, "request {round}");
        last = process;
    }

    assert_eq!(checker.recycles(), Recycles { worn: 1, bloated: 0 });
}

#[test]
fn dropping_a_checker_stuck_in_a_request_does_not_wait_for_it() {
    let project = project(true);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
    config.expected_versions = &[FAKE_VERSION];
    config.request_timeout = Duration::from_secs(60);
    config.shutdown_wait = Duration::from_millis(300);
    config.extra_env.push(("REPLAY_SCENARIO".to_string(), "hang".to_string()));
    let checker = OnDemand::new(config);
    // The process starts and never answers; the request is still in flight when the checker is dropped.
    let _pending = checker.request(vec![target(&output, "Button")]);
    std::thread::sleep(Duration::from_millis(300));

    let started = std::time::Instant::now();
    drop(checker);

    assert!(started.elapsed() < Duration::from_secs(5), "dropping took {:?}", started.elapsed());
}

#[test]
fn a_reply_that_is_not_json_is_a_warning_at_once_not_a_wait_for_the_timeout() {
    let project = project(true);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
    config.expected_versions = &[FAKE_VERSION];
    config.request_timeout = Duration::from_secs(20);
    config.extra_env.push(("REPLAY_SCENARIO".to_string(), "invalid_json".to_string()));
    config
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/recording.json", env!("CARGO_MANIFEST_DIR"))));
    let checker = OnDemand::new(config);

    let answered = ask(&checker, target(&output, "Button"));

    // A timeout would be a different diagnostic than `InvalidJson`.
    assert!(answered.components.is_empty());
    assert_eq!(answered.diagnostics.len(), 1, "{:?}", answered.diagnostics);
    assert_eq!(answered.diagnostics[0].severity, DiagnosticSeverity::Warning);
    assert!(answered.diagnostics[0].message.contains("InvalidJson"), "{:?}", answered.diagnostics);
}

#[test]
fn creating_a_checker_and_asking_for_a_missing_tsc_does_not_block() {
    let project = project(false);
    let output = structural(&project);
    let started = std::time::Instant::now();
    let checker = checker(&project);
    checker.warm_up();
    assert!(started.elapsed() < Duration::from_millis(500), "creating and warming took {:?}", started.elapsed());

    let answered = ask(&checker, target(&output, "Button"));
    assert!(answered.components.is_empty());
    assert_eq!(answered.diagnostics.len(), 1);
}
