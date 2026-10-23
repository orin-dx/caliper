use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::*;
use crate::types::{DefaultValue, DetectionRule, PropParent, PropType};

fn prop(name: &str, default: Option<&str>) -> ParsedProp {
    ParsedProp::new(
        name.to_string(),
        PropType::Union(vec![PropType::StringLiteral("sm".into()), PropType::StringLiteral("lg".into())]),
        false,
        default.map(|value| DefaultValue { value: value.to_string(), computed: false }),
        "Size of the control".to_string(),
        BTreeMap::from([("since".to_string(), "2.0".to_string())]),
        Some(PropParent { name: "SizeProps".to_string(), file_name: "/src/size.ts".to_string() }),
        vec![],
    )
}

fn component(name: &str, props: &[ParsedProp], notable: &[ParsedProp]) -> ComponentEntry {
    let by_name = |props: &[ParsedProp]| props.iter().map(|p| (p.name.clone(), p.clone())).collect();
    ComponentEntry {
        display_name: name.to_string(),
        file_path: format!("/src/{name}.tsx").into(),
        description: format!("The {name}."),
        props: by_name(props),
        inheritance: vec![],
        notable_inherited: by_name(notable),
        discriminant_prop: None,
        composes: vec!["Base".to_string()],
        tags: BTreeMap::new(),
        methods: vec![],
        detected_by: DetectionRule::FunctionDeclaration,
    }
}

fn output(components: Vec<ComponentEntry>) -> ExtractionOutput {
    ExtractionOutput {
        components: components.into_iter().map(|c| (c.display_name.clone(), c)).collect(),
        enums: BTreeMap::new(),
        diagnostics: vec![],
        stats: ExtractionStats { components_extracted: 2, ..Default::default() },
    }
}

fn id_of(prop: &ParsedProp) -> String {
    PropId::of(&serde_json::to_vec(prop).unwrap()).to_string()
}

#[test]
fn an_id_is_the_fnv1a_hash_of_the_props_compact_json() {
    // The published FNV-1a 64 test vectors.
    assert_eq!(PropId::of(b"").to_string(), "cbf29ce484222325");
    assert_eq!(PropId::of(b"a").to_string(), "af63dc4c8601ec8c");
    assert_eq!(PropId::of(b"foobar").to_string(), "85944171f73967e8");
}

#[test]
fn a_prop_shared_by_components_is_written_once_and_a_different_default_is_its_own_entry() {
    let size = prop("size", None);
    let small = prop("size", Some("sm"));
    let out = output(vec![
        component("Button", &[size.clone(), prop("tone", None)], &[]),
        component("Input", &[size.clone()], &[size.clone()]),
        component("Chip", &[small.clone()], &[]),
    ]);

    let json = serde_json::to_value(&out).unwrap();

    let ids = |key: &str, field: &str| json["components"][key][field].clone();
    assert_eq!(ids("Button", "props"), json!({ "size": id_of(&size), "tone": id_of(&prop("tone", None)) }));
    assert_eq!(ids("Input", "props"), json!({ "size": id_of(&size) }));
    assert_eq!(ids("Input", "notableInherited"), json!({ "size": id_of(&size) }));
    assert_eq!(ids("Chip", "props"), json!({ "size": id_of(&small) }));
    let table: BTreeMap<String, Value> = serde_json::from_value(json["props"].clone()).unwrap();
    assert_eq!(
        table,
        BTreeMap::from([
            (id_of(&size), serde_json::to_value(&size).unwrap()),
            (id_of(&small), serde_json::to_value(&small).unwrap()),
            (id_of(&prop("tone", None)), serde_json::to_value(prop("tone", None)).unwrap()),
        ])
    );
}

#[test]
fn expanding_the_ids_gives_exactly_the_component_entry_json() {
    let entry = component("Button", &[prop("size", Some("sm")), prop("tone", None)], &[prop("id", None)]);
    let json = serde_json::to_value(output(vec![entry.clone()])).unwrap();

    let mut written = json["components"]["Button"].clone();
    for field in ["props", "notableInherited"] {
        for value in written[field].as_object_mut().unwrap().values_mut() {
            *value = json["props"][value.as_str().unwrap()].clone();
        }
    }
    assert_eq!(written, serde_json::to_value(&entry).unwrap());
    assert_eq!(
        json.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["components", "props", "enums", "diagnostics", "stats"]
    );
}

#[test]
fn the_canonical_json_reads_back_into_the_same_output() {
    let out = output(vec![
        component("Button", &[prop("size", None)], &[prop("size", None)]),
        component("Chip", &[prop("size", Some("sm"))], &[]),
    ]);
    let read: ExtractionOutput = serde_json::from_str(&serde_json::to_string(&out).unwrap()).unwrap();
    assert_eq!(read, out);
}

#[test]
fn reading_an_id_missing_from_the_table_fails_naming_it() {
    let json = json!({
        "components": { "Button": serde_json::to_value(component("Button", &[], &[])).unwrap() },
        "props": {},
        "enums": {},
        "diagnostics": [],
        "stats": serde_json::to_value(ExtractionStats::default()).unwrap(),
    });
    let mut json = json;
    json["components"]["Button"]["props"] = json!({ "size": "00000000000000aa" });

    let error = serde_json::from_value::<ExtractionOutput>(json).unwrap_err();
    assert_eq!(error.to_string(), "prop id 00000000000000aa is not in the `props` table");
}

#[test]
fn two_different_props_with_one_id_fail_instead_of_sharing_an_entry() {
    let size = prop("size", None);
    let tone = prop("tone", None);
    fn row(p: &ParsedProp) -> Row<'_> {
        (p.name.as_str(), PropId(7), p, serde_json::to_vec(p).unwrap())
    }
    let mut table = PropTable { entries: BTreeMap::new() };

    let first: Result<_, serde_json::Error> = table.add(vec![row(&size)]);
    assert_eq!(first.unwrap(), [("size", PropId(7))]);
    let again: Result<_, serde_json::Error> = table.add(vec![row(&size)]);
    assert_eq!(again.unwrap(), [("size", PropId(7))], "the same prop again shares the entry");

    let clash: Result<_, serde_json::Error> = table.add(vec![row(&tone)]);
    let message = clash.unwrap_err().to_string();
    assert!(message.starts_with("prop 'tone' and a different prop have the same id 0000000000000007: "), "{message}");
    assert_eq!(table.entries.len(), 1);
}
