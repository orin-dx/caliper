#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! The structural lane never expands TypeScript's `Partial`, `Required`, `Pick`, `Omit` or `NonNullable`: the application
//! stays a named type and the checker answers it. `ReactElement` stays narrower than a node, and a union that holds a
//! type parameter keeps its concrete members.

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use camino::Utf8PathBuf;

fn type_text(source: &str, prop: &str) -> String {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("Box.tsx"), source).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.iter().find(|(key, _)| key.ends_with("Box")).expect("the component Box").1;
    entry.props[prop].prop_type.raw_string()
}

const PRELUDE: &str = r#"
/** The class keys. */
interface BaseClasses { /** Root. */ root: string; }
interface Classes extends BaseClasses { label: string; icon?: string | undefined }
type Transition = { mounted: boolean; duration?: number };
"#;

fn text_of(declared: &str) -> String {
    let source = format!("{PRELUDE}\nexport function Box(props: {{ value?: {declared} }}) {{ return null; }}");
    type_text(&source, "value")
}

#[test]
fn partial_stays_an_application() {
    assert_eq!(text_of("Partial<Classes>"), "Partial<Classes>");
}

#[test]
fn required_stays_an_application() {
    assert_eq!(text_of("Required<Classes>"), "Required<Classes>");
}

#[test]
fn pick_stays_an_application() {
    assert_eq!(text_of("Pick<Classes, \"root\" | \"label\">"), "Pick<Classes, \"root\" | \"label\">");
}

#[test]
fn omit_stays_an_application() {
    assert_eq!(text_of("Omit<Classes, \"root\">"), "Omit<Classes, \"root\">");
}

#[test]
fn non_nullable_stays_an_application() {
    assert_eq!(text_of("NonNullable<number | null | undefined>"), "NonNullable<number | null>");
}

#[test]
fn a_react_element_stays_narrower_than_a_node() {
    let source = "import * as React from 'react';\nexport function Box(props: { icon: React.ReactElement; el: JSX.Element }) { return null; }";
    assert_eq!(type_text(source, "icon"), "ReactElement");
    assert_eq!(type_text(source, "el"), "JSX.Element");
}

#[test]
fn a_union_with_a_type_parameter_keeps_its_concrete_members() {
    let source = "import * as React from 'react';\ninterface ShowProps<T> { when: T | null | undefined }\nfunction Show<T>(props: ShowProps<T>) { return null; }\nexport function Box(props: React.ComponentProps<typeof Show>) { return null; }";
    assert_eq!(type_text(source, "when"), "unknown | null");
}
