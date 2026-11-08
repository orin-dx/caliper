#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `React.AriaAttributes`, `React.HTMLProps<T>` and `React.RefAttributes<T>` are props providers, not placeholders:
//! antd's `HTMLAriaDataAttributes`, `Omit<React.HTMLProps<HTMLDivElement>, ...>` and shadcn's react-aria wrappers lose
//! their props otherwise. Each follows the HTML attribute mode as the element layers do, and a lazy session answers as
//! a batch run does.
use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{ComponentAnswer, HtmlAttributeMode, PipelineOptions, WatchSession, extract};
use caliper_docgen_core::types::{ComponentEntry, ExtractionOutput};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const SOURCE: &str = r#"
import * as React from 'react';

export interface AriaPanelProps extends React.AriaAttributes {
    own?: string;
}
export function AriaPanel(props: AriaPanelProps) { return null; }

export type AriaAlias = React.AriaAttributes & { own?: string };
export const AriaAliased = (props: AriaAlias) => null;

export type AriaPicked = Pick<React.AriaAttributes, 'aria-label'> & { own?: string };
export const AriaPickedPanel = (props: AriaPicked) => null;

export type HtmlBox = Omit<React.HTMLProps<HTMLDivElement>, 'title' | 'aria-label'> & { own?: string };
export const HtmlBoxed = (props: HtmlBox) => null;

export interface DivBox extends React.HTMLProps<HTMLDivElement> {
    own?: string;
}
export function DivBoxed(props: DivBox) { return null; }

export type HtmlPicked = Pick<React.HTMLProps<HTMLDivElement>, 'id' | 'ref'> & { own?: string };
export const HtmlPickedBox = (props: HtmlPicked) => null;

export type RefProps = { own?: string } & React.RefAttributes<HTMLButtonElement>;
export const Refed = (props: RefProps) => null;

export interface RefExtended extends React.RefAttributes<HTMLButtonElement> {
    own?: string;
}
export function RefExtendedBox(props: RefExtended) { return null; }
"#;

const SHADOWED: &str = r#"
import * as React from 'react';

interface AriaAttributes {
    custom?: string;
}
export interface ShadowProps extends AriaAttributes {
    own?: string;
}
export function Shadow(props: ShadowProps) { return null; }
"#;

/// A project of `files`, inside this crate so that `react` resolves to the workspace's `@types/react`.
fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    for (name, body) in files {
        std::fs::write(dir.path().join(name), body).unwrap();
    }
    dir
}

fn options(dir: &TempDir, mode: HtmlAttributeMode) -> PipelineOptions {
    let root = Utf8PathBuf::from_path_buf(dir.path().to_owned()).unwrap();
    PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: mode,
        ..Default::default()
    }
}

fn entry<'a>(output: &'a ExtractionOutput, component: &str) -> &'a ComponentEntry {
    output.components.get(component).unwrap_or_else(|| panic!("no component {component}"))
}

fn props(output: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    entry(output, component).props.keys().cloned().collect()
}

fn notable(output: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    entry(output, component).notable_inherited.keys().cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// The members `@types/react` declares on its `AriaAttributes` interface, read from its text.
fn react_aria_attributes() -> BTreeSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/@types/react/index.d.ts");
    let text = std::fs::read_to_string(path).unwrap();
    let body = text.split("    interface AriaAttributes {").nth(1).unwrap().split("\n    }\n").next().unwrap();
    let names: BTreeSet<String> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("\"aria-"))
        .map(|rest| format!("aria-{}", rest.split('"').next().unwrap()))
        .collect();
    assert!(names.len() > 40, "read {} aria attributes", names.len());
    names
}

fn with(base: &BTreeSet<String>, extra: &[&str]) -> BTreeSet<String> {
    base.iter().cloned().chain(extra.iter().map(|name| (*name).to_owned())).collect()
}

#[test]
fn aria_attributes_are_listed_in_full_mode_as_react_declares_them() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let output = extract(&options(&dir, HtmlAttributeMode::Full));
    let aria = react_aria_attributes();
    for component in ["AriaPanel", "AriaAliased"] {
        assert_eq!(props(&output, component), with(&aria, &["own"]), "{component}");
        assert_eq!(notable(&output, component), set(&[]), "{component}");
    }
    let label = &entry(&output, "AriaPanel").props["aria-label"];
    assert_eq!(label.parent.as_ref().map(|parent| parent.name.as_str()), Some("AriaAttributes"));
    assert!(!label.description.is_empty(), "the declaration's documentation comes with it");
}

#[test]
fn aria_attributes_are_notable_in_curated_mode() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let output = extract(&options(&dir, HtmlAttributeMode::Curated));
    for component in ["AriaPanel", "AriaAliased"] {
        assert_eq!(props(&output, component), set(&["own"]), "{component}");
        assert_eq!(notable(&output, component), react_aria_attributes(), "{component}");
    }
}

#[test]
fn aria_attributes_are_left_out_in_none_mode_without_a_warning() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let output = extract(&options(&dir, HtmlAttributeMode::None));
    for component in ["AriaPanel", "AriaAliased", "HtmlBoxed", "DivBoxed", "Refed", "RefExtendedBox"] {
        assert_eq!(props(&output, component), set(&["own"]), "{component}");
        assert_eq!(notable(&output, component), set(&[]), "{component}");
    }
    let unresolved: Vec<_> = output.diagnostics.iter().filter(|d| d.message.contains("Cannot resolve")).collect();
    assert!(unresolved.is_empty(), "{unresolved:?}");
}

#[test]
fn a_pick_of_aria_attributes_lists_only_the_picked_one_in_every_mode() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated] {
        let output = extract(&options(&dir, mode));
        assert_eq!(props(&output, "AriaPickedPanel"), set(&["aria-label", "own"]), "{mode:?}");
        assert_eq!(notable(&output, "AriaPickedPanel"), set(&[]), "{mode:?}");
    }
}

#[test]
fn ref_attributes_give_ref_and_never_key() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let full = extract(&options(&dir, HtmlAttributeMode::Full));
    let curated = extract(&options(&dir, HtmlAttributeMode::Curated));
    for component in ["Refed", "RefExtendedBox"] {
        assert_eq!(props(&full, component), set(&["own", "ref"]), "{component} (full)");
        assert_eq!(props(&curated, component), set(&["own"]), "{component} (curated)");
        assert_eq!(notable(&curated, component), set(&["ref"]), "{component} (curated)");
    }
    let ty = serde_json::to_value(&entry(&full, "Refed").props["ref"].prop_type).unwrap();
    assert!(ty.to_string().contains("HTMLButtonElement"), "the element argument types the ref: {ty}");
}

#[test]
fn html_props_are_the_elements_attributes_and_ref_in_full_mode() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let output = extract(&options(&dir, HtmlAttributeMode::Full));
    let boxed = props(&output, "DivBoxed");
    assert!(
        ["ref", "form", "target", "aria-label", "title", "onClick", "className"].iter().all(|n| boxed.contains(*n))
    );
    assert!(!boxed.contains("key"));
    // The alias path reads the same attributes as `extends`, less what `Omit` removes.
    let mut expected = boxed;
    expected.remove("title");
    expected.remove("aria-label");
    assert_eq!(props(&output, "HtmlBoxed"), expected);
}

#[test]
fn html_props_follow_the_curated_element_layer() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let output = extract(&options(&dir, HtmlAttributeMode::Curated));
    assert_eq!(props(&output, "HtmlBoxed"), set(&["own"]));
    let listed = notable(&output, "HtmlBoxed");
    assert!(
        ["ref", "id", "className", "onClick", "aria-describedby"].iter().all(|n| listed.contains(*n)),
        "{listed:?}"
    );
    assert!(!listed.contains("title") && !listed.contains("aria-label"), "omitted: {listed:?}");
    assert!(!listed.contains("form") && !listed.contains("key"), "not notable: {listed:?}");
    assert_eq!(listed.len(), notable(&output, "DivBoxed").len() - 2);
}

#[test]
fn a_pick_of_html_props_lists_only_the_picked_ones() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated] {
        let output = extract(&options(&dir, mode));
        assert_eq!(props(&output, "HtmlPickedBox"), set(&["id", "own", "ref"]), "{mode:?}");
        assert_eq!(notable(&output, "HtmlPickedBox"), set(&[]), "{mode:?}");
    }
}

#[test]
fn a_project_interface_named_like_react_s_is_the_project_s() {
    let dir = project(&[("Shadow.tsx", SHADOWED)]);
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated, HtmlAttributeMode::None] {
        let output = extract(&options(&dir, mode));
        assert_eq!(props(&output, "Shadow"), set(&["custom", "own"]), "{mode:?}");
        assert_eq!(notable(&output, "Shadow"), set(&[]), "{mode:?}");
    }
}

/// A lazy session answers each key as a batch run publishes it (ADR 0016), for the new providers in every mode.
#[test]
fn a_lazy_session_answers_as_a_batch_run_does() {
    let dir = project(&[("Components.tsx", SOURCE), ("Shadow.tsx", SHADOWED)]);
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated, HtmlAttributeMode::None] {
        let options = options(&dir, mode);
        let batch = extract(&options);
        let keys: Vec<String> = batch.components.keys().cloned().collect();
        assert_eq!(keys.len(), 9, "{mode:?}: {keys:?}");

        let together = WatchSession::lazy(options.clone());
        drop(together.initialize());
        let answers = together.components(&keys);
        let one_by_one = WatchSession::lazy(options.clone());
        drop(one_by_one.initialize());
        for (key, answer) in keys.iter().zip(answers) {
            let lazy: ComponentAnswer = answer.unwrap_or_else(|| panic!("{mode:?}: no answer for {key}"));
            assert_eq!(lazy.entry, batch.components[key], "{mode:?}: {key}");
            let single = one_by_one.component(key).unwrap_or_else(|| panic!("{mode:?}: no answer for {key}"));
            assert_eq!(single.entry, batch.components[key], "{mode:?}: {key}, asked alone");
        }
    }
}
