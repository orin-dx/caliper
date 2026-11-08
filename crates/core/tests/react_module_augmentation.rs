#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `declare module "react" { interface HTMLAttributes<T> { tw?: string } }` merges into React's own interface, so the
//! attribute reaches every HTML-based component when the augmenting file is in the loaded set: a project file, or a
//! dependency declaration the loader reached. `curated` and `none` treat it as any other attribute.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession, extract};
use caliper_docgen_core::types::ExtractionOutput;
use camino::Utf8PathBuf;
use tempfile::TempDir;

const COMPONENT: &str = r#"
import * as React from 'react';
export const Box = (props: React.ComponentProps<'div'> & { own?: boolean }) => <div />;
export const Plain = (props: React.HTMLAttributes<HTMLDivElement>) => <div />;
"#;

const AUGMENTATION: &str = r#"
import type {} from 'react';
declare module 'react' {
    interface HTMLAttributes<T> {
        /** Tailwind classes. */
        tw?: string;
        'data-weight'?: number;
    }
}
"#;

/// A package whose declaring file also augments React, the way a bundled types file does.
const PACKAGE_TYPES: &str = r#"
export interface PackageOptions { width?: number }
declare module 'react' {
    interface HTMLAttributes<T> { tw?: string }
}
"#;

const DEPENDENT: &str = r#"
import * as React from 'react';
import type { PackageOptions } from 'augpkg';
export const Box = (props: React.ComponentProps<'div'> & PackageOptions) => <div />;
"#;

struct Project {
    _tmp: TempDir,
    root: Utf8PathBuf,
}

fn project(files: &[(&str, &str)]) -> Project {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    for (name, source) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    Project { _tmp: tmp, root }
}

fn options(project: &Project, mode: HtmlAttributeMode, load_dependencies: bool) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![project.root.join("src")],
        cache_dir: Some(project.root.join(".cache")),
        html_attributes: mode,
        load_dependencies,
        ..Default::default()
    }
}

fn listed(output: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    let entry = output.components.get(component).unwrap_or_else(|| panic!("no component {component}"));
    entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect()
}

fn added(project: &Project, bare: &Project, mode: HtmlAttributeMode, component: &str) -> BTreeSet<String> {
    let with = listed(&extract(&options(project, mode, false)), component);
    let without = listed(&extract(&options(bare, mode, false)), component);
    with.difference(&without).cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn local() -> (Project, Project) {
    let with = project(&[("src/Components.tsx", COMPONENT), ("src/augment.d.ts", AUGMENTATION)]);
    let bare = project(&[("src/Components.tsx", COMPONENT)]);
    (with, bare)
}

#[test]
fn a_project_augmentation_adds_its_members_in_full_mode_to_every_html_component() {
    let (with, bare) = local();
    for component in ["Box", "Plain"] {
        assert_eq!(added(&with, &bare, HtmlAttributeMode::Full, component), set(&["tw", "data-weight"]), "{component}");
    }
}

#[test]
fn the_augmented_member_is_typed_and_attributed_as_the_augmenting_declaration() {
    let (with, _bare) = local();
    let output = extract(&options(&with, HtmlAttributeMode::Full, false));
    let tw = &output.components["Box"].props["tw"];
    assert_eq!(serde_json::to_value(&tw.prop_type).unwrap(), serde_json::json!({ "kind": "string" }));
    assert_eq!(tw.description, "Tailwind classes.");
    let parent = tw.parent.as_ref().expect("a parent");
    assert_eq!(parent.name, "HTMLAttributes");
    assert!(parent.file_name.ends_with("src/augment.d.ts"), "{}", parent.file_name);
    assert!(!tw.required);
}

#[test]
fn curated_and_none_treat_an_augmented_member_as_any_other_attribute() {
    let (with, bare) = local();
    // Not in the curated table, so left out as every other uncurated attribute is.
    assert_eq!(added(&with, &bare, HtmlAttributeMode::Curated, "Box"), set(&[]));
    assert_eq!(added(&with, &bare, HtmlAttributeMode::None, "Box"), set(&[]));
    assert_eq!(listed(&extract(&options(&with, HtmlAttributeMode::None, false)), "Box"), set(&["own"]));
}

#[test]
fn without_the_augmenting_file_nothing_is_added() {
    let (_with, bare) = local();
    let names = listed(&extract(&options(&bare, HtmlAttributeMode::Full, false)), "Box");
    assert!(!names.contains("tw") && !names.contains("data-weight"));
    assert!(names.contains("className") && names.contains("own"));
}

#[test]
fn an_augmentation_of_another_module_is_not_react() {
    let other = project(&[
        ("src/Components.tsx", COMPONENT),
        (
            "src/augment.d.ts",
            "import type {} from 'react';\ndeclare module 'other' { interface HTMLAttributes<T> { tw?: string } }\n",
        ),
    ]);
    let names = listed(&extract(&options(&other, HtmlAttributeMode::Full, false)), "Box");
    assert!(!names.contains("tw"));
}

fn dependent(with_package: bool) -> Project {
    let mut files = vec![
        ("src/Components.tsx", DEPENDENT),
        ("node_modules/augpkg/package.json", r#"{"name":"augpkg","version":"1.0.0","types":"index.d.ts"}"#),
    ];
    if with_package {
        files.push(("node_modules/augpkg/index.d.ts", PACKAGE_TYPES));
    } else {
        files.push(("node_modules/augpkg/index.d.ts", "export interface PackageOptions { width?: number }\n"));
    }
    project(&files)
}

#[test]
fn a_dependency_augmentation_applies_only_when_its_file_is_loaded() {
    let augmenting = dependent(true);
    let plain = dependent(false);
    let loaded = listed(&extract(&options(&augmenting, HtmlAttributeMode::Full, true)), "Box");
    let not_loaded = listed(&extract(&options(&augmenting, HtmlAttributeMode::Full, false)), "Box");
    let unrelated = listed(&extract(&options(&plain, HtmlAttributeMode::Full, true)), "Box");
    assert!(loaded.contains("tw"), "the loaded declaration augments React");
    assert!(!not_loaded.contains("tw"), "dependencies are not loaded");
    assert!(!unrelated.contains("tw"), "a loaded file that does not augment React adds nothing");
    assert_eq!(loaded.difference(&unrelated).cloned().collect::<BTreeSet<_>>(), set(&["tw"]));
}

#[test]
fn a_lazy_session_answers_an_augmented_component_as_a_batch_run_does() {
    let (local_project, _bare) = local();
    let package = dependent(true);
    for (project, load_dependencies) in [(&local_project, false), (&package, true)] {
        let options = options(project, HtmlAttributeMode::Full, load_dependencies);
        let batch = extract(&options);
        assert!(batch.components["Box"].props.contains_key("tw"));
        let session = WatchSession::lazy(options);
        drop(session.initialize());
        let answer = session.component("Box").expect("an answer");
        assert_eq!(answer.entry, batch.components["Box"], "load_dependencies: {load_dependencies}");
    }
}

/// The shape of a bundled package: the entry re-exports a type from the file that augments React, and the project
/// imports only a value from the entry.
fn reaching() -> Project {
    project(&[
        (
            "src/Components.tsx",
            "import * as React from 'react';\nimport { make } from 'reexp';\nexport const Box = (props: React.ComponentProps<'div'>) => <div onClick={make} />;\n",
        ),
        ("node_modules/reexp/package.json", r#"{"name":"reexp","version":"1.0.0","types":"index.d.ts"}"#),
        (
            "node_modules/reexp/index.d.ts",
            "export declare const make: () => void;\nexport type { Options } from './types.js';\n",
        ),
        ("node_modules/reexp/types.d.ts", PACKAGE_TYPES),
    ])
}

#[test]
fn a_module_the_project_imports_augments_react_through_its_re_exports_even_for_a_value_import() {
    let reaching = reaching();
    let loaded = listed(&extract(&options(&reaching, HtmlAttributeMode::Full, true)), "Box");
    let not_loaded = listed(&extract(&options(&reaching, HtmlAttributeMode::Full, false)), "Box");
    assert_eq!(loaded.difference(&not_loaded).cloned().collect::<BTreeSet<_>>(), set(&["tw"]));
}

#[test]
fn a_lazy_session_loads_what_an_import_elsewhere_in_the_project_augments() {
    let reaching = reaching();
    std::fs::write(
        reaching.root.join("src/Other.tsx"),
        "import * as React from 'react';\nexport const Other = (props: React.ComponentProps<'span'>) => <span />;\n",
    )
    .unwrap();
    let options = options(&reaching, HtmlAttributeMode::Full, true);
    let batch = extract(&options);
    let session = WatchSession::lazy(options);
    drop(session.initialize());
    // `Other.tsx` imports nothing from the package, yet TypeScript's program, and so the batch run, includes it.
    assert!(batch.components["Other"].props.contains_key("tw"));
    assert_eq!(session.component("Other").expect("an answer").entry, batch.components["Other"]);
}
