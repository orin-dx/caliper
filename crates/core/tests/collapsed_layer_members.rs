#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A source too large to list is collapsed into an inheritance layer, and the layer names the props it holds: a reader
//! can tell a prop collapsed away from a prop that is not there, and the output still reads back unchanged.

use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
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
         export function WithSmall(props: FromSmall) {{ return null; }}\n\
         export function WithOmit(props: Omit<Big, 'p0'> & {{ own?: boolean }}) {{ return null; }}\n\
         type OmitsP0 = Omit<Big, 'p0'>;\n\
         type KeepsP0 = Big;\n\
         export function WithUnion(props: OmitsP0 | KeepsP0) {{ return null; }}\n"
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
fn an_omit_that_leaves_a_source_at_its_threshold_lists_what_reaches() {
    // `Big` declares 31; `Omit<Big, 'p0'>` lets 30 reach the component, which is not past the threshold (30): listed.
    let output = run(CollapseInheritedProps::All);
    let entry = &output.components["WithOmit"];
    assert!(entry.inheritance.iter().all(|layer| layer.members.is_empty()));
    let mut expected: Vec<String> = names("p").into_iter().filter(|name| name != "p0").collect();
    expected.push("own".to_string());
    expected.sort();
    assert_eq!(props(&output, "WithOmit"), expected);
}

#[test]
fn a_union_layer_keeps_a_prop_that_one_branch_omits_and_another_keeps() {
    let output = run(CollapseInheritedProps::All);
    let layer = &output.components["WithUnion"].inheritance[0];
    assert_eq!(layer.type_name, "Big");
    let mut expected = names("p");
    expected.sort();
    let mut members = layer.members.clone();
    members.sort();
    assert_eq!(members, expected);
    assert_eq!(layer.total_props, 31);
    assert!(layer.omitted.is_empty());
}

#[test]
fn the_canonical_json_writes_a_members_list_id() {
    let output = run(CollapseInheritedProps::All);
    let json = serde_json::to_value(&output).unwrap();

    let layer = &json["components"]["WithInterface"]["inheritance"][0];
    let id = layer["members"].as_str().expect("a name list id");
    assert_eq!(json["nameLists"][id], json!(output.components["WithInterface"].inheritance[0].members));
    assert!(json["components"]["WithSmall"]["inheritance"][0].get("members").is_none());
}

/// A project interface of `count` props, `System`, that a component's props type extends.
fn run_project_source(count: usize, collapse: CollapseInheritedProps) -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let fields: String = (0..count).map(|i| format!("s{i}?: string; ")).collect();
    std::fs::write(
        root.join("System.tsx"),
        format!(
            "export interface System {{ {fields}}}\n\
             interface BoxProps extends System {{ own?: boolean }}\n\
             export function Box(props: BoxProps) {{ return null; }}\n"
        ),
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: collapse,
        ..Default::default()
    };
    extract(&options)
}

#[test]
fn by_default_a_project_source_past_the_large_source_threshold_collapses_to_a_layer() {
    // A style system in the project's own source (Chakra's `SystemProperties`, 705) is as large as an installed one.
    let output = run_project_source(201, CollapseInheritedProps::default());
    let entry = &output.components["Box"];
    assert_eq!(props(&output, "Box"), ["own"]);
    assert_eq!(entry.inheritance.len(), 1);
    assert_eq!((entry.inheritance[0].type_name.as_str(), entry.inheritance[0].total_props), ("System", 201));
    assert_eq!(entry.inheritance[0].members.len(), 201);
}

#[test]
fn a_project_source_under_the_large_source_threshold_stays_expanded_and_none_expands_any() {
    assert_eq!(run_project_source(200, CollapseInheritedProps::default()).components["Box"].props.len(), 201);
    let none = run_project_source(201, CollapseInheritedProps::None);
    assert_eq!(none.components["Box"].props.len(), 202);
    assert!(none.components["Box"].inheritance.is_empty());
}

/// Extracts `source` (a `Components.tsx`) with the default collapse.
fn run_source(source: &str) -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join("Components.tsx"), source).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        ..Default::default()
    };
    extract(&options)
}

fn interface(name: &str, prefix: &str, count: usize) -> String {
    let fields: String = (0..count).map(|i| format!("{prefix}{i}?: string; ")).collect();
    format!("export interface {name} {{ {fields}}}\n")
}

fn keys(prefix: &str, range: std::ops::Range<usize>) -> String {
    range.map(|i| format!("'{prefix}{i}'")).collect::<Vec<_>>().join(" | ")
}

#[test]
fn an_omit_lists_only_the_source_it_leaves_small_and_keeps_every_other_collapsed() {
    // `A` declares 250; the `Omit` leaves 150 of it, so `A` is listed. `Big` declares 700 and none is omitted: one layer.
    let source = format!(
        "{}{}export function Box(props: Omit<A & Big, {}>) {{ return null; }}\n",
        interface("A", "a", 250),
        interface("Big", "b", 700),
        keys("a", 0..100)
    );
    let output = run_source(&source);
    let entry = &output.components["Box"];
    let listed_a = entry.props.keys().filter(|name| name.starts_with('a')).count();
    let listed_b = entry.props.keys().filter(|name| name.starts_with('b')).count();
    assert_eq!((listed_a, listed_b), (150, 0));
    let layers: Vec<(&str, u32)> = entry
        .inheritance
        .iter()
        .filter(|layer| !layer.members.is_empty())
        .map(|l| (l.type_name.as_str(), l.total_props))
        .collect();
    assert_eq!(layers, [("Big", 700)]);
}

#[test]
fn a_deeply_nested_omit_reads_its_base_once_per_level() {
    // Sixteen nested `Omit`s, the innermost leaving `Big` at 200 (listed): a base read again at each level would cost
    // 2^16 reads of a 700-member interface.
    let mut ty = "Big".to_string();
    for level in 0..16 {
        let range = if level == 0 { 0..500 } else { 600 + level..601 + level };
        ty = format!("Omit<{ty}, {}>", keys("b", range));
    }
    let source = format!("{}export function Deep(props: {ty}) {{ return null; }}\n", interface("Big", "b", 700));
    let started = std::time::Instant::now();
    let output = run_source(&source);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    let entry = &output.components["Deep"];
    assert!(entry.inheritance.iter().all(|layer| layer.members.is_empty()));
    assert_eq!(entry.props.len(), 200 - 15);
}
