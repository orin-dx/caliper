#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Relevance evidence reaches the output (ADR 0028, slice T7): a collapsed layer names in `used` its ambient members that
//! consumers pass at the floor or the body reads, while the props listed and the layers' members stay as they were.

use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::ExtractionOutput;
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
    let used = |type_name: &str| -> Vec<&str> {
        let layer = button.inheritance.iter().find(|layer| layer.type_name == type_name).unwrap();
        layer.used.iter().map(String::as_str).collect()
    };
    // Ambient and passed at every call site, or read in the body: a `used` name in its layer, in the layer's order.
    assert_eq!(used("DomBits"), ["d1", "d2"]);
    // A project member put forward is a typed row, never a `used` name; with nothing put forward, none is named.
    assert_eq!(used("Big"), [] as [&str; 0]);
}

#[test]
fn the_canonical_json_writes_used_at_schema_version_8() {
    let (_dir, output) = extract_fixture();
    let json: serde_json::Value =
        serde_json::from_str(&caliper_docgen_core::pipeline::extraction_output_to_json(&output).unwrap()).unwrap();
    assert_eq!(json["schemaVersion"], 8);
    let layers = json["components"]["Button"]["inheritance"].as_array().unwrap();
    let dom = layers.iter().find(|layer| layer["typeName"] == "DomBits").unwrap();
    assert_eq!(dom["used"], serde_json::json!(["d1", "d2"]));
    assert!(json["components"]["Button"].get("relevance").is_none());
    let big = layers.iter().find(|layer| layer["typeName"] == "Big").unwrap();
    assert!(big.get("used").is_none(), "an empty `used` is not written");
}

/// A collapsed ambient prop the signature gives a destructuring default is read by the body, so its layer names it in
/// `used`; a default on a project member adds nothing (it is a typed row).
#[test]
fn a_collapsed_ambient_prop_with_a_destructuring_default_is_used() {
    let dir = tempfile::Builder::new().prefix("RelevanceDefault").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fields = |prefix: &str| -> String { (0..33).map(|i| format!("{prefix}{i}?: string; ")).collect() };
    std::fs::create_dir_all(root.join("vendor/@types/react")).unwrap();
    std::fs::write(root.join("vendor/@types/react/dom.ts"), format!("export interface DomBits {{ {}}}\n", fields("d")))
        .unwrap();
    std::fs::write(root.join("big.ts"), format!("export interface Big {{ {}}}\n", fields("b"))).unwrap();
    std::fs::write(
        root.join("Button.tsx"),
        "import type { DomBits } from './vendor/@types/react/dom';\n\
         import type { Big } from './big';\n\
         export interface ButtonProps extends DomBits, Big { tone?: string }\n\
         export function Button({ d2 = '', b2 = '' }: ButtonProps) { return <button>{d2}{b2}</button>; }\n",
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        use_cache: false,
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: CollapseInheritedProps::All,
        ..PipelineOptions::default()
    };
    let output = extract(&options);
    let layers = &output.components["Button"].inheritance;
    let used = |type_name: &str| layers.iter().find(|layer| layer.type_name == type_name).unwrap().used.clone();
    assert_eq!(used("DomBits"), ["d2"], "{layers:?}");
    assert!(used("Big").is_empty(), "a project member is a typed row, never `used`: {layers:?}");
}

/// A file-level element whose tag is a local binding of the same name as an imported component is not a call site of
/// that component: usages are keyed by binding, and an inner binding resolves to nothing.
#[test]
fn a_local_binding_named_like_an_imported_component_is_not_its_call_site() {
    let dir = tempfile::Builder::new().prefix("RelevanceScope").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fields: String = (0..33).map(|i| format!("d{i}?: string; ")).collect();
    std::fs::create_dir_all(root.join("vendor/@types/react")).unwrap();
    std::fs::write(root.join("vendor/@types/react/dom.ts"), format!("export interface DomBits {{ {fields}}}\n"))
        .unwrap();
    std::fs::write(
        root.join("Button.tsx"),
        "import type { DomBits } from './vendor/@types/react/dom';\n\
         export interface ButtonProps extends DomBits { tone?: string }\n\
         export function Button(props: ButtonProps) { return null; }\n",
    )
    .unwrap();
    let local = "function helper() { const Button = (p: { d1?: string }) => null; \
                 return [<Button d1=\"x\" />, <Button d1=\"x\" />, <Button d1=\"x\" />]; }";
    std::fs::write(
        root.join("Page.tsx"),
        format!(
            "import {{ Button }} from './Button';\n{local}\nexport const a = <Button d2=\"x\" />;\n\
                 export const b = <Button d2=\"x\" />;\nexport const c = <Button d2=\"x\" />;\nhelper();\n"
        ),
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        use_cache: false,
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: CollapseInheritedProps::All,
        ..PipelineOptions::default()
    };
    let output = extract(&options);
    let layer = &output.components["Button"].inheritance[0];
    assert_eq!(layer.used, ["d2"], "the imported binding's sites only: {layer:?}");
}
