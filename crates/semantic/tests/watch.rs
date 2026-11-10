#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! A warm checker kept across edits (ADR 0026 S6), against a real checker: `tsrs` when `TSRS_BIN` names its binary, else
//! the Go `typescript@7.1` when `CALIPER_TEST_TSC` names its `bin/tsc`. Without either every test returns at once, and on
//! CI (`CI` is set) that is a failure.
//!
//! An edit the session announces is read from the kept snapshot brought up to date; one nobody announces is found
//! before the checker answers from the old text, and the answer comes from the disk with a warning naming the file.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use caliper_docgen_core::pipeline::{ComponentAnswer, PipelineOptions, WatchSession};
use caliper_docgen_core::types::semantic::{SemanticOutcome, SemanticProvider, SemanticTarget, SourceChange};
use caliper_docgen_core::types::{DiagnosticSeverity, ExtractionOutput};
use caliper_docgen_semantic::{OnDemand, OnDemandConfig, Snapshots, targets_of};
use camino::{Utf8Path, Utf8PathBuf};

fn real_checker() -> Option<Utf8PathBuf> {
    let checker = ["TSRS_BIN", "CALIPER_TEST_TSC"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()));
    if checker.is_none() {
        assert!(std::env::var_os("CI").is_none(), "on CI, TSRS_BIN or CALIPER_TEST_TSC must name a real checker");
        eprintln!("skipped: neither TSRS_BIN nor CALIPER_TEST_TSC names a real checker");
    }
    checker.map(Utf8PathBuf::from)
}

const BUTTON: &str = "import type { Shared } from \"./shared\";\nexport function Button(props: Shared & { label: string }) { return null; }\n";
const CARD: &str = "export function Card(props: { title: string }) { return null; }\n";

fn shared(version: usize) -> String {
    format!("export interface Shared {{ shared{version}: string }}\n")
}

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

impl Project {
    /// `Button.tsx`, which takes its props from `shared.ts`, and `Card.tsx`, all written an hour ago: a stamp taken
    /// within two seconds of an edit cannot tell it from the next, and a checker reads such a file again unasked.
    fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("Watch").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let fixture = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project/tsconfig.json");
        std::fs::copy(fixture, root.join("tsconfig.json")).unwrap();
        let project = Self { _dir: dir, root };
        for (name, text) in
            [("Button.tsx", BUTTON.to_string()), ("Card.tsx", CARD.to_string()), ("shared.ts", shared(0))]
        {
            project.write_settled(name, &text);
        }
        project
    }

    fn file(&self, name: &str) -> Utf8PathBuf {
        self.root.join(name)
    }

    fn write_settled(&self, name: &str, text: &str) {
        std::fs::write(self.file(name), text).unwrap();
        let file = std::fs::File::options().write(true).open(self.file(name)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
    }

    fn checker(&self, tsc: &Utf8Path) -> Arc<OnDemand> {
        let mut config = OnDemandConfig::new(self.root.clone());
        config.tsconfig = Some(self.file("tsconfig.json"));
        config.tsc_path = Some(tsc.to_owned());
        config.request_timeout = Duration::from_secs(60);
        Arc::new(OnDemand::new(config))
    }

    fn structural(&self) -> ExtractionOutput {
        caliper_docgen_core::pipeline::extract(&PipelineOptions {
            src_dirs: vec![self.root.clone()],
            ..PipelineOptions::default()
        })
    }
}

fn target(output: &ExtractionOutput, name: &str) -> SemanticTarget {
    targets_of(output).into_iter().find(|target| target.display_name == name).unwrap()
}

fn ask(checker: &OnDemand, target: SemanticTarget) -> SemanticOutcome {
    let outcome = checker.request(vec![target]).recv_timeout(Duration::from_secs(60)).expect("the checker answers");
    // The kept snapshot's files are stamped after the reply; an empty request queued behind it returns once they are.
    drop(checker.request(Vec::new()).recv_timeout(Duration::from_secs(60)));
    outcome
}

fn prop_names(outcome: &SemanticOutcome) -> Vec<&str> {
    outcome.components[0].props.iter().map(|prop| prop.name.as_str()).collect()
}

fn warnings(outcome: &SemanticOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}

#[test]
fn an_edit_the_checker_is_told_about_is_read_into_the_kept_snapshot() {
    let Some(tsc) = real_checker() else { return };
    let project = Project::new();
    let output = project.structural();
    let checker = project.checker(&tsc);
    assert_eq!(prop_names(&ask(&checker, target(&output, "Button"))), ["shared0", "label"]);

    project.write_settled("shared.ts", &shared(1));
    checker.changed(SourceChange::Files(vec![project.file("shared.ts")]));
    let edited = ask(&checker, target(&output, "Button"));
    let other_file = ask(&checker, target(&output, "Card"));

    assert_eq!(warnings(&edited), Vec::<&str>::new());
    assert_eq!(prop_names(&edited), ["shared1", "label"]);
    assert_eq!(prop_names(&other_file), ["title"]);
    assert_eq!(
        checker.snapshots(),
        Snapshots { created: 1, updated: 2 },
        "the edit, then the program re-rooted at Card"
    );
}

/// ADR 0026 S6's gate: an edit nobody tells the checker about fails loudly, naming the file, and is not answered from
/// the text the checker read before.
#[test]
fn an_edit_the_checker_is_not_told_about_fails_loudly_and_is_answered_from_the_disk() {
    let Some(tsc) = real_checker() else { return };
    let project = Project::new();
    let output = project.structural();
    let checker = project.checker(&tsc);
    assert_eq!(prop_names(&ask(&checker, target(&output, "Button"))), ["shared0", "label"]);

    project.write_settled("shared.ts", &shared(1));
    let edited = ask(&checker, target(&output, "Button"));

    let warned = warnings(&edited);
    assert_eq!(warned.len(), 1, "{:?}", edited.diagnostics);
    assert!(warned[0].contains("was not told") && warned[0].contains(project.file("shared.ts").as_str()), "{warned:?}");
    assert_eq!(prop_names(&edited), ["shared1", "label"], "answered from the disk, not the old snapshot");
    assert_eq!(checker.snapshots(), Snapshots { created: 2, updated: 0 });
}

/// A generic component is read at its default from a second snapshot over the kept one's program (S5); on a snapshot
/// brought up to date the answer is the one the first snapshot gave. The fixture is read where it is, so `react`
/// resolves to the workspace's `@types/react`.
#[test]
fn a_generic_component_on_an_updated_snapshot_is_answered_as_on_a_fresh_one() {
    let Some(tsc) = real_checker() else { return };
    let root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic");
    let output = caliper_docgen_core::pipeline::extract(&PipelineOptions {
        src_dirs: vec![root.clone()],
        ..PipelineOptions::default()
    });
    let mut config = OnDemandConfig::new(root.clone());
    config.tsconfig = Some(root.join("tsconfig.json"));
    config.tsc_path = Some(tsc);
    config.request_timeout = Duration::from_secs(60);
    let checker = OnDemand::new(config);

    let poly = SemanticTarget { react_wanted: None, ..target(&output, "Poly") };
    let fresh = ask(&checker, poly.clone());
    checker.changed(SourceChange::Files(vec![root.join("Poly.tsx")]));
    let updated = ask(&checker, poly);

    assert!(prop_names(&fresh).len() >= 260, "{} props", prop_names(&fresh).len());
    assert_eq!(updated, fresh);
    assert_eq!(checker.snapshots(), Snapshots { created: 1, updated: 1 });
}

fn session(project: &Project, checker: Arc<OnDemand>) -> WatchSession {
    let options = PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() };
    let session = WatchSession::lazy(options).with_semantic(checker, Duration::from_secs(60));
    drop(session.initialize());
    session
}

fn by_checker(answer: &ComponentAnswer) -> bool {
    answer.diagnostics.iter().any(|d| d.message.contains("resolved by the TypeScript checker"))
}

/// Through the session a Storybook addon runs: each edit applied with `update_file` reaches the checker, so no answer
/// warns, and after the edits every answer equals a fresh session's, in either order of asking.
#[test]
fn a_session_tells_its_checker_every_edit_and_answers_as_a_fresh_one_does() {
    let Some(tsc) = real_checker() else { return };
    let project = Project::new();
    let checker = project.checker(&tsc);
    let watched = session(&project, checker.clone());
    drop(watched.component("Button"));

    for version in 1..=5 {
        project.write_settled("shared.ts", &shared(version));
        watched.update_file(&project.file("shared.ts"));
        let answer = watched.component("Button").unwrap();
        assert!(by_checker(&answer), "{:?}", answer.diagnostics);
        assert!(
            answer.diagnostics.iter().all(|d| d.severity != DiagnosticSeverity::Warning),
            "{:?}",
            answer.diagnostics
        );
        assert!(
            answer.entry.props.contains_key(&format!("shared{version}")),
            "edit {version}: {:?}",
            answer.entry.props
        );
    }
    assert_eq!(checker.snapshots().created, 1, "{:?}", checker.snapshots());

    let fresh = session(&project, project.checker(&tsc));
    let keys = ["Button", "Card"];
    let watched_answers: Vec<_> = keys.iter().map(|key| watched.component(key).unwrap()).collect();
    let fresh_answers: Vec<_> = keys.iter().rev().map(|key| fresh.component(key).unwrap()).rev().collect();
    let bytes = |answers: &[ComponentAnswer]| {
        answers
            .iter()
            .map(|answer| format!("{}\n{:?}", serde_json::to_string(&answer.entry).unwrap(), answer.diagnostics))
            .collect::<Vec<_>>()
    };
    assert_eq!(bytes(&watched_answers), bytes(&fresh_answers));
}
