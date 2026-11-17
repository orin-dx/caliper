#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! The checker path over a fixture project of hand-written cases (`tests/fixtures/cases`): the 14 of the checker-first
//! deep dive in `Cases.tsx`, and one component per further mapping rule in `Rules.tsx`. The built-in checker's answer is
//! held to a JSON snapshot (`tests/snapshots/`, refreshed with `cargo insta review`), and to be the same when the checker
//! is asked last to first (type handles change with the order of queries; the answer may not). Merged over the
//! structural result, it puts a union's members back in the order they were written in.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::ExtractionOutput;
use caliper_docgen_core::types::semantic::{SemanticOutcome, merge_semantic};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;
use serde_json::{Value, json};

mod support;
use support::copy_fixture;

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// The cases in a scratch directory.
fn project() -> Project {
    let dir = tempfile::Builder::new().prefix("Cases").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cases"), &root);
    Project { _dir: dir, root }
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

/// How long a conversion may take before the test says it does not end (a type that contains itself must not hang it).
const END_OF_CONVERSION: Duration = Duration::from_secs(60);

/// The checker's answer, asked first to last, or last to first when `reverse`.
fn answer(project: &Project, output: &ExtractionOutput, reverse: bool) -> SemanticOutcome {
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(output));
    request.request_timeout = Duration::from_secs(60);
    request.reverse_queries = reverse;
    let (sender, receiver) = std::sync::mpsc::channel();
    // A conversion that does not end keeps its thread until the test process exits; the test fails meanwhile.
    std::thread::spawn(move || sender.send(resolve(request)).ok());
    receiver.recv_timeout(END_OF_CONVERSION).expect("the conversion ends")
}

/// An output key without the scratch directory: a name two components share is keyed `Name (<path>)`, and the
/// snapshot keeps only the path's file name.
fn portable(key: &str) -> String {
    match key.strip_suffix(')').and_then(|key| key.split_once(" (")) {
        Some((name, path)) => format!("{name} ({})", Utf8PathBuf::from(path).file_name().unwrap_or(path)),
        None => key.to_string(),
    }
}

/// The outcome as the JSON the snapshot holds: each component's props in the order the checker gave them.
fn rendered(outcome: &SemanticOutcome) -> Value {
    let components: serde_json::Map<String, Value> = outcome
        .components
        .iter()
        .map(|component| {
            let props: Vec<Value> = component
                .props
                .iter()
                .map(|prop| {
                    json!({
                        "name": prop.name,
                        "required": prop.required,
                        "type": serde_json::to_value(&prop.prop_type).unwrap(),
                    })
                })
                .collect();
            (portable(&component.key), Value::Array(props))
        })
        .collect();
    let fallbacks: serde_json::Map<String, Value> =
        outcome.fallbacks.iter().map(|fallback| (portable(&fallback.key), json!(fallback.reason))).collect();
    json!({ "components": components, "fallbacks": fallbacks })
}

fn prop_type(rendered: &Value, component: &str, prop: &str) -> Value {
    rendered["components"][component]
        .as_array()
        .and_then(|props| props.iter().find(|candidate| candidate["name"] == prop))
        .unwrap_or_else(|| panic!("{component}.{prop} is in the answer"))["type"]
        .clone()
}

#[test]
fn the_checker_gives_the_snapshot() {
    let project = project();
    let output = structural(&project);

    let outcome = answer(&project, &output, false);

    assert_eq!(outcome.diagnostics, []);
    insta::assert_json_snapshot!("cases", rendered(&outcome));
}

/// Type handles are the checker's and change with the order its types are first asked about; a union of objects is
/// sorted by them. The answer must not show it.
#[test]
fn asking_the_checker_last_to_first_gives_the_same_bytes() {
    let project = project();
    let output = structural(&project);

    let forward = answer(&project, &output, false);
    let reversed = answer(&project, &output, true);

    assert_eq!(forward.diagnostics, []);
    assert_eq!(reversed.diagnostics, []);
    assert_eq!(
        serde_json::to_string(&rendered(&forward)).unwrap(),
        serde_json::to_string(&rendered(&reversed)).unwrap()
    );
}

/// The checker sorts a union's members by kind and value; the merge puts them back in the order they were written
/// in, from the structural pass, which read them from the type node.
#[test]
fn the_merge_puts_a_unions_members_back_in_declaration_order() {
    let project = project();
    let mut output = structural(&project);
    let outcome = answer(&project, &output, false);
    let size = |outcome: &SemanticOutcome| {
        let component = outcome.components.iter().find(|component| component.key == "Plain").unwrap();
        let prop = component.props.iter().find(|prop| prop.name == "size").unwrap();
        serde_json::to_value(&prop.prop_type).unwrap()
    };
    let literals = |value: Value| -> Vec<String> {
        value["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| member["value"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(literals(size(&outcome)), ["lg", "md", "sm"], "the checker's order");

    merge_semantic(&mut output, outcome);

    let merged = serde_json::to_value(&output.components["Plain"].props["size"].prop_type).unwrap();
    assert_eq!(literals(merged), ["sm", "md", "lg"], "the order the author wrote");
}

#[test]
fn a_mapped_conditional_or_aliased_type_is_what_the_checker_resolved() {
    let project = project();
    let output = structural(&project);
    let rendered = rendered(&answer(&project, &output, false));

    // `Partial<Omit<Base, 'd'>> & { [K in 'x' | 'y']: number }`: members the checker computed.
    let names = |component: &str| -> Vec<String> {
        rendered["components"][component]
            .as_array()
            .unwrap()
            .iter()
            .map(|prop| prop["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("Mapper"), ["a", "b", "c", "x", "y", "extra"]);
    // A conditional type on the component's props type: both branches it could not pick are open.
    assert_eq!(prop_type(&rendered, "CondC", "k")["reason"]["type"], "conditionalType");
    // A lib utility is evaluated, not named.
    assert_eq!(prop_type(&rendered, "Aliases", "partial")["kind"], "object");
    assert_eq!(prop_type(&rendered, "Aliases", "record")["kind"], "object");
    // An interface or alias a dependency declares keeps its name; a literal union it declares is expanded.
    assert_eq!(prop_type(&rendered, "Aliases", "theme"), json!({ "kind": "named", "name": "DepTheme", "args": [] }));
    assert_eq!(prop_type(&rendered, "Aliases", "style"), json!({ "kind": "named", "name": "DepStyle", "args": [] }));
    assert_eq!(prop_type(&rendered, "Aliases", "size")["kind"], "union");
}

#[test]
fn the_error_type_is_an_unresolvable_import_and_a_written_any_is_any() {
    let project = project();
    let output = structural(&project);
    let rendered = rendered(&answer(&project, &output, false));

    assert_eq!(prop_type(&rendered, "Lost", "gone")["reason"]["type"], "unresolvableImport");
    assert_eq!(prop_type(&rendered, "Scalars", "any"), json!({ "kind": "any" }));
}

#[test]
fn nothing_the_checker_hands_out_by_number_is_in_the_answer() {
    let project = project();
    let output = structural(&project);
    let text = serde_json::to_string(&rendered(&answer(&project, &output, false))).unwrap();

    // A symbol-keyed property is `[Symbol.iterator]`, not the checker's `__@iterator@<symbol id>`.
    assert!(!text.contains("__@"), "{text}");
    assert!(text.contains("[Symbol.iterator]"));
}

fn field(name: &str, prop_type: Value) -> Value {
    json!({ "name": name, "propType": prop_type, "required": true, "description": "" })
}

fn recursive(name: &str, args: Vec<Value>) -> Value {
    json!({ "kind": "recursive", "name": name, "args": args })
}

/// The type at a JSON pointer inside a prop's type.
fn within(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).unwrap_or_else(|| panic!("{pointer} is in {value}")).clone()
}

/// The answer asked first to last and last to first.
fn answers(check: impl Fn(&Value)) {
    let project = project();
    let output = structural(&project);
    for reverse in [false, true] {
        check(&rendered(&answer(&project, &output, reverse)));
    }
}

/// An alias of an array or a tuple of itself is a cycle of types with no object in between: the conversion has to end
/// at the first lap, with the reference back to the alias, whichever way round the checker is asked.
#[test]
fn a_type_that_contains_itself_through_an_array_or_a_tuple_ends_in_a_back_reference() {
    answers(|rendered| {
        let looped = recursive("LoopArray", vec![]);
        assert_eq!(prop_type(rendered, "Instances", "loopArray"), json!({ "kind": "array", "element": looped }));
        let tuple = prop_type(rendered, "Instances", "loopTuple");
        assert_eq!(within(&tuple, "/elements/0/type"), recursive("LoopTuple", vec![]));
        let readonly = prop_type(rendered, "Instances", "readonlyLoop");
        assert_eq!(within(&readonly, "/element/element"), recursive("ReadonlyLoop", vec![]));
    });
}

/// A back-reference is to a type that encloses this one: `Box<string>` inside `Box<Box<string>>` is another
/// instantiation of the declaration, finite, and stays an object. The reference keeps the declaration's name.
#[test]
fn an_instantiation_is_not_its_declaration() {
    answers(|rendered| {
        let string = json!({ "kind": "string" });
        let point = json!({ "kind": "object", "fields": [field("x", json!({ "kind": "number" }))] });
        let pair = |first: Value, second: Value| json!({ "kind": "object", "fields": [field("first", first), field("second", second)] });
        let boxed = |inner: Value| json!({ "kind": "object", "fields": [field("value", inner)] });

        assert_eq!(prop_type(rendered, "Instances", "boxed"), boxed(boxed(string.clone())));
        assert_eq!(prop_type(rendered, "Instances", "twice"), pair(point.clone(), point.clone()));
        assert_eq!(prop_type(rendered, "Instances", "nestedPair"), pair(pair(point.clone(), point.clone()), point));

        // The same instantiation inside itself is a back-reference, with the arguments of the instantiation.
        let tree = prop_type(rendered, "Instances", "tree");
        assert_eq!(within(&tree, "/fields/1/propType/element"), recursive("Tree", vec![string]));
        let mutual = prop_type(rendered, "Instances", "mutual");
        assert_eq!(
            within(&mutual, "/fields/0/propType/fields/0/propType"),
            recursive("Ping", vec![json!({ "kind": "number" })])
        );

        // A generic that names a new instantiation each time is never the same type again: the depth cap ends it, and
        // nothing is a back-reference.
        let grow = serde_json::to_string(&prop_type(rendered, "Instances", "grow")).unwrap();
        assert!(grow.contains(r#""limit":{"type":"depth"}"#) && !grow.contains("recursive"), "{grow}");
    });
}

/// Every cap fires where it says it does, with its limit and, for a width, how many there were.
#[test]
fn every_cap_cuts_where_it_is_set() {
    let project = project();
    let output = structural(&project);
    let rendered = rendered(&answer(&project, &output, false));
    let cut = |component: &str, prop: &str, pointer: &str| {
        let ty = within(&prop_type(&rendered, component, prop), pointer);
        assert_eq!(ty["kind"], "truncated", "{component}.{prop}{pointer}: {ty}");
        ty["limit"].clone()
    };
    let members = |total: u32| json!({ "type": "members", "total": total });
    let depth = json!({ "type": "depth" });

    // Fields are cut only past the budget of members that are not the project's own (see `families.rs`): the project's
    // interfaces of 101 and 42 fields are listed whole, at the top and below it.
    assert_eq!(prop_type(&rendered, "Caps", "wideTop")["fields"].as_array().unwrap().len(), 101);
    assert_eq!(
        within(&prop_type(&rendered, "Caps", "wide"), "/fields/0/propType")["fields"].as_array().unwrap().len(),
        42
    );
    // Members, at the top (500) and below (120), of a union, an intersection and a tuple.
    assert_eq!(cut("Caps", "many", ""), members(600));
    assert_eq!(cut("Caps", "nestedMany", "/fields/0/propType"), members(200));
    assert_eq!(cut("Caps", "allOf", ""), members(501));
    assert_eq!(cut("Caps", "nestedAllOf", "/fields/0/propType"), members(121));
    assert_eq!(cut("Caps", "longTuple", ""), members(501));
    assert_eq!(cut("Caps", "nestedTuple", "/fields/0/propType"), members(121));
    // Below its limit a union is listed whole.
    assert_eq!(within(&prop_type(&rendered, "Caps", "withinLimits"), "/fields/0/propType/kind"), "union");

    // Depth 5: an object, and each other kind of type that has parts, but not a type without any.
    let below_four_objects = "/fields/0/propType".repeat(5);
    assert_eq!(cut("Caps", "deep", &below_four_objects), depth);
    for prop in ["union", "intersection", "array", "tuple", "template"] {
        assert_eq!(cut("DeepKinds", prop, &below_four_objects), depth, "{prop}");
    }
    assert_eq!(within(&prop_type(&rendered, "DeepKinds", "scalar"), &format!("{below_four_objects}/kind")), "string");
}
