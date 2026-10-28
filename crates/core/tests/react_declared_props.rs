#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A prop `@types/react` declares is never silently left out of a props table: every DOM event handler (the `*Capture`
//! twins only in `full`), `aria-*`, `id`, `className` and `style` reach the output however the props type reaches
//! React's attributes, and `ref` where the component takes one. `key` is React's own and stays out.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const SOURCE: &str = r#"
import * as React from 'react';

export const OmitDiv = (props: Omit<React.HTMLAttributes<HTMLDivElement>, 'title'> & { open?: boolean }) => <div />;
export const PickDiv = (props: Pick<React.HTMLAttributes<HTMLDivElement>, 'onClick' | 'id'>) => <div />;
export const CompProps = (props: React.ComponentProps<'div'>) => <div />;
export const NoRef = (props: React.PropsWithoutRef<React.ComponentProps<'button'>>) => <button />;
export interface InheritedProps extends React.HTMLAttributes<HTMLDivElement> { own?: boolean }
export const Inherited = (props: InheritedProps) => <div />;
export const Beside = (props: React.ComponentProps<'div'> & { children?: React.ReactNode; id?: string | number }) => <div />;
export const Fwd = React.forwardRef<HTMLDivElement, { label?: string }>(function Fwd(props, ref) {
    return <div ref={ref}>{props.label}</div>;
});
"#;

/// Every prop name the component lists: its own and inherited ones, and the curated attributes kept apart from them.
fn listed(mode: HtmlAttributeMode, component: &str) -> BTreeSet<String> {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join("Components.tsx"), SOURCE).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: mode,
        ..Default::default()
    };
    let output = extract(&options);
    let entry = output.components.get(component).unwrap_or_else(|| panic!("no component {component}"));
    entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect()
}

fn handlers(names: &BTreeSet<String>) -> BTreeSet<String> {
    names.iter().filter(|name| name.starts_with("on") && name[2..].starts_with(char::is_uppercase)).cloned().collect()
}

fn without_capture(names: BTreeSet<String>) -> BTreeSet<String> {
    names.into_iter().filter(|name| !name.ends_with("Capture")).collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_curated_table_lists_every_handler_but_the_capture_twins_that_full_adds() {
    for component in ["OmitDiv", "CompProps", "NoRef", "Inherited"] {
        let full = handlers(&listed(HtmlAttributeMode::Full, component));
        let curated = handlers(&listed(HtmlAttributeMode::Curated, component));
        assert!(curated.contains("onClick") && curated.contains("onFocus") && curated.contains("onMouseEnter"));
        assert!(full.iter().any(|name| name.ends_with("Capture")), "{component}: full lists the twins");
        assert_eq!(curated, without_capture(full), "{component}");
        assert_eq!(handlers(&listed(HtmlAttributeMode::None, component)), set(&[]), "{component}: none lists none");
    }
}

#[test]
fn an_omit_over_html_attributes_lists_the_rest_and_its_own_props() {
    for mode in [HtmlAttributeMode::Curated, HtmlAttributeMode::Full] {
        let names = listed(mode, "OmitDiv");
        assert!(names.is_superset(&set(&["open", "id", "className", "style", "aria-label", "onClick"])), "{mode:?}");
        assert!(!names.contains("title"), "{mode:?} omitted title");
        assert!(!names.contains("ref") && !names.contains("key"), "{mode:?}");
    }
    assert_eq!(listed(HtmlAttributeMode::None, "OmitDiv"), set(&["open"]));
}

#[test]
fn a_pick_over_html_attributes_lists_only_what_it_picks() {
    // `Full` and `None` list none of them, as before this change; a follow-up.
    assert_eq!(listed(HtmlAttributeMode::Curated, "PickDiv"), set(&["id", "onClick"]));
}

#[test]
fn component_props_of_an_element_list_its_attributes_and_ref_but_not_key() {
    for mode in [HtmlAttributeMode::Curated, HtmlAttributeMode::Full] {
        let names = listed(mode, "CompProps");
        assert!(names.is_superset(&set(&["ref", "id", "className", "style", "onClick", "aria-label"])), "{mode:?}");
        assert!(!names.contains("key"), "{mode:?}");
    }
    assert_eq!(listed(HtmlAttributeMode::None, "CompProps"), set(&[]));
}

#[test]
fn props_without_ref_list_no_ref() {
    for mode in [HtmlAttributeMode::Curated, HtmlAttributeMode::Full] {
        let names = listed(mode, "NoRef");
        assert!(names.is_superset(&set(&["id", "onClick", "disabled", "type"])), "{mode:?}");
        assert!(!names.contains("ref"), "{mode:?}");
    }
}

#[test]
fn an_interface_extending_html_attributes_lists_them_and_no_ref() {
    for mode in [HtmlAttributeMode::Curated, HtmlAttributeMode::Full] {
        let names = listed(mode, "Inherited");
        assert!(names.is_superset(&set(&["own", "id", "className", "onClick", "onKeyDown", "aria-label"])), "{mode:?}");
        assert!(!names.contains("ref"), "{mode:?}");
    }
    assert_eq!(listed(HtmlAttributeMode::None, "Inherited"), set(&["own"]));
}

#[test]
fn a_forward_ref_component_lists_ref() {
    for mode in [HtmlAttributeMode::Curated, HtmlAttributeMode::Full] {
        assert_eq!(listed(mode, "Fwd"), set(&["label", "ref"]), "{mode:?}");
    }
    assert_eq!(listed(HtmlAttributeMode::None, "Fwd"), set(&["label"]));
}

#[test]
fn a_prop_declared_beside_an_element_has_one_type_when_the_declarations_agree() {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join("Components.tsx"), SOURCE).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };
    let output = extract(&options);
    let beside = &output.components["Beside"];
    let type_of = |name: &str| serde_json::to_value(&beside.props[name].prop_type).unwrap();
    // `ReactNode` written twice is one type, and `string` narrows the `string | number` the component declares.
    assert_eq!(type_of("children"), serde_json::json!({ "kind": "named", "name": "ReactNode", "args": [] }));
    assert_eq!(type_of("id"), serde_json::json!({ "kind": "string" }));
}
