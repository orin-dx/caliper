use super::*;
use caliper_docgen_core::pipeline::rdt::type_json as rdt_type_json;

fn args_for(src: &str, json: bool) -> crate::ExtractArgs {
    crate::ExtractArgs {
        src: vec![src.to_owned()],
        out: None,
        format: crate::OutputFormat::Canonical,
        cache_dir: None,
        html_attributes: None,
        extra_builtins: vec![],
        json,
    }
}

#[test]
fn json_mode_still_returns_the_error_exit_code() {
    // Adversarial review finding: --json returned before the error-
    // diagnostic exit-code check, so `extract --json` always exited 0
    // even on hard errors — the one signal CI actually depends on.
    let code = cmd_extract(args_for("/nonexistent/does-not-exist", true), true, None)
        .expect("cmd_extract itself should not error");
    assert_eq!(code, 2, "expected exit code 2 for a nonexistent src dir even in --json mode");
}

#[test]
fn non_json_mode_returns_the_same_error_exit_code() {
    // Same check, non-json path — both must agree.
    let code = cmd_extract(args_for("/nonexistent/does-not-exist", false), true, None)
        .expect("cmd_extract itself should not error");
    assert_eq!(code, 2);
}

// ── SPEC-CLI-001a AC-001: extract against a source directory that
// produces zero Error-severity diagnostics returns exit code 0.

#[test]
fn clean_run_returns_exit_code_zero() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Widget.tsx"),
        "export function Widget(props: { label: string }) { return null; }\n",
    )
    .unwrap();
    let code =
        cmd_extract(args_for(tmp.path().to_str().unwrap(), true), true, None).expect("cmd_extract should not error");
    assert_eq!(code, 0, "expected exit code 0 for a clean run");
}

// ── SPEC-CLI-001b AC-002/AC-003/AC-004: extract --out's actual call site,
// not just write_atomic in isolation — these previously had zero coverage;
// a regression reverting extract.rs's --out handling to a bare
// std::fs::write would have passed every existing test in the suite.

#[test]
fn extract_out_writes_valid_output_with_no_leftover_temp_file() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Widget.tsx"),
        "export function Widget(props: { label: string }) { return null; }\n",
    )
    .unwrap();

    let out_path = tmp.path().join("out.json");
    let mut args = args_for(tmp.path().to_str().unwrap(), false);
    args.out = Some(out_path.to_str().unwrap().to_owned());

    let code = cmd_extract(args, true, None).expect("cmd_extract should succeed");
    assert_eq!(code, 0);

    let contents = std::fs::read_to_string(&out_path).expect("expected the --out file to exist");
    let parsed: serde_json::Value = serde_json::from_str(&contents).expect("--out file should be valid JSON");
    assert!(
        parsed["components"]["Widget"].is_object(),
        "expected the Widget component in the --out file, got {contents}"
    );

    let tmp_file = tmp.path().join(".out.json.tmp");
    assert!(!tmp_file.exists(), "no leftover temp file should remain after a successful --out write");
}

// The test above alone can't distinguish write_atomic from a naive
// std::fs::write — both produce identical end states for a plain path
// (verified: reverting extract.rs's --out arm to bare std::fs::write
// still passed it). A symlinked --out target can distinguish them: a
// rename-based write REPLACES the symlink itself with a real file at
// PATH, while a naive write FOLLOWS the symlink and corrupts whatever
// it points to instead — the one observable difference a black-box
// test can exploit without instrumenting the write mid-flight.

#[cfg(unix)]
#[test]
fn extract_out_through_a_symlink_replaces_the_link_not_its_target() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Widget.tsx"),
        "export function Widget(props: { label: string }) { return null; }\n",
    )
    .unwrap();

    let real_target = tmp.path().join("real-target.json");
    std::fs::write(&real_target, "SENTINEL: pre-existing unrelated content").unwrap();
    let out_path = tmp.path().join("out.json");
    std::os::unix::fs::symlink(&real_target, &out_path).unwrap();

    let mut args = args_for(tmp.path().to_str().unwrap(), false);
    args.out = Some(out_path.to_str().unwrap().to_owned());
    let code = cmd_extract(args, true, None).expect("cmd_extract should succeed");
    assert_eq!(code, 0);

    assert!(
        !std::fs::symlink_metadata(&out_path).unwrap().file_type().is_symlink(),
        "expected the rename-based write to replace the symlink at PATH with a real file, but it's still a symlink"
    );
    let target_contents = std::fs::read_to_string(&real_target).unwrap();
    assert_eq!(
        target_contents, "SENTINEL: pre-existing unrelated content",
        "the symlink's original target must be untouched — a write following the symlink instead of \
         replacing it would have corrupted this unrelated file"
    );
    let out_contents = std::fs::read_to_string(&out_path).unwrap();
    assert!(out_contents.contains("\"Widget\""), "expected the new content at PATH itself, got {out_contents}");
}

#[test]
fn extract_out_with_a_missing_parent_directory_surfaces_the_write_error() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Widget.tsx"),
        "export function Widget(props: { label: string }) { return null; }\n",
    )
    .unwrap();

    let out_path = tmp.path().join("no-such-dir").join("out.json");
    let mut args = args_for(tmp.path().to_str().unwrap(), false);
    args.out = Some(out_path.to_str().unwrap().to_owned());

    let err = cmd_extract(args, true, None).expect_err("expected an Err when --out's parent dir is missing");
    // miette's pretty-printer word-wraps long paths mid-string (inserting a
    // "\n  │ " continuation), so a long temp-dir path can't be matched as one
    // contiguous substring — check the load-bearing pieces separately instead.
    let message = format!("{err:?}");
    assert!(message.contains("Writing to"), "expected the error to say 'Writing to', got {message}");
    // The wrap can also split inside "no-such-dir" itself, so drop the whitespace and gutter first.
    let unwrapped: String = message.chars().filter(|c| !c.is_whitespace() && *c != '│').collect();
    assert!(unwrapped.contains("no-such-dir"), "expected the error to name the target path, got {message}");
    assert!(
        message.contains("No such file or directory"),
        "expected the underlying io::Error's message, got {message}"
    );
    assert!(!out_path.exists(), "PATH must not be created when the write fails");
}

#[test]
fn extract_out_failing_at_the_rename_step_cleans_up_the_temp_file() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Widget.tsx"),
        "export function Widget(props: { label: string }) { return null; }\n",
    )
    .unwrap();

    // PATH already exists as a directory — write-to-temp succeeds (the
    // parent exists), but renaming a regular file onto an existing
    // directory fails, exercising the rename-step failure path
    // specifically (distinct from AC-003's missing-parent-dir case).
    let out_path = tmp.path().join("out.json");
    std::fs::create_dir_all(&out_path).unwrap();

    let mut args = args_for(tmp.path().to_str().unwrap(), false);
    args.out = Some(out_path.to_str().unwrap().to_owned());

    let err = cmd_extract(args, true, None).expect_err("expected an Err when the rename step fails");
    let message = format!("{err:?}");
    assert!(message.contains("Writing to"), "expected the error to say 'Writing to', got {message}");
    assert!(message.contains("out.json"), "expected the error to name the target path, got {message}");
    assert!(message.contains("Is a directory"), "expected the underlying io::Error's message, got {message}");

    let tmp_file = tmp.path().join(".out.json.tmp");
    assert!(!tmp_file.exists(), "the temp file must be cleaned up after a failed rename, found {tmp_file:?}");
}

// ── rdt_output_includes_composes ──────────────────────────────────────────
//
// `ComponentEntry.composes` (react-docgen's own "props come from this type,
// listed by name instead of flattened" field) was populated by the resolver
// but silently dropped by serialize_rdt — this is the RDT-format half of
// that fix; the resolver half is
// pipeline::tests::test_unresolvable_intersection_member_records_raw_type_in_composes.

#[test]
fn rdt_output_includes_composes() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Comp.tsx"),
        r#"
export type Weird<T> = T extends string ? { a: string } : { b: number };
export type CompProps = Weird<'x'> & { c: boolean };
export function Comp(props: CompProps) { return null; }
"#,
    )
    .unwrap();

    let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = caliper_docgen_core::pipeline::PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(camino::Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = caliper_docgen_core::pipeline::extract(&options);

    let rdt_json = serialize_rdt(&output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rdt_json).unwrap();
    let composes = parsed["Comp"]["composes"].as_array().expect("expected a composes array in RDT output");
    assert!(!composes.is_empty(), "expected the unresolvable Weird<'x'> member to appear in RDT's composes field");
}

// ── SPEC-SERIALIZATION-001 AC-1/AC-2/AC-3/AC-4: rdt_type_json had zero
// direct tests — it's private and was only ever exercised indirectly.

use caliper_docgen_core::types::PropType;

#[test]
fn rdt_type_json_zero_and_one_member_unions_never_produce_enum_shape() {
    assert_eq!(rdt_type_json(&PropType::Union(vec![])), serde_json::json!({"name": ""}));
    assert_eq!(
        rdt_type_json(&PropType::Union(vec![PropType::StringLiteral("x".into())])),
        serde_json::json!({"name": "\"x\""})
    );
    assert_eq!(
        rdt_type_json(&PropType::LiteralUnion { members: vec![], has_default: false }),
        serde_json::json!({"name": ""})
    );
    assert_eq!(
        rdt_type_json(&PropType::LiteralUnion { members: vec!["only".into()], has_default: false }),
        serde_json::json!({"name": "\"only\""})
    );
}

#[test]
fn rdt_type_json_two_plus_member_literal_unions_produce_exact_enum_shape() {
    let union = PropType::Union(vec![PropType::StringLiteral("red".into()), PropType::NumberLiteral(5.0)]);
    assert_eq!(
        rdt_type_json(&union),
        serde_json::json!({"name": "enum", "raw": "\"red\" | 5", "value": [{"value": "\"red\""}, {"value": "5"}]})
    );

    let literal_union = PropType::LiteralUnion { members: vec!["red".into(), "blue".into()], has_default: false };
    assert_eq!(
        rdt_type_json(&literal_union),
        serde_json::json!({"name": "enum", "raw": "\"red\" | \"blue\"", "value": [{"value": "\"red\""}, {"value": "\"blue\""}]})
    );
}

#[test]
fn rdt_type_json_enum_shape_matches_is_literal_union_exactly() {
    let fixtures: Vec<PropType> = vec![
        PropType::Union(vec![]),
        PropType::Union(vec![PropType::StringLiteral("a".into())]),
        PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::StringLiteral("b".into())]),
        PropType::Union(vec![
            PropType::StringLiteral("a".into()),
            PropType::StringLiteral("b".into()),
            PropType::StringLiteral("c".into()),
        ]),
        PropType::Union(vec![
            PropType::StringLiteral("a".into()),
            PropType::Named { name: "Foo".into(), args: vec![] },
        ]),
        PropType::LiteralUnion { members: vec![], has_default: false },
        PropType::LiteralUnion { members: vec!["a".into()], has_default: false },
        PropType::LiteralUnion { members: vec!["a".into(), "b".into()], has_default: false },
    ];
    for fixture in fixtures {
        let is_enum_shape = rdt_type_json(&fixture)["name"] == "enum";
        assert_eq!(
            is_enum_shape,
            fixture.is_literal_union(),
            "rdt_type_json's enum-shape decision must exactly match is_literal_union() for {fixture:?}"
        );
    }
}

#[test]
fn rdt_type_json_union_with_one_non_literal_member_never_produces_enum_shape() {
    let union = PropType::Union(vec![
        PropType::StringLiteral("a".into()),
        PropType::StringLiteral("b".into()),
        PropType::Named { name: "Foo".into(), args: vec![] },
    ]);
    let json = rdt_type_json(&union);
    assert_ne!(json["name"], "enum", "a union with a non-literal member must not produce the enum shape, got {json}");
}

// ── SPEC-SERIALIZATION-001 AC-8: serialize_rdt emits composes verbatim —
// strengthened from a non-empty check to an exact match.

#[test]
fn serialize_rdt_emits_composes_exactly() {
    use caliper_docgen_core::types::ComponentEntry;
    let entry = ComponentEntry {
        display_name: "Widget".to_string(),
        file_path: "Widget.tsx".into(),
        description: String::new(),
        props: Default::default(),
        inheritance: vec![],
        notable_inherited: Default::default(),
        discriminant_prop: None,
        composes: vec!["BaseA".to_string(), "BaseB".to_string()],
        tags: Default::default(),
        methods: vec![],
        detected_by: caliper_docgen_core::types::DetectionRule::FunctionDeclaration,
    };
    let mut components = std::collections::BTreeMap::new();
    components.insert("Widget".to_string(), entry);
    let output = caliper_docgen_core::types::ExtractionOutput {
        components,
        enums: Default::default(),
        diagnostics: vec![],
        stats: Default::default(),
    };

    let rdt_json = serialize_rdt(&output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rdt_json).unwrap();
    assert_eq!(parsed["Widget"]["composes"], serde_json::json!(["BaseA", "BaseB"]));
}

// ── RDT's default `skipChildrenPropWithoutDoc`: an undocumented `children`
// never appears in `--format rdt`, a documented one does, canonical keeps both.

fn component_with_props(name: &str, props: Vec<(&str, &str)>) -> caliper_docgen_core::types::ComponentEntry {
    use caliper_docgen_core::types::{ComponentEntry, ParsedProp};
    ComponentEntry {
        display_name: name.to_string(),
        file_path: format!("{name}.tsx").into(),
        description: String::new(),
        props: props
            .into_iter()
            .map(|(prop_name, description)| {
                let prop = ParsedProp::new(
                    prop_name.to_string(),
                    PropType::ReactNode,
                    false,
                    None,
                    description.to_string(),
                    Default::default(),
                    None,
                    vec![],
                );
                (prop_name.to_string(), prop)
            })
            .collect(),
        inheritance: vec![],
        notable_inherited: Default::default(),
        discriminant_prop: None,
        composes: vec![],
        tags: Default::default(),
        methods: vec![],
        detected_by: caliper_docgen_core::types::DetectionRule::FunctionDeclaration,
    }
}

#[test]
fn serialize_rdt_omits_undocumented_children_but_keeps_documented_ones() {
    let mut components = std::collections::BTreeMap::new();
    components.insert("Bare".to_string(), component_with_props("Bare", vec![("children", ""), ("label", "")]));
    components.insert(
        "Documented".to_string(),
        component_with_props("Documented", vec![("children", "Slot content."), ("label", "")]),
    );
    let output = caliper_docgen_core::types::ExtractionOutput {
        components,
        enums: Default::default(),
        diagnostics: vec![],
        stats: Default::default(),
    };

    let parsed: serde_json::Value = serde_json::from_str(&serialize_rdt(&output).unwrap()).unwrap();
    let prop_names = |component: &str| -> Vec<String> {
        let mut names: Vec<String> = parsed[component]["props"].as_object().unwrap().keys().cloned().collect();
        names.sort();
        names
    };
    assert_eq!(prop_names("Bare"), vec!["label".to_string()]);
    assert_eq!(prop_names("Documented"), vec!["children".to_string(), "label".to_string()]);
    assert_eq!(parsed["Documented"]["props"]["children"]["description"], "Slot content.");

    // Canonical output is untouched: the undocumented `children` is still there.
    assert!(output.components["Bare"].props.contains_key("children"));
}

#[test]
fn docgen_info_keeps_undocumented_children_for_storybook() {
    let entry = component_with_props("Bare", vec![("children", ""), ("label", "")]);
    let doc = caliper_docgen_core::pipeline::rdt::docgen_info(&entry);
    let mut names: Vec<&String> = doc["props"].as_object().unwrap().keys().collect();
    names.sort();
    assert_eq!(names, vec!["children", "label"]);
}

/// Runs `extract` end to end over one button component and returns its canonical JSON. Lives inside the crate so
/// `@types/react` resolves for `full` mode.
fn extract_button(mode: crate::HtmlAttributeModeArg) -> serde_json::Value {
    let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(
        tmp.path().join("Button.tsx"),
        r#"
import * as React from "react";
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> { label: string }
export function Button(props: ButtonProps) { return null; }
"#,
    )
    .unwrap();
    let out = tmp.path().join("out.json");
    let args = crate::ExtractArgs {
        src: vec![tmp.path().to_str().unwrap().to_owned()],
        out: Some(out.to_str().unwrap().to_owned()),
        format: crate::OutputFormat::Canonical,
        cache_dir: Some(tmp.path().join("cache").to_str().unwrap().to_owned()),
        html_attributes: Some(mode),
        extra_builtins: vec![],
        json: false,
    };

    assert_eq!(cmd_extract(args, false, None).unwrap(), 0);

    let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    written["components"]["Button"].clone()
}

fn keys(component: &serde_json::Value, field: &str) -> Vec<String> {
    component[field].as_object().unwrap().keys().cloned().collect()
}

#[test]
fn html_attributes_none_extracts_own_props_only() {
    let button = extract_button(crate::HtmlAttributeModeArg::None);
    assert_eq!(keys(&button, "props"), vec!["label"]);
    assert!(keys(&button, "notableInherited").is_empty());
}

#[test]
fn html_attributes_curated_keeps_own_props_and_lists_notable_inherited_ones() {
    let button = extract_button(crate::HtmlAttributeModeArg::Curated);
    assert_eq!(keys(&button, "props"), vec!["label"]);
    assert!(keys(&button, "notableInherited").contains(&"onClick".to_owned()));
}

#[test]
fn html_attributes_full_merges_the_real_button_attributes_into_props() {
    let button = extract_button(crate::HtmlAttributeModeArg::Full);
    let props = keys(&button, "props");
    assert!(["label", "onClick", "disabled"].iter().all(|name| props.contains(&name.to_string())), "{props:?}");
    assert!(keys(&button, "notableInherited").is_empty());
}

// ── SPEC-SERIALIZATION-001 AC-10: end-to-end — a props interface
// extending a string-literal-union type alias routes through
// ResolvedChain::give_up (resolver/alias.rs), producing a composes value
// with each member quote-wrapped by to_raw_string(), and that exact
// quoted value survives into serialize_rdt's output.

#[test]
fn literal_union_extends_base_produces_quoted_composes_end_to_end() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(
        tmp.path().join("Variant.tsx"),
        r#"
type Variant = 'a' | 'b';
interface Props extends Variant {}
export function Comp(props: Props) { return null; }
"#,
    )
    .unwrap();

    let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = caliper_docgen_core::pipeline::PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(camino::Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    let output = caliper_docgen_core::pipeline::extract(&options);

    let comp = output.components.get("Comp").expect("expected a Comp component");
    assert_eq!(
        comp.composes,
        vec!["\"a\" | \"b\"".to_string()],
        "expected quote-wrapped members in composes, got {:?}",
        comp.composes
    );

    let rdt_json = serialize_rdt(&output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rdt_json).unwrap();
    assert_eq!(parsed["Comp"]["composes"], serde_json::json!(["\"a\" | \"b\""]));
}
