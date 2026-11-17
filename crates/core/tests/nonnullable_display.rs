#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `NonNullable<Alias<T> | undefined>` inside the arguments of a generic alias that prints by name (csstype's
//! `Fallback<T>` under Chakra's `ConditionalValue`): the argument shows reduced, not as a `NonNullable` call.

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use camino::Utf8PathBuf;

fn type_text(source: &str, prop: &str) -> String {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("Box.tsx"), source).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.values().next().expect("one component");
    entry.props[prop].prop_type.raw_string()
}

const PRELUDE: &str = r#"
type Border<T = string | 0> = "none" | T | (string & {});
type Nullable = Border | null;
type Maybe = string | undefined;
type Fallback<T> = { [P in keyof T]: T[P] | NonNullable<T[P]>[] };
interface Shorthand { border?: Border | undefined; maybe?: Nullable | undefined; flag?: Maybe | undefined }
interface Styles extends Fallback<Shorthand> {}
type Conditional<T> = T extends infer U ? U | { _hover: U } : never;
"#;

fn text_of(prop: &str) -> String {
    let source = format!(
        "{PRELUDE}\nexport function Box(props: {{ {prop}: Conditional<Styles[\"{prop}\"]> }}) {{ return null; }}"
    );
    type_text(&source, prop)
}

#[test]
fn a_non_literal_alias_member_loses_undefined() {
    assert_eq!(text_of("border"), "Conditional<Border | undefined | Border[]>");
}

#[test]
fn a_member_that_may_be_nullish_keeps_its_call() {
    assert_eq!(text_of("maybe"), "Conditional<Nullable | undefined | NonNullable<Nullable | undefined>[]>");
    assert_eq!(text_of("flag"), "Conditional<Maybe | undefined | NonNullable<Maybe | undefined>[]>");
}
