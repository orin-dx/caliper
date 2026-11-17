#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The structural lane's value for `Enum.Member` against the checker's: wherever the structural lane gives a value it is
//! the one TypeScript computes, and it gives one for every initializer form it folds (`extractor/enum_fold.rs`), over
//! each declaration kind. A form it does not fold stays unresolved there and the checker answers it.

use std::collections::BTreeMap;
use std::time::Duration;

use caliper_docgen_core::pipeline::{PipelineOptions, extract};
use caliper_docgen_core::types::{EnumMemberValue, PropType};
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

/// Single-member enums `enum E<i> { A = <init> }`, the initializer forms of TypeScript's enum grammar, and whether the
/// structural lane folds it.
const FORMS: [(&str, bool); 65] = [
    ("1", true),
    ("-1", true),
    ("+1", true),
    ("~1", true),
    ("(2)", true),
    ("((2))", true),
    ("0x10", true),
    ("0b101", true),
    ("0o17", true),
    ("1e3", true),
    ("1_000", true),
    ("1.5", true),
    ("-0", true),
    ("0", true),
    ("1 << 2", true),
    ("1 << 33", true),
    ("1 << 31", true),
    ("-8 >> 1", true),
    ("-8 >>> 28", true),
    ("-1 >>> 0", true),
    ("6 & 3", true),
    ("6 | 1", true),
    ("3 | 1", true),
    ("\"\\uD83D\\uDE00\"", true),
    ("\"\\uD83D\" + \"\\uDE00\"", false),
    ("6 ^ 3", true),
    ("2 ** 3", true),
    ("2 ** -1", true),
    ("7 % 4", true),
    ("-7 % 4", true),
    ("7 / 2", true),
    ("1 + 2 * 3", true),
    ("-(1 + 2)", true),
    ("(1 + 2) * 3", true),
    ("5 - 2 - 1", true),
    ("~~5", true),
    ("-~5", true),
    ("4294967296 | 0", true),
    ("2147483648 | 0", true),
    ("\"a\"", true),
    ("'a'", true),
    ("`t`", true),
    ("\"a\" + \"b\"", true),
    ("\"a\" + 1", true),
    ("1 + \"a\"", true),
    ("\"a\" + -1", true),
    ("(\"a\")", true),
    ("\"a\" + (\"b\" + \"c\")", true),
    ("1 / 0", false),
    ("0 / 0", false),
    ("2 ** 1024", false),
    ("\"a\" + 1.5", false),
    ("`t${1}`", false),
    ("OTHER", false),
    ("compute()", false),
    ("true", false),
    ("1 < 2", false),
    ("!1", false),
    ("typeof 1", false),
    ("void 0", false),
    ("null", false),
    ("1n", false),
    ("1 as number", false),
    ("B", false),
    ("E.B", false),
];

const MULTI: &str = r#"
enum M {
  Shift = 1 << 2, Next, Plain = 3, After, Cat = "a" + "b", Ref = OTHER, AfterRef, Neg = -1, AfterNeg,
  Bit = Shift | 1, Same = M.Plain + 1, Key = M["Plain"] << 1, Str = "x", Joined = Str + "y", Fn = compute(), AfterFn,
}
declare enum DE { A = 1 << 2, B, C = "x" + "y" }
declare const enum DC { A = 1 << 2, B }
const enum CE { A = 1 << 2, B, C = A | B }
enum Merged { A = 1 }
enum Merged { B = A + 1, C }
"#;

const MEMBERS: [&str; 33] = [
    "M.Shift",
    "M.Next",
    "M.Plain",
    "M.After",
    "M.Cat",
    "M.Ref",
    "M.AfterRef",
    "M.Neg",
    "M.AfterNeg",
    "M.Bit",
    "M.Same",
    "M.Key",
    "M.Str",
    "M.Joined",
    "M.Fn",
    "M.AfterFn",
    "DE.A",
    "DE.B",
    "DE.C",
    "DC.A",
    "DC.B",
    "CE.A",
    "CE.B",
    "CE.C",
    "Merged.A",
    "Merged.B",
    "Merged.C",
    "Amb.A",
    "Amb.B",
    "Amb.C",
    "Amb.D",
    "Amb.E",
    "Amb.F",
];

fn source() -> String {
    let mut text =
        String::from("import { Amb } from \"./ambient\";\ndeclare const compute: () => number;\nconst OTHER = 7;\n");
    for (i, (init, _)) in FORMS.iter().enumerate() {
        text.push_str(&format!("enum E{i} {{ A = {init} }}\n"));
    }
    text.push_str(MULTI);
    let mut props: Vec<String> = (0..FORMS.len()).map(|i| format!("f{i}: E{i}.A")).collect();
    props.extend(MEMBERS.iter().map(|member| format!("{}: {member}", member.replace('.', "_"))));
    text.push_str(&format!("export function C(props: {{ {} }}) {{ return null; }}\n", props.join("; ")));
    text
}

fn value(ty: &PropType) -> Option<EnumMemberValue> {
    match ty {
        PropType::EnumMember { value, .. } => Some(value.clone()),
        _ => None,
    }
}

#[test]
fn a_value_the_structural_lane_gives_an_enum_member_is_the_one_the_checker_gives() {
    let dir = tempfile::Builder::new().prefix("EnumValues").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::write(root.join("lib.tsx"), source()).unwrap();
    std::fs::write(
        root.join("ambient.d.ts"),
        "export declare enum Amb { A = 1 << 2, B, C = \"x\" + \"y\", D = 3, E, F = -1 << 1 }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "jsx": "react-jsx", "module": "esnext", "moduleResolution": "bundler" } }"#,
    )
    .unwrap();

    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let structural = &output.components["C"];
    let mut request = SemanticRequest::new(root, targets_of(&output));
    request.request_timeout = Duration::from_secs(60);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(resolve(request)).ok());
    let outcome = receiver.recv_timeout(Duration::from_secs(120)).expect("the conversion ends");
    assert_eq!(outcome.diagnostics, []);
    let answer = outcome.components.iter().find(|component| component.key == "C").expect("the checker answers C");
    let checker: BTreeMap<&str, &PropType> =
        answer.props.iter().map(|prop| (prop.name.as_str(), &prop.prop_type)).collect();

    let mut names: Vec<(String, Option<bool>)> =
        FORMS.iter().enumerate().map(|(i, (_, folds))| (format!("f{i}"), Some(*folds))).collect();
    names.extend(MEMBERS.iter().map(|member| (member.replace('.', "_"), None)));
    let mut folded = 0;
    for (name, folds) in &names {
        let ours = value(&structural.props[name.as_str()].prop_type);
        if let Some(expected) = folds {
            assert_eq!(ours.is_some(), *expected, "{name}: {:?}", structural.props[name.as_str()].prop_type);
            // The checker knows every form that is an enum initializer at all; a few it rejects.
            if *expected {
                assert!(
                    value(checker[name.as_str()]).is_some(),
                    "{name}: the checker has a value: {:?}",
                    checker[name.as_str()]
                );
            }
        }
        if let Some(ours) = ours {
            folded += 1;
            assert_eq!(Some(ours), value(checker[name.as_str()]), "{name}: {:?}", checker[name.as_str()]);
        }
    }
    assert!(folded >= 50, "{folded}");
}
