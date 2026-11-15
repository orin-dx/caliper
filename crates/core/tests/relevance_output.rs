#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Relevance evidence reaches the output (ADR 0028, slice T7): each listed prop carries where `decide` puts it, and a
//! member of a collapsed layer that evidence puts forward is named with its placement, while the props listed and the
//! layers stay as they were.

use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::{ExtractionOutput, Relevance};
use camino::Utf8PathBuf;

/// A component inheriting a 33-member source declared under an `@types/react` path (ambient by path) and a 33-member
/// project source, both collapsed in the `All` mode; three call sites pass `d1` and `b1`; the body reads `d2` and `b2`.
fn extract_fixture() -> (tempfile::TempDir, ExtractionOutput) {
    let dir = tempfile::Builder::new().prefix("RelevanceOutput").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fields = |prefix: &str| -> String { (0..33).map(|i| format!("{prefix}{i}?: string; ")).collect() };
    let dom = root.join("vendor/@types/react");
    std::fs::create_dir_all(&dom).unwrap();
    std::fs::write(dom.join("dom.ts"), format!("export interface DomBits {{ {}}}\n", fields("d"))).unwrap();
    std::fs::write(root.join("big.ts"), format!("export interface Big {{ {}}}\n", fields("b"))).unwrap();
    std::fs::write(
        root.join("Button.tsx"),
        "import type { DomBits } from './vendor/@types/react/dom';\n\
         import type { Big } from './big';\n\
         export interface ButtonProps extends DomBits, Big { tone?: string }\n\
         export function Button(props: ButtonProps) { const { d2, b2 } = props; return <button>{d2}{b2}</button>; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Page.tsx"),
        "import { Button } from './Button';\n\
         export const a = <Button d1=\"x\" b1=\"y\" />;\n\
         export const b = <Button d1=\"x\" b1=\"y\" />;\n\
         export const c = <Button d1=\"x\" b1=\"y\" />;\n",
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        use_cache: false,
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: CollapseInheritedProps::All,
        ..PipelineOptions::default()
    };
    (dir, extract(&options))
}

#[test]
fn a_collapsed_layer_names_the_ambient_members_consumers_pass() {
    let (_dir, output) = extract_fixture();
    let button = &output.components["Button"];
    let listed: Vec<&str> = button.props.keys().map(String::as_str).collect();
    assert_eq!(listed, ["tone"], "the listed props do not change");
    assert!(button.inheritance.iter().any(|layer| layer.members.iter().any(|m| m == "d1")), "{:?}", button.inheritance);
    let relevance = &button.relevance;
    assert_eq!(relevance.get("tone"), Some(&Relevance::Row), "own: {relevance:?}");
    // Passed at every call site: a typed row, ambient or not.
    assert_eq!(relevance.get("d1"), Some(&Relevance::Row), "{relevance:?}");
    assert_eq!(relevance.get("b1"), Some(&Relevance::Row), "{relevance:?}");
    // Read in the body: an ambient member is a `used` name, a project one a row.
    assert_eq!(relevance.get("d2"), Some(&Relevance::Used), "{relevance:?}");
    assert_eq!(relevance.get("b2"), Some(&Relevance::Row), "{relevance:?}");
    // No signal: left in its layer, not named.
    assert_eq!(relevance.get("d3"), None, "{relevance:?}");
    assert_eq!(relevance.get("b3"), None, "{relevance:?}");
}

#[test]
fn the_canonical_json_writes_relevance_at_schema_version_8() {
    let (_dir, output) = extract_fixture();
    let json: serde_json::Value =
        serde_json::from_str(&caliper_docgen_core::pipeline::extraction_output_to_json(&output).unwrap()).unwrap();
    assert_eq!(json["schemaVersion"], 8);
    assert_eq!(json["components"]["Button"]["relevance"]["d2"], "used");
    assert_eq!(json["components"]["Button"]["relevance"]["tone"], "row");
}
