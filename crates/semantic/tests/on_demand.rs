#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The warm checker against `replay-tsc` (the recording of the real `7.1.0-beta`): one process answers requests for
//! one component at a time, and a checker that cannot start says so and never fails a request.

use std::time::Duration;

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions, WatchSession};
use caliper_docgen_core::types::semantic::{SemanticOutcome, SemanticProvider, SemanticTarget};
use caliper_docgen_core::types::{DiagnosticSeverity, ExtractionOutput};
use caliper_docgen_semantic::{targets_of, OnDemand, OnDemandConfig};
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
fn a_project_without_typescript_answers_every_request_with_a_warning_and_no_components() {
    let project = project(false);
    let output = structural(&project);
    let checker = checker(&project);

    let first = ask(&checker, target(&output, "Button"));
    let second = ask(&checker, target(&output, "Other"));

    for outcome in [first, second] {
        assert!(outcome.components.is_empty());
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert!(
            outcome.diagnostics[0].message.contains("no node_modules/typescript/bin/tsc"),
            "{:?}",
            outcome.diagnostics
        );
    }
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

/// Against the real `tsc` (`7.1.0-beta`): set `CALIPER_TEST_TSC` to its `bin/tsc`. Skipped, loudly, when it is not set.
#[test]
fn the_real_checker_answers_the_requested_component() {
    let Some(tsc) = std::env::var_os("CALIPER_TEST_TSC") else {
        eprintln!("skipped: CALIPER_TEST_TSC does not name a typescript@7.1 beta `tsc`");
        return;
    };
    let project = project(false);
    let output = structural(&project);
    let mut config = OnDemandConfig::new(project.root.clone());
    config.tsc_path = Some(Utf8PathBuf::from(tsc.to_str().unwrap()));
    config.request_timeout = Duration::from_secs(60);
    let checker = OnDemand::new(config);

    let first = ask(&checker, target(&output, "Button"));
    let second = ask(&checker, target(&output, "Button"));

    assert_eq!(first.diagnostics, []);
    let names: Vec<_> = first.components[0].props.iter().map(|prop| prop.name.as_str()).collect();
    assert_eq!(names, ["label", "size", "disabled", "onClick", "tags", "pair", "box"]);
    assert_eq!(second.components[0].props.len(), 7, "the same warm process answers again");
}

/// With the real checker and the real `@types/react` (the repository's own, via `node_modules/@types/react`), a
/// component's prop names are the same with the checker as without it, in every `htmlAttributes` mode: the checker
/// improves types and optionality, and `htmlAttributes` means one thing in both engines.
#[test]
fn the_real_checker_lists_the_same_prop_names_as_the_structural_pass_in_every_html_attributes_mode() {
    let Some(tsc) = std::env::var_os("CALIPER_TEST_TSC") else {
        eprintln!("skipped: CALIPER_TEST_TSC does not name a typescript@7.1 beta `tsc`");
        return;
    };
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
        ("WithChildren.tsx", "import type { PropsWithChildren } from \"react\";\nexport function WithChildren(props: PropsWithChildren<{ label: string }>) { return null; }\n"),
        ("OwnChildren.tsx", "import type { ReactNode } from \"react\";\nexport function OwnChildren(props: { label: string; children?: ReactNode }) { return null; }\n"),
        ("Div.tsx", "import type { HTMLAttributes } from \"react\";\nexport interface DivProps extends HTMLAttributes<HTMLDivElement> { label: string }\nexport function Div(props: DivProps) { return null; }\n"),
    ];
    for (name, source) in sources {
        std::fs::write(root.join(name), source).unwrap();
    }

    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated, HtmlAttributeMode::None] {
        let options =
            || PipelineOptions { src_dirs: vec![root.clone()], html_attributes: mode, ..PipelineOptions::default() };
        let structural = WatchSession::lazy(options());
        let _ = structural.initialize();
        let mut config = OnDemandConfig::new(root.clone());
        config.tsc_path = Some(Utf8PathBuf::from(tsc.to_str().unwrap()));
        config.request_timeout = Duration::from_secs(60);
        let checked = WatchSession::lazy(options())
            .with_semantic(std::sync::Arc::new(OnDemand::new(config)), Duration::from_secs(60));
        let _ = checked.initialize();

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
