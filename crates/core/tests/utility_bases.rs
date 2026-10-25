#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Utility types used as a props base, which TypeScript expands and a props table must list: `Pick` of one of React's
//! interfaces (antd's `ProgressAriaProps`), `Record` over a union of literals (antd's `Partial<Record<Breakpoint, …>>`
//! on `Col`), and `PropsWithChildren` with no argument.

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use caliper_docgen_core::types::{DiagnosticSeverity, ExtractionOutput};
use camino::Utf8PathBuf;
use serde_json::{json, Value};
use tempfile::TempDir;

/// Extracts a project of `files`, inside this crate so that `react` resolves to the workspace's `@types/react`.
fn extract_files(files: &[(&str, &str)]) -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    for (name, body) in files {
        std::fs::write(root.join(name), body).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    extract(&options)
}

/// `(name, required, type)` of each prop of the component, by name.
fn props_of(output: &ExtractionOutput, component: &str) -> Vec<(String, bool, Value)> {
    let entry = output.components.get(component).unwrap_or_else(|| panic!("no component {component}"));
    let mut props: Vec<_> = entry
        .props
        .iter()
        .map(|(name, prop)| (name.to_string(), prop.required, serde_json::to_value(&prop.prop_type).unwrap()))
        .collect();
    props.sort_by(|a, b| a.0.cmp(&b.0));
    props
}

fn warnings(output: &ExtractionOutput) -> Vec<String> {
    output.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Warning).map(|d| d.message.clone()).collect()
}

#[test]
fn a_pick_of_a_react_interface_lists_the_picked_members_from_its_declaration() {
    let output = extract_files(&[(
        "Progress.tsx",
        r#"
        import * as React from 'react';
        type AriaProps = Pick<React.AriaAttributes, 'aria-label' | 'aria-labelledby'>;
        interface ProgressProps extends AriaProps { percent?: number }
        export function Progress(props: ProgressProps) { return <div />; }
        "#,
    )]);
    assert_eq!(
        props_of(&output, "Progress"),
        vec![
            ("aria-label".to_owned(), false, json!({ "kind": "string" })),
            ("aria-labelledby".to_owned(), false, json!({ "kind": "string" })),
            ("percent".to_owned(), false, json!({ "kind": "number" })),
        ]
    );
    assert_eq!(warnings(&output), Vec::<String>::new());
}

#[test]
fn a_pick_of_html_attributes_reaches_the_members_its_parent_interfaces_declare() {
    // `className` and `role` are `HTMLAttributes`' own; `onClick` is `DOMAttributes`', one parent up.
    let output = extract_files(&[(
        "Group.tsx",
        r#"
        import * as React from 'react';
        interface GroupProps extends Pick<React.HTMLAttributes<HTMLDivElement>, 'className' | 'onClick' | 'role'> {}
        export function Group(props: GroupProps) { return <div />; }
        "#,
    )]);
    let props = props_of(&output, "Group");
    let names: Vec<_> = props.iter().map(|(name, ..)| name.as_str()).collect();
    assert_eq!(names, ["className", "onClick", "role"]);
    assert_eq!(props[0].2, json!({ "kind": "string" }));
    assert_eq!(
        props[1].2,
        json!({
            "kind": "named",
            "name": "MouseEventHandler",
            "args": [{ "kind": "named", "name": "HTMLDivElement", "args": [] }],
        })
    );
    assert!(props.iter().all(|(_, required, _)| !required));
    assert_eq!(warnings(&output), Vec::<String>::new());
}

#[test]
fn a_pick_of_a_react_alias_is_not_read_as_an_interface() {
    // `ComponentProps<typeof X>` is an alias resolved as X's props; it needs no "not found" note of its own.
    let output = extract_files(&[(
        "Dialog.tsx",
        r#"
        import * as React from 'react';
        function Base(props: { isDismissable?: boolean; label?: string }) { return <div />; }
        interface DialogProps extends Pick<React.ComponentProps<typeof Base>, 'label'> {}
        export function Dialog(props: DialogProps) { return <div />; }
        "#,
    )]);
    assert!(
        !output.diagnostics.iter().any(|d| d.message.contains("is picked from")),
        "diagnostics: {:?}",
        output.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn a_pick_key_that_react_does_not_declare_is_reported_not_dropped_silently() {
    let output = extract_files(&[(
        "Progress.tsx",
        r#"
        import * as React from 'react';
        type AriaProps = Pick<React.AriaAttributes, 'aria-label' | 'aria-nope'>;
        interface ProgressProps extends AriaProps { percent?: number }
        export function Progress(props: ProgressProps) { return <div />; }
        "#,
    )]);
    let names: Vec<_> = props_of(&output, "Progress").into_iter().map(|(name, ..)| name).collect();
    assert_eq!(names, ["aria-label", "percent"]);
    assert!(
        output.diagnostics.iter().any(|d| d.message.contains("'aria-nope' is picked from 'React.AriaAttributes'")),
        "diagnostics: {:?}",
        output.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn a_record_over_an_alias_of_literals_is_one_prop_per_literal() {
    let output = extract_files(&[(
        "Col.tsx",
        r#"
        type Breakpoint = 'xs' | 'sm' | 'md';
        interface ColSize { span?: number }
        interface ColProps extends Partial<Record<Breakpoint, number | ColSize>> { span?: number }
        export function Col(props: ColProps) { return <div />; }
        "#,
    )]);
    let breakpoint = json!({
        "kind": "union",
        "members": [{ "kind": "number" }, { "kind": "named", "name": "ColSize", "args": [] }],
    });
    assert_eq!(
        props_of(&output, "Col"),
        vec![
            ("md".to_owned(), false, breakpoint.clone()),
            ("sm".to_owned(), false, breakpoint.clone()),
            ("span".to_owned(), false, json!({ "kind": "number" })),
            ("xs".to_owned(), false, breakpoint),
        ]
    );
}

#[test]
fn a_record_without_partial_makes_each_literal_required() {
    let output = extract_files(&[(
        "Grid.tsx",
        r#"
        type Side = 'top' | 'left';
        interface GridProps extends Record<Side, boolean> {}
        export function Grid(props: GridProps) { return <div />; }
        "#,
    )]);
    assert_eq!(
        props_of(&output, "Grid"),
        vec![
            ("left".to_owned(), true, json!({ "kind": "boolean" })),
            ("top".to_owned(), true, json!({ "kind": "boolean" })),
        ]
    );
}

#[test]
fn a_record_over_an_as_const_array_in_another_file_names_the_array_values() {
    let output = extract_files(&[
        (
            "responsive.ts",
            r#"export const responsiveArray = ['xl', 'lg'] as const; export type Breakpoint = (typeof responsiveArray)[number];"#,
        ),
        (
            "Col.tsx",
            r#"
            import type { Breakpoint } from './responsive';
            interface ColProps extends Partial<Record<Breakpoint, number>> {}
            export function Col(props: ColProps) { return <div />; }
            "#,
        ),
    ]);
    let names: Vec<_> = props_of(&output, "Col").into_iter().map(|(name, ..)| name).collect();
    assert_eq!(names, ["lg", "xl"]);
}

#[test]
fn a_record_over_string_keys_is_an_index_signature_that_names_no_prop_and_warns_of_nothing() {
    let output = extract_files(&[(
        "Bag.tsx",
        r#"
        interface BagProps extends Record<string, unknown> { label?: string }
        export function Bag(props: BagProps) { return <div />; }
        "#,
    )]);
    assert_eq!(props_of(&output, "Bag"), vec![("label".to_owned(), false, json!({ "kind": "string" }))]);
    assert_eq!(warnings(&output), Vec::<String>::new());
}

#[test]
fn props_with_children_and_no_argument_is_only_the_children() {
    let output = extract_files(&[(
        "Provider.tsx",
        r#"
        import * as React from 'react';
        export const Provider: React.FC<React.PropsWithChildren> = ({ children }) => <div>{children}</div>;
        "#,
    )]);
    assert_eq!(props_of(&output, "Provider"), vec![("children".to_owned(), false, json!({ "kind": "reactNode" }))]);
}
