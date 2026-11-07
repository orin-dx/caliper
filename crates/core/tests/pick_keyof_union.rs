#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Props types built from React's element attributes keep the HTML layer: `Pick<…, keyof React.HTMLAttributes<…> | …>`
//! (cmdk), a union of props interfaces bare or inside `Omit` (Radix slider, accordion), and the global
//! `JSX.IntrinsicElements['img']` (next/image) read as `React.JSX` does. A lazy session answers as a batch run does.
use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{ComponentAnswer, HtmlAttributeMode, PipelineOptions, WatchSession, extract};
use caliper_docgen_core::types::{ComponentEntry, ExtractionOutput};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const SOURCE: &str = r#"
import * as React from 'react';

export interface HtmlOnly extends React.HTMLAttributes<HTMLInputElement> {}
export const HtmlOnlyBox = (props: HtmlOnly) => null;

export type KeyofPicked = Pick<
    React.DetailedHTMLProps<React.InputHTMLAttributes<HTMLInputElement>, HTMLInputElement>,
    keyof React.HTMLAttributes<HTMLInputElement> | 'value'
> & { own?: string };
export const KeyofPick = (props: KeyofPicked) => null;

export type LiteralPicked = Pick<
    React.DetailedHTMLProps<React.InputHTMLAttributes<HTMLInputElement>, HTMLInputElement>,
    'id' | 'className' | 'value'
> & { own?: string };
export const LiteralPick = (props: LiteralPicked) => null;

export interface BaseProps extends React.HTMLAttributes<HTMLSpanElement> {
    base?: string;
}
export interface HorizontalProps extends BaseProps {
    orientation?: 'horizontal';
    h?: number;
}
export interface VerticalProps extends BaseProps {
    orientation: 'vertical';
    v?: number;
}
export const Horizontal = (props: HorizontalProps) => null;
export const Vertical = (props: VerticalProps) => null;
export type EitherProps = HorizontalProps | VerticalProps;
export const Either = (props: EitherProps) => null;

export type OmittedEither = Omit<HorizontalProps | VerticalProps, 'title'>;
export const OmitEither = (props: OmittedEither) => null;

export type OmittedOne = Omit<HorizontalProps, 'title'>;
export const OmitOne = (props: OmittedOne) => null;

type SpanProps = React.ComponentPropsWithoutRef<'span'>;
interface ImplProps extends SpanProps {
    impl?: string;
}
export interface SliderHProps extends ImplProps {
    orientation?: 'horizontal';
    h?: number;
}
export interface SliderVProps extends ImplProps {
    orientation: 'vertical';
    v?: number;
}
export type SliderImplUnion = SliderHProps | SliderVProps;
export const SliderUnion = (props: SliderImplUnion) => null;
export type SliderOmitted = Omit<SliderHProps | SliderVProps, 'title'>;
export const SliderOmit = (props: SliderOmitted) => null;
export const SliderOne = (props: Omit<SliderHProps, 'title'>) => null;

type BaseUIProps<E extends React.ElementType, State> = React.ComponentPropsWithRef<E> & { render?: State };
export type HeadingProps = BaseUIProps<'h1' | 'h2' | 'h3', { open: boolean }>;
export const Heading = (props: HeadingProps) => null;
export type OneTag = BaseUIProps<'h1', { open: boolean }>;
export const OneTagged = (props: OneTag) => null;

export type GlobalImg = JSX.IntrinsicElements['img'] & { own?: string };
export const GlobalImage = (props: GlobalImg) => null;

export type QualifiedImg = React.JSX.IntrinsicElements['img'] & { own?: string };
export const QualifiedImage = (props: QualifiedImg) => null;
"#;

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

fn run(mode: HtmlAttributeMode) -> (TempDir, ExtractionOutput) {
    let dir = project(&[("Components.tsx", SOURCE)]);
    let output = extract(&options(&dir, mode));
    (dir, output)
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn union(a: &BTreeSet<String>, b: &BTreeSet<String>) -> BTreeSet<String> {
    a.union(b).cloned().collect()
}

fn without(a: &BTreeSet<String>, names: &[&str]) -> BTreeSet<String> {
    a.iter().filter(|name| !names.contains(&name.as_str())).cloned().collect()
}

/// `keyof React.HTMLAttributes<T>` is the interface's own and inherited members: the picked props are exactly those,
/// plus the literal key, whichever mode lists the element's other attributes.
#[test]
fn a_pick_by_keyof_react_attributes_lists_exactly_those_attributes() {
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated] {
        let (_dir, output) = run(HtmlAttributeMode::Full);
        let html = props(&output, "HtmlOnlyBox");
        assert!(html.len() > 200 && html.contains("aria-label") && html.contains("onClick"), "{}", html.len());
        let (_dir, output) = run(mode);
        assert_eq!(props(&output, "KeyofPick"), union(&html, &set(&["value", "own"])), "{mode:?}");
        assert_eq!(notable(&output, "KeyofPick"), set(&[]), "{mode:?}");
        assert_eq!(props(&output, "LiteralPick"), set(&["id", "className", "value", "own"]), "{mode:?}");
    }
}

#[test]
fn a_union_of_props_interfaces_keeps_the_html_layer_of_its_members() {
    let (_dir, full) = run(HtmlAttributeMode::Full);
    let all = union(&props(&full, "Horizontal"), &props(&full, "Vertical"));
    assert_eq!(props(&full, "Either"), all);
    assert_eq!(props(&full, "SliderUnion"), union(&props(&full, "SliderOne"), &set(&["v", "title"])));
    assert_eq!(props(&full, "OmitEither"), without(&all, &["title"]));
    assert_eq!(props(&full, "SliderOmit"), without(&props(&full, "SliderUnion"), &["title"]));
    assert!(props(&full, "OmitEither").contains("v") && props(&full, "OmitEither").contains("aria-label"));

    let (_dir, curated) = run(HtmlAttributeMode::Curated);
    assert_eq!(notable(&curated, "Either"), notable(&curated, "Horizontal"));
    assert_eq!(notable(&curated, "SliderUnion"), union(&notable(&curated, "SliderOne"), &set(&["title"])));
    assert!(notable(&curated, "Either").contains("className"));
    assert_eq!(props(&curated, "Either"), set(&["base", "orientation", "h", "v"]));
    assert_eq!(props(&curated, "OmitEither"), set(&["base", "orientation", "h", "v"]));
    let omitted = notable(&curated, "OmitEither");
    assert_eq!(omitted, without(&notable(&curated, "Either"), &["title"]));
    assert!(!omitted.contains("title") && omitted.contains("className"));
}

#[test]
fn the_global_jsx_namespace_reads_as_react_s() {
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated] {
        let (_dir, output) = run(mode);
        assert_eq!(props(&output, "GlobalImage"), props(&output, "QualifiedImage"), "{mode:?}");
        assert_eq!(notable(&output, "GlobalImage"), notable(&output, "QualifiedImage"), "{mode:?}");
        let all = union(&props(&output, "GlobalImage"), &notable(&output, "GlobalImage"));
        assert!(all.contains("src") && all.contains("alt") && all.contains("own"), "{mode:?}");
    }
}

#[test]
fn a_generic_alias_bound_to_a_union_of_tags_has_their_attributes() {
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated] {
        let (_dir, output) = run(mode);
        assert_eq!(props(&output, "Heading"), props(&output, "OneTagged"), "{mode:?}");
        assert_eq!(notable(&output, "Heading"), notable(&output, "OneTagged"), "{mode:?}");
        let all = union(&props(&output, "Heading"), &notable(&output, "Heading"));
        assert!(all.contains("render") && all.contains("className") && all.contains("aria-label"), "{mode:?}");
    }
}

/// A lazy session answers each key as a batch run does (ADR 0016).
#[test]
fn a_lazy_session_answers_as_a_batch_run_does() {
    let dir = project(&[("Components.tsx", SOURCE)]);
    for mode in [HtmlAttributeMode::Full, HtmlAttributeMode::Curated, HtmlAttributeMode::None] {
        let options = options(&dir, mode);
        let batch = extract(&options);
        let keys: Vec<String> = batch.components.keys().cloned().collect();
        assert_eq!(keys.len(), 15, "{mode:?}: {keys:?}");

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
