#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `Alias<typeof Component>['prop']`: an indexed access reads a property of the object type it resolves to, the way
//! TypeScript's `getIndexedAccessType` does, rather than of the declarations written for it.

use std::collections::BTreeMap;

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use camino::Utf8PathBuf;
use serde_json::{json, Value};

/// The props of the component `name` (`(required, type)` per prop) in a project of `files`, and its diagnostics as
/// `"Code: message"`.
fn props_of(files: &[(&str, &str)], name: &str) -> (BTreeMap<String, (bool, Value)>, Vec<String>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    for (path, source) in files {
        std::fs::write(root.join(path), source).unwrap();
    }
    let options =
        PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() };
    let output = extract(&options);
    let entry = output
        .components
        .iter()
        .find(|(_, entry)| entry.display_name == name)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| panic!("no component {name}: {:?}", output.components.keys()));
    let props = entry
        .props
        .iter()
        .map(|(name, prop)| (name.clone(), (prop.required, serde_json::to_value(&prop.prop_type).unwrap())))
        .collect();
    let diagnostics = output.diagnostics.iter().map(|d| format!("{:?}: {}", d.code, d.message)).collect();
    (props, diagnostics)
}

fn tooltip() -> (&'static str, &'static str) {
    (
        "tooltip.tsx",
        r#"
        export interface TooltipProps { visible?: boolean; overlay: string; delay: number | string }
        export default function Tooltip(props: TooltipProps) { return <b />; }
        "#,
    )
}

fn count(diagnostics: &[String], code: &str) -> usize {
    diagnostics.iter().filter(|d| d.starts_with(code)).count()
}

#[test]
fn a_member_of_a_components_props_through_component_props_of_typeof() {
    let (props, diagnostics) = props_of(
        &[
            tooltip(),
            (
                "user.tsx",
                r#"
                import * as React from 'react';
                import Tooltip from './tooltip';
                type RcTooltipProps = React.ComponentPropsWithoutRef<typeof Tooltip>;
                export function User(props: {
                    visible: RcTooltipProps['visible'];
                    overlay?: RcTooltipProps['overlay'];
                    delay: RcTooltipProps['delay'];
                }) { return <b />; }
                "#,
            ),
        ],
        "User",
    );
    // The property's own type, however the outer prop is declared: `visible` is optional on `Tooltip`, required here.
    assert_eq!(props["visible"], (true, json!({ "kind": "boolean" })));
    assert_eq!(props["overlay"], (false, json!({ "kind": "string" })));
    assert_eq!(
        props["delay"],
        (true, json!({ "kind": "union", "members": [{ "kind": "number" }, { "kind": "string" }] })),
    );
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 0, "{diagnostics:?}");
}

#[test]
fn a_member_of_a_generic_alias_applied_to_a_type() {
    let (props, diagnostics) = props_of(
        &[
            tooltip(),
            (
                "user.tsx",
                r#"
                import type { TooltipProps } from './tooltip';
                type WithExtra<T> = T & { extra: number };
                export function User(props: { a: WithExtra<TooltipProps>['visible']; b: WithExtra<TooltipProps>['extra'] }) {
                    return <b />;
                }
                "#,
            ),
        ],
        "User",
    );
    assert_eq!(props["a"], (true, json!({ "kind": "boolean" })));
    assert_eq!(props["b"], (true, json!({ "kind": "number" })));
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 0, "{diagnostics:?}");
}

#[test]
fn a_member_the_type_does_not_have_stays_opaque_with_one_diagnostic() {
    let (props, diagnostics) = props_of(
        &[
            tooltip(),
            (
                "user.tsx",
                r#"
                import * as React from 'react';
                import Tooltip from './tooltip';
                type RcTooltipProps = React.ComponentPropsWithoutRef<typeof Tooltip>;
                export function User(props: { ok: RcTooltipProps['overlay']; missing: RcTooltipProps['nope'] }) {
                    return <b />;
                }
                "#,
            ),
        ],
        "User",
    );
    assert_eq!(props["ok"], (true, json!({ "kind": "string" })));
    assert_eq!(props["missing"].1["kind"], json!("opaque"));
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 1, "{diagnostics:?}");
}

#[test]
fn a_component_that_reads_its_own_props_terminates() {
    let (props, diagnostics) = props_of(
        &[(
            "self.tsx",
            r#"
            import * as React from 'react';
            export function Self(props: { a: string; b: React.ComponentProps<typeof Self>['a'] }) {
                return <b />;
            }
            "#,
        )],
        "Self",
    );
    assert_eq!(props["a"], (true, json!({ "kind": "string" })));
    // TypeScript calls this circular and gives `any`. The read asks for `a`, which does not depend on `b`, so it comes
    // back whole; the loop through `b` inside it is cut by the nesting limit, and nothing is reported for it.
    assert_eq!(props["b"], (true, json!({ "kind": "string" })));
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 0, "{diagnostics:?}");
    assert_eq!(count(&diagnostics, "MaxDepthExceeded"), 0, "{diagnostics:?}");
}

#[test]
fn a_member_that_only_reads_itself_stays_opaque() {
    let (props, diagnostics) = props_of(
        &[(
            "loop.tsx",
            r#"
            import * as React from 'react';
            export function Loop(props: { b: React.ComponentProps<typeof Loop>['b'] }) {
                return <b />;
            }
            "#,
        )],
        "Loop",
    );
    assert_eq!(props["b"].1["kind"], json!("opaque"));
    assert_eq!(props["b"].1["reason"]["type"], json!("indexedAccess"));
    assert_eq!(count(&diagnostics, "IndexedAccessOpaque"), 1, "{diagnostics:?}");
}
