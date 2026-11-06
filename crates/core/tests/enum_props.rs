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
    // An empty `typescript` install, so no missing-package warning joins the diagnostics.
    let lib = root.join("node_modules/typescript/lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(root.join("node_modules/typescript/package.json"), r#"{"name":"typescript","version":"5.9.3"}"#)
        .unwrap();
    for file in ["lib.es5.d.ts", "lib.dom.d.ts"] {
        std::fs::write(lib.join(file), "").unwrap();
    }
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

#[test]
fn a_string_literal_is_not_assignable_to_a_string_enum() {
    let (props, _) = props_of(&[(
        "case.tsx",
        r#"
        enum Tone { Primary = "primary" }
        export function C(props: { value: "primary" extends Tone ? number : string }) { return <b />; }
        "#,
    )]);
    // TypeScript selects `string`; the conditional stays unevaluated rather than choosing the wrong branch.
    assert_eq!(
        props["value"],
        json!({
            "kind": "opaque",
            "raw": "\"primary\" extends Tone ? number : string",
            "reason": { "type": "conditionalType" }
        })
    );
}

#[test]
fn merged_enum_declarations_contribute_all_their_values() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        enum Tone { Primary = "primary" }
        enum Tone { Secondary = "secondary" }
        export function C(props: { tone: Tone; literal: "primary" | "secondary" }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["tone"], props["literal"]);
    assert_eq!(diagnostics, Vec::<String>::new());
}

#[test]
fn a_merged_enum_with_a_computed_member_in_one_declaration_has_no_partial_union() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        declare const compute: () => string;
        enum Tone { Primary = "primary" }
        enum Tone { Secondary = compute() }
        export function C(props: { tone: Tone }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["tone"], json!({ "kind": "named", "name": "Tone", "args": [] }));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
}

#[test]
fn an_ambient_enum_without_initializers_has_no_invented_values() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        declare enum Level { Low, High }
        export function C(props: { level: Level }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["level"], json!({ "kind": "named", "name": "Level", "args": [] }));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
}

#[test]
fn an_ambient_const_enum_counts_up_like_any_other() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        declare const enum Level { Low, High }
        export function C(props: { level: Level; literal: 0 | 1 }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["level"], props["literal"]);
    assert_eq!(diagnostics, Vec::<String>::new());
}

#[test]
fn an_enum_with_a_value_that_is_not_a_literal_keeps_the_unresolved_name_and_its_warning() {
    for members in ["Low = compute(), High = 1", "Low = 1, High = Low", "Low = true", "Low = 1 << 2", ""] {
        let source = format!(
            "declare const compute: () => number;\nenum Level {{ {members} }}\n\
             export function C(props: {{ level: Level }}) {{ return <b />; }}"
        );
        let (props, diagnostics) = props_of(&[("case.tsx", source.as_str())]);
        assert_eq!(props["level"], json!({ "kind": "named", "name": "Level", "args": [] }), "{members}");
        assert_eq!(diagnostics.len(), 1, "{members}: {diagnostics:?}");
        assert!(diagnostics[0].starts_with("Cannot resolve type 'Level'"), "{members}: {diagnostics:?}");
    }
}

#[test]
fn an_enum_in_a_declaration_file_or_a_declare_namespace_has_no_invented_values() {
    for (file, source) in [
        (
            "case.d.ts",
            "export enum Level { Low, High }\nexport declare function C(props: { level: Level }): JSX.Element;",
        ),
        (
            "case.tsx",
            "declare namespace Ui { enum Level { Low, High } }\n\
             export function C(props: { level: Ui.Level }) { return <b />; }",
        ),
    ] {
        let (props, _) = props_of(&[(file, source)]);
        assert_ne!(props["level"]["kind"], "union", "{file}: {}", props["level"]);
    }
}

#[test]
fn enums_in_function_bodies_do_not_replace_or_merge_into_a_module_level_name() {
    let (props, diagnostics) = props_of(&[(
        "case.tsx",
        r#"
        type Tone = string;
        function first() { enum Tone { A = "a" } return Tone.A; }
        function second() { enum Tone { B = "b" } return Tone.B; }
        enum Level { Low = 1 }
        function third() { enum Level { High = 2 } return Level.High; }
        export function C(props: { tone: Tone; level: Level }) { return <b />; }
        "#,
    )]);
    assert_eq!(props["tone"], json!({ "kind": "string" }));
    assert_eq!(props["level"], json!({ "kind": "numberLiteral", "value": 1.0 }));
    assert_eq!(diagnostics, Vec::<String>::new());
}
