#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The size budget on a converted type. A style system in the project's own source is never cut by the field caps (what
//! the project declares is listed in full), so its style objects expanded without bound: Chakra's `SystemStyleObject`
//! took 25 MB of JSON per prop and 42 GB of memory for one extract. The project here is generated in the shape of one:
//! 130 conditions, 100 style properties that each take a value or an object of the conditions, and twenty components
//! that take a style object. A type of more than the budget is cut where it is converted; a type of exactly the budget
//! is not.

use std::fmt::Write as _;
use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::semantic::SemanticOutcome;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;
use serde_json::Value;

/// `MAX_SIZE` in `src/convert.rs`: 256 KiB over the 14 bytes of the smallest type.
const MAX_SIZE: usize = 256 * 1024 / 14;

/// Each field of the generated `Near` and `Over` is `string | number`: three types.
const FIELD_TYPES: usize = 3;

fn source() -> String {
    let mut source = String::from("interface Conditions {\n");
    for key in 0..130 {
        writeln!(source, "  _c{key:03}: string;").unwrap();
    }
    source.push_str(
        "}\n\ntype ConditionalValue<V> = V | { [K in keyof Conditions]?: V };\n\ninterface SystemProperties {\n",
    );
    for property in 0..100 {
        writeln!(source, "  p{property:03}?: ConditionalValue<string | number>;").unwrap();
    }
    source.push_str("}\n\n");
    for component in 0..20 {
        writeln!(
            source,
            "export function Styled{component:02}(props: {{ css?: SystemProperties; label?: string }}) {{\n  return null\n}}\n"
        )
        .unwrap();
    }
    // An object of one type and its fields' types: exactly the budget, and three types past it.
    let fields = (MAX_SIZE - 1) / FIELD_TYPES;
    for (name, count) in [("Near", fields), ("Over", fields + 1)] {
        writeln!(source, "interface {name} {{").unwrap();
        for field in 0..count {
            writeln!(source, "  f{field:05}: string | number;").unwrap();
        }
        source.push_str("}\n\n");
    }
    source.push_str("export function Budget(props: { near: Near; over: Over }) {\n  return null\n}\n");
    source
}

fn outcome() -> (tempfile::TempDir, SemanticOutcome) {
    let dir = tempfile::Builder::new().prefix("Size").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::write(root.join("Styled.tsx"), source()).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] }, "include": ["*.tsx"] }"#,
    )
    .unwrap();
    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let mut request = SemanticRequest::new(root, targets_of(&output));
    request.request_timeout = Duration::from_secs(120);
    let outcome = resolve(request);
    assert_eq!(outcome.fallbacks, []);
    (dir, outcome)
}

fn prop_type(outcome: &SemanticOutcome, component: &str, prop: &str) -> Value {
    let component = outcome
        .components
        .iter()
        .find(|candidate| candidate.key == component)
        .unwrap_or_else(|| panic!("{component} is in the answer"));
    let prop = component.props.iter().find(|candidate| candidate.name == prop).unwrap();
    serde_json::to_value(&prop.prop_type).unwrap()
}

/// How many types the JSON of a type holds: each is an object with a `kind`.
fn types_in(value: &Value) -> usize {
    match value {
        Value::Object(map) => usize::from(map.contains_key("kind")) + map.values().map(types_in).sum::<usize>(),
        Value::Array(items) => items.iter().map(types_in).sum(),
        _ => 0,
    }
}

fn truncated_as(value: &Value, text: &str) -> bool {
    match value {
        Value::Object(map) => {
            (map.get("kind") == Some(&Value::from("truncated")) && map.get("text") == Some(&Value::from(text)))
                || map.values().any(|inner| truncated_as(inner, text))
        }
        Value::Array(items) => items.iter().any(|inner| truncated_as(inner, text)),
        _ => false,
    }
}

/// A style object of the project's own declaring, past the budget, is cut by its name in every component that takes
/// it, and no prop of the answer holds more types than the budget. Without the cut, each `css` holds 39,401.
#[test]
fn a_project_style_object_past_the_size_budget_is_cut_by_name() {
    let (_dir, outcome) = outcome();
    let mut total = 0;
    for component in outcome.components.iter().filter(|component| component.key.starts_with("Styled")) {
        for prop in &component.props {
            let types = types_in(&serde_json::to_value(&prop.prop_type).unwrap());
            assert!(types <= MAX_SIZE, "{}.{} holds {types} types", component.key, prop.name);
            total += types;
        }
    }
    for component in 0..20 {
        let css = prop_type(&outcome, &format!("Styled{component:02}"), "css");
        assert!(truncated_as(&css, "SystemProperties"), "Styled{component:02}.css is cut: {css}");
    }
    // Twenty cut style objects and twenty strings: a handful of types each.
    assert!(total < 20 * 10, "the style props hold {total} types");
}

/// A type of exactly the budget is listed in full; three types more, and it is cut.
#[test]
fn a_type_of_the_size_budget_is_kept_and_one_past_it_is_cut() {
    let (_dir, outcome) = outcome();
    let near = prop_type(&outcome, "Budget", "near");
    assert!(!truncated_as(&near, "Near"), "near is kept");
    assert_eq!(types_in(&near), MAX_SIZE);
    let over = prop_type(&outcome, "Budget", "over");
    assert!(truncated_as(&over, "Over"), "over is cut: {over}");
}
