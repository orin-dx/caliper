#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `(typeof X)[number]` over `const X = ['a', ...Y] as const` is the union of X's own values and Y's, however many
//! spreads deep. antd's `ButtonColorType` is `['default', 'primary', 'danger', ...PresetColors]` with `PresetColors`
//! in another file.

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use caliper_docgen_core::types::DiagnosticSeverity;
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// The `color` prop's type of the only component, and the messages of the warnings.
fn color_of(files: &[(&str, &str)]) -> (Value, Vec<String>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (name, body) in files {
        std::fs::write(root.join(name), body).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.get("Button").expect("Button");
    let color = serde_json::to_value(&entry.props["color"].prop_type).unwrap();
    let warnings = output
        .diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Warning)
        .map(|d| d.message.clone())
        .collect();
    (color, warnings)
}

fn literals(members: &[&str]) -> Value {
    json!({ "kind": "literalUnion", "members": members, "hasDefault": false })
}

const BUTTON: &str = r#"
    import { ALL } from "./colors";
    type Color = (typeof ALL)[number];
    export function Button(props: { color?: Color }) { return <button />; }
"#;

#[test]
fn a_spread_of_an_array_in_another_file_adds_its_values() {
    let (color, warnings) = color_of(&[
        ("palette.ts", r#"export const PRESETS = ["blue", "red"] as const;"#),
        (
            "colors.ts",
            r#"import { PRESETS } from "./palette"; export const ALL = ["default", "primary", ...PRESETS] as const;"#,
        ),
        ("button.tsx", BUTTON),
    ]);
    assert_eq!(color, literals(&["default", "primary", "blue", "red"]));
    assert_eq!(warnings, Vec::<String>::new());
}

#[test]
fn a_spread_of_a_spread_adds_the_values_of_both() {
    let (color, _) = color_of(&[
        ("palette.ts", r#"export const BASE = ["blue"] as const; export const MORE = [...BASE, "red"] as const;"#),
        ("colors.ts", r#"import { MORE } from "./palette"; export const ALL = [...MORE, "default"] as const;"#),
        ("button.tsx", BUTTON),
    ]);
    assert_eq!(color, literals(&["blue", "red", "default"]));
}

#[test]
fn a_value_in_two_spreads_is_listed_once() {
    let (color, _) = color_of(&[
        (
            "colors.ts",
            r#"const A = ["x", "y"] as const; const B = ["y", "z"] as const; export const ALL = [...A, ...B] as const;"#,
        ),
        ("button.tsx", BUTTON),
    ]);
    assert_eq!(color, literals(&["x", "y", "z"]));
}

#[test]
fn a_spread_of_something_that_is_not_a_project_array_keeps_the_rest_and_warns() {
    let (color, warnings) = color_of(&[
        (
            "colors.ts",
            r#"import { PRESETS } from "some-package"; export const ALL = ["default", ...PRESETS] as const;"#,
        ),
        ("button.tsx", BUTTON),
    ]);
    assert_eq!(color, literals(&["default"]));
    assert_eq!(
        warnings,
        vec![
            "'...PRESETS' in an `as const` array is not an array declared in the project; its values are left out of the literal union"
                .to_owned()
        ]
    );
}

#[test]
fn arrays_that_spread_each_other_end_and_warn() {
    let (color, warnings) = color_of(&[
        (
            "colors.ts",
            r#"const A = ["a", ...B] as const; const B = ["b", ...A] as const; export const ALL = [...A] as const;"#,
        ),
        ("button.tsx", BUTTON),
    ]);
    assert_eq!(color, literals(&["a", "b"]));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].starts_with("'...A' in an `as const` array spreads itself"), "{warnings:?}");
}
