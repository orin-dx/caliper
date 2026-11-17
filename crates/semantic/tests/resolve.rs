#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The backend end to end against the built-in checker, and the fallback that must never be an error: a checker that
//! does not answer in time.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::semantic::{SemanticOutcome, merge_semantic};
use caliper_docgen_core::types::{
    DefaultValue, DiagnosticCode, DiagnosticSeverity, ExtractionOutput, FunctionParam, ParsedProp, PropType,
};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

struct Project {
    // Held for its Drop.
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// The fixture project in a scratch directory.
fn project() -> Project {
    project_of("project", &["Button.tsx", "tsconfig.json"])
}

/// The same, for the fixture project `tests/fixtures/<fixture>` with the files named.
fn project_of(fixture: &str, files: &[&str]) -> Project {
    let dir = tempfile::Builder::new().prefix("Resolve").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in files {
        std::fs::copy(fixtures.join(fixture).join(name), root.join(name)).unwrap();
    }
    Project { _dir: dir, root }
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

fn request(project: &Project, output: &ExtractionOutput) -> SemanticRequest {
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(output));
    request.request_timeout = Duration::from_secs(60);
    request
}

fn prop(output: &ExtractionOutput, component: &str, name: &str) -> ParsedProp {
    (*output.components[component].props[name]).clone()
}

#[test]
fn function_components_get_the_checkers_props_merged_over_the_structural_ones() {
    let project = project();
    let mut output = structural(&project);
    let structural_size = prop(&output, "Button", "size");

    let outcome = resolve(request(&project, &output));
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
    assert_eq!(
        prop(&output, "Button", "pair").prop_type,
        PropType::Tuple(vec![
            caliper_docgen_core::types::TupleElement::required(PropType::String),
            caliper_docgen_core::types::TupleElement::required(PropType::Number)
        ])
    );
    assert_eq!(prop(&output, "Button", "tags").prop_type, PropType::Array(Box::new(PropType::String)));

    // The checker sorts union members; the merge puts them back in the order the alias writes them.
    let size = prop(&output, "Button", "size");
    assert_eq!(
        size.prop_type,
        PropType::Union(vec![PropType::StringLiteral("sm".to_string()), PropType::StringLiteral("lg".to_string())])
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
    let project = project_of("markers", &["Markers.tsx", "tsconfig.json"]);
    let mut output = structural(&project);
    let structural_names: Vec<_> = output.components["Markers"].props.keys().cloned().collect();
    assert_eq!(structural_names, ["___triple", "__size", "_single", "plain"]);

    let outcome = resolve(request(&project, &output));
    assert!(outcome.diagnostics.is_empty(), "{:?} / {:?}", outcome.diagnostics, outcome.fallbacks);

    let answered = &outcome.components.iter().find(|component| component.key == "Markers").unwrap().props;
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
    let project = project_of("markers", &["Markers.tsx", "tsconfig.json"]);
    let output = structural(&project);

    let outcome = resolve(request(&project, &output));
    assert!(outcome.diagnostics.is_empty(), "{:?} / {:?}", outcome.diagnostics, outcome.fallbacks);

    let callbacks = &outcome.components.iter().find(|component| component.key == "Callbacks").unwrap().props;
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

/// Which parameters are optional is stated only by the printed type, so the printed type has to be one that states it:
/// an alias is printed as what it stands for, a long type is not cut short, a `this` parameter is not counted, and each
/// signature of an overload set has its own markers. Where nothing can be read, no parameter is made required.
#[test]
fn optional_parameters_are_read_through_an_alias_a_long_type_a_this_parameter_and_overloads() {
    let project = project_of("markers", &["Markers.tsx", "tsconfig.json"]);
    let output = structural(&project);

    let outcome = resolve(request(&project, &output));
    assert!(outcome.diagnostics.is_empty(), "{:?} / {:?}", outcome.diagnostics, outcome.fallbacks);

    let aliased = &outcome.components.iter().find(|component| component.key == "Aliased").unwrap().props;
    let prop_type = |name: &str| aliased.iter().find(|prop| prop.name == name).unwrap().prop_type.clone();
    let text = |name: &str| prop_type(name).raw_string();

    // `onChange: Handler` with `type Handler = (value?: string) => void`: the callback may be called with no argument.
    assert_eq!(
        prop_type("onChange"),
        PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam {
                name: Some("value".to_string()),
                param_type: PropType::String,
                optional: true,
                rest: false
            }],
            return_type: Box::new(PropType::Void),
        }
    );
    // Parameters after a long one are still read: the checker cuts the printed type short unless it is told not to.
    assert!(text("long").ends_with(", c?: number, d?: string) => void"), "{}", text("long"));
    assert_eq!(text("withThis"), "(a?: number) => void");
    assert_eq!(text("overloaded"), "{ (a?: string): string; (a: number, b?: number): number }");
    assert_eq!(text("withCall"), "{ (a?: string): number; displayName: string }");
    assert_eq!(text("ctor"), "{ new (a?: string): Date }");

    // A callable interface is printed as its name whatever the flags: nothing states `a?`, so it is not a function
    // with a required `a`; it is opaque, and the prop is degraded.
    assert!(matches!(prop_type("named"), PropType::Opaque(_)), "{:?}", prop_type("named"));
    assert_eq!(serde_json::to_value(prop_type("named")).unwrap()["reason"]["type"], "unsupportedExpression");
}

#[test]
fn provenance_lists_the_resolved_components_and_the_one_the_module_does_not_export() {
    let project = project();
    let mut output = structural(&project);

    let outcome = resolve(request(&project, &output));
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

/// A checker that does not answer in time leaves the structural result, plus exactly one warning that says so.
#[test]
fn a_checker_that_does_not_answer_in_time_falls_back_with_one_diagnostic() {
    let project = project();
    let output = structural(&project);
    let mut request = request(&project, &output);
    request.request_timeout = Duration::from_micros(1);

    let mut merged = output.clone();
    merge_semantic(&mut merged, resolve(request));

    assert_eq!(merged.components, output.components, "the structural components are untouched");
    assert_eq!(merged.diagnostics.len(), output.diagnostics.len() + 1);
    let added = merged.diagnostics.last().unwrap();
    assert_eq!(added.code, DiagnosticCode::SemanticBackend);
    assert_eq!(added.severity, DiagnosticSeverity::Warning);
    assert!(added.message.contains("timed out"), "{} should say it timed out", added.message);
}

#[test]
fn a_run_with_nothing_exported_never_starts_the_checker() {
    let project = project();
    let mut output = structural(&project);
    for entry in output.components.values_mut() {
        entry.exported = false;
        entry.exports.clear();
    }
    let before = output.components.clone();

    let outcome = resolve(request(&project, &output));

    assert!(outcome.components.is_empty());
    assert!(outcome.diagnostics.is_empty(), "no checker was needed");
    merge_semantic(&mut output, outcome);
    assert_eq!(output.components, before);
}

#[test]
fn an_empty_outcome_merges_to_the_structural_output_unchanged() {
    let project = project();
    let mut output = structural(&project);
    let before = serde_json::to_string(&output).unwrap();

    merge_semantic(&mut output, SemanticOutcome::default());

    assert_eq!(serde_json::to_string(&output).unwrap(), before);
}

/// The program is created from `createPrograms[].compilerOptions`; the Node client sends the same list under
/// `options`. The checker accepts both, so this pins the key the backend sends.
#[test]
fn the_snapshot_is_created_with_compiler_options_for_the_root_files() {
    let project = project();
    let output = structural(&project);
    let log = project.root.join("requests.json");
    let mut request = request(&project, &output);
    request.record_to = Some(log.clone());

    let outcome = resolve(request);
    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);

    let recorded: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
    let sent: Vec<serde_json::Value> =
        recorded.into_iter().filter(|request: &serde_json::Value| request["method"] == "createSnapshot").collect();
    let [snapshot] = sent.as_slice() else { panic!("one createSnapshot expected: {sent:?}") };
    let [program] = snapshot["params"]["createPrograms"].as_array().unwrap().as_slice() else {
        panic!("one program expected: {snapshot}")
    };
    assert!(program["compilerOptions"].is_object(), "{program}");
    assert!(program.get("options").is_none(), "{program}");
    assert!(program["rootFiles"].as_array().is_some_and(|files| !files.is_empty()), "{program}");
}
