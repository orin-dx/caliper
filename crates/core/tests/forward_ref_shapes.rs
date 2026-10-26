#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A component made by React's `forwardRef` takes a `ref`, however the call is spelled and whether or not it names the
//! element type: the props table lists `ref` for each spelling and only for those.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const SOURCE: &str = r#"
import * as React from 'react';
import { forwardRef, memo } from 'react';

type forwardRefType = typeof forwardRef;

interface OwnProps { label?: string }

export const Qualified = React.forwardRef(function Qualified(props: OwnProps, ref: React.Ref<HTMLDivElement>) {
    return <div ref={ref}>{props.label}</div>;
});
export const Bare = forwardRef((props: OwnProps, ref: React.Ref<HTMLDivElement>) => <div ref={ref} />);
export const Spelled = (forwardRef as forwardRefType)(function Spelled(props: OwnProps, ref: React.Ref<HTMLDivElement>) {
    return <div ref={ref}>{props.label}</div>;
});
const Inner = (props: OwnProps, ref: React.ForwardedRef<HTMLDivElement>) => <div ref={ref}>{props.label}</div>;
export const ByIdentifier = React.forwardRef(Inner) as (props: OwnProps & { ref?: React.Ref<HTMLDivElement> }) => React.ReactElement;
export const Memoized = memo(forwardRef<HTMLDivElement, OwnProps>(function Memoized(props, ref) {
    return <div ref={ref}>{props.label}</div>;
}));

export const Memo = memo(function Memo(props: OwnProps) { return <div>{props.label}</div>; });
export const Plain = (props: OwnProps) => <div>{props.label}</div>;
"#;

fn listed(component: &str) -> BTreeSet<String> {
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
    let entry = output.components.get(component).unwrap_or_else(|| panic!("no component {component}"));
    entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn every_spelling_of_forward_ref_lists_ref_beside_its_own_props() {
    for component in ["Qualified", "Bare", "Spelled", "ByIdentifier", "Memoized"] {
        assert_eq!(listed(component), set(&["label", "ref"]), "{component}");
    }
}

#[test]
fn components_not_made_by_forward_ref_list_no_ref() {
    for component in ["Memo", "Plain"] {
        assert_eq!(listed(component), set(&["label"]), "{component}");
    }
}
