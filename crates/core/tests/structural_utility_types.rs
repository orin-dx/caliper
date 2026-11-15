#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A prop typed by TypeScript's own `Partial`, `Required`, `Pick`, `Omit` or `NonNullable` over a type whose members are
//! known reads as the type TypeScript computes (MUI's `classes?: Partial<AccordionClasses>`), not as the application;
//! `ReactElement` stays narrower than a node, and `unknown` absorbs the other members of a union.

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
fn partial_of_an_interface_lists_its_members_and_its_parents_optional() {
    assert_eq!(text_of("Partial<Classes>"), "{ label?: string; icon?: string; root?: string }");
}

#[test]
fn required_of_an_object_drops_undefined_from_optional_members() {
    assert_eq!(text_of("Required<Classes>"), "{ label: string; icon: string; root: string }");
}

#[test]
fn pick_and_omit_filter_the_members_by_their_literal_keys() {
    assert_eq!(text_of("Pick<Classes, \"root\" | \"label\">"), "{ label: string; root: string }");
    assert_eq!(text_of("Partial<Omit<Transition, \"mounted\">>"), "{ duration?: number }");
}

#[test]
fn omit_of_an_array_by_the_keys_of_an_array_is_empty() {
    assert_eq!(text_of("Classes | Omit<Classes[], keyof any[]>"), "Classes | {  }");
}

#[test]
fn a_generic_interface_or_a_key_set_not_known_stays_an_application() {
    let source = format!(
        "{PRELUDE}\ninterface Slots<T> {{ root: T }}\nexport function Box(props: {{ a?: Partial<Slots<string>>; b?: Omit<Classes, string> }}) {{ return null; }}"
    );
    assert_eq!(type_text(&source, "a"), "Partial<Slots<string>>");
    assert_eq!(type_text(&source, "b"), "Omit<Classes, string>");
}

#[test]
fn non_nullable_of_a_known_type_drops_null_and_undefined() {
    assert_eq!(text_of("NonNullable<number | null | undefined>"), "number");
}

#[test]
fn a_react_element_stays_narrower_than_a_node() {
    let source = "import * as React from 'react';\nexport function Box(props: { icon: React.ReactElement; el: JSX.Element }) { return null; }";
    assert_eq!(type_text(source, "icon"), "ReactElement");
    assert_eq!(type_text(source, "el"), "JSX.Element");
}

#[test]
fn unknown_absorbs_the_other_members_of_a_union() {
    let source = "import * as React from 'react';\ninterface ShowProps<T> { when: T | null | undefined }\nfunction Show<T>(props: ShowProps<T>) { return null; }\nexport function Box(props: React.ComponentProps<typeof Show>) { return null; }";
    assert_eq!(type_text(source, "when"), "unknown");
}
