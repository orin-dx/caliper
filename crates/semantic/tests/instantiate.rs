#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! Generic components against a real checker (ADR 0026 S5): `tsrs` when `TSRS_BIN` names its binary, else the Go
//! `typescript@7.1` when `CALIPER_TEST_TSC` names its `bin/tsc`. Without either every test returns at once, and on CI
//! (`CI` is set) that is a failure. The fixture is read where it is, so `react` resolves to the workspace's
//! `@types/react`.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::PropType;
use caliper_docgen_core::types::semantic::SemanticOutcome;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

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

/// The fixture's components answered by `checker`, with every prop `@types/react` declares asked for.
fn answer(checker: Utf8PathBuf) -> SemanticOutcome {
    let root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic");
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut targets = targets_of(&output);
    for target in &mut targets {
        target.react_wanted = None;
    }
    let mut request = SemanticRequest::new(root.clone(), targets);
    request.tsc_path = Some(checker);
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
        .find(|component| component.display_name == name)
        .unwrap_or_else(|| panic!("{name} was not answered: {:?}", outcome.fallbacks));
    component.props.iter().map(|prop| (prop.name.as_str(), &prop.prop_type)).collect()
}

#[test]
fn a_polymorphic_component_is_read_at_its_default_element() {
    let Some(checker) = real_checker() else { return };
    let outcome = answer(checker);

    let poly = props(&outcome, "Poly");
    assert!(poly.len() >= 260, "Poly<\"div\"> has {} props", poly.len());
    let at = |name: &str| poly.iter().find(|(prop, _)| *prop == name).map(|(_, prop_type)| *prop_type);
    assert_eq!(at("as"), Some(&PropType::StringLiteral("div".to_string())));
    assert!(at("onClick").is_some() && at("tone").is_some());
    assert_eq!(at("href"), None, "a div has no href");
}

#[test]
fn a_default_no_other_file_can_name_keeps_the_structural_result_and_says_why() {
    let Some(checker) = real_checker() else { return };
    let outcome = answer(checker);

    let fallback =
        outcome.fallbacks.iter().find(|fallback| fallback.display_name == "Configured").expect("Configured fell back");
    assert_eq!(
        fallback.reason, "the instantiation Configured<Settings> could not be formed: Cannot find name 'Settings'.",
        "{}",
        fallback.reason
    );
}

#[test]
fn a_type_parameter_without_a_default_stays_open() {
    let Some(checker) = real_checker() else { return };
    let outcome = answer(checker);

    let listing = props(&outcome, "Listing");
    assert_eq!(listing.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["items", "selected"]);
}
