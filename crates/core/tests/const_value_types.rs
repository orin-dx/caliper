#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! The type of a `const` that spells its own type: `typeof SHOW_PARENT` over `const SHOW_PARENT = 'SHOW_PARENT'` is the
//! literal `"SHOW_PARENT"` (antd's Cascader), and `keyof typeof Icons` over an object literal is the union of its keys
//! (antd's Result `status`). A union of `true` and `false` is `boolean`, as TypeScript prints it.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// Prop name to `(required, type)` of the component `name` in the files.
fn props_of(files: &[(&str, &str)], name: &str) -> BTreeMap<String, (bool, Value)> {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (file, source) in files {
        std::fs::write(root.join(file), source).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.values().find(|entry| entry.display_name == name).unwrap_or_else(|| {
        panic!("no component {name}: {:?}", output.components.keys());
    });
    entry
        .props
        .iter()
        .map(|(name, prop)| (name.clone(), (prop.required, serde_json::to_value(&prop.prop_type).unwrap())))
        .collect()
}

fn literals(values: &[&str]) -> Value {
    json!({ "kind": "union", "members": values.iter().map(|v| json!({ "kind": "stringLiteral", "value": v })).collect::<Vec<_>>() })
}

#[test]
fn typeof_a_string_const_is_its_literal() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            export const SHOW_PARENT = 'SHOW_PARENT'
            export const SHOW_CHILD = 'SHOW_CHILD' as const
            interface P { strategy?: typeof SHOW_PARENT | typeof SHOW_CHILD; one?: typeof SHOW_PARENT }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    assert_eq!(props["strategy"], (false, literals(&["SHOW_PARENT", "SHOW_CHILD"])));
    assert_eq!(props["one"], (false, json!({ "kind": "stringLiteral", "value": "SHOW_PARENT" })));
}

#[test]
fn typeof_a_number_or_boolean_const_is_its_literal() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            const LIMIT = 10
            const NEGATIVE = -1
            const ENABLED = true
            interface P { limit?: typeof LIMIT; floor?: typeof NEGATIVE; on?: typeof ENABLED }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    assert_eq!(props["limit"], (false, json!({ "kind": "numberLiteral", "value": 10.0 })));
    assert_eq!(props["floor"], (false, json!({ "kind": "numberLiteral", "value": -1.0 })));
    assert_eq!(props["on"], (false, json!({ "kind": "boolLiteral", "value": true })));
}

#[test]
fn typeof_an_imported_const_is_its_literal() {
    let props = props_of(
        &[
            ("constants.ts", "export const SHOW_PARENT = 'SHOW_PARENT'\n"),
            (
                "case.tsx",
                r#"
                import { SHOW_PARENT } from './constants'
                interface P { strategy?: typeof SHOW_PARENT }
                export function C(props: P) { return <b /> }
                "#,
            ),
        ],
        "C",
    );
    assert_eq!(props["strategy"], (false, json!({ "kind": "stringLiteral", "value": "SHOW_PARENT" })));
}

#[test]
fn typeof_a_let_or_a_computed_const_stays_a_reference() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            export let mutable = 'a'
            export const computed = String(1)
            interface P { m?: typeof mutable; c?: typeof computed }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    assert_eq!(props["m"], (false, json!({ "kind": "named", "name": "typeof mutable", "args": [] })));
    assert_eq!(props["c"], (false, json!({ "kind": "named", "name": "typeof computed", "args": [] })));
}

#[test]
fn keyof_typeof_an_object_const_is_the_union_of_its_keys() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            const IconMap = { success: 1, error: 2, 'in-progress': 3, info: 4 }
            const Sizes = { sm: 'a', lg: 'b' } as const
            interface P { status?: keyof typeof IconMap; size?: keyof typeof Sizes }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    assert_eq!(props["status"], (false, literals(&["success", "error", "in-progress", "info"])));
    assert_eq!(props["size"], (false, literals(&["sm", "lg"])));
}

#[test]
fn keyof_typeof_an_object_with_a_spread_or_a_computed_key_stays_opaque() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            const Base = { a: 1 }
            const key = 'k'
            const Spread = { ...Base, b: 2 }
            const Computed = { [key]: 1 }
            interface P { spread?: keyof typeof Spread; computed?: keyof typeof Computed }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    for name in ["spread", "computed"] {
        assert_eq!(props[name].1["kind"], "opaque", "{name}: the key set is not known, so nothing is claimed");
    }
}

#[test]
fn keyof_an_interface_is_still_not_resolved() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            interface Shape { a: string; [extra: string]: unknown }
            interface P { key?: keyof Shape }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    assert_eq!(props["key"].1["kind"], "opaque", "an index signature makes the keys `string | number`");
}

#[test]
fn a_union_of_true_and_false_is_boolean() {
    let props = props_of(
        &[(
            "case.tsx",
            r#"
            type Flag = true | false
            interface P { a?: true | false; b?: Flag | string; c?: boolean | true; d?: true | string }
            export function C(props: P) { return <b /> }
            "#,
        )],
        "C",
    );
    assert_eq!(props["a"], (false, json!({ "kind": "boolean" })));
    assert_eq!(
        props["b"],
        (false, json!({ "kind": "union", "members": [{ "kind": "boolean" }, { "kind": "string" }] })),
        "boolean stands where the literals were"
    );
    assert_eq!(props["c"], (false, json!({ "kind": "boolean" })));
    assert_eq!(
        props["d"],
        (
            false,
            json!({ "kind": "union", "members": [{ "kind": "boolLiteral", "value": true }, { "kind": "string" }] })
        ),
        "one literal alone stays a literal"
    );
}
