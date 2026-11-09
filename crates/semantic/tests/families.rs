#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! Wide types on the checker path (`tests/fixtures/families`): what the project declares is listed whole however wide,
//! and what a dependency declares is named. The fixture has an interface of 150 own fields, a package interface of 150
//! fields, and a `@types/react` stand-in whose DOM attribute interface has over a hundred properties. The answer is
//! replayed from its recording (`families.recording.json`, made by the `record_fixture` example), always, and when
//! `TSRS_BIN` names a real `tsrs` it is asked live, first to last and last to first, and must give the same bytes.

use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::ExtractionOutput;
use caliper_docgen_core::types::semantic::SemanticOutcome;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;
use serde_json::{Value, json};

mod support;
use support::copy_fixture;

const FAKE_VERSION: &str = "7.1.0-test";

struct Project {
    _dir: tempfile::TempDir,
    root: Utf8PathBuf,
}

/// The fixture in a scratch directory whose name has an upper-case letter (see `replay_tsc.rs`), with `replay-tsc`
/// installed as `node_modules/typescript/bin/tsc` when `replay`.
fn project(replay: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Families").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/families"), &root);
    if replay {
        let package = root.join("node_modules/typescript");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), format!(r#"{{"version": "{FAKE_VERSION}"}}"#)).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_replay-tsc"), package.join("bin/tsc")).unwrap();
    }
    Project { _dir: dir, root }
}

fn structural(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions { src_dirs: vec![project.root.clone()], ..PipelineOptions::default() })
}

fn answer(project: &Project, live: Option<(&Utf8PathBuf, bool)>) -> Value {
    let output = structural(project);
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    match live {
        None => {
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/families.recording.json", env!("CARGO_MANIFEST_DIR")),
            ));
        }
        Some((bin, reverse)) => {
            request.tsc_path = Some(bin.clone());
            request.reverse_queries = reverse;
        }
    }
    let outcome = resolve(request);
    assert_eq!(outcome.diagnostics, []);
    assert_eq!(outcome.fallbacks, []);
    rendered(&outcome)
}

/// Each component's props, in the order the checker gave them.
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
            (component.display_name.clone(), Value::Array(props))
        })
        .collect();
    json!({ "components": components })
}

#[allow(clippy::print_stderr, reason = "a skipped test says so, since a silent skip looks like a pass")]
fn tsrs_bin() -> Option<Utf8PathBuf> {
    let bin = std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty());
    if bin.is_none() {
        eprintln!("skipped: TSRS_BIN is not set");
    }
    bin.map(Utf8PathBuf::from)
}

/// The answer replayed and, when `TSRS_BIN` names a real `tsrs`, live both ways round, held to the same checks.
fn answers(check: impl Fn(&Value)) {
    check(&answer(&project(true), None));
    let Some(bin) = tsrs_bin() else { return };
    let real = project(false);
    for reverse in [false, true] {
        check(&answer(&real, Some((&bin, reverse))));
    }
}

fn prop_type(rendered: &Value, component: &str, prop: &str) -> Value {
    rendered["components"][component]
        .as_array()
        .and_then(|props| props.iter().find(|candidate| candidate["name"] == prop))
        .unwrap_or_else(|| panic!("{component}.{prop} is in the answer"))["type"]
        .clone()
}

fn field_names(ty: &Value) -> Vec<String> {
    ty["fields"].as_array().unwrap().iter().map(|field| field["name"].as_str().unwrap().to_string()).collect()
}

fn named(name: &str, args: Vec<Value>) -> Value {
    json!({ "kind": "named", "name": name, "args": args })
}

fn html(element: &str) -> Value {
    json!({ "kind": "htmlAttributes", "element": element, "omitted": [] })
}

fn literal(value: &str) -> Value {
    json!({ "kind": "stringLiteral", "value": value })
}

#[test]
fn the_recorded_answers_of_the_checker_convert_to_the_snapshot() {
    insta::assert_json_snapshot!("families", answer(&project(true), None));
}

#[test]
fn a_real_tsrs_gives_the_snapshot_whichever_way_round_it_is_asked() {
    let Some(bin) = tsrs_bin() else { return };
    let real = project(false);
    let forward = answer(&real, Some((&bin, false)));
    let reversed = answer(&real, Some((&bin, true)));

    insta::assert_json_snapshot!("families", forward);
    assert_eq!(serde_json::to_string(&forward).unwrap(), serde_json::to_string(&reversed).unwrap());
}

/// The cap counts members a project did not declare. Its own interface of 150 fields is listed whole, at the top of a
/// prop's type and below it.
#[test]
fn an_interface_the_project_declares_is_never_cut() {
    answers(|rendered| {
        let wide = prop_type(rendered, "Own", "wide");
        assert_eq!(wide["kind"], "object");
        assert_eq!(field_names(&wide).len(), 150);
        let nested = prop_type(rendered, "Own", "nested");
        assert_eq!(field_names(&nested["fields"][0]["propType"]).len(), 150);
        assert!(!serde_json::to_string(&rendered["components"]["Own"]).unwrap().contains("\"truncated\""));
    });
}

/// A package's 150 fields stand as its name: in the utility wrapper that applied to it, or as the layer an interface of
/// the project extends, with the project's own fields beside it.
#[test]
fn what_a_dependency_declares_stands_as_a_named_layer() {
    answers(|rendered| {
        let layer = named("DepWide", vec![]);
        assert_eq!(prop_type(rendered, "Dependency", "omitted"), named("Omit", vec![layer.clone(), literal("f001")]));
        assert_eq!(prop_type(rendered, "Dependency", "partial"), named("Partial", vec![layer.clone()]));

        let extended = prop_type(rendered, "Dependency", "extended");
        assert_eq!(extended["kind"], "intersection");
        assert_eq!(field_names(&extended["members"][0]), ["own", "second"]);
        assert_eq!(extended["members"][1], layer);

        // The project's interface that extends another of the project's is seen through to the package.
        let chain = prop_type(rendered, "Dependency", "chain");
        assert_eq!(field_names(&chain["members"][0]), ["third", "own", "second"]);
        assert_eq!(chain["members"][1], layer);

        // Below the budget the result is listed, wrapper or not.
        assert_eq!(field_names(&prop_type(rendered, "Dependency", "small")), ["b"]);
    });
}

/// React's DOM attribute types are recognized by where they are declared and by name, and the element is read from the
/// type argument. The wrapper stays a name; the alias a wrapper was written through (`ComponentPropsWithoutRef`) is not
/// kept by the checker, so the `Omit` it expands to is.
#[test]
fn a_wrapper_over_a_react_attribute_family_is_kept() {
    answers(|rendered| {
        let omit = |inner: Value, key: &str| named("Omit", vec![inner, literal(key)]);
        let link = prop_type(rendered, "Wrappers", "link");
        assert_eq!(link, omit(omit(html("a"), "ref"), "href"));
        assert_eq!(prop_type(rendered, "Wrappers", "partial"), named("Partial", vec![html("div")]));
        assert_eq!(prop_type(rendered, "Wrappers", "twice"), omit(omit(html("span"), "color"), "style"));
        assert_eq!(prop_type(rendered, "Wrappers", "required"), named("Required", vec![html("a")]));

        // `Pick` of two properties is small: it is expanded, not kept as a wrapper.
        assert_eq!(field_names(&prop_type(rendered, "Wrappers", "picked")), ["disabled", "type"]);

        // A family that is not wrapped is the name it always had, its arguments included.
        let element = named("HTMLAnchorElement", vec![]);
        assert_eq!(prop_type(rendered, "Wrappers", "direct"), named("AnchorHTMLAttributes", vec![element]));
        assert_eq!(prop_type(rendered, "Wrappers", "detailed")["name"], "DetailedHTMLProps");

        // An element the table does not know is not guessed: the wrapper holds the interface by name.
        let unknown = prop_type(rendered, "Wrappers", "unknownElement");
        assert_eq!(unknown["name"], "Omit");
        assert_eq!(unknown["args"][0]["name"], "HTMLAttributes");
    });
}

/// An interface that extends `Omit<HTMLAttributes<HTMLDivElement>, "color">` lists its own fields and names the base. A
/// generic instantiation does not state its bases in terms of its arguments, so the base is cut with its real count.
#[test]
fn an_interface_extending_a_wrapper_lists_its_own_fields_and_names_the_base() {
    answers(|rendered| {
        let boxed = prop_type(rendered, "Boxes", "box");
        assert_eq!(boxed["kind"], "intersection");
        assert_eq!(field_names(&boxed["members"][0]), ["color", "as"]);
        assert_eq!(boxed["members"][1], named("Omit", vec![html("div"), literal("color")]));

        let generic = prop_type(rendered, "Boxes", "boxed");
        assert_eq!(field_names(&generic["members"][0]), ["value"]);
        assert_eq!(generic["members"][1]["kind"], "truncated");
        assert_eq!(generic["members"][1]["limit"]["type"], "members");
        assert!(generic["members"][1]["limit"]["total"].as_u64().unwrap() > 100);
    });
}

/// `ReactNode` inside a union arrives as its members. They are the terminal again, whatever else the union holds stays,
/// and a union that holds only some of them is not the terminal.
#[test]
fn react_node_flattened_into_a_union_is_the_terminal_again() {
    answers(|rendered| {
        let node = prop_type(rendered, "Terminals", "node");
        assert_eq!(node["members"][0], json!({ "kind": "reactNode" }));
        assert_eq!(node["members"][1]["kind"], "function");
        assert_eq!(node["members"].as_array().unwrap().len(), 2);

        let mixed = prop_type(rendered, "Terminals", "mixed");
        let kinds: Vec<&str> =
            mixed["members"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["reactNode", "named", "object"]);

        let portal = prop_type(rendered, "Terminals", "portal");
        assert_eq!(portal, json!({ "kind": "union", "members": [{ "kind": "string" }, named("ReactPortal", vec![])] }));
    });
}

/// `ElementType` below the top of a prop is wider than the nested budget once the checker flattens it: it is the
/// terminal, not a cut. At the top, and where it lists a few tags, the members are listed as before.
#[test]
fn element_type_wider_than_the_budget_is_the_terminal() {
    answers(|rendered| {
        let at = prop_type(rendered, "Polymorphic", "at");
        assert_eq!(at["fields"][0]["propType"], json!({ "kind": "elementType" }));

        assert_eq!(prop_type(rendered, "Polymorphic", "top")["kind"], "union");
        let few = prop_type(rendered, "Polymorphic", "few");
        assert_eq!(few["fields"][0]["propType"]["kind"], "union");
    });
}
