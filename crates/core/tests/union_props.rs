#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A component whose props are a union of object types (`(A & {…}) | (B & {…})`, antd's `CascaderPanel`, shadcn's
//! `SidebarMenuButton`) has the props of every branch: a prop the branches type differently is the union of those
//! types, and one that some branch lacks or marks optional is optional.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// Prop name to `(required, type)` of the only component in `source`.
fn props_of(source: &str) -> BTreeMap<String, (bool, Value)> {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("case.tsx"), source).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    assert_eq!(output.components.len(), 1, "{:?}", output.components.keys());
    let entry = output.components.into_values().next().unwrap();
    entry
        .props
        .iter()
        .map(|(name, prop)| (name.clone(), (prop.required, serde_json::to_value(&prop.prop_type).unwrap())))
        .collect()
}

fn bool_literals() -> Value {
    json!({ "kind": "union", "members": [
        { "kind": "boolLiteral", "value": false },
        { "kind": "boolLiteral", "value": true },
    ] })
}

#[test]
fn a_union_of_intersections_widens_a_prop_the_branches_type_differently() {
    let props = props_of(
        r#"
        interface Base { label?: string }
        type P = (Base & { multiple?: false; extra: string }) | (Base & { multiple: true; other?: number });
        export function C(props: P) { return <b />; }
        "#,
    );
    assert_eq!(props["multiple"], (false, bool_literals()), "optional in one branch, typed differently in each");
    assert_eq!(props["label"], (false, json!({ "kind": "string" })), "the same in both branches stays as written");
    assert_eq!(props["extra"], (false, json!({ "kind": "string" })), "required in one branch only");
    assert_eq!(props["other"], (false, json!({ "kind": "number" })), "optional, and absent from the other branch");
}

#[test]
fn a_prop_required_in_every_branch_stays_required() {
    let props = props_of(
        r#"
        type P = ({ kind: "a"; id: string } & { x: number }) | ({ kind: "b"; id: string } & { y: number });
        export function C(props: P) { return <b />; }
        "#,
    );
    assert_eq!(props["id"], (true, json!({ "kind": "string" })));
    assert_eq!(props["x"], (false, json!({ "kind": "number" })));
    assert_eq!(props["y"], (false, json!({ "kind": "number" })));
}

#[test]
fn one_named_member_and_one_inline_member_are_merged_too() {
    let props = props_of(
        r#"
        interface Open { open?: undefined }
        type P = Open | { open: boolean; onOpenChange: (open: boolean) => void };
        export function C(props: P) { return <b />; }
        "#,
    );
    assert_eq!(props["open"].0, false, "optional in the first branch");
    assert_eq!(props["onOpenChange"].0, false, "absent from the first branch");
}

#[test]
fn a_single_object_type_is_unchanged() {
    let props = props_of(
        r#"
        interface P { a: string; b?: number }
        export function C(props: P) { return <b />; }
        "#,
    );
    assert_eq!(props["a"], (true, json!({ "kind": "string" })));
    assert_eq!(props["b"], (false, json!({ "kind": "number" })));
}
