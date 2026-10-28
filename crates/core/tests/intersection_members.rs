#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `(A & B)['key']`: a property of an intersection is read from the constituents that declare it, as TypeScript reads
//! it. A constituent with only an index signature adds nothing to a name another constituent declares (Chakra's
//! `Tokens = { spacing: SpacingToken } & { [token: string]: never }`).

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// The props of the component `name` (`(required, type)` per prop) in a project of one `source`, and its diagnostics
/// as `"Code: message"`.
fn props_of(source: &str, name: &str) -> (BTreeMap<String, (bool, Value)>, Vec<String>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("component.tsx"), source).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output
        .components
        .iter()
        .find(|(_, entry)| entry.display_name == name)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| panic!("no component {name}: {:?}", output.components.keys()));
    let props = entry
        .props
        .iter()
        .map(|(name, prop)| (name.clone(), (prop.required, serde_json::to_value(&prop.prop_type).unwrap())))
        .collect();
    let diagnostics = output.diagnostics.iter().map(|d| format!("{:?}: {}", d.code, d.message)).collect();
    (props, diagnostics)
}

fn count(diagnostics: &[String], code: &str) -> usize {
    diagnostics.iter().filter(|d| d.starts_with(code)).count()
}

#[test]
fn a_property_beside_an_index_signature_is_read_from_the_constituent_that_declares_it() {
    let (props, diagnostics) = props_of(
        r#"
        type Spacing = "1" | "2";
        type Tokens = { spacing: Spacing; count: number } & { [token: string]: never };
        export function Box(props: { margin: Tokens["spacing"]; n: Tokens["count"] }) { return <b />; }
        "#,
        "Box",
    );
    assert_eq!(props["n"], (true, json!({ "kind": "number" })));
    assert_eq!(props["margin"], (true, json!({ "kind": "literalUnion", "members": ["1", "2"], "hasDefault": false })));
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 0, "{diagnostics:?}");
}

#[test]
fn a_property_declared_by_two_constituents_is_their_intersection() {
    let (props, diagnostics) = props_of(
        r#"
        type A = { id: string };
        type B = { id: "x" | "y" };
        export function Box(props: { id: (A & B)["id"] }) { return <b />; }
        "#,
        "Box",
    );
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 0, "{diagnostics:?}");
    // Not reduced to `"x" | "y"` the way TypeScript simplifies it; both constituents' types are kept.
    assert_eq!(
        props["id"],
        (
            true,
            json!({ "kind": "intersection", "members": [
                { "kind": "string" },
                { "kind": "union", "members": [
                    { "kind": "stringLiteral", "value": "x" },
                    { "kind": "stringLiteral", "value": "y" },
                ] },
            ] })
        )
    );
}

#[test]
fn a_property_no_constituent_declares_stays_opaque_with_one_diagnostic() {
    let (props, diagnostics) = props_of(
        r#"
        type Tokens = { spacing: "1" | "2" } & { other: number };
        export function Box(props: { missing: Tokens["nope"] }) { return <b />; }
        "#,
        "Box",
    );
    assert_eq!(props["missing"].1["kind"], json!("opaque"));
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 1, "{diagnostics:?}");
}
