use super::*;
use caliper_docgen_core::pipeline::rdt::type_json as rdt_type_json;

fn args_for(src: &str, json: bool) -> crate::ExtractArgs {
    crate::ExtractArgs {
        src: vec![src.to_owned()],
        out: None,
        format: crate::OutputFormat::Canonical,
        cache_dir: None,
        no_cache: false,
        html_attributes: None,
        extra_builtins: vec![],
        include_unexported: false,
        json,
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
        no_checker: true,
        tsc_path: None,
        semantic_timeout_ms: crate::DEFAULT_SEMANTIC_TIMEOUT_MS,
        no_resolver_facts: false,
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
export type Weird<T> = T extends { kind: infer K } ? { a: K } : { b: number };
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

    let rdt_json = serialize_rdt(&output, false).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rdt_json).unwrap();
    let composes = parsed["Comp"]["composes"].as_array().expect("expected a composes array in RDT output");
    assert!(!composes.is_empty(), "expected the unresolvable Weird<'x'> member to appear in RDT's composes field");
}

#[test]
fn rdt_output_gives_each_component_its_absolute_file_path() {
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
    std::fs::write(tmp.path().join("Badge.tsx"), "export function Badge(props: { tone: string }) { return null; }")
        .unwrap();
    let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = caliper_docgen_core::pipeline::PipelineOptions {
        src_dirs: vec![dir.clone()],
        cache_dir: Some(dir.join("cache")),
        ..Default::default()
    };

    let output = caliper_docgen_core::pipeline::extract(&options);
    let parsed: serde_json::Value = serde_json::from_str(&serialize_rdt(&output, false).unwrap()).unwrap();
    assert!(dir.is_absolute());
    assert_eq!(parsed["Badge"]["filePath"], dir.join("Badge.tsx").as_str());
}

// ── SPEC-SERIALIZATION-001 AC-1/AC-2/AC-3/AC-4: rdt_type_json had zero
// direct tests — it's private and was only ever exercised indirectly.

use caliper_docgen_core::types::PropType;

#[test]
fn rdt_type_json_empty_unions_never_produce_enum_shape() {
    assert_eq!(rdt_type_json(&PropType::Union(vec![])), serde_json::json!({"name": ""}));
    assert_eq!(
        rdt_type_json(&PropType::LiteralUnion { members: vec![], has_default: false }),
        serde_json::json!({"name": ""})
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
        exported: true,
        exports: Vec::new(),
    };
    let mut components = std::collections::BTreeMap::new();
    components.insert("Widget".to_string(), entry);
    let output = caliper_docgen_core::types::ExtractionOutput {
        components,
        enums: Default::default(),
        diagnostics: vec![],
        stats: Default::default(),
    };

    let rdt_json = serialize_rdt(&output, false).unwrap();
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
                (prop_name.to_string(), std::sync::Arc::new(prop))
            })
            .collect(),
        inheritance: vec![],
        notable_inherited: Default::default(),
        discriminant_prop: None,
        composes: vec![],
        tags: Default::default(),
        methods: vec![],
        detected_by: caliper_docgen_core::types::DetectionRule::FunctionDeclaration,
        exported: true,
        exports: Vec::new(),
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

    let parsed: serde_json::Value = serde_json::from_str(&serialize_rdt(&output, false).unwrap()).unwrap();
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

#[test]
fn rdt_omits_unexported_components_unless_asked() {
    let mut components = std::collections::BTreeMap::new();
    components.insert("Public".to_string(), component_with_props("Public", vec![]));
    components.insert(
        "Internal".to_string(),
        caliper_docgen_core::types::ComponentEntry { exported: false, ..component_with_props("Internal", vec![]) },
    );
    let output = caliper_docgen_core::types::ExtractionOutput {
        components,
        enums: Default::default(),
        diagnostics: vec![],
        stats: Default::default(),
    };
    let names = |json: String| -> Vec<String> {
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        parsed.as_object().unwrap().keys().cloned().collect()
    };

    assert_eq!(names(serialize_rdt(&output, false).unwrap()), vec!["Public"]);
    assert_eq!(names(serialize_rdt(&output, true).unwrap()), vec!["Internal", "Public"]);
    let canonical: serde_json::Value = serde_json::to_value(&output).unwrap();
    assert_eq!(canonical["components"]["Internal"]["exported"], false);
    assert_eq!(canonical["components"]["Public"]["exported"], true);
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
        no_cache: false,
        html_attributes: Some(mode),
        extra_builtins: vec![],
        include_unexported: false,
        json: false,
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
        no_checker: true,
        tsc_path: None,
        semantic_timeout_ms: crate::DEFAULT_SEMANTIC_TIMEOUT_MS,
        no_resolver_facts: false,
    };

    assert_eq!(cmd_extract(args, false, None).unwrap(), 0);

    let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    // Name to prop id, merged from the prop sets the component lists.
    let mut button = written["components"]["Button"].clone();
    let mut props = serde_json::Map::new();
    for set in button["props"].as_array().unwrap() {
        props.extend(written["propSets"][set.as_str().unwrap()].as_object().unwrap().clone());
    }
    button["props"] = serde_json::Value::Object(props);
    button
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

    let rdt_json = serialize_rdt(&output, false).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rdt_json).unwrap();
    assert_eq!(parsed["Comp"]["composes"], serde_json::json!(["\"a\" | \"b\""]));
}

/// `extract` over one component, with the checker pointed at a `tsc` that is not there, or with `--no-checker`.
fn extract_with_semantic_flag(tmp: &tempfile::TempDir, semantic: bool) -> serde_json::Value {
    std::fs::write(tmp.path().join("Plain.tsx"), "export function Plain(props: { a: string }) { return null; }\n")
        .unwrap();
    let out = tmp.path().join(format!("out-{semantic}.json"));
    let args = crate::ExtractArgs {
        src: vec![tmp.path().to_str().unwrap().to_owned()],
        out: Some(out.to_str().unwrap().to_owned()),
        format: crate::OutputFormat::Canonical,
        cache_dir: Some(tmp.path().join("cache").to_str().unwrap().to_owned()),
        no_cache: false,
        html_attributes: None,
        extra_builtins: vec![],
        include_unexported: false,
        json: false,
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
        no_checker: !semantic,
        tsc_path: semantic.then(|| tmp.path().join("no-such-tsc").to_str().unwrap().to_owned()),
        semantic_timeout_ms: crate::DEFAULT_SEMANTIC_TIMEOUT_MS,
        no_resolver_facts: false,
    };

    assert_eq!(cmd_extract(args, true, None).unwrap(), 0);

    serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap()
}

#[test]
fn the_no_checker_output_has_no_semantic_backend_diagnostics() {
    let output = extract_with_semantic_flag(&tempfile::TempDir::new().unwrap(), false);

    let codes: Vec<_> = output["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].clone()).collect();
    assert!(!codes.contains(&serde_json::json!("SEMANTIC_BACKEND")), "{codes:?}");
}

#[test]
fn an_unavailable_checker_adds_one_warning_and_leaves_every_component_as_it_was() {
    let tmp = tempfile::TempDir::new().unwrap();
    let plain = extract_with_semantic_flag(&tmp, false);
    let flagged = extract_with_semantic_flag(&tmp, true);

    assert_eq!(flagged["components"], plain["components"]);
    assert_eq!(flagged["enums"], plain["enums"]);
    let added: Vec<_> = flagged["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| !plain["diagnostics"].as_array().unwrap().contains(d))
        .collect();
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!(added[0]["code"], "SEMANTIC_BACKEND");
    assert_eq!(added[0]["severity"], "warning");
    assert!(added[0]["message"].as_str().unwrap().contains("no tsc binary at"), "{added:?}");
}

/// A checker that never answers is waited for only `--semantic-timeout-ms`, then the run takes the structural result
/// with one warning that says it timed out.
#[cfg(unix)]
#[test]
fn a_checker_that_never_answers_is_given_up_after_the_semantic_timeout() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("Plain.tsx"), "export function Plain(props: { a: string }) { return null; }\n")
        .unwrap();
    // A supported version whose binary starts and then says nothing.
    let package = tmp.path().join("checker/node_modules/typescript");
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::write(package.join("package.json"), r#"{"version": "7.1.0-beta"}"#).unwrap();
    let tsc = package.join("bin/tsc");
    std::fs::write(&tsc, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&tsc, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = tmp.path().join("out.json");
    let args = crate::ExtractArgs {
        src: vec![tmp.path().to_str().unwrap().to_owned()],
        out: Some(out.to_str().unwrap().to_owned()),
        format: crate::OutputFormat::Canonical,
        cache_dir: None,
        no_cache: true,
        html_attributes: None,
        extra_builtins: vec![],
        include_unexported: false,
        json: false,
        load_dependencies: false,
        no_load_dependencies: false,
        collapse_inherited_props: None,
        no_checker: false,
        tsc_path: Some(tsc.to_str().unwrap().to_owned()),
        semantic_timeout_ms: 300,
        no_resolver_facts: false,
    };

    let started = std::time::Instant::now();
    assert_eq!(cmd_extract(args, true, None).unwrap(), 0);

    assert!(started.elapsed() < std::time::Duration::from_secs(10), "waited {:?}", started.elapsed());
    let output: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    let warnings: Vec<&str> = output["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "SEMANTIC_BACKEND")
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("timed out"), "{warnings:?}");
}

#[test]
fn the_checker_is_on_by_default_with_a_ten_second_timeout() {
    use clap::Parser;

    let parsed = crate::Cli::try_parse_from(["caliper-docgen", "extract"]).unwrap();
    let crate::Command::Extract(args) = parsed.command else { panic!("not extract") };
    assert!(!args.no_checker);
    assert_eq!(args.semantic_timeout_ms, 10_000);

    for flag in [&["--semantic-timeout-ms", "500"][..], &["--tsc-path", "/opt/tsc"], &["--no-resolver-facts"]] {
        let refused = crate::Cli::try_parse_from([&["caliper-docgen", "extract", "--no-checker"][..], flag].concat());
        assert!(refused.is_err(), "{flag:?} with --no-checker is refused");
    }
    let removed = crate::Cli::try_parse_from(["caliper-docgen", "extract", "--experimental-semantic-types"]);
    assert!(removed.is_err(), "the old opt-in flag is gone");
}

// The RDT-shaped projection Storybook's `extractArgTypes` turns into controls.

#[test]
fn rdt_type_json_single_literal_is_a_one_option_enum() {
    assert_eq!(
        rdt_type_json(&PropType::StringLiteral("button".into())),
        serde_json::json!({"name": "enum", "raw": "\"button\"", "value": [{"value": "\"button\""}]})
    );
    assert_eq!(
        rdt_type_json(&PropType::NumberLiteral(1.0)),
        serde_json::json!({"name": "enum", "raw": "1", "value": [{"value": "1"}]})
    );
    assert_eq!(
        rdt_type_json(&PropType::Union(vec![PropType::StringLiteral("x".into())])),
        serde_json::json!({"name": "enum", "raw": "\"x\"", "value": [{"value": "\"x\""}]})
    );
    assert_eq!(
        rdt_type_json(&PropType::LiteralUnion { members: vec!["only".into()], has_default: false }),
        serde_json::json!({"name": "enum", "raw": "\"only\"", "value": [{"value": "\"only\""}]})
    );
}

#[test]
fn rdt_type_json_leaves_empty_unions_a_bare_name_and_a_lone_boolean_literal_a_type_name() {
    assert_eq!(rdt_type_json(&PropType::Union(vec![])), serde_json::json!({"name": ""}));
    assert_eq!(
        rdt_type_json(&PropType::LiteralUnion { members: vec![], has_default: false }),
        serde_json::json!({"name": ""})
    );
    // Storybook's own docgen gives a lone `true` no options either.
    assert_eq!(rdt_type_json(&PropType::BoolLiteral(true)), serde_json::json!({"name": "true"}));
}

#[test]
fn rdt_type_json_boolean_in_a_mixed_literal_union_is_never_a_string_option() {
    let mixed = PropType::Union(vec![
        PropType::StringLiteral("a".into()),
        PropType::NumberLiteral(1.0),
        PropType::BoolLiteral(true),
    ]);
    assert_eq!(rdt_type_json(&mixed), serde_json::json!({"name": "\"a\" | 1 | true"}));
    // Without a boolean the shape is the one the string and number options always had.
    let plain = PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::NumberLiteral(1.0)]);
    assert_eq!(
        rdt_type_json(&plain),
        serde_json::json!({"name": "enum", "raw": "\"a\" | 1", "value": [{"value": "\"a\""}, {"value": "1"}]})
    );
}

/// Storybook splits a type name at `|` and `JSON.parse`s each part. Run through its real `extractArgTypes`, the type
/// names below made `value?: 'a|b' | true` an `object` control and turned `'a\\b'` into a backspace, so a union whose
/// text does not survive that is an enum, whose options keep the literals ("true" then is the string).
#[test]
fn rdt_type_json_boolean_union_with_text_storybook_would_split_or_misparse_stays_an_enum() {
    let with_bool = |literal: PropType, last: PropType| PropType::Union(vec![literal, last]);
    assert_eq!(
        rdt_type_json(&with_bool(PropType::StringLiteral("a|b".into()), PropType::BoolLiteral(true))),
        serde_json::json!({"name": "enum", "raw": "\"a|b\" | true", "value": [{"value": "\"a|b\""}, {"value": "true"}]})
    );
    assert_eq!(
        rdt_type_json(&with_bool(PropType::StringLiteral("a\"b".into()), PropType::BoolLiteral(true))),
        serde_json::json!({"name": "enum", "raw": "\"a\"b\" | true", "value": [{"value": "\"a\"b\""}, {"value": "true"}]})
    );
    let backslash = PropType::Union(vec![
        PropType::StringLiteral("a\\b".into()),
        PropType::NumberLiteral(1.0),
        PropType::BoolLiteral(false),
    ]);
    assert_eq!(
        rdt_type_json(&backslash),
        serde_json::json!({"name": "enum", "raw": "\"a\\b\" | 1 | false", "value": [{"value": "\"a\\b\""}, {"value": "1"}, {"value": "false"}]})
    );
    // Guards: safe text keeps the type name (a real boolean option); no boolean keeps the enum.
    let safe = PropType::Union(vec![
        PropType::StringLiteral("a".into()),
        PropType::NumberLiteral(1.0),
        PropType::BoolLiteral(true),
    ]);
    assert_eq!(rdt_type_json(&safe), serde_json::json!({"name": "\"a\" | 1 | true"}));
    let no_bool = with_bool(PropType::StringLiteral("a|b".into()), PropType::StringLiteral("c".into()));
    assert_eq!(
        rdt_type_json(&no_bool),
        serde_json::json!({"name": "enum", "raw": "\"a|b\" | \"c\"", "value": [{"value": "\"a|b\""}, {"value": "\"c\""}]})
    );
}

#[test]
fn rdt_type_json_non_literal_unions_and_ordinary_types_are_unchanged() {
    assert_eq!(rdt_type_json(&PropType::String), serde_json::json!({"name": "string"}));
    let open = PropType::Union(vec![
        PropType::StringLiteral("a".into()),
        PropType::StringLiteral("b".into()),
        PropType::String,
    ]);
    assert_eq!(rdt_type_json(&open), serde_json::json!({"name": "\"a\" | \"b\" | string"}));
    let nullable =
        PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::StringLiteral("b".into()), PropType::Null]);
    assert_eq!(rdt_type_json(&nullable), serde_json::json!({"name": "\"a\" | \"b\" | null"}));
}

/// The probe rows of the Storybook output review.
const CONTROL_PROBE: &str = r#"
import * as React from "react";
type Size = 'sm' | 'md' | 'lg';
export interface ProbeProps {
  /** single literal */ kind?: 'button';
  /** single number */ one?: 1;
  /** mixed literal union */ mixed?: 'a' | 1 | true;
  /** literal union */ size?: Size;
  /** open union */ loose?: 'a' | 'b' | string;
  /** plain */ label: string;
  /** @ignore */ hidden?: string;
  /** @internal */ internal?: string;
  /** @deprecated use size */ old?: string;
}
export function Probe(props: ProbeProps) { return null; }
"#;

fn extract_control_probe() -> caliper_docgen_core::types::ExtractionOutput {
    let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(tmp.path().join("Probe.tsx"), CONTROL_PROBE).unwrap();
    let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    let options = caliper_docgen_core::pipeline::PipelineOptions {
        src_dirs: vec![dir],
        cache_dir: Some(camino::Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        ..Default::default()
    };
    caliper_docgen_core::pipeline::extract(&options)
}

#[test]
fn an_ignore_tagged_prop_has_no_rdt_entry_and_other_tags_keep_theirs() {
    let output = extract_control_probe();
    let doc = caliper_docgen_core::pipeline::rdt::docgen_info(&output.components["Probe"]);
    let props = doc["props"].as_object().unwrap();
    assert!(!props.contains_key("hidden"), "{:?}", props.keys().collect::<Vec<_>>());
    assert!(props.contains_key("internal") && props.contains_key("old"));
    let rdt: serde_json::Value = serde_json::from_str(&serialize_rdt(&output, false).unwrap()).unwrap();
    assert!(!rdt["Probe"]["props"].as_object().unwrap().contains_key("hidden"));
}

#[test]
fn the_storybook_docgen_info_projects_the_probe_rows_the_way_storybook_reads_them() {
    let output = extract_control_probe();
    let entry = output.components.get("Probe").expect("a Probe component");
    let doc = caliper_docgen_core::pipeline::rdt::docgen_info(entry);
    let ty = |prop: &str| doc["props"][prop]["type"].clone();

    assert_eq!(
        ty("kind"),
        serde_json::json!({"name": "enum", "raw": "\"button\"", "value": [{"value": "\"button\""}]})
    );
    assert_eq!(ty("one"), serde_json::json!({"name": "enum", "raw": "1", "value": [{"value": "1"}]}));
    assert_eq!(ty("mixed"), serde_json::json!({"name": "\"a\" | 1 | true"}));
    assert_eq!(
        ty("size"),
        serde_json::json!({
            "name": "enum",
            "raw": "\"sm\" | \"md\" | \"lg\"",
            "value": [{"value": "\"sm\""}, {"value": "\"md\""}, {"value": "\"lg\""}]
        })
    );
    assert_eq!(ty("loose"), serde_json::json!({"name": "\"a\" | \"b\" | string"}));
    assert_eq!(ty("label"), serde_json::json!({"name": "string"}));

    // Storybook's docgen drops `@ignore` and nothing else.
    let mut names: Vec<&str> = doc["props"].as_object().unwrap().keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["internal", "kind", "label", "loose", "mixed", "old", "one", "size"]);

    // `--format rdt` projects the same props.
    let rdt: serde_json::Value = serde_json::from_str(&serialize_rdt(&output, false).unwrap()).unwrap();
    assert!(rdt["Probe"]["props"].get("hidden").is_none());
    assert_eq!(rdt["Probe"]["props"]["kind"]["type"], ty("kind"));

    // The canonical output keeps the ignored prop and its tag.
    assert_eq!(output.components["Probe"].props["hidden"].tags.get("ignore").map(String::as_str), Some(""));
}
