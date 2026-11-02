#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A source too large to list is collapsed into an inheritance layer, and the layer names the props it holds: a reader
//! can tell a prop collapsed away from a prop that is not there, and the output still reads back unchanged.

use caliper_docgen_core::pipeline::{extract, CollapseInheritedProps, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::ExtractionOutput;
use camino::Utf8PathBuf;
use serde_json::json;
use tempfile::TempDir;

fn names(prefix: &str) -> Vec<String> {
    (0..31).map(|i| format!("{prefix}{i}")).collect()
}

fn source() -> String {
    let interface_fields: String = names("p").iter().map(|name| format!("{name}?: string; ")).collect();
    let alias_fields: String = names("q").iter().map(|name| format!("{name}?: string; ")).collect();
    format!(
        "export interface Big {{ {interface_fields}}}\n\
         export type BigAlias = {{ {alias_fields}}};\n\
         interface FromInterface extends Big {{ own?: boolean }}\n\
         interface FromAlias extends BigAlias {{ own?: boolean }}\n\
         interface Small {{ a?: string; b?: string }}\n\
         interface FromSmall extends Small {{ own?: boolean }}\n\
         export function WithInterface(props: FromInterface) {{ return null; }}\n\
         export function WithAlias(props: FromAlias) {{ return null; }}\n\
         export function WithSmall(props: FromSmall) {{ return null; }}\n"
    )
}

fn run(collapse: CollapseInheritedProps) -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join("Components.tsx"), source()).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: collapse,
        ..Default::default()
    };
    extract(&options)
}

fn props(output: &ExtractionOutput, component: &str) -> Vec<String> {
    output.components[component].props.keys().cloned().collect()
}

#[test]
fn a_collapsed_interface_layer_names_the_props_it_holds() {
    let output = run(CollapseInheritedProps::All);
    let entry = &output.components["WithInterface"];
    assert_eq!(props(&output, "WithInterface"), ["own"]);
    assert_eq!(entry.inheritance.len(), 1);
    assert_eq!(entry.inheritance[0].type_name, "Big");
    assert_eq!(entry.inheritance[0].total_props, 31);
    let mut expected = names("p");
    expected.sort();
    let mut members = entry.inheritance[0].members.clone();
    members.sort();
    assert_eq!(members, expected);
}

#[test]
fn a_collapsed_alias_layer_names_the_props_it_holds() {
    let output = run(CollapseInheritedProps::All);
    let entry = &output.components["WithAlias"];
    assert_eq!(props(&output, "WithAlias"), ["own"]);
    assert_eq!(entry.inheritance[0].type_name, "BigAlias");
    assert_eq!(entry.inheritance[0].members, names("q"));
}

#[test]
fn an_expanded_source_lists_its_props_and_a_layer_without_members() {
    let output = run(CollapseInheritedProps::All);
    assert_eq!(props(&output, "WithSmall"), ["a", "b", "own"]);
    assert!(output.components["WithSmall"].inheritance.iter().all(|layer| layer.members.is_empty()));
}

#[test]
fn with_collapse_off_every_prop_is_listed_and_no_layer_has_members() {
    let output = run(CollapseInheritedProps::None);
    assert_eq!(props(&output, "WithInterface").len(), 32);
    assert!(output.components["WithInterface"].inheritance.iter().all(|layer| layer.members.is_empty()));
}

#[test]
fn the_canonical_json_writes_a_members_list_id_and_reads_back() {
    let output = run(CollapseInheritedProps::All);
    let json = serde_json::to_value(&output).unwrap();

    let layer = &json["components"]["WithInterface"]["inheritance"][0];
    let id = layer["members"].as_str().expect("a name list id");
    assert_eq!(json["nameLists"][id], json!(output.components["WithInterface"].inheritance[0].members));
    assert!(json["components"]["WithSmall"]["inheritance"][0].get("members").is_none());

    let read: ExtractionOutput = serde_json::from_value(json).unwrap();
    assert_eq!(read.components, output.components);
}
