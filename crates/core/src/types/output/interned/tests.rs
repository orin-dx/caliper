use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::*;
use crate::types::{DefaultValue, DetectionRule, DiagnosticCode, DiagnosticSeverity, PropParent, PropType};

fn prop(name: &str, default: Option<&str>) -> ParsedProp {
    prop_in(name, default, Some("SizeProps"))
}

fn prop_in(name: &str, default: Option<&str>, parent: Option<&str>) -> ParsedProp {
    ParsedProp::new(
        name.to_string(),
        PropType::Union(vec![PropType::StringLiteral("sm".into()), PropType::StringLiteral("lg".into())]),
        false,
        default.map(|value| DefaultValue { value: value.to_string(), computed: false }),
        "Size of the control".to_string(),
        BTreeMap::from([("since".to_string(), "2.0".to_string())]),
        parent.map(|name| PropParent { name: name.to_string(), file_name: format!("/src/{name}.ts") }),
        vec![],
    )
}

fn layer(omitted: &[&str]) -> InheritedLayer {
    InheritedLayer {
        type_name: "ButtonHTMLAttributes".to_string(),
        file_name: "react.d.ts".to_string(),
        omitted: omitted.iter().map(|name| name.to_string()).collect(),
        html_element: Some("button".to_string()),
        total_props: 3,
        members: vec![],
    }
}

/// A collapsed source: it lists the props it declares and none of them as props.
fn collapsed(members: &[&str]) -> InheritedLayer {
    InheritedLayer {
        type_name: "DOMProps".to_string(),
        file_name: "dom.d.ts".to_string(),
        omitted: vec![],
        html_element: None,
        total_props: members.len() as u32,
        members: members.iter().map(|name| name.to_string()).collect(),
    }
}

fn component(name: &str, props: &[ParsedProp], notable: &[ParsedProp]) -> ComponentEntry {
    let by_name =
        |props: &[ParsedProp]| props.iter().map(|p| (p.name.clone(), std::sync::Arc::new(p.clone()))).collect();
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
        exported: true,
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

/// The id the JSON gives the prop set that holds `props`, in name order.
fn set_id(props: &[&ParsedProp]) -> String {
    let mut entries: Vec<(&str, PropId)> =
        props.iter().map(|p| (p.name.as_str(), PropId::of(&serde_json::to_vec(p).unwrap()))).collect();
    entries.sort();
    PropId::of_set(&entries).to_string()
}

/// A prop set as the table holds it: prop name to prop id.
fn set_json(props: &[&ParsedProp]) -> Value {
    Value::Object(props.iter().map(|p| (p.name.clone(), json!(id_of(p)))).collect())
}

#[test]
fn a_set_id_hashes_the_names_and_prop_ids_with_boundaries_no_name_can_forge() {
    assert_eq!(PropId::of_set(&[]).to_string(), "cbf29ce484222325", "the empty set hashes to the FNV offset basis");
    let (x, y) = (PropId(1), PropId(2));
    assert_ne!(PropId::of_set(&[("a", x), ("b", y)]), PropId::of_set(&[("ab", x), ("", y)]));
    assert_ne!(PropId::of_set(&[("a", x)]), PropId::of_set(&[("a", y)]));
    assert_ne!(PropId::of_list(&["a".into(), "b".into()]), PropId::of_list(&["ab".into()]));
}

#[test]
fn a_prop_shared_by_components_is_written_once_and_a_different_default_is_its_own_entry() {
    let size = prop("size", None);
    let small = prop("size", Some("sm"));
    let tone = prop("tone", None);
    let out = output(vec![
        component("Button", &[size.clone(), tone.clone()], &[]),
        component("Input", &[size.clone()], &[size.clone()]),
        component("Chip", &[small.clone()], &[]),
    ]);

    let json = serde_json::to_value(&out).unwrap();

    let field = |key: &str, field: &str| json["components"][key][field].clone();
    assert_eq!(field("Button", "props"), json!([set_id(&[&size, &tone])]));
    assert_eq!(field("Input", "props"), json!([set_id(&[&size])]));
    assert_eq!(field("Input", "notableInherited"), json!({ "size": id_of(&size) }));
    assert_eq!(field("Chip", "props"), json!([set_id(&[&small])]));
    let table: BTreeMap<String, Value> = serde_json::from_value(json["props"].clone()).unwrap();
    assert_eq!(
        table,
        BTreeMap::from([
            (id_of(&size), serde_json::to_value(&size).unwrap()),
            (id_of(&small), serde_json::to_value(&small).unwrap()),
            (id_of(&tone), serde_json::to_value(&tone).unwrap()),
        ])
    );
}

#[test]
fn props_a_type_declares_form_one_set_that_components_inheriting_it_share() {
    let (size, tone) = (prop("size", None), prop("tone", None));
    let on_click = prop_in("onClick", None, Some("ButtonProps"));
    let value = prop_in("value", None, Some("InputProps"));
    let id = prop_in("id", None, None);
    let out = output(vec![
        component("Button", &[size.clone(), tone.clone(), on_click.clone(), id.clone()], &[]),
        component("Input", &[size.clone(), tone.clone(), value.clone()], &[]),
    ]);

    let json = serde_json::to_value(&out).unwrap();

    // A prop with no declaring type is a set of its own, first; the rest follow in declaring-type order.
    assert_eq!(
        json["components"]["Button"]["props"],
        json!([set_id(&[&id]), set_id(&[&on_click]), set_id(&[&size, &tone])])
    );
    assert_eq!(json["components"]["Input"]["props"], json!([set_id(&[&value]), set_id(&[&size, &tone])]));
    let sets: BTreeMap<String, Value> = serde_json::from_value(json["propSets"].clone()).unwrap();
    assert_eq!(
        sets,
        BTreeMap::from([
            (set_id(&[&id]), set_json(&[&id])),
            (set_id(&[&on_click]), set_json(&[&on_click])),
            (set_id(&[&size, &tone]), set_json(&[&size, &tone])),
            (set_id(&[&value]), set_json(&[&value])),
        ]),
        "the set Button and Input both inherit is written once"
    );
}

#[test]
fn an_omitted_list_two_layers_share_is_written_once() {
    let mut button = component("Button", &[], &[]);
    button.inheritance = vec![layer(&["ref"]), layer(&["color", "size"])];
    let mut input = component("Input", &[], &[]);
    input.inheritance = vec![layer(&["ref"]), layer(&[])];

    let json = serde_json::to_value(output(vec![button, input])).unwrap();

    let list = |names: &[&str]| PropId::of_list(&names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).to_string();
    let omitted = |key: &str| -> Vec<Value> {
        json["components"][key]["inheritance"].as_array().unwrap().iter().map(|l| l["omitted"].clone()).collect()
    };
    assert_eq!(omitted("Button"), [json!(list(&["ref"])), json!(list(&["color", "size"]))]);
    assert_eq!(omitted("Input"), [json!(list(&["ref"])), json!(list(&[]))]);
    let lists: BTreeMap<String, Value> = serde_json::from_value(json["nameLists"].clone()).unwrap();
    assert_eq!(
        lists,
        BTreeMap::from([
            (list(&["ref"]), json!(["ref"])),
            (list(&["color", "size"]), json!(["color", "size"])),
            (list(&[]), json!([])),
        ])
    );
}

#[test]
fn a_collapsed_layer_writes_its_members_once_and_an_expanded_layer_writes_none() {
    let mut accordion = component("Accordion", &[], &[]);
    accordion.inheritance = vec![collapsed(&["onClick", "onFocus"]), layer(&[])];
    let mut button = component("Button", &[], &[]);
    button.inheritance = vec![collapsed(&["onClick", "onFocus"])];

    let json = serde_json::to_value(output(vec![accordion, button])).unwrap();

    let list = |names: &[&str]| PropId::of_list(&names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).to_string();
    let members = list(&["onClick", "onFocus"]);
    let layers = |key: &str| json["components"][key]["inheritance"].clone();
    assert_eq!(
        layers("Accordion"),
        json!([
            {"typeName": "DOMProps", "fileName": "dom.d.ts", "omitted": list(&[]), "htmlElement": null, "totalProps": 2, "members": members},
            {"typeName": "ButtonHTMLAttributes", "fileName": "react.d.ts", "omitted": list(&[]), "htmlElement": "button", "totalProps": 3},
        ])
    );
    assert_eq!(layers("Button")[0]["members"], json!(members));
    let lists: BTreeMap<String, Value> = serde_json::from_value(json["nameLists"].clone()).unwrap();
    assert_eq!(lists, BTreeMap::from([(members, json!(["onClick", "onFocus"])), (list(&[]), json!([]))]));
}

#[test]
fn a_collapsed_layers_members_read_back() {
    let mut accordion = component("Accordion", &[prop("size", None)], &[]);
    accordion.inheritance = vec![collapsed(&["onClick", "onFocus"]), layer(&["ref"])];
    let out = output(vec![accordion]);
    let read: ExtractionOutput = serde_json::from_str(&serde_json::to_string(&out).unwrap()).unwrap();
    assert_eq!(read, out);
    assert_eq!(read.components["Accordion"].inheritance[0].members, ["onClick", "onFocus"]);
}

#[test]
fn expanding_the_ids_gives_exactly_the_component_entry_json() {
    let mut entry = component(
        "Button",
        &[prop("size", Some("sm")), prop("tone", None), prop_in("onClick", None, Some("ButtonProps"))],
        &[prop("id", None)],
    );
    entry.inheritance = vec![layer(&["ref", "color"])];
    let json = serde_json::to_value(output(vec![entry.clone()])).unwrap();

    let mut written = json["components"]["Button"].clone();
    let mut props = serde_json::Map::new();
    for set_id in written["props"].as_array().unwrap() {
        for (name, id) in json["propSets"][set_id.as_str().unwrap()].as_object().unwrap() {
            props.insert(name.clone(), json["props"][id.as_str().unwrap()].clone());
        }
    }
    written["props"] = Value::Object(props);
    for value in written["notableInherited"].as_object_mut().unwrap().values_mut() {
        *value = json["props"][value.as_str().unwrap()].clone();
    }
    written["inheritance"][0]["omitted"] =
        json["nameLists"][written["inheritance"][0]["omitted"].as_str().unwrap()].clone();

    assert_eq!(written, serde_json::to_value(&entry).unwrap());
    assert_eq!(
        json.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["schemaVersion", "components", "props", "propSets", "nameLists", "enums", "diagnostics", "stats"]
    );
}

#[test]
fn the_canonical_json_reads_back_into_the_same_output() {
    let mut button = component(
        "Button",
        &[prop("size", None), prop_in("onClick", None, Some("ButtonProps")), prop_in("id", None, None)],
        &[prop("size", None)],
    );
    button.inheritance = vec![layer(&["ref"]), layer(&[])];
    let out = output(vec![button, component("Chip", &[prop("size", Some("sm"))], &[])]);
    let read: ExtractionOutput = serde_json::from_str(&serde_json::to_string(&out).unwrap()).unwrap();
    assert_eq!(read, out);
}

/// A canonical document whose one component is `Button` with these `props`, `inheritance` and tables.
fn document(props: Value, inheritance: Value, prop_sets: Value, name_lists: Value) -> Value {
    let mut button = serde_json::to_value(component("Button", &[], &[])).unwrap();
    button["props"] = props;
    button["inheritance"] = inheritance;
    json!({
        "schemaVersion": crate::types::SCHEMA_VERSION,
        "components": { "Button": button },
        "props": {},
        "propSets": prop_sets,
        "nameLists": name_lists,
        "enums": {},
        "diagnostics": [],
        "stats": serde_json::to_value(ExtractionStats::default()).unwrap(),
    })
}

fn read_error(document: Value) -> String {
    serde_json::from_value::<ExtractionOutput>(document).unwrap_err().to_string()
}

#[test]
fn reading_an_id_missing_from_its_table_fails_naming_it() {
    let no_layers = json!([]);
    assert_eq!(
        read_error(document(json!(["00000000000000bb"]), no_layers.clone(), json!({}), json!({}))),
        "prop set id 00000000000000bb is not in the `propSets` table"
    );
    assert_eq!(
        read_error(document(
            json!(["00000000000000bb"]),
            no_layers.clone(),
            json!({ "00000000000000bb": { "size": "00000000000000aa" } }),
            json!({})
        )),
        "prop id 00000000000000aa is not in the `props` table"
    );
    let layers = json!([serde_json::to_value(layer(&[])).unwrap()]);
    let mut layers_with_id = layers.clone();
    layers_with_id[0]["omitted"] = json!("00000000000000cc");
    assert_eq!(
        read_error(document(json!([]), layers_with_id, json!({}), json!({}))),
        "name list id 00000000000000cc is not in the `nameLists` table"
    );
}

#[test]
fn a_prop_name_in_two_of_a_components_sets_fails_instead_of_one_replacing_the_other() {
    let size = prop("size", None);
    let mut doc = serde_json::to_value(output(vec![component("Button", &[size.clone()], &[])])).unwrap();
    let only_set = doc["components"]["Button"]["props"][0].clone();
    let other_set = json!("00000000000000dd");
    doc["propSets"][other_set.as_str().unwrap()] = set_json(&[&size]);
    doc["components"]["Button"]["props"] = json!([only_set, other_set]);

    assert_eq!(read_error(doc), "prop `size` is in two of the component's prop sets");
}

#[test]
fn two_different_props_with_one_id_fail_instead_of_sharing_an_entry() {
    let size = prop("size", None);
    let tone = prop("tone", None);
    let json = |p: &ParsedProp| serde_json::to_vec(p).unwrap();
    let mut table = PropTable { entries: BTreeMap::new() };

    let first: Result<(), serde_json::Error> = table.add(&size, PropId(7), json(&size));
    first.unwrap();
    let again: Result<(), serde_json::Error> = table.add(&size, PropId(7), json(&size));
    again.unwrap();
    assert_eq!(table.entries.len(), 1, "the same prop again shares the entry");

    let clash: Result<(), serde_json::Error> = table.add(&tone, PropId(7), json(&tone));
    let message = clash.unwrap_err().to_string();
    assert!(message.starts_with("prop 'tone' and a different prop have the same id 0000000000000007: "), "{message}");
    assert_eq!(table.entries.len(), 1);
}

#[test]
fn the_output_names_the_schema_version_it_is_written_in() {
    let json = serde_json::to_value(output(vec![])).unwrap();
    assert_eq!(json["schemaVersion"], json!(crate::types::SCHEMA_VERSION));
}

#[test]
fn a_version_this_build_does_not_read_fails_and_output_with_no_version_counts_as_the_first() {
    let mut json = serde_json::to_value(output(vec![component("Button", &[prop("size", None)], &[])])).unwrap();

    json["schemaVersion"] = json!(crate::types::SCHEMA_VERSION + 1);
    assert_eq!(
        read_error(json.clone()),
        format!(
            "output schema version {} is not the version {} this build reads",
            crate::types::SCHEMA_VERSION + 1,
            crate::types::SCHEMA_VERSION
        )
    );

    json.as_object_mut().unwrap().remove("schemaVersion");
    assert_eq!(
        read_error(json),
        format!("output schema version 1 is not the version {} this build reads", crate::types::SCHEMA_VERSION)
    );
}

fn diagnostic(message: &str, affects: &[&str]) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(DiagnosticSeverity::Warning, DiagnosticCode::OpaqueType, message);
    diagnostic.affects = affects.iter().map(|key| key.to_string()).collect();
    diagnostic
}

#[test]
fn the_components_a_diagnostic_affects_are_a_list_id_shared_by_diagnostics_naming_the_same_ones() {
    let mut out = output(vec![component("Button", &[], &[]), component("Chip", &[], &[])]);
    out.diagnostics =
        vec![diagnostic("a", &["Button", "Chip"]), diagnostic("b", &["Button", "Chip"]), diagnostic("c", &[])];

    let json = serde_json::to_value(&out).unwrap();

    let list = PropId::of_list(&["Button".to_string(), "Chip".to_string()]).to_string();
    let affects: Vec<Option<&str>> =
        json["diagnostics"].as_array().unwrap().iter().map(|d| d.get("affects").map(|a| a.as_str().unwrap())).collect();
    assert_eq!(affects, [Some(list.as_str()), Some(list.as_str()), None]);
    assert_eq!(json["nameLists"][&list], json!(["Button", "Chip"]));
    let read: ExtractionOutput = serde_json::from_value(json).unwrap();
    assert_eq!(read, out);
}

#[test]
fn reading_an_affects_id_missing_from_the_name_lists_fails_naming_it() {
    let mut out = output(vec![]);
    out.diagnostics = vec![diagnostic("a", &["Button"])];
    let mut json = serde_json::to_value(&out).unwrap();
    json["nameLists"] = json!({});

    assert_eq!(
        read_error(json),
        format!("name list id {} is not in the `nameLists` table", PropId::of_list(&["Button".to_string()]))
    );
}
