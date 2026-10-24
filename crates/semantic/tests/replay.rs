#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The backend end to end against `replay-tsc`, which answers from a recording of the real `7.1.0-beta`
//! (`tests/fixtures/recording.json`, refreshed by the `record_fixture` example), and the fallbacks that must never be
//! errors: no `tsc`, an unpinned version, a crash, a hang.

use std::time::Duration;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use caliper_docgen_core::types::semantic::{merge_semantic, SemanticOutcome};
use caliper_docgen_core::types::{
    DefaultValue, DiagnosticCode, DiagnosticSeverity, ExtractionOutput, FunctionParam, ParsedProp, PropType,
};
use caliper_docgen_semantic::{resolve, targets_of, SemanticRequest};
use camino::Utf8PathBuf;

const FAKE_VERSION: &str = "7.1.0-test";

struct Project {
    // Held for its Drop.
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

fn request(project: &Project, output: &ExtractionOutput, scenario: Option<&str>) -> SemanticRequest {
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(output));
    request.expected_versions = &[FAKE_VERSION];
    request.request_timeout = Duration::from_secs(20);
    request
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/recording.json", env!("CARGO_MANIFEST_DIR"))));
    if let Some(scenario) = scenario {
        request.extra_env.push(("REPLAY_SCENARIO".to_string(), scenario.to_string()));
    }
    request
}

fn prop(output: &ExtractionOutput, component: &str, name: &str) -> ParsedProp {
    (*output.components[component].props[name]).clone()
}

#[test]
fn function_components_get_the_checkers_props_merged_over_the_structural_ones() {
    let project = project(true);
    let mut output = structural(&project);
    let structural_size = prop(&output, "Button", "size");

    let outcome = resolve(request(&project, &output, None));
    merge_semantic(&mut output, outcome);

    let button = &output.components["Button"];
    let names: Vec<_> = button.props.keys().map(String::as_str).collect();
    assert_eq!(names, ["box", "disabled", "label", "onClick", "pair", "size", "tags"]);
    let field = |name: &str, prop_type: PropType, required: bool| caliper_docgen_core::types::ObjectField {
        name: name.to_string(),
        prop_type,
        required,
        description: String::new(),
    };
    assert_eq!(
        prop(&output, "Button", "box").prop_type,
        PropType::Object(vec![field("a", PropType::Number, true), field("b", PropType::String, false),])
    );
    assert_eq!(prop(&output, "Button", "disabled").prop_type, PropType::Boolean);
    assert!(!prop(&output, "Button", "disabled").required);
    assert_eq!(prop(&output, "Button", "label").prop_type, PropType::String);
    assert_eq!(prop(&output, "Button", "label").description, "Visible text.");
    assert_eq!(
        prop(&output, "Button", "onClick").prop_type,
        PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam { name: Some("event".to_string()), param_type: PropType::Number }],
            return_type: Box::new(PropType::Void),
        }
    );
    assert_eq!(prop(&output, "Button", "pair").prop_type, PropType::Tuple(vec![PropType::String, PropType::Number]));
    assert_eq!(prop(&output, "Button", "tags").prop_type, PropType::Array(Box::new(PropType::String)));

    // The checker orders union members by creation, not by source.
    let size = prop(&output, "Button", "size");
    assert_eq!(
        size.prop_type,
        PropType::Union(vec![PropType::StringLiteral("lg".to_string()), PropType::StringLiteral("sm".to_string())])
    );
    assert!(!size.required);
    assert_eq!(size.default_value, Some(DefaultValue { value: "\"sm\"".to_string(), computed: false }));
    assert_eq!(size.default_value, structural_size.default_value, "the default value stays structural");

    assert_eq!(prop(&output, "Other", "a").prop_type, PropType::Number);
}

#[test]
fn provenance_lists_the_resolved_components_and_the_one_the_module_does_not_export() {
    let project = project(true);
    let mut output = structural(&project);

    let outcome = resolve(request(&project, &output, None));
    merge_semantic(&mut output, outcome);

    let notes: Vec<_> = output
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::SemanticBackend)
        .map(|d| (d.severity.clone(), d.message.clone(), d.affects.clone()))
        .collect();
    assert_eq!(
        notes,
        [
            (
                DiagnosticSeverity::Info,
                "2 component(s) resolved by the TypeScript checker".to_string(),
                vec!["Button".to_string(), "Other".to_string()]
            ),
            (
                DiagnosticSeverity::Info,
                "1 component(s) kept their structural result: not exported by its module".to_string(),
                vec!["Hidden".to_string()]
            ),
        ]
    );
}

/// An unavailable backend is the structural result plus exactly one warning, whatever the reason.
fn assert_falls_back(project: &Project, request: SemanticRequest, output: &ExtractionOutput, reason: &str) {
    let mut merged = output.clone();
    merge_semantic(&mut merged, resolve(request));

    assert_eq!(merged.components, output.components, "the structural components are untouched");
    assert_eq!(merged.diagnostics.len(), output.diagnostics.len() + 1);
    let added = merged.diagnostics.last().unwrap();
    assert_eq!(added.code, DiagnosticCode::SemanticBackend);
    assert_eq!(added.severity, DiagnosticSeverity::Warning);
    assert!(added.message.contains(reason), "{} should mention {reason:?}", added.message);
    let _ = project;
}

#[test]
fn no_typescript_installed_falls_back_with_one_diagnostic() {
    let project = project(false);
    let output = structural(&project);
    // The scratch directory sits inside the system temp dir, which could itself have a node_modules above it.
    assert_falls_back(&project, request(&project, &output, None), &output, "no node_modules/typescript/bin/tsc");
}

#[test]
fn an_unpinned_typescript_version_falls_back_with_one_diagnostic() {
    let project = project(true);
    let output = structural(&project);
    let mut request = request(&project, &output, None);
    request.expected_versions = &["7.1.0-beta"];

    assert_falls_back(&project, request, &output, "not pinned");
}

#[test]
fn a_missing_explicit_tsc_path_falls_back_with_one_diagnostic() {
    let project = project(true);
    let output = structural(&project);
    let mut request = request(&project, &output, None);
    request.tsc_path = Some(project.root.join("nowhere/tsc"));

    assert_falls_back(&project, request, &output, "no tsc binary at");
}

#[test]
fn a_tsc_that_crashes_falls_back_with_one_diagnostic() {
    let project = project(true);
    let output = structural(&project);

    assert_falls_back(&project, request(&project, &output, Some("crash")), &output, "stopped answering");
}

#[test]
fn a_tsc_that_hangs_times_out_and_falls_back_with_one_diagnostic() {
    let project = project(true);
    let output = structural(&project);
    let mut request = request(&project, &output, Some("hang"));
    request.request_timeout = Duration::from_millis(400);

    assert_falls_back(&project, request, &output, "timed out");
}

#[test]
fn a_run_with_nothing_exported_never_starts_tsc() {
    let project = project(false);
    let mut output = structural(&project);
    for entry in output.components.values_mut() {
        entry.exported = false;
    }
    let before = output.components.clone();

    let outcome = resolve(request(&project, &output, None));

    assert!(outcome.components.is_empty());
    assert!(outcome.diagnostics.is_empty(), "no tsc was needed, so none was missing");
    merge_semantic(&mut output, outcome);
    assert_eq!(output.components, before);
}

#[test]
fn an_empty_outcome_merges_to_the_structural_output_unchanged() {
    let project = project(false);
    let mut output = structural(&project);
    let before = serde_json::to_string(&output).unwrap();

    merge_semantic(&mut output, SemanticOutcome::default());

    assert_eq!(serde_json::to_string(&output).unwrap(), before);
}
