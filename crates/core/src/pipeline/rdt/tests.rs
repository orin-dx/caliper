use std::collections::BTreeMap;
use std::sync::Arc;

use super::{docgen_info, omits, serialize, type_json, write};
use crate::types::{ComponentEntry, DefaultValue, DetectionRule, ExtractionOutput, ParsedProp, PropParent, PropType};

fn prop(name: &str, prop_type: PropType, description: &str) -> Arc<ParsedProp> {
    Arc::new(ParsedProp::new(name.into(), prop_type, true, None, description.into(), BTreeMap::new(), None, vec![]))
}

fn entry(name: &str, props: &[(&str, &Arc<ParsedProp>)], notable: &[(&str, &Arc<ParsedProp>)]) -> ComponentEntry {
    let pairs = |list: &[(&str, &Arc<ParsedProp>)]| -> BTreeMap<String, Arc<ParsedProp>> {
        list.iter().map(|(key, prop)| ((*key).to_owned(), Arc::clone(prop))).collect()
    };
    ComponentEntry {
        display_name: name.into(),
        file_path: format!("src/{name}.tsx").into(),
        description: String::new(),
        props: pairs(props),
        inheritance: vec![],
        notable_inherited: pairs(notable),
        discriminant_prop: None,
        composes: vec![],
        tags: BTreeMap::new(),
        methods: vec![],
        detected_by: DetectionRule::FunctionDeclaration,
        exported: true,
        exports: vec![],
    }
}

fn output(components: Vec<ComponentEntry>) -> ExtractionOutput {
    ExtractionOutput {
        components: components.into_iter().map(|entry| (entry.display_name.clone(), entry)).collect(),
        enums: BTreeMap::new(),
        diagnostics: vec![],
        stats: Default::default(),
    }
}

/// The document as it was built before it was streamed: one `serde_json::Value` for the whole of it.
fn reference(output: &ExtractionOutput, include_unexported: bool) -> String {
    let map: serde_json::Map<String, serde_json::Value> = output
        .components
        .iter()
        .filter(|(_, entry)| include_unexported || entry.exported)
        .map(|(name, entry)| {
            let props: serde_json::Map<String, serde_json::Value> = entry
                .props
                .iter()
                .chain(entry.notable_inherited.iter().filter(|(name, _)| !entry.props.contains_key(*name)))
                .filter(|(_, prop)| !omits(prop) && !prop.tags.contains_key("ignore"))
                .map(|(key, prop)| {
                    let obj = serde_json::json!({
                        "name": prop.name,
                        "type": type_json(&prop.prop_type),
                        "required": prop.required,
                        "defaultValue": prop.default_value.as_ref().map(|d| serde_json::json!({"value": d.value, "computed": d.computed})),
                        "description": prop.description,
                        "parent": prop.parent.as_ref().map(|p| serde_json::json!({"name": p.name, "fileName": p.file_name})),
                    });
                    (key.clone(), obj)
                })
                .collect();
            let mut doc = serde_json::json!({
                "displayName": entry.display_name,
                "props": serde_json::Value::Object(props),
                "description": entry.description,
                "methods": [],
                "tags": entry.tags,
                "composes": entry.composes,
            });
            doc["filePath"] = serde_json::json!(entry.file_path);
            (name.clone(), doc)
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Object(map)).unwrap()
}

#[test]
fn a_prop_is_written_at_its_place_in_the_document() {
    let size = prop(
        "size",
        PropType::LiteralUnion { members: vec!["sm".into(), "lg".into()], has_default: false },
        "How big.",
    );
    let out = output(vec![entry("Button", &[("size", &size)], &[])]);

    // Lines of the document, not of a prop rendered alone: the nesting is the document's.
    let expected = r#"{
  "Button": {
    "displayName": "Button",
    "props": {
      "size": {
        "name": "size",
        "type": {
          "name": "enum",
          "raw": "\"sm\" | \"lg\"",
          "value": [
            {
              "value": "\"sm\""
            },
            {
              "value": "\"lg\""
            }
          ]
        },
        "required": true,
        "defaultValue": null,
        "description": "How big.",
        "parent": null
      }
    },
    "description": "",
    "methods": [],
    "tags": {},
    "composes": [],
    "filePath": "src/Button.tsx"
  }
}"#;
    assert_eq!(serialize(&out, false).unwrap(), expected);
}

#[test]
fn no_components_and_no_props_are_empty_objects() {
    assert_eq!(serialize(&output(vec![]), false).unwrap(), "{}");
    let out = output(vec![entry("Bare", &[], &[])]);
    let text = serialize(&out, false).unwrap();
    assert!(text.contains("\"props\": {},"), "{text}");
    assert!(text.contains("\"methods\": [],"), "{text}");
    assert_eq!(text, reference(&out, false));
}

#[test]
fn the_document_has_no_trailing_newline() {
    let text = serialize(&output(vec![entry("Bare", &[], &[])]), false).unwrap();
    assert!(text.ends_with('}'));
    let mut bytes = Vec::new();
    write(&output(vec![]), false, &mut bytes).unwrap();
    assert_eq!(bytes, b"{}");
}

#[test]
fn it_matches_the_whole_document_built_as_a_value() {
    let tricky = "quote \" backslash \\ tab \t newline \n return \r nul \u{0} bell \u{7} del \u{7f} sep \u{2028}\u{2029} é 日本 😀 </script>";
    let documented = Arc::new(ParsedProp::new(
        "weird \"name\"\n".into(),
        PropType::LiteralUnion { members: vec!["a\"b".into(), "c\\d".into(), "e\nf".into()], has_default: true },
        false,
        Some(DefaultValue { value: "\"it's\"\n".into(), computed: true }),
        tricky.into(),
        BTreeMap::from([("see".to_owned(), tricky.to_owned())]),
        Some(PropParent { name: "Props".into(), file_name: "/a b/\u{2028}.ts".into() }),
        vec![],
    ));
    let nested = prop(
        "nested",
        PropType::Union(vec![PropType::String, PropType::Array(Box::new(PropType::Number)), PropType::Null]),
        "",
    );
    let children_bare = prop("children", PropType::ReactNode, "");
    let children_documented = prop("children", PropType::ReactNode, "Slot.");
    let ignored = Arc::new(ParsedProp::new(
        "hidden".into(),
        PropType::String,
        false,
        None,
        "x".into(),
        BTreeMap::from([("ignore".to_owned(), String::new())]),
        None,
        vec![],
    ));

    // One allocation shared by two components, once among a component's own props and once among another's inherited.
    let mut first = entry(
        "A",
        &[("id", &documented), ("nested", &nested), ("children", &children_bare), ("hidden", &ignored)],
        &[("id", &nested), ("title", &documented)],
    );
    first.description = tricky.into();
    first.tags = BTreeMap::from([("since".to_owned(), "1\u{2028}".to_owned()), ("a".to_owned(), String::new())]);
    first.composes = vec!["B".into(), "C \"d\"".into()];
    let second = entry("B", &[("children", &children_documented)], &[("title", &documented), ("nested", &nested)]);
    let mut unexported = entry("Z", &[("id", &documented)], &[]);
    unexported.exported = false;
    let out = output(vec![first, second, unexported]);

    for include_unexported in [false, true] {
        assert_eq!(serialize(&out, include_unexported).unwrap(), reference(&out, include_unexported));
    }
}

#[test]
fn a_shared_allocation_reads_the_same_under_every_key_and_list() {
    // A prop always sits three objects deep, so one rendering serves every place it occurs.
    let shared = prop("shared", PropType::String, "d");
    let out = output(vec![
        entry("Own", &[("shared", &shared), ("alias", &shared)], &[]),
        entry("Inherited", &[], &[("shared", &shared)]),
    ]);
    assert_eq!(serialize(&out, false).unwrap(), reference(&out, false));
}

#[test]
fn a_prop_named_in_both_lists_is_listed_once_as_the_component_declares_it() {
    let own = prop("x", PropType::String, "own");
    let inherited = prop("x", PropType::Number, "inherited");
    let out = output(vec![entry("A", &[("x", &own)], &[("x", &inherited)])]);
    let text = serialize(&out, false).unwrap();
    assert_eq!(text.matches("\"x\": {").count(), 1);
    assert!(text.contains("\"description\": \"own\""));
    assert_eq!(text, reference(&out, false));
}

#[test]
fn docgen_info_is_one_component_with_children_kept() {
    let children = prop("children", PropType::ReactNode, "");
    let info = docgen_info(&entry("A", &[("children", &children)], &[]));
    assert!(info["props"]["children"].is_object());
    assert!(info.get("filePath").is_none());
}
