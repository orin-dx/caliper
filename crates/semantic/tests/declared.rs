#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! Where the checker says each prop is declared (ADR 0026 S7a): `parent`, `declarations`, tags and `composes` come from
//! its answer, not from the structural prop of the same name. The `declared` fixture, replayed from
//! `declared.recording.json` (made by the `record_fixture` example against the pinned `tsrs`) and, when `TSRS_BIN`
//! names a real `tsrs`, asked live:
//!
//! - `Button`: `tone` declared in `ButtonProps` in `Button.tsx`, `size` in `BaseProps` in `Base.ts`, each with its
//!   tags; it is made of React's `HTMLAttributes` (under an `Omit`) and `RefAttributes`;
//! - `Picked`: `disabled` reaches it through `Pick<BaseProps, …>`, a mapped type whose property names no parent, and
//!   is still `BaseProps`'s;
//! - `Card`: `label` and `count` are members of type literals, which name no interface.

use std::collections::BTreeMap;
use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::semantic::{SemanticOutcome, merge_semantic};
use caliper_docgen_core::types::{ExtractionOutput, PropParent};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

mod support;
use support::copy_fixture;

const FAKE_VERSION: &str = "7.1.0-test";

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// The fixture in a scratch directory whose name has an upper-case letter, so a file the checker names lower-cased is
/// told from the path on disk, with `replay-tsc` installed as `node_modules/typescript/bin/tsc` when `replay`.
fn project(replay: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Declared").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/declared"), &root);
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

fn answer(project: &Project, output: &ExtractionOutput, real: Option<&Utf8PathBuf>) -> SemanticOutcome {
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(output));
    request.request_timeout = Duration::from_secs(60);
    match real {
        Some(bin) => request.tsc_path = Some(bin.clone()),
        None => {
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/declared.recording.json", env!("CARGO_MANIFEST_DIR")),
            ));
        }
    }
    let outcome = resolve(request);
    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert!(outcome.fallbacks.is_empty(), "{:?}", outcome.fallbacks);
    outcome
}

#[allow(clippy::print_stderr, reason = "a skipped test says so, since a silent skip looks like a pass")]
fn tsrs_bin() -> Option<Utf8PathBuf> {
    let bin = std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty());
    if bin.is_none() {
        eprintln!("skipped: TSRS_BIN is not set");
    }
    bin.map(Utf8PathBuf::from)
}

/// Each prop of each component as `(component, prop) -> (parent, declarations, tags)`, and each component's `composes`.
type Facts =
    (BTreeMap<(String, String), (Option<PropParent>, Vec<PropParent>, BTreeMap<String, String>)>, Vec<Vec<String>>);

fn facts(outcome: &SemanticOutcome) -> Facts {
    let mut props = BTreeMap::new();
    for component in &outcome.components {
        for prop in &component.props {
            props.insert(
                (component.key.clone(), prop.name.clone()),
                (prop.parent.clone(), prop.declarations.clone(), prop.tags.clone()),
            );
        }
    }
    (props, outcome.components.iter().map(|component| component.composes.clone()).collect())
}

fn expected(root: &Utf8PathBuf) -> Facts {
    let parent = |name: &str, file: &str| PropParent { name: name.to_string(), file_name: root.join(file).to_string() };
    let tags = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect()
    };
    let declared = |name: &str, file: &str, tagged: &[(&str, &str)]| {
        (Some(parent(name, file)), vec![parent(name, file)], tags(tagged))
    };
    let key = |component: &str, prop: &str| (component.to_string(), prop.to_string());
    let props = BTreeMap::from([
        (key("Button", "disabled"), declared("BaseProps", "Base.ts", &[])),
        (key("Button", "size"), declared("BaseProps", "Base.ts", &[("default", "'md'"), ("deprecated", "use scale")])),
        (
            key("Button", "tone"),
            declared("ButtonProps", "Button.tsx", &[("see", "https://example.com"), ("since", "2.0")]),
        ),
        (key("Card", "count"), (None, vec![], tags(&[]))),
        (key("Card", "label"), (None, vec![], tags(&[]))),
        (key("Picked", "disabled"), declared("BaseProps", "Base.ts", &[])),
        (key("Picked", "inline"), declared("PickedProps", "Button.tsx", &[])),
    ]);
    // In key order: Button, Card, Picked.
    let composes = vec![vec!["HTMLAttributes".to_string(), "RefAttributes".to_string()], vec![], vec![]];
    (props, composes)
}

fn assert_declared(real: Option<&Utf8PathBuf>) {
    let project = project(real.is_none());
    let mut output = structural(&project);
    let outcome = answer(&project, &output, real);

    assert_eq!(facts(&outcome), expected(&project.root));

    // The merge takes them over the structural ones, `None` included.
    merge_semantic(&mut output, outcome);
    let card = &output.components["Card"].props;
    assert_eq!((card["label"].parent.clone(), card["count"].parent.clone()), (None, None));
    let picked = &output.components["Picked"].props["disabled"];
    assert_eq!(picked.parent.as_ref().map(|parent| parent.name.as_str()), Some("BaseProps"));
    assert_eq!(output.components["Button"].composes, ["HTMLAttributes", "RefAttributes"]);
}

#[test]
fn parent_declarations_tags_and_composes_are_the_checkers_replayed() {
    assert_declared(None);
}

#[test]
fn parent_declarations_tags_and_composes_are_the_checkers_from_a_real_tsrs() {
    let Some(bin) = tsrs_bin() else { return };
    assert_declared(Some(&bin));
}
