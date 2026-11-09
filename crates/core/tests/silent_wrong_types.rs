#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A type is never silently wrong. A utility type behind an alias keeps the wrapper it was written with, a type
//! literal that declares a call signature never becomes an empty object, and a template literal index key is kept.

use caliper_docgen_core::pipeline::{PipelineOptions, extract, rdt};
use camino::Utf8PathBuf;
use serde_json::{Value, json};

const SOURCE: &str = r#"
import * as React from 'react';
type Styles<N extends string, P> = Partial<Record<N, P>>;
type Base = { x: string; y: number; z?: boolean };
type Without = Omit<Base, 'x'>;
type Only = Pick<Base, 'x'>;
type All = Required<Base>;
type Overloads = { (a: string): number; (b: number): string };
type Single = { <T>(a: T, b?: number): T[] };
type Mixed = { (a: string): number; label: string };
type Constructor = { new (a: string): Base };
type DataKeys = { [k: `data-${string}`]: string; id: string };
export interface WidgetProps {
  aliased?: Styles<'root' | 'item', string>;
  direct?: Partial<Record<'root' | 'item', string>>;
  without?: Without;
  withoutDirect?: Omit<Base, 'x'>;
  only?: Only;
  onlyDirect?: Pick<Base, 'x'>;
  all?: All;
  allDirect?: Required<Base>;
  overloads?: Overloads;
  single?: Single;
  mixed?: Mixed;
  constructor?: Constructor;
  dataKeys?: DataKeys;
}
export function Widget(props: WidgetProps) { return <div />; }
"#;

struct Extracted {
    canonical: Value,
    rdt: Value,
}

fn extracted() -> Extracted {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("case.tsx"), SOURCE).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.into_values().next().unwrap();
    let canonical = Value::Object(
        entry.props.iter().map(|(name, prop)| (name.clone(), serde_json::to_value(prop).unwrap())).collect(),
    );
    let rdt =
        Value::Object(entry.props.iter().map(|(name, prop)| (name.clone(), rdt::type_json(&prop.prop_type))).collect());
    Extracted { canonical, rdt }
}

#[test]
fn an_alias_of_a_utility_type_reaches_the_output_as_the_utility_does_written_in_the_prop() {
    let out = extracted();
    for (aliased, direct) in
        [("aliased", "direct"), ("without", "withoutDirect"), ("only", "onlyDirect"), ("all", "allDirect")]
    {
        assert_eq!(out.canonical[aliased]["type"], out.canonical[direct]["type"], "{aliased} against {direct}");
        assert_eq!(out.rdt[aliased], out.rdt[direct], "RDT projection of {aliased} against {direct}");
    }
    // The wrapper is what carries the optional keys: it was `Record<…>`, every key required.
    assert_eq!(out.canonical["aliased"]["type"]["name"], json!("Partial"));
    assert_eq!(
        out.rdt["aliased"]["name"],
        json!("Partial<Record<\"root\" | \"item\", string>>"),
        "projects as Partial, not Record"
    );
    assert_eq!(out.canonical["without"]["type"]["name"], json!("Omit"));
    assert_eq!(out.canonical["only"]["type"]["name"], json!("Pick"));
    assert_eq!(out.canonical["all"]["type"]["name"], json!("Required"));
}

#[test]
fn a_type_literal_with_one_call_signature_is_that_function() {
    let out = extracted();
    let single = &out.canonical["single"]["type"];
    assert_eq!(single["kind"], json!("function"), "{single}");
    assert_eq!(single["typeParams"], json!(["T"]));
    assert_eq!(single["params"].as_array().unwrap().len(), 2);
    assert!(out.canonical["single"].get("degradedBy").is_none());
}

#[test]
fn a_type_literal_with_several_signatures_or_a_signature_and_members_keeps_every_signature() {
    let out = extracted();
    for (prop, signatures, fields) in [("overloads", 2, 0), ("mixed", 1, 1), ("constructor", 1, 0)] {
        let ty = &out.canonical[prop]["type"];
        assert_eq!(ty["kind"], json!("callable"), "{prop}: {ty}");
        assert_eq!(ty["signatures"].as_array().unwrap().len(), signatures, "{prop}");
        assert_eq!(ty["fields"].as_array().unwrap().len(), fields, "{prop}");
        assert!(out.canonical[prop].get("degradedBy").is_none(), "{prop}");
    }
    assert_eq!(out.canonical["constructor"]["type"]["signatures"][0]["construct"], json!(true));
    assert_eq!(out.rdt["overloads"]["name"], json!("{ (a: string): number; (b: number): string }"));
    assert_eq!(out.rdt["mixed"]["name"], json!("{ (a: string): number; label: string }"));
    assert_eq!(out.rdt["constructor"]["name"], json!("{ new (a: string): Base }"));
}

#[test]
fn a_template_literal_index_key_is_kept_as_its_pattern() {
    let out = extracted();
    let key = &out.canonical["dataKeys"]["type"]["fields"][0]["indexKey"];
    assert_eq!(key["kind"], json!("templateLiteral"), "{key}");
    assert_eq!(key["parts"], json!([{ "text": "data-" }, { "type": { "kind": "string" } }]));
    assert!(out.canonical["dataKeys"].get("degradedBy").is_none(), "{}", out.canonical["dataKeys"]);
}
