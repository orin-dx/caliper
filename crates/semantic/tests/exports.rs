#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The checker asks for a component by the exports the extractor recorded for it (ADR 0027), and never by its display
//! name or the module's `default`. The `exports` fixture, replayed from `exports.recording.json` (made by the
//! `record_fixture` example against `typescript@7.1.0-beta`) and, when `TSRS_BIN` names a real `tsrs`, asked live:
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
}

/// `tests/fixtures/exports` in a scratch directory whose name has an upper-case letter (see `replay_tsc.rs`), with
/// `replay-tsc` installed as `node_modules/typescript/bin/tsc` when `replay`.
fn project(replay: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Exports").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exports"), &root);
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

fn request(project: &Project, targets: Vec<Target>, real: Option<&Utf8PathBuf>) -> SemanticRequest {
    let mut request = SemanticRequest::new(project.root.clone(), targets);
    request.request_timeout = Duration::from_secs(60);
    match real {
        Some(bin) => request.tsc_path = Some(bin.clone()),
        None => {
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/exports.recording.json", env!("CARGO_MANIFEST_DIR")),
            ));
        }
    }
    request
}

#[allow(clippy::print_stderr, reason = "a skipped test says so, since a silent skip looks like a pass")]
fn tsrs_bin() -> Option<Utf8PathBuf> {
    let bin = std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty());
    if bin.is_none() {
        eprintln!("skipped: TSRS_BIN is not set");
    }
    bin.map(Utf8PathBuf::from)
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

fn each_component_gets_the_props_of_its_own_export(real: Option<&Utf8PathBuf>) {
    let project = project(real.is_none());
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
    let Some(bin) = tsrs_bin() else { return };
    each_component_gets_the_props_of_its_own_export(Some(&bin));
}

/// A component asked for by an export its module does not have keeps its structural result and says why; it does not
/// take the module's `default` (`Other`) instead.
#[test]
fn a_missing_export_keeps_the_structural_result_and_never_falls_back_to_default() {
    let project = project(true);
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
    let Some(bin) = tsrs_bin() else { return };
    let project = project(false);
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
