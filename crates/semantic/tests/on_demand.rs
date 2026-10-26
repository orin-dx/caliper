#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The warm checker against `replay-tsc` (the recording of the real `7.1.0-beta`): one process answers requests for
//! one component at a time, and a checker that cannot start says so and never fails a request.

use std::time::Duration;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
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
