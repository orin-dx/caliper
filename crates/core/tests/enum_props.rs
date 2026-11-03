#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A prop typed with a TypeScript enum is the union of the enum's values, the way TypeScript reads it, and not an
//! unresolved name with a warning per use.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// Prop name to type of the only component in the project, with the messages of every diagnostic.
fn props_of(files: &[(&str, &str)]) -> (BTreeMap<String, Value>, Vec<String>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    assert_eq!(output.components.len(), 1, "{:?}", output.components.keys());
    let diagnostics = output.diagnostics.iter().map(|d| d.message.clone()).collect();
    let entry = output.components.into_values().next().unwrap();
    let props =
        entry.props.iter().map(|(name, prop)| (name.clone(), serde_json::to_value(&prop.prop_type).unwrap())).collect();
    (props, diagnostics)
}

#[test]
fn a_string_enum_prop_is_the_union_of_its_values() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        enum Tone { Primary = "primary", Secondary = "secondary" }
        export function C(props: { tone: Tone; literal: "primary" | "secondary" }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["tone"], props["literal"]);
    assert_eq!(
        props["tone"],
        json!({
            "kind": "union",
            "members": [
                { "kind": "stringLiteral", "value": "primary" },
                { "kind": "stringLiteral", "value": "secondary" },
            ]
        })
    );
    assert_eq!(diagnostics, Vec::<String>::new());
}

#[test]
fn a_numeric_enum_counts_up_from_its_last_initializer() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        enum Level { Low, Mid = 5, High }
        export function C(props: { level: Level; literal: 0 | 5 | 6 }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["level"], props["literal"]);
    assert_eq!(diagnostics, Vec::<String>::new());
}

#[test]
fn an_enum_imported_from_another_file_resolves_too() {
    let (props, diagnostics) = props_of(&[
        ("tone.ts", r#"export enum Tone { Primary = "primary", Secondary = "secondary" }"#),
        (
            "case.tsx",
            r#"
            import { Tone } from "./tone";
            export function C(props: { tone: Tone; literal: "primary" | "secondary" }) { return <b />; }
            "#,
        ),
    ]);
    assert_eq!(props["tone"], props["literal"]);
    assert_eq!(diagnostics, Vec::<String>::new());
}
