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
    project_of("project", &["Button.tsx", "tsconfig.json"], with_tsc)
}

/// The same, for the fixture project `tests/fixtures/<fixture>` with the files named.
fn project_of(fixture: &str, files: &[&str], with_tsc: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Replay").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in files {
        std::fs::copy(fixtures.join(fixture).join(name), root.join(name)).unwrap();
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
    request_replaying(project, output, scenario, "recording.json")
}

/// The same, answered from `tests/fixtures/<recording>`.
fn request_replaying(
    project: &Project,
    output: &ExtractionOutput,
    scenario: Option<&str>,
    recording: &str,
) -> SemanticRequest {
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(output));
    request.expected_versions = &[FAKE_VERSION];
    request.request_timeout = Duration::from_secs(20);
    request
        .extra_env
        .push(("REPLAY_FIXTURE".to_string(), format!("{}/tests/fixtures/{recording}", env!("CARGO_MANIFEST_DIR"))));
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
    let field = |name: &str, prop_type: PropType, required: bool| {
        caliper_docgen_core::types::ObjectField::new(name, prop_type, required, String::new())
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
            params: vec![FunctionParam::new(Some("event".to_string()), PropType::Number)],
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

/// The checker keeps a name that starts with two underscores in its escaped form, with a third added; the name a
/// reader sees is the one the source writes.
#[test]
fn a_prop_name_that_starts_with_underscores_is_the_name_the_source_writes() {
    let project = project_of("markers", &["Markers.tsx", "tsconfig.json"], true);
    let mut output = structural(&project);
    let structural_names: Vec<_> = output.components["Markers"].props.keys().cloned().collect();
    assert_eq!(structural_names, ["___triple", "__size", "_single", "plain"]);

    let outcome = resolve(request_replaying(&project, &output, None, "markers.recording.json"));
    assert!(outcome.diagnostics.is_empty(), "{:?} / {:?}", outcome.diagnostics, outcome.fallbacks);

    let answered = &outcome.components.iter().find(|component| component.display_name == "Markers").unwrap().props;
    let mut answered_names: Vec<_> = answered.iter().map(|prop| prop.name.as_str()).collect();
    answered_names.sort_unstable();
    assert_eq!(answered_names, ["___triple", "__size", "_single", "plain"], "the checker's `___size` is not a prop");

    merge_semantic(&mut output, outcome);
    let names: Vec<_> = output.components["Markers"].props.keys().cloned().collect();
    assert_eq!(names, ["___triple", "__size", "_single", "plain"]);
    assert_eq!(prop(&output, "Markers", "__size").prop_type, PropType::String);
    assert_eq!(prop(&output, "Markers", "___triple").prop_type, PropType::Boolean);
    assert_eq!(prop(&output, "Markers", "_single").prop_type, PropType::Number);
}

/// A parameter written `name?: T` or `...name: T[]` keeps its marker, so the type reads as the source writes it, and a
/// parameter whose type merely includes `undefined` does not become optional.
#[test]
fn function_parameters_keep_their_optional_and_rest_markers() {
    let project = project_of("markers", &["Markers.tsx", "tsconfig.json"], true);
    let output = structural(&project);

    let outcome = resolve(request_replaying(&project, &output, None, "markers.recording.json"));
    assert!(outcome.diagnostics.is_empty(), "{:?} / {:?}", outcome.diagnostics, outcome.fallbacks);

    let callbacks = &outcome.components.iter().find(|component| component.display_name == "Callbacks").unwrap().props;
    let params = |name: &str| match &callbacks.iter().find(|prop| prop.name == name).unwrap().prop_type {
        PropType::Function { params, return_type, .. } => (params.clone(), (**return_type).clone()),
        other => panic!("{name} is not a function: {other:?}"),
    };
    let param = |name: &str, param_type: PropType, optional: bool, rest: bool| FunctionParam {
        name: Some(name.to_string()),
        param_type,
        optional,
        rest,
    };

    assert_eq!(
        params("format"),
        (
            vec![
                param("percent", PropType::Number, true, false),
                param("rest", PropType::Array(Box::new(PropType::String)), false, true),
            ],
            PropType::String
        )
    );
    assert_eq!(
        params("close"),
        (vec![param("args", PropType::Array(Box::new(PropType::Any)), false, true)], PropType::Void)
    );
    assert_eq!(
        params("onClick"),
        (vec![param("e", PropType::Named { name: "Event".into(), args: vec![] }, true, false)], PropType::Void)
    );
    assert_eq!(
        params("plain"),
        (vec![param("a", PropType::Number, false, false), param("b", PropType::String, true, false)], PropType::Void)
    );
    assert_eq!(
        params("explicit"),
        (
            vec![
                param("a", PropType::Number, false, false),
                param("b", PropType::Union(vec![PropType::Undefined, PropType::String]), false, false),
            ],
            PropType::Void
        )
    );
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
fn an_unsupported_typescript_version_falls_back_with_one_diagnostic() {
    let project = project(true);
    let output = structural(&project);
    let mut request = request(&project, &output, None);
    request.expected_versions = &["7.1.0-beta"];

    assert_falls_back(&project, request, &output, "not a supported version");
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
