#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The `@types/react` types a component's props are made of (`composes`), where the props type is an instantiation of a
//! generic interface (ADR 0026 S7a). The checker states the bases of the declaration of an interface and not of
//! `Props<string>`, a reference to it, so they are read from the generic type it instantiates. The `composes` fixture,
//! replayed from `composes.recording.json` (made by the `record_fixture` example against the pinned `tsrs`) and asked
//! live: a `tsrs` process when `TSRS_BIN` names one, the Go `typescript@7.1` when `CALIPER_TEST_TSC` names its
//! `bin/tsc`, else the built-in checker:
//!
//! - `Direct`: `DivProps<string>` extends `HTMLAttributes<HTMLDivElement>`;
//! - `Chained`: `Chain<number>` extends `ButtonBase<T>`, which extends `HTMLAttributes<HTMLButtonElement>`, and
//!   `RefAttributes`, named after it;
//! - `Bare`: `Own<string>` extends a generic interface of the project that extends no React type, and has none;
//! - `Mixed`: `DivProps<boolean>` as a member of an intersection;
//! - `Plainly`: a plain interface that extends `HTMLAttributes<HTMLSpanElement>`, as before.

use std::collections::BTreeMap;
use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::semantic::{SemanticOutcome, merge_semantic};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

mod support;
use support::copy_fixture;

const FAKE_VERSION: &str = "7.1.0-test";

/// The checker binary to run as a process; `None` asks the built-in checker.
fn real_checker() -> Option<Utf8PathBuf> {
    ["TSRS_BIN", "CALIPER_TEST_TSC"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .map(Utf8PathBuf::from)
}

/// How a test asks: `replay-tsc` answering from the recording, or a real checker live (a binary, or the built-in one).
enum Asked<'a> {
    Replayed,
    Live(Option<&'a Utf8PathBuf>),
}

fn composed(asked: Asked<'_>) -> (SemanticOutcome, caliper_docgen_core::types::ExtractionOutput) {
    let dir = tempfile::Builder::new().prefix("Composes").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/composes"), &root);
    if let Asked::Replayed = asked {
        let package = root.join("node_modules/typescript");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), format!(r#"{{"version": "{FAKE_VERSION}"}}"#)).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_replay-tsc"), package.join("bin/tsc")).unwrap();
    }
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut request = SemanticRequest::new(root.clone(), targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    match asked {
        Asked::Live(bin) => request.tsc_path = bin.cloned(),
        Asked::Replayed => {
            request.tsc_path = Some(root.join("node_modules/typescript/bin/tsc"));
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/composes.recording.json", env!("CARGO_MANIFEST_DIR")),
            ));
        }
    }
    let outcome = resolve(request);
    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert!(outcome.fallbacks.is_empty(), "{:?}", outcome.fallbacks);
    (outcome, output)
}

fn names(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(ToString::to_string).collect())
}

fn assert_composes(asked: Asked<'_>) {
    let (outcome, mut output) = composed(asked);

    let answered: BTreeMap<&str, &Option<Vec<String>>> =
        outcome.components.iter().map(|component| (component.key.as_str(), &component.composes)).collect();
    assert_eq!(
        answered,
        BTreeMap::from([
            ("Bare", &names(&[])),
            ("Chained", &names(&["HTMLAttributes", "RefAttributes"])),
            ("Direct", &names(&["HTMLAttributes"])),
            ("Mixed", &names(&["HTMLAttributes"])),
            ("Plainly", &names(&["HTMLAttributes"])),
        ])
    );

    merge_semantic(&mut output, outcome);
    let merged: BTreeMap<&str, &[String]> =
        output.components.iter().map(|(key, entry)| (key.as_str(), entry.composes.as_slice())).collect();
    assert_eq!(
        merged,
        BTreeMap::from([
            ("Bare", &[][..]),
            ("Chained", &["HTMLAttributes".to_string(), "RefAttributes".to_string()][..]),
            ("Direct", &["HTMLAttributes".to_string()][..]),
            ("Mixed", &["HTMLAttributes".to_string()][..]),
            ("Plainly", &["HTMLAttributes".to_string()][..]),
        ])
    );
}

#[test]
fn an_instantiated_generic_interface_composes_what_its_generic_type_extends_replayed() {
    assert_composes(Asked::Replayed);
}

#[test]
fn an_instantiated_generic_interface_composes_what_its_generic_type_extends_from_a_real_checker() {
    let checker = real_checker();
    assert_composes(Asked::Live(checker.as_ref()));
}
