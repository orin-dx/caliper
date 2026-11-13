#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! Generic components against the built-in checker (ADR 0026 S5). The fixture is read where it is, so `react` resolves to the workspace's `@types/react`.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::PropType;
use caliper_docgen_core::types::semantic::SemanticOutcome;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

/// The components of `tests/fixtures/generic`, answered by the checker.
fn answer() -> SemanticOutcome {
    answer_in("generic")
}

/// The components of `tests/fixtures/<fixture>`, answered by the checker, with every prop `@types/react` declares asked
/// for.
fn answer_in(fixture: &str) -> SemanticOutcome {
    let root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut targets = targets_of(&output);
    for target in &mut targets {
        target.react_wanted = None;
    }
    let mut request = SemanticRequest::new(root.clone(), targets);
    request.tsconfig = Some(root.join("tsconfig.json"));
    request.request_timeout = Duration::from_secs(60);
    let outcome = resolve(request);
    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    outcome
}

fn props<'a>(outcome: &'a SemanticOutcome, name: &str) -> Vec<(&'a str, &'a PropType)> {
    let component = outcome
        .components
        .iter()
        .find(|component| component.key == name)
        .unwrap_or_else(|| panic!("{name} was not answered: {:?}", outcome.fallbacks));
    component.props.iter().map(|prop| (prop.name.as_str(), &prop.prop_type)).collect()
}

#[test]
fn a_polymorphic_component_is_read_at_its_default_element() {
    let outcome = answer();

    let poly = props(&outcome, "Poly");
    assert!(poly.len() >= 260, "Poly<\"div\"> has {} props", poly.len());
    let at = |name: &str| poly.iter().find(|(prop, _)| *prop == name).map(|(_, prop_type)| *prop_type);
    assert_eq!(at("as"), Some(&PropType::StringLiteral("div".to_string())));
    assert!(at("onClick").is_some() && at("tone").is_some());
    assert_eq!(at("href"), None, "a div has no href");
}

#[test]
fn a_default_no_other_file_can_name_keeps_the_structural_result_and_says_why() {
    let outcome = answer();

    let fallback =
        outcome.fallbacks.iter().find(|fallback| fallback.key == "Configured").expect("Configured fell back");
    assert_eq!(
        fallback.reason, "the instantiation Configured<Settings> could not be formed: Cannot find name 'Settings'.",
        "{}",
        fallback.reason
    );
}

#[test]
fn a_type_parameter_without_a_default_stays_open() {
    let outcome = answer();

    let listing = props(&outcome, "Listing");
    assert_eq!(listing.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["items", "selected"]);
}

/// An error the checker reports on another file's first line arrives in the probe file's diagnostics too. It is not an
/// error of any instantiation: the component whose probe is the probe file's first line still reads at its default.
#[test]
fn an_error_in_another_file_does_not_reject_a_valid_instantiation() {
    let outcome = answer_in("generic-foreign-error");

    assert_eq!(outcome.fallbacks.iter().map(|f| (&f.key, &f.reason)).collect::<Vec<_>>(), []);
    let valid = props(&outcome, "Valid");
    assert_eq!(valid.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["value"]);
    assert_eq!(valid[0].1, &PropType::StringLiteral("a".to_string()));
}

/// The module's own `FormData` is not the global one the probe file sees by the same name: the component reads at the
/// module's interface or keeps its structural result, and never takes the global `FormData`'s members.
#[test]
fn a_default_that_names_a_local_type_is_never_read_as_the_global_one_of_that_name() {
    let outcome = answer_in("generic-shadowed");

    match outcome.components.iter().find(|component| component.key == "Form") {
        Some(component) => {
            let value = component.props.iter().find(|prop| prop.name == "value").expect("value");
            let PropType::Object(fields) = &value.prop_type else {
                panic!("read at the global FormData, not the module's own: {:?}", value.prop_type)
            };
            assert_eq!(fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(), ["id"]);
        }
        None => {
            let fallback = outcome.fallbacks.iter().find(|fallback| fallback.key == "Form").expect("answered");
            assert!(
                fallback.reason.starts_with("the instantiation Form<FormData> could not be formed"),
                "{}",
                fallback.reason
            );
        }
    }
}
