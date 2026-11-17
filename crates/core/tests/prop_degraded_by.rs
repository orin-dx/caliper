#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A prop whose type holds an `opaque` member says why in the canonical JSON: `degradedBy`, the reason kinds of its
//! opaque members. A prop the resolver evaluated in full has no such field, so a reader of the previous output ignores
//! it and the schema version stays (ADR 0017).

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use camino::Utf8PathBuf;
use serde_json::{Value, json};

/// Each prop of the only component in `source`, as the canonical JSON writes it.
fn props_json(source: &str) -> Value {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("case.tsx"), source).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    assert_eq!(output.components.len(), 1, "{:?}", output.components.keys());
    let entry = output.components.into_values().next().unwrap();
    Value::Object(entry.props.iter().map(|(name, prop)| (name.clone(), serde_json::to_value(prop).unwrap())).collect())
}

#[test]
fn a_prop_with_an_opaque_member_names_the_reasons() {
    let props = props_json(
        r#"
        interface P<T> {
          conditional?: T extends string ? 1 : 2
          mapped?: { [K in keyof T]: T[K] }
          both?: (T extends string ? 1 : 2) | { [K in keyof T]: T[K] }
          plain?: string
        }
        export function C<T>(props: P<T>) { return <b /> }
        "#,
    );
    assert_eq!(props["conditional"]["degradedBy"], json!(["conditionalType"]));
    assert_eq!(props["mapped"]["degradedBy"], json!(["mappedType"]));
    assert_eq!(props["both"]["degradedBy"], json!(["conditionalType", "mappedType"]), "each once, in type order");
    assert!(props["plain"].get("degradedBy").is_none(), "an evaluated type has no field: {}", props["plain"]);
}

#[test]
fn a_type_nested_in_an_object_or_an_array_is_read_too() {
    let props = props_json(
        r#"
        interface P<T> {
          field?: { v: T extends string ? 1 : 2 }
          list?: Array<T extends string ? 1 : 2>
        }
        export function C<T>(props: P<T>) { return <b /> }
        "#,
    );
    assert_eq!(props["field"]["degradedBy"], json!(["conditionalType"]));
    assert_eq!(props["list"]["degradedBy"], json!(["conditionalType"]));
}
