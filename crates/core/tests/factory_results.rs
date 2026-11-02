#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A PascalCase binding initialized with a call to a plain function is a component when what the function returns is
//! one: a generic factory that returns a local asserted to a component type, or a function that returns a rendering
//! function. A call to a function that returns anything else is not, and is not reported either.

use caliper_docgen_core::pipeline::{extract, HtmlAttributeMode, PipelineOptions};
use caliper_docgen_core::types::{DetectionRule, DiagnosticCode, ExtractionOutput};
use camino::Utf8PathBuf;
use tempfile::TempDir;

const FACTORY: &str = r#"
import * as React from 'react';

export interface PanelProps { /** The heading. */ title: string }
export interface PanelRef { focus(): void }

export const makePanel = <T,>(config: { value: T }) => {
  const Inner = (props: PanelProps & { item?: T }, ref: React.ForwardedRef<PanelRef>) => <div>{props.title}</div>;
  const Panel = React.forwardRef(Inner) as React.ForwardRefExoticComponent<
    PanelProps & { item?: T } & React.RefAttributes<PanelRef>
  >;
  return Panel;
};

export function createLoader(library: string) {
  return function Loader({ name, ...rest }: { /** The icon to load. */ name: string } & React.ComponentProps<'svg'>) {
    return <svg {...rest} data-library={library} data-name={name} />;
  };
}

export const makeLabel = (count: number) => {
  const label = String(count) as string;
  return label;
};

export function makeList(count: number) {
  return Array.from({ length: count }, (_, index) => index);
}
"#;

const USE: &str = r#"
import { makePanel, createLoader, makeLabel, makeList } from './factory';

export const Panel = makePanel<string>({ value: 'a' });
export const IconA = createLoader('a');
export const Label = makeLabel(3);
export const Items = makeList(3);
export const Missing = notDeclaredAnywhere(1);
"#;

struct Project {
    _tmp: TempDir,
    root: Utf8PathBuf,
}

fn project() -> Project {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    for (name, source) in [("src/factory.tsx", FACTORY), ("src/use.tsx", USE)] {
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
        html_attributes: HtmlAttributeMode::None,
        ..Default::default()
    })
}

#[test]
fn a_generic_factory_that_returns_an_asserted_component_type_builds_a_component_with_its_props() {
    let project = project();
    let output = run(&project);
    let panel = output.components.get("Panel").unwrap_or_else(|| panic!("missing: {:?}", output.components.keys()));
    assert_eq!(panel.detected_by, DetectionRule::FactoryResult);
    assert_eq!(panel.props.keys().map(String::as_str).collect::<Vec<_>>(), ["item", "title"]);
    assert_eq!(panel.props["title"].description, "The heading.");
    // `T` was bound to `string` by the call.
    assert_eq!(serde_json::to_value(&panel.props["item"].prop_type).unwrap(), serde_json::json!({ "kind": "string" }));
}

#[test]
fn a_function_that_returns_a_rendering_function_builds_a_component_with_that_functions_props() {
    let project = project();
    let output = run(&project);
    let icon = output.components.get("IconA").unwrap_or_else(|| panic!("missing: {:?}", output.components.keys()));
    assert_eq!(icon.detected_by, DetectionRule::FactoryResult);
    assert_eq!(icon.props.keys().map(String::as_str).collect::<Vec<_>>(), ["name"]);
    assert_eq!(icon.props["name"].description, "The icon to load.");
}

#[test]
fn a_call_to_a_function_that_returns_no_component_is_neither_published_nor_reported() {
    let project = project();
    let output = run(&project);
    let mut names: Vec<&str> = output.components.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["IconA", "Panel"]);
    let skipped: Vec<&str> = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::SkippedCandidate)
        .map(|diagnostic| diagnostic.message.as_str())
        .filter(|message| ["Label", "Items", "Missing"].iter().any(|name| message.contains(&format!("'{name}'"))))
        .collect();
    assert_eq!(skipped, Vec::<&str>::new());
}
