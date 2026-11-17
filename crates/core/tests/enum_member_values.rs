#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `Enum.Member` as a prop's type has the member's value as TypeScript folds it, or stays unresolved when the structural
//! lane cannot fold it: a member is never typed with its own name. `crates/semantic/tests/enum_member_values.rs` asks
//! the checker for the same forms.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use camino::Utf8PathBuf;
use serde_json::{Value, json};

fn project(source: &str) -> (Utf8PathBuf, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    // An empty `typescript` install, so no missing-package warning joins the diagnostics.
    let lib = root.join("node_modules/typescript/lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(root.join("node_modules/typescript/package.json"), r#"{"name":"typescript","version":"5.9.3"}"#)
        .unwrap();
    for file in ["lib.es5.d.ts", "lib.dom.d.ts"] {
        std::fs::write(lib.join(file), "").unwrap();
    }
    std::fs::write(root.join("case.tsx"), source).unwrap();
    (root, tmp)
}

/// Prop name to type of the only component, and the messages of every diagnostic.
fn props_of(source: &str) -> (BTreeMap<String, Value>, Vec<String>) {
    let (root, _tmp) = project(source);
    let options = PipelineOptions { src_dirs: vec![root], use_cache: false, ..Default::default() };
    let output = extract(&options);
    assert_eq!(output.components.len(), 1, "{:?}", output.components.keys());
    let diagnostics = output.diagnostics.iter().map(|d| d.message.clone()).collect();
    let entry = output.components.into_values().next().unwrap();
    let props =
        entry.props.iter().map(|(name, prop)| (name.clone(), serde_json::to_value(&prop.prop_type).unwrap())).collect();
    (props, diagnostics)
}

/// The type of a prop typed `E.A`, for `enum E { A = <init> }`, and the diagnostics.
fn member_of(init: &str) -> (Value, Vec<String>) {
    let source = format!(
        "declare const compute: () => number;\ndeclare const OTHER: number;\nenum E {{ A = {init} }}\n\
         export function C(props: {{ a: E.A }}) {{ return <b />; }}"
    );
    let (props, diagnostics) = props_of(&source);
    (props["a"].clone(), diagnostics)
}

fn member(name: &str, value: Value) -> Value {
    json!({ "kind": "enumMember", "enum": "E", "member": name, "value": value })
}

fn number(value: f64) -> Value {
    json!({ "kind": "numberLiteral", "value": value })
}

fn string(value: &str) -> Value {
    json!({ "kind": "stringLiteral", "value": value })
}

fn unresolved(name: &str) -> Value {
    json!({ "kind": "named", "name": name, "args": [] })
}

/// Initializers the structural lane folds, each with the value the checker gives it.
const FOLDED_NUMBERS: [(&str, f64); 25] = [
    ("1", 1.0),
    ("-1", -1.0),
    ("+1", 1.0),
    ("~1", -2.0),
    ("(2)", 2.0),
    ("0x10", 16.0),
    ("1e3", 1000.0),
    ("1_000", 1000.0),
    ("1 << 2", 4.0),
    ("-8 >> 1", -4.0),
    ("-8 >>> 28", 15.0),
    ("6 & 3", 2.0),
    ("6 | 1", 7.0),
    ("3 | 1", 3.0),
    ("6 ^ 3", 5.0),
    ("2 ** 3", 8.0),
    ("7 % 4", 3.0),
    ("7 / 2", 3.5),
    ("1 + 2 * 3", 7.0),
    ("-(1 + 2)", -3.0),
    ("(1 + 2) * 3", 9.0),
    ("5 - 2 - 1", 2.0),
    ("0", 0.0),
    ("-0", 0.0),
    ("1 << 31", -2_147_483_648.0),
];

const FOLDED_STRINGS: [(&str, &str); 7] = [
    ("\"a\"", "a"),
    ("'a'", "a"),
    ("`t`", "t"),
    ("\"a\" + \"b\"", "ab"),
    ("\"a\" + 1", "a1"),
    ("1 + \"a\"", "1a"),
    // A surrogate pair inside one literal is well-formed text.
    ("\"\\uD83D\\uDE00\"", "\u{1f600}"),
];

/// Initializers the structural lane does not fold (the checker folds some of them).
const UNFOLDED: [&str; 18] = [
    "compute()",
    "OTHER",
    "true",
    "1 / 0",
    "0 / 0",
    "2 ** 1024",
    "\"a\" + 1.5",
    "\"a\" + OTHER",
    "`t${1}`",
    "1 < 2",
    "!1",
    "typeof 1",
    "void 0",
    "B",
    "E.B",
    // A lone surrogate is no text: TypeScript joins two halves into one emoji, and the replacement character is not it.
    "\"\\uD83D\" + \"\\uDE00\"",
    "\"\\uD83D\"",
    "`\\uD83D`",
];

#[test]
fn an_enum_member_the_structural_lane_folds_has_the_value_typescript_gives_it() {
    for (init, value) in FOLDED_NUMBERS {
        let (ty, diagnostics) = member_of(init);
        assert_eq!(ty, member("A", number(value)), "{init}");
        assert_eq!(diagnostics, Vec::<String>::new(), "{init}");
    }
    for (init, value) in FOLDED_STRINGS {
        let (ty, diagnostics) = member_of(init);
        assert_eq!(ty, member("A", string(value)), "{init}");
        assert_eq!(diagnostics, Vec::<String>::new(), "{init}");
    }
}

#[test]
fn an_enum_member_the_structural_lane_cannot_fold_is_not_its_own_name() {
    for init in UNFOLDED {
        let (ty, diagnostics) = member_of(init);
        assert_eq!(ty, unresolved("E.A"), "{init}");
        assert_eq!(diagnostics.len(), 1, "{init}: {diagnostics:?}");
        assert!(diagnostics[0].starts_with("Cannot resolve type 'E.A'"), "{init}: {diagnostics:?}");
    }
}

#[test]
fn a_member_that_reads_an_earlier_member_of_its_enum_has_its_value() {
    let (props, diagnostics) = props_of(
        r#"
        enum E { A = 1, B = A | 2, C = E.B + 1, D = E["C"] << 1, F = "x", G = F + "y" }
        export function C(props: { b: E.B; c: E.C; d: E.D; g: E.G }) { return <b />; }
        "#,
    );
    assert_eq!(props["b"], member("B", number(3.0)));
    assert_eq!(props["c"], member("C", number(4.0)));
    assert_eq!(props["d"], member("D", number(8.0)));
    assert_eq!(props["g"], member("G", string("xy")));
    assert_eq!(diagnostics, Vec::<String>::new());
}

/// A read names the enum being declared only when its object is that enum and no member is named like it; a read through
/// another enum with an earlier member of the same name is not that member.
#[test]
fn a_read_through_another_object_or_a_shadowed_enum_name_is_not_an_earlier_member() {
    let (props, _) = props_of(
        r#"
        enum Other { A = 100 }
        enum E { A = 1, ViaOther = Other.A, Plain = E.A }
        export function C(props: { viaOther: E.ViaOther; plain: E.Plain }) { return <b />; }
        "#,
    );
    assert_eq!(props["viaOther"], unresolved("E.ViaOther"));
    assert_eq!(props["plain"], member("Plain", number(1.0)));
    // Inside `enum S`, the member `S` hides the enum, so `S.S` is no read of the enum's own member.
    let (props, _) = props_of(
        r#"
        enum S { S = 1, B = S.S }
        export function C(props: { b: S.B }) { return <b />; }
        "#,
    );
    assert_eq!(props["b"], json!({ "kind": "named", "name": "S.B", "args": [] }));
}

/// A computed key holding a lone surrogate names no member: both halves read as the replacement character in a member
/// name, so the key `E["\uDE00"]` must not find the member written `"\uD83D"`.
#[test]
fn a_member_read_by_a_key_with_a_lone_surrogate_is_unresolved() {
    let (props, _) = props_of(
        r#"
        enum E { "\uD83D" = 1, B = E["\uDE00"] }
        export function C(props: { b: E.B }) { return <b />; }
        "#,
    );
    assert_eq!(props["b"], unresolved("E.B"));
}

/// Numbering counts up from the last known value; after one that is not known, no later member has a value.
#[test]
fn auto_increment_follows_only_known_values() {
    let (props, _) = props_of(
        r#"
        declare const compute: () => number;
        enum E { Shift = 1 << 2, Next, Plain = 3, After, Fn = compute(), AfterFn, Str = "s", AfterStr }
        export function C(props: { shift: E.Shift; next: E.Next; after: E.After; fn: E.Fn; afterFn: E.AfterFn; afterStr: E.AfterStr }) { return <b />; }
        "#,
    );
    assert_eq!(props["shift"], member("Shift", number(4.0)));
    assert_eq!(props["next"], member("Next", number(5.0)));
    assert_eq!(props["after"], member("After", number(4.0)));
    assert_eq!(props["fn"], unresolved("E.Fn"));
    assert_eq!(props["afterFn"], unresolved("E.AfterFn"));
    assert_eq!(props["afterStr"], unresolved("E.AfterStr"));
}

/// The exported enum map lists each member's value, `null` for one that is not known, never its name.
#[test]
fn the_public_enum_map_does_not_invent_a_value_for_an_unknown_member() {
    let (root, _tmp) = project(
        r#"
        declare const compute: () => number;
        export enum Level { Shift = 1 << 2, Next, Cat = "a" + "b", Fn = compute(), Flag = true }
        export function C(props: { level: Level.Shift }) { return <b />; }
        "#,
    );
    let output = extract(&PipelineOptions { src_dirs: vec![root], use_cache: false, ..Default::default() });
    let entries = serde_json::to_value(&output.enums["Level"]).unwrap();
    let values: Vec<Value> = entries.as_array().unwrap().iter().map(|entry| entry["value"].clone()).collect();
    assert_eq!(values, vec![json!(4.0), json!(5.0), json!("ab"), Value::Null, Value::Null]);
}
