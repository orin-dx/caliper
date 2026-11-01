#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `readonly`, index signatures and the name of a ref type reach the output (schema version 4), and an index
//! signature is never mistaken for a prop.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::{ExtractionOutput, ObjectField, PropType, SCHEMA_VERSION};
use camino::Utf8PathBuf;
use serde_json::json;
use tempfile::TempDir;

const SOURCE: &str = r#"
import * as React from 'react';

interface WidgetProps {
    tags?: readonly string[];
    pair: readonly [number, number];
    mixed: string[] | readonly string[];
    box: { readonly id: string; label?: string; [key: string]: string | undefined };
    byObject: React.RefObject<HTMLDivElement>;
    byRef: React.Ref<HTMLDivElement>;
    forwarded: React.ForwardedRef<HTMLDivElement>;
    byCallback: React.RefCallback<HTMLDivElement>;
}
export function Widget(props: WidgetProps) { return <div />; }

export function Bag(props: { label: string } & { [key: string]: unknown }) { return <div />; }

interface Loose { [key: string]: unknown; label: string }
export function Open(props: Loose) { return <div />; }

export function Words(props: { words: { [word: string]: number } }) { return <div />; }
"#;

fn run() -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join("Components.tsx"), SOURCE).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };
    extract(&options)
}

fn names(output: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    let entry = &output.components[component];
    entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn wire(output: &ExtractionOutput, component: &str, prop: &str) -> serde_json::Value {
    serde_json::to_value(&output.components[component].props[prop].prop_type).unwrap()
}

#[test]
fn the_output_is_schema_version_4() {
    assert_eq!(SCHEMA_VERSION, 4);
}

#[test]
fn a_component_lists_exactly_its_declared_props() {
    let output = run();
    assert_eq!(
        names(&output, "Widget"),
        set(&["tags", "pair", "mixed", "box", "byObject", "byRef", "forwarded", "byCallback"])
    );
}

#[test]
fn readonly_arrays_and_tuples_keep_the_operator() {
    let output = run();
    let strings = PropType::Array(Box::new(PropType::String));
    let props = &output.components["Widget"].props;
    assert_eq!(props["tags"].prop_type, PropType::Readonly(Box::new(strings.clone())));
    assert_eq!(
        props["pair"].prop_type,
        PropType::Readonly(Box::new(PropType::Tuple(vec![PropType::Number, PropType::Number])))
    );
    assert_eq!(props["mixed"].prop_type, PropType::Union(vec![strings.clone(), PropType::Readonly(Box::new(strings))]));
    assert_eq!(props["tags"].prop_type.raw_string(), "readonly string[]");
    assert_eq!(props["pair"].prop_type.raw_string(), "readonly [number, number]");
    assert_eq!(props["mixed"].prop_type.raw_string(), "string[] | readonly string[]");
    assert_eq!(
        wire(&output, "Widget", "tags"),
        json!({"kind": "readonly", "element": {"kind": "array", "element": {"kind": "string"}}})
    );
}

#[test]
fn an_inline_object_keeps_its_index_signature_and_readonly_field() {
    let output = run();
    let PropType::Object(fields) = &output.components["Widget"].props["box"].prop_type else {
        panic!("expected an object type");
    };
    assert_eq!(
        *fields,
        vec![
            ObjectField {
                name: "id".into(),
                prop_type: PropType::String,
                required: true,
                description: String::new(),
                readonly: true,
                index_key: None,
            },
            ObjectField::new("label", PropType::String, false, ""),
            ObjectField {
                name: "key".into(),
                prop_type: PropType::String,
                required: true,
                description: String::new(),
                readonly: false,
                index_key: Some(Box::new(PropType::String)),
            },
        ]
    );
    assert_eq!(
        output.components["Widget"].props["box"].prop_type.raw_string(),
        "{ readonly id: string; label?: string; [key: string]: string }"
    );
    assert_eq!(
        wire(&output, "Widget", "box")["fields"][2],
        json!({
            "name": "key",
            "propType": {"kind": "string"},
            "required": true,
            "description": "",
            "indexKey": {"kind": "string"},
        })
    );
}

#[test]
fn a_ref_type_keeps_the_name_it_was_written_with() {
    let output = run();
    let element = Some("HTMLDivElement".to_owned());
    let props = &output.components["Widget"].props;
    for (prop, name) in
        [("byObject", "RefObject"), ("byRef", "Ref"), ("forwarded", "ForwardedRef"), ("byCallback", "RefCallback")]
    {
        assert_eq!(props[prop].prop_type, PropType::Ref { name: name.into(), element: element.clone() }, "{prop}");
        assert_eq!(props[prop].prop_type.raw_string(), format!("{name}<HTMLDivElement>"), "{prop}");
    }
    assert_eq!(
        wire(&output, "Widget", "byObject"),
        json!({"kind": "ref", "name": "RefObject", "element": "HTMLDivElement"})
    );
}

#[test]
fn an_index_signature_is_not_a_prop() {
    let output = run();
    assert_eq!(names(&output, "Bag"), set(&["label"]));
    assert_eq!(names(&output, "Open"), set(&["label"]));
    assert_eq!(names(&output, "Words"), set(&["words"]));
    assert_eq!(output.components["Words"].props["words"].prop_type.raw_string(), "{ [word: string]: number }");
}
