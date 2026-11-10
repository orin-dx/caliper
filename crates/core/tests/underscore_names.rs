#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A component whose name has to start with an underscore is still a component: an identifier cannot start with a digit,
//! so React Spectrum's `3D.tsx` illustration exports `_3D`. A lowercase name after the underscore is a helper, and a
//! PascalCase name still has to render.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::{DiagnosticCode, ExtractionOutput};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const SOURCE: &str = r#"
import { memo } from 'react';

interface TileProps { tone?: 'warm' | 'cool' }
interface RowProps { rows: string[] }

export function _Card(props: TileProps) {
    return <div data-tone={props.tone} />;
}
export const _Tile = (props: TileProps) => <div data-tone={props.tone} />;
export const _Memoized = memo((props: TileProps) => <div data-tone={props.tone} />);
export function _42Label(props: TileProps) {
    return <span data-tone={props.tone} />;
}

export function _renderRow(props: RowProps) {
    return <li>{props.rows}</li>;
}
export function _3d(props: RowProps) {
    return <li>{props.rows}</li>;
}
export function _Count(props: RowProps) {
    return props.rows.length;
}
"#;

const DEFAULT_EXPORT: &str = r#"
import type { ReactNode } from 'react';

export default function _3D(props: { size?: 'L' | 'M' | 'S' }): ReactNode {
    return <svg data-size={props.size} />;
}
"#;

fn run(file: &str, source: &str) -> ExtractionOutput {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join(file), source).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        ..Default::default()
    };
    extract(&options)
}

fn props(output: &ExtractionOutput, component: &str) -> BTreeSet<String> {
    let entry = output.components.get(component).unwrap_or_else(|| panic!("no component {component}"));
    entry.props.keys().cloned().collect()
}

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_default_exported_function_named_with_a_leading_underscore_and_digit_is_a_component() {
    let output = run("3D.tsx", DEFAULT_EXPORT);
    assert_eq!(props(&output, "_3D"), names(&["size"]));
}

#[test]
fn a_leading_underscore_before_a_capital_names_a_component_in_every_declaration_form() {
    let output = run("Tiles.tsx", SOURCE);
    for component in ["_Card", "_Tile", "_Memoized", "_42Label"] {
        assert_eq!(props(&output, component), names(&["tone"]), "{component}");
    }
}

#[test]
fn a_leading_underscore_before_a_lowercase_letter_stays_a_helper() {
    let output = run("Tiles.tsx", SOURCE);
    for helper in ["_renderRow", "_3d"] {
        assert!(!output.components.contains_key(helper), "{helper} is not a component");
    }
}

#[test]
fn an_underscore_name_that_does_not_render_is_still_declined_with_a_reason() {
    let output = run("Tiles.tsx", SOURCE);
    assert!(!output.components.contains_key("_Count"));
    let declined =
        output.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate && d.message.contains("_Count"));
    assert!(declined, "{:?}", output.diagnostics);
}
