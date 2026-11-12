#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
//! What the conversion memo costs and what it may not change. Two fixture projects, each replayed from its recording
//! (`memo.recording.json`, `cycles.recording.json`, made by the `record_fixture` example) and asked live (the
//! built-in checker, or the `tsrs` binary `TSRS_BIN` names), forward and last to first:
//!
//! - `memo`: one big type under many different enclosing types. The number of round trips to the checker is held to a
//!   bound, so a memo keyed by the whole enclosing context (which converts the type again under each) fails, and a
//!   type converted again at another depth does not ask for its members or type arguments again;
//! - `cycles`: a type reached both inside a cycle and outside it, in either order, is a back-reference only where the
//!   type it points to encloses it, so a memo that reused a conversion across contexts it depends on fails.

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

/// `tests/fixtures/<fixture>` in a scratch directory whose name has an upper-case letter (see `replay_tsc.rs`), with
/// `replay-tsc` installed as `node_modules/typescript/bin/tsc` when `replay`.
fn project(fixture: &str, replay: bool) -> Project {
    let dir = tempfile::Builder::new().prefix("Memo").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    copy_fixture(&Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture), &root);
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

/// The `tsrs` binary `TSRS_BIN` names, to run as a process; `None` asks the built-in checker.
fn tsrs_bin() -> Option<Utf8PathBuf> {
    std::env::var("TSRS_BIN").ok().filter(|bin| !bin.is_empty()).map(Utf8PathBuf::from)
}

/// The outcome of one run, and the calls the backend made to the checker.
struct Run {
    outcome: SemanticOutcome,
    round_trips: usize,
    /// Each call as `{method, params, result}`, in the order sent.
    calls: Vec<Value>,
}

/// Asks the checker about every component of `fixture`: replayed, or live (`reverse`: asked last to first).
fn run(fixture: &str, live: Option<(&Option<Utf8PathBuf>, bool)>) -> Run {
    let project = project(fixture, live.is_none());
    let output = structural(&project);
    let calls = project.root.join("calls.json");
    let mut request = SemanticRequest::new(project.root.clone(), targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    request.record_to = Some(calls.clone());
    match live {
        None => {
            request.tsc_path = Some(project.root.join("node_modules/typescript/bin/tsc"));
            request.expected_versions = &[FAKE_VERSION];
            request.extra_env.push((
                "REPLAY_FIXTURE".to_string(),
                format!("{}/tests/fixtures/{fixture}.recording.json", env!("CARGO_MANIFEST_DIR")),
            ));
        }
        Some((bin, reverse)) => {
            request.tsc_path = bin.clone();
            request.reverse_queries = reverse;
        }
    }
    let outcome = resolve(request);
    assert_eq!(outcome.diagnostics, []);
    assert_eq!(outcome.fallbacks, []);
    let recorded: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(calls).unwrap()).unwrap();
    Run { outcome, round_trips: recorded.len(), calls: recorded }
}

/// Every way to ask: replayed, and live, first to last and last to first.
fn each_way(fixture: &str, check: impl Fn(&Run, &str)) {
    check(&run(fixture, None), "replayed");
    let bin = tsrs_bin();
    for reverse in [false, true] {
        check(&run(fixture, Some((&bin, reverse))), if reverse { "live, last to first" } else { "live" });
    }
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

/// The most round trips the `memo` fixture takes with the memo shared across contexts: it takes 244, where keyed by the
/// whole enclosing context it took 1,211.
const ROUND_TRIPS: usize = 300;

/// A type that many different types enclose is converted once, not once under each: sixteen interfaces and twelve
/// aliases of a union all hold the same big object, and so do `Box<T>` instantiated under one another.
#[test]
fn a_type_many_types_enclose_is_converted_once() {
    each_way("memo", |run, how| {
        assert!(run.round_trips <= ROUND_TRIPS, "{how}: {} round trips, at most {ROUND_TRIPS}", run.round_trips);

        // What was shared is still what each place holds.
        let shared = |component: &str, prop: &str, pointer: &str| {
            let ty = prop_type(&run.outcome, component, prop);
            ty.pointer(pointer).unwrap_or_else(|| panic!("{pointer} is in {ty}")).clone()
        };
        let first = shared("Many", "w1", "/fields/0/propType");
        assert_eq!(first["kind"], "object", "{how}");
        for index in 2..=16 {
            assert_eq!(shared("Many", &format!("w{index}"), "/fields/0/propType"), first, "{how}: w{index}");
        }
        // At the same depth, whichever instantiation of `Box` it is in.
        assert_eq!(shared("Many", "box1", "/fields/1/propType"), first, "{how}");
        assert_eq!(shared("Many", "box3", "/fields/0/propType/fields/1/propType")["kind"], "object", "{how}");
        // A union's members all hold it, one level further down.
        let member = shared("Unions", "wide", "/members/0/fields/1/propType");
        assert_eq!(member["kind"], "object", "{how}");
        for index in 1..12 {
            assert_eq!(shared("Unions", "wide", &format!("/members/{index}/fields/1/propType")), member, "{how}");
        }
    });
}

/// What a type's parts are does not change inside a snapshot, so a type converted again (the memo is keyed by depth too)
/// does not ask the checker for its members or type arguments a second time. Each repeat is a round trip, and on
/// Chakra's `Button`, whose style props hold the same unions again one level down, they were one request in eight.
#[test]
fn the_parts_of_a_type_are_asked_for_once() {
    each_way("memo", |run, how| {
        let mut seen = std::collections::BTreeMap::new();
        for call in &run.calls {
            let method = call["method"].as_str().unwrap_or_default();
            if matches!(method, "getTypesOfType" | "getTypeArguments") {
                *seen.entry((method.to_string(), call["params"].to_string())).or_insert(0) += 1;
            }
        }
        let repeated: Vec<String> = seen
            .iter()
            .filter(|(_, count)| **count > 1)
            .map(|((method, params), count)| format!("{method} {params} x{count}"))
            .collect();
        assert!(!seen.is_empty(), "{how}: no parts were asked for");
        assert_eq!(repeated, Vec::<String>::new(), "{how}");
    });
}

fn field(name: &str, prop_type: Value) -> Value {
    json!({ "name": name, "propType": prop_type, "required": true, "description": "" })
}

fn object(fields: Vec<Value>) -> Value {
    json!({ "kind": "object", "fields": fields })
}

fn recursive(name: &str) -> Value {
    json!({ "kind": "recursive", "name": name, "args": [] })
}

/// `Node1` and `Node2` contain each other. Each is a back-reference only inside the other, and a type that was
/// converted outside the cycle (`Holder`, whose `Node2` is not inside `Node1`) is not reused inside it, nor the other
/// way round, whichever of the two the memo saw first.
#[test]
fn a_type_inside_and_outside_a_cycle_is_a_back_reference_only_inside() {
    let number = json!({ "kind": "number" });
    let string = json!({ "kind": "string" });
    // `Node1` as a prop: its `Node2` sits inside it, so that one's `back` points back.
    let node1 = object(vec![
        field("next", object(vec![field("back", recursive("Node1")), field("tag", string.clone())])),
        field("id", number.clone()),
    ]);
    // `Node2` as a prop: the mirror image.
    let node2 = object(vec![
        field("back", object(vec![field("next", recursive("Node2")), field("id", number)])),
        field("tag", string),
    ]);
    // `Holder` encloses neither: its `Node2` is the same as the prop's, expanded all the way to the lap.
    let expected = [
        ("holder", object(vec![field("held", node2.clone())])),
        ("first", node1.clone()),
        ("second", node2.clone()),
        ("wrapped", object(vec![field("inner", node1.clone())])),
        ("both", object(vec![field("a", node1), field("b", node2)])),
    ];
    each_way("cycles", |run, how| {
        for component in ["OutsideFirst", "InsideFirst"] {
            for (prop, want) in &expected {
                assert_eq!(&prop_type(&run.outcome, component, prop), want, "{how}: {component}.{prop}");
            }
        }
    });
}
