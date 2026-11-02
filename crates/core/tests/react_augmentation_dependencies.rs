#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! What a dependency's `declare module 'react'` block adds to HTML components, member by member: the type, description
//! and parent of each, what an augmenting interface inherits, generic parameters, and what TypeScript does not apply.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::ExtractionOutput;
use camino::Utf8PathBuf;
use tempfile::TempDir;

const COMPONENT: &str = r#"
import * as React from 'react';
import type { PackageOptions } from 'augpkg';
export const Box = (props: React.ComponentProps<'div'> & PackageOptions) => <div />;
export const Field = (props: React.ComponentProps<'input'> & PackageOptions) => <input />;
"#;

const PLAIN_TYPES: &str = "export interface PackageOptions { width?: number }\n";

const AUGMENTING_TYPES: &str = r#"
export interface PackageOptions { width?: number }
declare module 'react' {
    interface Inherited {
        /** Comes from the interface the augmentation extends. */
        inherited?: boolean;
    }
    interface HTMLAttributes<T> extends Inherited {
        /** Tailwind classes. */
        tw?: string;
        /** The element this attribute set describes. */
        host?: T;
    }
    export interface DivHTMLAttributes<T> {
        divOnly?: string;
    }
    interface CSSProperties {
        '--brand'?: string;
    }
}
"#;

struct Project {
    _tmp: TempDir,
    root: Utf8PathBuf,
}

fn project(types: &str) -> Project {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    for (name, source) in [
        ("src/Components.tsx", COMPONENT),
        ("node_modules/augpkg/package.json", r#"{"name":"augpkg","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/augpkg/index.d.ts", types),
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    Project { _tmp: tmp, root }
}

fn run(project: &Project) -> ExtractionOutput {
    extract(&PipelineOptions {
        src_dirs: vec![project.root.join("src")],
        cache_dir: Some(project.root.join(".cache")),
        html_attributes: HtmlAttributeMode::Full,
        load_dependencies: true,
        ..Default::default()
    })
}

fn added(augmented: &ExtractionOutput, plain: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    let names = |output: &ExtractionOutput| -> BTreeSet<String> {
        let entry = &output.components[component];
        entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect()
    };
    names(augmented).difference(&names(plain)).cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn outputs() -> (Project, ExtractionOutput, ExtractionOutput) {
    let augmenting = project(AUGMENTING_TYPES);
    let plain = project(PLAIN_TYPES);
    let (augmented, plain) = (run(&augmenting), run(&plain));
    (augmenting, augmented, plain)
}

#[test]
fn a_dependency_augmentation_adds_exactly_the_members_typescript_applies() {
    let (_project, augmented, plain) = outputs();
    // `DivHTMLAttributes` is exported from the block, so TypeScript does not merge it into React's, and `CSSProperties`
    // is not an attribute set of any element.
    for component in ["Box", "Field"] {
        assert_eq!(added(&augmented, &plain, component), set(&["tw", "host", "inherited"]), "{component}");
    }
}

#[test]
fn a_member_has_the_augmenting_declarations_type_description_and_parent() {
    let (project, augmented, _plain) = outputs();
    let tw = &augmented.components["Box"].props["tw"];
    assert_eq!(serde_json::to_value(&tw.prop_type).unwrap(), serde_json::json!({ "kind": "string" }));
    assert_eq!(tw.description, "Tailwind classes.");
    assert!(!tw.required);
    let parent = tw.parent.as_ref().expect("a parent");
    assert_eq!(parent.name, "HTMLAttributes");
    assert_eq!(parent.file_name, project.root.join("node_modules/augpkg/index.d.ts").as_str());
}

#[test]
fn an_augmenting_interface_brings_the_members_of_the_interface_it_extends() {
    let (project, augmented, _plain) = outputs();
    let inherited = &augmented.components["Box"].props["inherited"];
    assert_eq!(serde_json::to_value(&inherited.prop_type).unwrap(), serde_json::json!({ "kind": "boolean" }));
    assert_eq!(inherited.description, "Comes from the interface the augmentation extends.");
    let parent = inherited.parent.as_ref().expect("a parent");
    assert_eq!(parent.name, "Inherited");
    assert_eq!(parent.file_name, project.root.join("node_modules/augpkg/index.d.ts").as_str());
}

#[test]
fn the_augmenting_interfaces_type_parameter_is_the_elements() {
    let (_project, augmented, _plain) = outputs();
    let host = &augmented.components["Box"].props["host"];
    assert_eq!(host.description, "The element this attribute set describes.");
    assert_eq!(host.prop_type.raw_string(), "HTMLDivElement");
    let field = &augmented.components["Field"].props["host"];
    assert_eq!(field.prop_type.raw_string(), "HTMLInputElement");
}
