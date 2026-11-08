#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The checker path over a fixture project of hand-written cases (`tests/fixtures/cases`): the 14 of the checker-first
//! deep dive in `Cases.tsx`, and one component per further mapping rule in `Rules.tsx`. The answer is held to a JSON
//! snapshot made from a real `tsrs` (`tests/snapshots/`), three ways:
//!
//! - replayed from its recording (`cases.recording.json`), always: the converter's output over exactly the answers the
//!   real checker gave, with no binary;
//! - against a real `tsrs` when `TSRS_BIN` names one, which also holds the answer to be the same when the checker is
//!   asked last to first (type handles change with the order of queries; the answer may not);
//! - merged over the structural result, which puts a union's members back in the order they were written in.
//!
//! Refresh the recording and the snapshot with `cargo run -p caliper-docgen-semantic --example record_fixture --
//! $TSRS_BIN cases`, then `cargo insta review`.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::ExtractionOutput;
use caliper_docgen_core::types::semantic::{SemanticOutcome, merge_semantic};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Value, json};

const FAKE_VERSION: &str = "7.1.0-test";

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// `from` copied to `to`, with a `modules` directory named `node_modules` (which the repository's ignore rules would not
/// keep).
fn copy_fixture(from: &Utf8Path, to: &Utf8Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_str().unwrap().to_string();
        let target = to.join(if name == "modules" { "node_modules" } else { &name });
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_fixture(&Utf8PathBuf::from_path_buf(entry.path()).unwrap(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// The cases in a scratch directory whose name has an upper-case letter (see `replay_tsc.rs`), with `replay-tsc`
/// installed as `node_modules/typescript/bin/tsc` when `replay`.
fn project(replay: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Cases").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cases"), &root);
    if replay {
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

enum Checker {
    Replay,
    Real { bin: Utf8PathBuf, reverse: bool },
}

fn answer(project: &Project, output: &ExtractionOutput, checker: &Checker) -> SemanticOutcome {
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(output));
    request.request_timeout = Duration::from_secs(60);
    match checker {
        Checker::Replay => {
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/cases.recording.json", env!("CARGO_MANIFEST_DIR")),
            ));
        }
        Checker::Real { bin, reverse } => {
            request.tsc_path = Some(bin.clone());
            request.reverse_queries = *reverse;
        }
    }
    resolve(request)
}

/// The outcome as the JSON the snapshot holds: each component's props in the order the checker gave them.
fn rendered(outcome: &SemanticOutcome) -> Value {
    let components: serde_json::Map<String, Value> = outcome
        .components
        .iter()
        .map(|component| {
            let props: Vec<Value> = component
                .props
                .iter()
                .map(|prop| {
                    json!({
                        "name": prop.name,
                        "required": prop.required,
                        "type": serde_json::to_value(&prop.prop_type).unwrap(),
                    })
                })
                .collect();
            (component.display_name.clone(), Value::Array(props))
        })
        .collect();
    let fallbacks: serde_json::Map<String, Value> =
        outcome.fallbacks.iter().map(|fallback| (fallback.display_name.clone(), json!(fallback.reason))).collect();
    json!({ "components": components, "fallbacks": fallbacks })
}

#[allow(clippy::print_stderr, reason = "a skipped test says so, since a silent skip looks like a pass")]
fn tsrs_bin() -> Option<Utf8PathBuf> {
    let bin = std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty());
    if bin.is_none() {
        eprintln!("skipped: TSRS_BIN is not set");
    }
    bin.map(Utf8PathBuf::from)
}

fn prop_type(rendered: &Value, component: &str, prop: &str) -> Value {
    rendered["components"][component]
        .as_array()
        .and_then(|props| props.iter().find(|candidate| candidate["name"] == prop))
        .unwrap_or_else(|| panic!("{component}.{prop} is in the answer"))["type"]
        .clone()
}

#[test]
fn the_recorded_answers_of_the_checker_convert_to_the_snapshot() {
    let project = project(true);
    let output = structural(&project);

    let outcome = answer(&project, &output, &Checker::Replay);

    assert_eq!(outcome.diagnostics, []);
    insta::assert_json_snapshot!("cases", rendered(&outcome));
}

#[test]
fn a_real_tsrs_gives_the_snapshot() {
    let Some(bin) = tsrs_bin() else { return };
    let project = project(false);
    let output = structural(&project);

    let outcome = answer(&project, &output, &Checker::Real { bin, reverse: false });

    assert_eq!(outcome.diagnostics, []);
    insta::assert_json_snapshot!("cases", rendered(&outcome));
}

/// Type handles are the checker's and change with the order its types are first asked about; a union of objects is
/// sorted by them. The answer must not show it.
#[test]
fn asking_the_checker_last_to_first_gives_the_same_bytes() {
    let Some(bin) = tsrs_bin() else { return };
    let project = project(false);
    let output = structural(&project);

    let forward = answer(&project, &output, &Checker::Real { bin: bin.clone(), reverse: false });
    let reversed = answer(&project, &output, &Checker::Real { bin, reverse: true });

    assert_eq!(forward.diagnostics, []);
    assert_eq!(reversed.diagnostics, []);
    assert_eq!(
        serde_json::to_string(&rendered(&forward)).unwrap(),
        serde_json::to_string(&rendered(&reversed)).unwrap()
    );
}

/// The checker sorts a union's members by kind and value; the merge puts them back in the order they were written
/// in, from the structural pass, which read them from the type node.
#[test]
fn the_merge_puts_a_unions_members_back_in_declaration_order() {
    let project = project(true);
    let mut output = structural(&project);
    let outcome = answer(&project, &output, &Checker::Replay);
    let size = |outcome: &SemanticOutcome| {
        let component = outcome.components.iter().find(|component| component.display_name == "Plain").unwrap();
        let prop = component.props.iter().find(|prop| prop.name == "size").unwrap();
        serde_json::to_value(&prop.prop_type).unwrap()
    };
    let literals = |value: Value| -> Vec<String> {
        value["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| member["value"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(literals(size(&outcome)), ["lg", "md", "sm"], "the checker's order");

    merge_semantic(&mut output, outcome);

    let merged = serde_json::to_value(&output.components["Plain"].props["size"].prop_type).unwrap();
    assert_eq!(literals(merged), ["sm", "md", "lg"], "the order the author wrote");
}

#[test]
fn a_mapped_conditional_or_aliased_type_is_what_the_checker_resolved() {
    let project = project(true);
    let output = structural(&project);
    let rendered = rendered(&answer(&project, &output, &Checker::Replay));

    // `Partial<Omit<Base, 'd'>> & { [K in 'x' | 'y']: number }`: members the checker computed.
    let names = |component: &str| -> Vec<String> {
        rendered["components"][component]
            .as_array()
            .unwrap()
            .iter()
            .map(|prop| prop["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("Mapper"), ["a", "b", "c", "x", "y", "extra"]);
    // A conditional type on the component's props type: both branches it could not pick are open.
    assert_eq!(prop_type(&rendered, "CondC", "k")["reason"]["type"], "conditionalType");
    // A lib utility is evaluated, not named.
    assert_eq!(prop_type(&rendered, "Aliases", "partial")["kind"], "object");
    assert_eq!(prop_type(&rendered, "Aliases", "record")["kind"], "object");
    // An interface or alias a dependency declares keeps its name; a literal union it declares is expanded.
    assert_eq!(prop_type(&rendered, "Aliases", "theme"), json!({ "kind": "named", "name": "DepTheme", "args": [] }));
    assert_eq!(prop_type(&rendered, "Aliases", "style"), json!({ "kind": "named", "name": "DepStyle", "args": [] }));
    assert_eq!(prop_type(&rendered, "Aliases", "size")["kind"], "union");
}

#[test]
fn the_error_type_is_an_unresolvable_import_and_a_written_any_is_any() {
    let project = project(true);
    let output = structural(&project);
    let rendered = rendered(&answer(&project, &output, &Checker::Replay));

    assert_eq!(prop_type(&rendered, "Lost", "gone")["reason"]["type"], "unresolvableImport");
    assert_eq!(prop_type(&rendered, "Scalars", "any"), json!({ "kind": "any" }));
}

#[test]
fn nothing_the_checker_hands_out_by_number_is_in_the_answer() {
    let project = project(true);
    let output = structural(&project);
    let text = serde_json::to_string(&rendered(&answer(&project, &output, &Checker::Replay))).unwrap();

    // A symbol-keyed property is `[Symbol.iterator]`, not the checker's `__@iterator@<symbol id>`.
    assert!(!text.contains("__@"), "{text}");
    assert!(text.contains("[Symbol.iterator]"));
}
