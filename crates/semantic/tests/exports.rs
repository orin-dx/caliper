#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The checker asks for a component by the exports the extractor recorded for it (ADR 0027), and never by its display
//! name or the module's `default`. The `exports` fixture, replayed from `exports.recording.json` (made by the
//! `record_fixture` example against `typescript@7.1.0-beta`) and asked live (the built-in checker, or `TSRS_BIN`):
//!
//! - `Menu.tsx`: `Menu.Item = Item; export default Menu`. `Item` is `default.Item`, so it has `value`, not `Menu`'s
//!   `label` and `open`;
//! - `Card.tsx`: `export { CardImpl as Card }` beside `export default function Other`. `CardImpl` is `Card`, so it has
//!   `title`, not `Other`'s `other`;
//! - `Tabs.tsx`: `export const Tabs = Object.assign(TabsRoot, { Tab })`. `Tab` is `Tabs.Tab`.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::semantic::merge_semantic;
use caliper_docgen_core::types::{ComponentExport, ExtractionOutput};
use caliper_docgen_semantic::{SemanticRequest, Target, resolve, targets_of};
use camino::Utf8PathBuf;

mod support;
use support::copy_fixture;

const FAKE_VERSION: &str = "7.1.0-test";

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
    fixture: &'static str,
}

/// `tests/fixtures/<fixture>` in a scratch directory whose name has an upper-case letter (see `replay_tsc.rs`), with
/// `replay-tsc` installed as `node_modules/typescript/bin/tsc` when `replay`.
fn project(fixture: &'static str, replay: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Exports").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture), &root);
    if replay {
        let package = root.join("node_modules/typescript");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), format!(r#"{{"version": "{FAKE_VERSION}"}}"#)).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_replay-tsc"), package.join("bin/tsc")).unwrap();
    }
    Project { _dir: dir, root, fixture }
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

fn request(project: &Project, targets: Vec<Target>, real: Option<&Option<Utf8PathBuf>>) -> SemanticRequest {
    let mut request = SemanticRequest::new(project.root.clone(), targets);
    request.request_timeout = Duration::from_secs(60);
    match real {
        Some(bin) => request.tsc_path = bin.clone(),
        None => {
            request.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/{}.recording.json", env!("CARGO_MANIFEST_DIR"), project.fixture),
            ));
        }
    }
    request
}

/// The `tsrs` binary `TSRS_BIN` names, to run as a process; `None` asks the built-in checker.
fn tsrs_bin() -> Option<Utf8PathBuf> {
    std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty()).map(Utf8PathBuf::from)
}

fn prop_names(output: &ExtractionOutput) -> Vec<(String, Vec<String>)> {
    output.components.iter().map(|(key, entry)| (key.clone(), entry.props.keys().cloned().collect())).collect()
}

fn checker_diagnostics(output: &ExtractionOutput) -> Vec<(String, Vec<String>)> {
    output
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("TypeScript checker") || d.message.contains("structural result"))
        .map(|d| (d.message.clone(), d.affects.clone()))
        .collect()
}

fn owned(expected: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    expected.iter().map(|(key, names)| (key.to_string(), names.iter().map(ToString::to_string).collect())).collect()
}

fn each_component_gets_the_props_of_its_own_export(real: Option<&Option<Utf8PathBuf>>) {
    let project = project("exports", real.is_none());
    let mut output = structural(&project);
    let expected = owned(&[
        ("CardImpl", &["title"]),
        ("Item", &["value"]),
        ("Menu", &["label", "open"]),
        ("Other", &["other"]),
        ("Tab", &["id"]),
        ("Tabs", &["selected"]),
    ]);
    assert_eq!(prop_names(&output), expected, "the structural pass");

    let outcome = resolve(request(&project, targets_of(&output), real));
    merge_semantic(&mut output, outcome);

    assert_eq!(prop_names(&output), expected, "the checker");
    let resolved = ["CardImpl", "Item", "Menu", "Other", "Tab", "Tabs"].map(String::from).to_vec();
    assert_eq!(
        checker_diagnostics(&output),
        [("6 component(s) resolved by the TypeScript checker".to_string(), resolved)]
    );
}

#[test]
fn each_component_gets_the_props_of_its_own_export_replayed() {
    each_component_gets_the_props_of_its_own_export(None);
}

#[test]
fn each_component_gets_the_props_of_its_own_export_from_a_real_tsrs() {
    let bin = tsrs_bin();
    each_component_gets_the_props_of_its_own_export(Some(&bin));
}

/// A component asked for by an export its module does not have keeps its structural result and says why; it does not
/// take the module's `default` (`Other`) instead.
#[test]
fn a_missing_export_keeps_the_structural_result_and_never_falls_back_to_default() {
    let project = project("exports", true);
    let mut output = structural(&project);
    output.components.get_mut("CardImpl").unwrap().exports = vec![ComponentExport::named("Gone")];
    let before = output.components["CardImpl"].clone();

    let outcome = resolve(request(&project, targets_of(&output), None));
    merge_semantic(&mut output, outcome);

    assert_eq!(output.components["CardImpl"], before);
    let resolved = ["Item", "Menu", "Other", "Tab", "Tabs"].map(String::from).to_vec();
    assert_eq!(
        checker_diagnostics(&output),
        [
            ("5 component(s) resolved by the TypeScript checker".to_string(), resolved),
            (
                "1 component(s) kept their structural result: the checker finds no export `Gone` in its module"
                    .to_string(),
                vec!["CardImpl".to_string()]
            )
        ]
    );
}

/// An export that resolves to a declaration in another file is not the component the extractor found in this one:
/// `Menu.tsx` re-exports `Card.tsx`'s default as `Elsewhere`.
#[test]
fn an_export_declared_in_another_file_keeps_the_structural_result() {
    let bin = tsrs_bin();
    let project = project("exports", false);
    let mut output = structural(&project);
    output.components.get_mut("Menu").unwrap().exports = vec![ComponentExport::named("Elsewhere")];
    let before = output.components["Menu"].clone();
    let targets = targets_of(&output).into_iter().filter(|target| target.key == "Menu").collect();

    let outcome = resolve(request(&project, targets, Some(&bin)));
    merge_semantic(&mut output, outcome);

    assert_eq!(output.components["Menu"], before);
    assert_eq!(
        checker_diagnostics(&output),
        [(
            "1 component(s) kept their structural result: the export `Elsewhere` is declared in another file"
                .to_string(),
            vec!["Menu".to_string()]
        )]
    );
}

/// What each component is, as a reader of the output sees it: its name, the exports it has and its props.
fn components_with_exports(output: &ExtractionOutput) -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut found: Vec<_> = output
        .components
        .values()
        .map(|entry| {
            (
                entry.display_name.clone(),
                entry.exports.iter().map(ToString::to_string).collect(),
                entry.props.keys().cloned().collect(),
            )
        })
        .collect();
    found.sort();
    found
}

/// `A.Item = …` and `B.Item = …` in one file are two components of one name. Each has the props of its own property,
/// not the first attached `Item`'s: the checker was asked for `A.Item` and for `B.Item` and no other.
fn two_inline_sub_components_of_one_name_each_get_their_own_props(real: Option<&Option<Utf8PathBuf>>) {
    let project = project("items", real.is_none());
    let mut output = structural(&project);
    let row = |name: &str, exports: &[&str], props: &[&str]| {
        (
            name.to_string(),
            exports.iter().map(ToString::to_string).collect::<Vec<_>>(),
            props.iter().map(ToString::to_string).collect::<Vec<_>>(),
        )
    };
    let expected = [
        row("A", &["A"], &["a"]),
        row("B", &["B"], &["b"]),
        row("Item", &["A.Item"], &["itemOfA"]),
        row("Item", &["B.Item"], &["itemOfB"]),
    ];
    assert_eq!(components_with_exports(&output), expected, "the structural pass");

    let outcome = resolve(request(&project, targets_of(&output), real));
    merge_semantic(&mut output, outcome);

    assert_eq!(components_with_exports(&output), expected, "the checker");
    let messages: Vec<&str> = output.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(messages.contains(&"4 component(s) resolved by the TypeScript checker"), "{messages:?}");
}

#[test]
fn two_inline_sub_components_of_one_name_each_get_their_own_props_replayed() {
    two_inline_sub_components_of_one_name_each_get_their_own_props(None);
}

#[test]
fn two_inline_sub_components_of_one_name_each_get_their_own_props_from_a_real_tsrs() {
    let bin = tsrs_bin();
    two_inline_sub_components_of_one_name_each_get_their_own_props(Some(&bin));
}
