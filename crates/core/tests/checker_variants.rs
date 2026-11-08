#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! What the structural path reads from the source in the shapes the checker states too (ADR 0026, slice S1): tuple
//! elements with their markers and labels, bigint and negative literals, `unique symbol`, enum members, template
//! literal patterns, objects with call or construct signatures, recursive references, and getters as properties. Each
//! is checked in the canonical JSON, the RDT projection and TOON, which must agree.
//!
//! The test reads only the JSON and the projections, never the Rust shapes, so it states the behaviour a consumer
//! sees.

use caliper_docgen_core::pipeline::{PipelineOptions, extract, rdt};
use caliper_docgen_core::toon::render_component_toon;
use camino::Utf8PathBuf;
use serde_json::{Value, json};

const SOURCE: &str = r#"
type Tail = [number, boolean];
type Prepend<T, U extends unknown[]> = [T, ...U];
enum Size { Small = "small", Medium = "medium", Large = "large" }
enum Level { Low = 1, High = 2 }
declare const brand: unique symbol;
type Ev = 'click' | 'focus';
type Json = string | number | boolean | null | Json[] | { [key: string]: Json };
type Loop = Loop;
export interface WidgetProps {
  rest?: [string, ...number[]];
  leadingRest?: [...string[], number];
  labeledRest?: [head: string, ...tail: number[]];
  optional?: [string, number?];
  labeledOptional?: [x: number, y?: number];
  spread?: [string, ...Tail];
  genericSpread?: Prepend<string, [number, boolean]>;
  bigints?: 10n | -20n;
  negative?: -1 | 0 | 1;
  symbol?: typeof brand;
  members?: Size.Small | Size.Large;
  member?: Level.High;
  pixels?: `${number}px`;
  events?: `on${Capitalize<Ev>}`;
  overloads?: { (value: string): string; (value: number): number };
  construct?: { new (value: string): Date };
  callable?: { (): void; displayName?: string };
  single?: { (value: string): number };
  json?: Json;
  loop?: Loop;
  getter?: { get size(): number };
  inlineAccessor?: { get value(): string; set value(next: string) };
  spreadOptional?: [...[string?], number];
  optionalUnion?: [(string | number)?];
  get label(): string;
  set label(next: string | number);
}
export function Widget(props: WidgetProps) { return <div />; }
"#;

struct Extracted {
    canonical: Value,
    rdt: Value,
    toon: String,
}

fn extracted() -> Extracted {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("widget.tsx"), SOURCE).unwrap();
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = &output.components["Widget"];
    let canonical = Value::Object(
        entry.props.iter().map(|(name, prop)| (name.clone(), serde_json::to_value(prop).unwrap())).collect(),
    );
    let rdt =
        Value::Object(entry.props.iter().map(|(name, prop)| (name.clone(), rdt::type_json(&prop.prop_type))).collect());
    Extracted { canonical, rdt, toon: render_component_toon(entry) }
}

/// The TOON row of prop `name`.
fn toon_row<'a>(toon: &'a str, name: &str) -> &'a str {
    toon.lines().find(|line| line.trim_start().starts_with(&format!("{name},"))).unwrap_or_else(|| panic!("{toon}"))
}

/// The prop's type as RDT names it, its canonical JSON, and its `degradedBy`.
fn prop<'a>(out: &'a Extracted, name: &str) -> (&'a Value, &'a Value, Option<&'a Value>) {
    (&out.rdt[name]["name"], &out.canonical[name]["type"], out.canonical[name].get("degradedBy"))
}

#[test]
fn tuple_elements_keep_their_rest_optional_and_label_markers() {
    let out = extracted();
    let cases = [
        ("rest", "[string, ...number[]]"),
        ("leadingRest", "[...string[], number]"),
        ("labeledRest", "[head: string, ...tail: number[]]"),
        ("optional", "[string, number?]"),
        ("labeledOptional", "[x: number, y?: number]"),
        ("optionalUnion", "[(string | number)?]"),
    ];
    for (name, printed) in cases {
        let (rdt, _, degraded) = prop(&out, name);
        assert_eq!(rdt, &json!(printed), "{name}");
        assert_eq!(degraded, None, "{name}");
        assert!(toon_row(&out.toon, name).contains(printed), "{name}: {}", toon_row(&out.toon, name));
    }
    // A rest element holds the element type, as the checker's tuple type arguments do: `number`, not `number[]`.
    assert_eq!(
        prop(&out, "labeledRest").1["elements"],
        json!([
            { "type": { "kind": "string" }, "label": "head" },
            { "type": { "kind": "number" }, "label": "tail", "modifier": "rest" },
        ])
    );
    assert_eq!(
        prop(&out, "optional").1["elements"][1],
        json!({ "type": { "kind": "number" }, "modifier": "optional" })
    );
}

#[test]
fn a_spread_of_a_known_tuple_is_flattened() {
    let out = extracted();
    for name in ["spread", "genericSpread"] {
        assert_eq!(prop(&out, name).0, &json!("[string, number, boolean]"), "{name}");
    }
    // An optional element spread in before a required one is required and admits `undefined`, as TypeScript has it.
    assert_eq!(prop(&out, "spreadOptional").0, &json!("[string | undefined, number]"));
}

#[test]
fn bigint_and_negative_literals_are_literals() {
    let out = extracted();
    let (rdt, wire, degraded) = prop(&out, "bigints");
    assert_eq!(rdt, &json!("10n | -20n"));
    assert_eq!(wire["members"][1], json!({ "kind": "bigintLiteral", "value": "-20" }));
    assert_eq!(degraded, None);
    let (_, wire, degraded) = prop(&out, "negative");
    assert_eq!(wire["members"][0], json!({ "kind": "numberLiteral", "value": -1.0 }));
    assert_eq!(degraded, None);
    // A union of number literals is a Storybook select, now with its negative option.
    assert_eq!(out.rdt["negative"]["name"], json!("enum"));
    assert_eq!(out.rdt["negative"]["value"][0], json!({ "value": "-1" }));
}

#[test]
fn a_unique_symbol_is_its_own_kind() {
    let out = extracted();
    let (rdt, wire, _) = prop(&out, "symbol");
    assert_eq!(rdt, &json!("typeof brand"));
    assert_eq!(wire, &json!({ "kind": "uniqueSymbol", "name": "brand" }));
}

#[test]
fn an_enum_member_keeps_the_enum_and_offers_its_value() {
    let out = extracted();
    let (_, wire, _) = prop(&out, "members");
    assert_eq!(
        wire["members"][0],
        json!({
            "kind": "enumMember", "enum": "Size", "member": "Small",
            "value": { "kind": "stringLiteral", "value": "small" },
        })
    );
    assert_eq!(
        out.rdt["members"],
        json!({
            "name": "enum",
            "raw": "Size.Small | Size.Large",
            "value": [{ "value": "\"small\"" }, { "value": "\"large\"" }],
        })
    );
    assert!(toon_row(&out.toon, "members").contains("Size.Small|Size.Large"), "{}", out.toon);
    assert_eq!(out.rdt["member"], json!({ "name": "enum", "raw": "Level.High", "value": [{ "value": "2" }] }));
}

#[test]
fn a_template_literal_is_a_pattern_or_its_strings() {
    let out = extracted();
    let (rdt, wire, degraded) = prop(&out, "pixels");
    assert_eq!(rdt, &json!("`${number}px`"));
    assert_eq!(
        wire,
        &json!({ "kind": "templateLiteral", "parts": [{ "type": { "kind": "number" } }, { "text": "px" }] })
    );
    assert_eq!(degraded, None, "the pattern is the exact type");
    assert!(toon_row(&out.toon, "pixels").contains("`${number}px`"));
    // `Capitalize` over a literal union is evaluated, as TypeScript does.
    assert_eq!(out.rdt["events"]["value"], json!([{ "value": "\"onClick\"" }, { "value": "\"onFocus\"" }]));
}

#[test]
fn an_object_with_signatures_keeps_them_all() {
    let out = extracted();
    let cases = [
        ("overloads", "{ (value: string): string; (value: number): number }", 2),
        ("construct", "{ new (value: string): Date }", 1),
        ("callable", "{ (): void; displayName?: string }", 1),
    ];
    for (name, printed, signatures) in cases {
        let (rdt, wire, degraded) = prop(&out, name);
        assert_eq!(rdt, &json!(printed), "{name}");
        assert_eq!(wire["kind"], json!("callable"), "{name}");
        assert_eq!(wire["signatures"].as_array().map(Vec::len), Some(signatures), "{name}");
        assert_eq!(degraded, None, "{name}");
        assert!(toon_row(&out.toon, name).contains(printed), "{name}");
    }
    assert_eq!(prop(&out, "construct").1["signatures"][0]["construct"], json!(true));
    // One call signature alone stays a function type.
    assert_eq!(prop(&out, "single").1["kind"], json!("function"));
}

#[test]
fn a_recursive_alias_refers_back_to_itself_by_name() {
    let out = extracted();
    let (rdt, wire, degraded) = prop(&out, "json");
    assert_eq!(rdt, &json!("string | number | boolean | null | Json[] | { [key: string]: Json }"));
    assert_eq!(
        wire["members"][4],
        json!({ "kind": "array", "element": { "kind": "recursive", "name": "Json", "args": [] } })
    );
    assert_eq!(degraded, None, "the enclosing expansion is the type");
    // `type Loop = Loop` is an error, not a recursive type: it stays degraded.
    assert_eq!(prop(&out, "loop").2, Some(&json!(["depthExceeded"])));
}

#[test]
fn a_getter_is_a_property_not_a_method() {
    let out = extracted();
    assert_eq!(prop(&out, "getter").0, &json!("{ readonly size: number }"));
    assert_eq!(prop(&out, "inlineAccessor").0, &json!("{ value: string }"));
    // A props interface's getter and setter are one prop, typed by the getter, not by the setter's wider parameter.
    assert_eq!(prop(&out, "label").0, &json!("string"));
}
