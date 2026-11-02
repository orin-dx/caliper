#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A module that exports a component under a function type, `export default Masonry as (<T>(props: P) => Element)`,
//! says in that type what the props are: antd's `Masonry` is declared with `forwardRef<Ref, MasonryProps>` and exported
//! with `PropsWithChildren<MasonryProps<T>>`, so `children` is one of its props.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// Prop name to `(required, type)` of the component `name` in `source`.
fn props_of(source: &str, name: &str) -> BTreeMap<String, (bool, Value)> {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("case.tsx"), source).unwrap();
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

const DECLARATION: &str = r#"
import * as React from 'react'
export interface MasonryProps<Item = any> { items?: Item[]; columns?: number }
export interface MasonryRef { nativeElement: HTMLDivElement }
const Masonry = React.forwardRef<MasonryRef, MasonryProps>((props, ref) => <div>{props.columns}</div>)
"#;

#[test]
fn the_function_type_a_default_export_is_asserted_to_gives_the_props() {
    let props = props_of(
        &format!(
            "{DECLARATION}
            export default Masonry as (<Item = any>(
              props: React.PropsWithChildren<MasonryProps<Item>> & React.RefAttributes<MasonryRef>,
            ) => React.ReactElement) & Pick<React.FC, 'displayName'>
            "
        ),
        "Masonry",
    );
    assert_eq!(
        props["children"],
        (false, json!({ "kind": "reactNode" })),
        "added by PropsWithChildren in the assertion"
    );
    assert_eq!(props["columns"], (false, json!({ "kind": "number" })));
    assert_eq!(
        props["items"],
        (false, json!({ "kind": "array", "element": { "kind": "named", "name": "Item", "args": [] } })),
        "the assertion's own type parameter stands for the item type"
    );
}

#[test]
fn without_an_assertion_the_declared_props_stand() {
    let props = props_of(&format!("{DECLARATION}\nexport default Masonry\n"), "Masonry");
    assert!(!props.contains_key("children"), "{:?}", props.keys());
    assert_eq!(
        props["items"],
        (false, json!({ "kind": "array", "element": { "kind": "any" } })),
        "the declaration's own default for its type parameter"
    );
}

#[test]
fn an_assertion_to_a_type_that_is_not_a_one_parameter_function_changes_nothing() {
    let props =
        props_of(&format!("{DECLARATION}\nexport default Masonry as React.ComponentType<MasonryProps>\n"), "Masonry");
    assert!(!props.contains_key("children"), "{:?}", props.keys());
    let props = props_of(
        &format!("{DECLARATION}\nexport default Masonry as unknown as (props: React.PropsWithChildren<MasonryProps>) => null\n"),
        "Masonry",
    );
    assert!(!props.contains_key("children"), "a double assertion is not read: {:?}", props.keys());
}
