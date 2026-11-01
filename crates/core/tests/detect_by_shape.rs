#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Components are found by the shape of the code and by where React's APIs were imported from, not by the names a
//! file happens to use: `memo(forwardRef(fn))` with no type arguments, renamed and namespaced imports of React's
//! `forwardRef`, `memo` and `createContext`, and a tagged factory's props type chosen by the tag it gives.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::ExtractionOutput;
use camino::Utf8PathBuf;
use tempfile::TempDir;

const SOURCE: &str = r#"
import * as R from 'react';
import { forwardRef as fr, memo as m, createContext as ctx } from 'react';
import { forwardRef } from './utils';

interface OwnProps { label?: string }

export const MemoForward = m(fr(function MemoForward(props: OwnProps, ref: R.Ref<HTMLDivElement>) {
    return <div ref={ref}>{props.label}</div>;
}));
export const MemoForwardArrow = R.memo(R.forwardRef((props: OwnProps, ref: R.Ref<HTMLDivElement>) => (
    <div ref={ref}>{props.label}</div>
)));
export const Renamed = fr(function Renamed(props: OwnProps, ref: R.Ref<HTMLDivElement>) {
    return <div ref={ref}>{props.label}</div>;
});
export const Foreign = forwardRef(function Foreign(props: OwnProps, ref: R.Ref<HTMLDivElement>) {
    return <div ref={ref}>{props.label}</div>;
});
export const RenamedMemo = m(function RenamedMemo(props: OwnProps) { return <div>{props.label}</div>; });
export const Theme = ctx<OwnProps>(() => <div />);
export const Plain = (props: OwnProps) => <div>{props.label}</div>;
"#;

const TAGGED: &str = r#"
import * as R from 'react';
import { chakra } from './styled-system';

interface Base<T> { id?: string }
export interface BadgeProps extends Base<"span"> { tone?: string }
export const Badge = chakra("span", { base: {} });

type PlainProps = { tone: string };
export const Plain = chakra("section", { base: {} });

interface RowProps extends Base<"div"> { dense?: boolean }
interface ColumnProps extends Base<"div"> { wide?: boolean }
export const Row = chakra("div", { base: {} });
export const Column = chakra("div", { base: {} });
export const Cell = chakra("div", { base: {} });

interface GroupProps extends Base<"ul"> { gap?: number }
const StyledGroup = chakra("ul", { base: {} });
export const Group = R.memo(R.forwardRef<HTMLUListElement, GroupProps>(function Group(props, ref) {
    return <StyledGroup ref={ref} />;
}));
"#;

fn run(file: &str, source: &str) -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join(file), source).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    };
    extract(&options)
}

fn listed(output: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    let entry = output.components.get(component).unwrap_or_else(|| panic!("no component {component}"));
    entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn react_apis_are_found_whatever_they_were_imported_as() {
    let output = run("Components.tsx", SOURCE);
    for component in ["MemoForward", "MemoForwardArrow", "Renamed", "Foreign"] {
        assert_eq!(listed(&output, component), set(&["label", "ref"]), "{component}");
    }
    assert_eq!(listed(&output, "RenamedMemo"), set(&["label"]));
    assert_eq!(listed(&output, "Plain"), set(&["label"]));
}

#[test]
fn a_context_made_by_a_renamed_create_context_is_not_a_component() {
    let output = run("Components.tsx", SOURCE);
    assert!(!output.components.contains_key("Theme"), "{:?}", output.components.keys().collect::<Vec<_>>());
}

#[test]
fn a_tagged_factory_takes_the_props_type_that_gives_its_tag() {
    let output = run("Styled.tsx", TAGGED);
    assert_eq!(listed(&output, "Badge"), set(&["id", "tone"]));
    assert!(!output.components.contains_key("Plain"));
}

#[test]
fn tagged_factories_sharing_a_tag_take_the_props_type_named_for_them() {
    let output = run("Styled.tsx", TAGGED);
    assert_eq!(listed(&output, "Row"), set(&["id", "dense"]));
    assert_eq!(listed(&output, "Column"), set(&["id", "wide"]));
    assert!(!output.components.contains_key("Cell"));
}

#[test]
fn a_private_factory_does_not_take_the_props_of_the_component_built_on_it() {
    let output = run("Styled.tsx", TAGGED);
    assert!(!output.components.contains_key("StyledGroup"));
    assert_eq!(listed(&output, "Group"), set(&["id", "gap", "ref"]));
}
