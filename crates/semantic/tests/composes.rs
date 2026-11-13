#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The `@types/react` types a component's props are made of (`composes`), where the props type is an instantiation of a
//! generic interface (ADR 0026 S7a). The checker states the bases of the declaration of an interface and not of
//! `Props<string>`, a reference to it, so they are read from the generic type it instantiates. The `composes` fixture,
//! asked of the built-in checker:
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

fn composed() -> (SemanticOutcome, caliper_docgen_core::types::ExtractionOutput) {
    let dir = tempfile::Builder::new().prefix("Composes").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/composes"), &root);
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut request = SemanticRequest::new(root.clone(), targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    let outcome = resolve(request);
    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert!(outcome.fallbacks.is_empty(), "{:?}", outcome.fallbacks);
    (outcome, output)
}

fn names(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(ToString::to_string).collect())
}

#[test]
fn an_instantiated_generic_interface_composes_what_its_generic_type_extends() {
    let (outcome, mut output) = composed();

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
