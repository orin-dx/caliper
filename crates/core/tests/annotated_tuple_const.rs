#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `(typeof X)[number]` over `declare const X: readonly ["a", "b"]` is `"a" | "b"`, the same as over
//! `const X = ["a", "b"] as const`. Radix declares `Align` and `Side` this way in its `.d.ts` files, and every shadcn
//! popover, tooltip and hover card takes them as props.

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// The `align` prop's type of the only component.
fn align_of(files: &[(&str, &str)]) -> Value {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (name, body) in files {
        std::fs::write(root.join(name), body).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output.components.get("Content").expect("Content");
    serde_json::to_value(&entry.props["align"].prop_type).unwrap()
}

fn literals(members: &[&str]) -> Value {
    json!({ "kind": "literalUnion", "members": members, "hasDefault": false })
}

const CONTENT: &str = r#"
    import type { Align } from "./popper";
    export function Content(props: { align?: Align }) { return <div />; }
"#;

#[test]
fn a_readonly_tuple_annotation_gives_the_union_of_its_elements() {
    let align = align_of(&[
        (
            "popper.ts",
            r#"
                declare const ALIGN_OPTIONS: readonly ["start", "center", "end"];
                export type Align = (typeof ALIGN_OPTIONS)[number];
            "#,
        ),
        ("content.tsx", CONTENT),
    ]);
    assert_eq!(align, literals(&["start", "center", "end"]));
}

#[test]
fn a_mutable_tuple_annotation_gives_the_same_union() {
    let align = align_of(&[
        (
            "popper.ts",
            r#"
                declare const ALIGN_OPTIONS: ["start", "end"];
                export type Align = (typeof ALIGN_OPTIONS)[number];
            "#,
        ),
        ("content.tsx", CONTENT),
    ]);
    assert_eq!(align, literals(&["start", "end"]));
}

#[test]
fn an_annotated_tuple_agrees_with_the_as_const_array_of_the_same_values() {
    let annotated = align_of(&[
        (
            "popper.ts",
            r#"
                declare const ALIGN_OPTIONS: readonly ["start", "center", "end"];
                export type Align = (typeof ALIGN_OPTIONS)[number];
            "#,
        ),
        ("content.tsx", CONTENT),
    ]);
    let initialized = align_of(&[
        (
            "popper.ts",
            r#"
                export const ALIGN_OPTIONS = ["start", "center", "end"] as const;
                export type Align = (typeof ALIGN_OPTIONS)[number];
            "#,
        ),
        ("content.tsx", CONTENT),
    ]);
    assert_eq!(annotated, initialized);
}

#[test]
fn a_tuple_with_a_non_literal_element_is_not_a_literal_union() {
    let align = align_of(&[
        (
            "popper.ts",
            r#"
                declare const ALIGN_OPTIONS: readonly ["start", string];
                export type Align = (typeof ALIGN_OPTIONS)[number];
            "#,
        ),
        ("content.tsx", CONTENT),
    ]);
    assert_ne!(align, literals(&["start"]), "a `string` element must not be dropped");
}

/// The `align` prop of `Content`, whose props are an interface a dependency declares and whose `Align` is declared, not
/// exported, beside the `declare const` it reads (react-popper's `index.d.mts`).
fn align_through_a_dependency(package_dts: &str) -> Value {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    let package = root.join("node_modules").join("popper");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.json"), r#"{ "name": "popper", "version": "1.0.0", "types": "index.d.ts" }"#)
        .unwrap();
    std::fs::write(package.join("index.d.ts"), package_dts).unwrap();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("content.tsx"),
        r#"
            import type { PopperContentProps } from "popper";
            export function Content(props: PopperContentProps) { return <div />; }
        "#,
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![src],
        cache_dir: Some(root.join(".cache")),
        load_dependencies: true,
        ..Default::default()
    };
    let output = extract(&options);
    let entry = output.components.get("Content").expect("Content");
    serde_json::to_value(&entry.props["align"].prop_type).unwrap()
}

#[test]
fn an_alias_over_a_tuple_a_dependency_declares_expands_in_that_dependency() {
    let align = align_through_a_dependency(
        r#"
            declare const ALIGN_OPTIONS: readonly ["start", "center", "end"];
            type Align = (typeof ALIGN_OPTIONS)[number];
            interface PopperContentProps { align?: Align }
            export { ALIGN_OPTIONS, type PopperContentProps };
        "#,
    );
    assert_eq!(align, literals(&["start", "center", "end"]));
}

#[test]
fn an_array_annotation_has_no_members_to_list() {
    let align = align_of(&[
        (
            "popper.ts",
            r#"
                declare const ALIGN_OPTIONS: readonly string[];
                export type Align = (typeof ALIGN_OPTIONS)[number];
            "#,
        ),
        ("content.tsx", CONTENT),
    ]);
    assert_ne!(align["kind"], "literalUnion");
}
