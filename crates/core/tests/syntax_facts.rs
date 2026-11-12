#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The two facts the checker does not have come from oxc's syntax, not from the structural resolver (ADR 0026, S7a2):
//! a prop's default (the component's code, else the checker's `@default` tag) and the order of a union's members (the
//! order its declaration writes them in). Each test empties the structural props first, as for a prop only the checker
//! returns, so whatever the merge states it read from the source.

use std::collections::BTreeMap;
use std::sync::Arc;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::semantic::{
    SemanticComponent, SemanticOutcome, SemanticProp, merge_semantic, merge_semantic_props,
};
use caliper_docgen_core::types::{DefaultValue, ExtractionOutput, PropParent, PropType};
use camino::Utf8PathBuf;
use rustc_hash::FxHashMap;
use tempfile::TempDir;

const BUTTON: &str = "import type { Tone } from './tone'\n\
    type Size = 'sm' | 'md' | 'lg'\n\
    export interface ButtonProps {\n\
      size?: Size\n\
      tone?: Tone | 'none'\n\
      variant?: 'solid' | 'ghost'\n\
      /** @default 'start' */\n\
      align?: 'start' | 'end'\n\
    }\n\
    export function Button({ size = 'md' }: ButtonProps & { shape?: 'square' | 'round' }) {\n\
      return null\n\
    }\n\
    Button.defaultProps = { variant: 'ghost' }\n";

struct Project {
    _dir: TempDir,
    output: ExtractionOutput,
    button: Utf8PathBuf,
}

fn project() -> Project {
    let dir = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
    std::fs::write(root.join("Button.tsx"), BUTTON).unwrap();
    std::fs::write(root.join("tone.ts"), "export type Tone = 'warm' | 'cold'\n").unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        ..Default::default()
    };
    let mut output = extract(&options);
    let button = output.components["Button"].file_path.clone();
    output.components.get_mut("Button").unwrap().props.clear();
    Project { _dir: dir, output, button }
}

fn lit(value: &str) -> PropType {
    PropType::StringLiteral(value.to_string())
}

fn union(members: &[&str]) -> PropType {
    PropType::Union(members.iter().map(|member| lit(member)).collect())
}

/// The checker's answer: members sorted, no default, `@default` among the tags.
fn answer(button: &Utf8PathBuf) -> Vec<Arc<SemanticProp>> {
    let declared_by = |interface: bool| {
        interface.then(|| PropParent { name: "ButtonProps".to_string(), file_name: button.to_string() })
    };
    let prop = |name: &str, prop_type: PropType, interface: bool, tags: &[(&str, &str)]| {
        Arc::new(SemanticProp {
            name: name.to_string(),
            prop_type,
            required: false,
            description: String::new(),
            tags: tags.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect::<BTreeMap<_, _>>(),
            parent: declared_by(interface),
            declarations: declared_by(interface).into_iter().collect(),
            react_declared: false,
        })
    };
    vec![
        prop("size", union(&["lg", "md", "sm"]), true, &[]),
        prop("tone", union(&["cold", "none", "warm"]), true, &[]),
        prop("variant", union(&["ghost", "solid"]), true, &[]),
        prop("align", union(&["end", "start"]), true, &[("default", "'start'")]),
        prop("shape", union(&["round", "square"]), false, &[]),
    ]
}

fn facts(output: &ExtractionOutput) -> Vec<(String, PropType, Option<String>)> {
    output.components["Button"]
        .props
        .values()
        .map(|prop| (prop.name.clone(), prop.prop_type.clone(), prop.default_value.as_ref().map(|d| d.value.clone())))
        .collect()
}

fn expected() -> Vec<(String, PropType, Option<String>)> {
    let fact = |name: &str, members: &[&str], default: Option<&str>| {
        (name.to_string(), union(members), default.map(str::to_string))
    };
    vec![
        fact("align", &["start", "end"], Some("'start'")),
        fact("shape", &["square", "round"], None),
        fact("size", &["sm", "md", "lg"], Some("'md'")),
        // `Tone` is imported: oxc places the member the file writes and leaves the imported ones in the checker's order.
        fact("tone", &["none", "cold", "warm"], None),
        fact("variant", &["solid", "ghost"], Some("'ghost'")),
    ]
}

#[test]
fn the_whole_project_merge_reads_defaults_and_union_order_from_the_source() {
    let Project { _dir, mut output, button } = project();
    let outcome = SemanticOutcome {
        components: vec![SemanticComponent { key: "Button".to_string(), props: answer(&button), composes: None }],
        ..Default::default()
    };

    merge_semantic(&mut output, outcome);

    assert_eq!(facts(&output), expected());
    let size = &output.components["Button"].props["size"];
    assert_eq!(size.default_value, Some(DefaultValue { value: "'md'".to_string(), computed: false }));
}

#[test]
fn a_session_merge_reads_them_too() {
    let Project { _dir, mut output, button } = project();
    let props = answer(&button);
    let entry = output.components.get_mut("Button").unwrap();

    let merged = merge_semantic_props(entry, props.iter(), None, &mut FxHashMap::default());

    assert_eq!(merged.unread, 0);
    assert_eq!(facts(&output), expected());
}

#[test]
fn a_union_declared_in_a_file_that_cannot_be_found_keeps_the_checker_order() {
    let Project { _dir, mut output, button } = project();
    let mut props = answer(&button);
    let moved = PropParent { name: "ButtonProps".to_string(), file_name: format!("{button}.missing") };
    let size = Arc::make_mut(&mut props[0]);
    size.parent = Some(moved.clone());
    size.declarations = vec![moved];
    let outcome = SemanticOutcome {
        components: vec![SemanticComponent { key: "Button".to_string(), props, composes: None }],
        ..Default::default()
    };

    merge_semantic(&mut output, outcome);

    assert_eq!(output.components["Button"].props["size"].prop_type, union(&["lg", "md", "sm"]));
    assert!(output.diagnostics.iter().all(|d| !d.message.contains("failed to read")), "{:?}", output.diagnostics);
}

#[test]
fn a_type_literal_that_does_not_hold_the_components_props_gives_no_order() {
    let dir = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
    // `Props` is imported; `Other` writes a prop of the same name in the other order.
    std::fs::write(
        root.join("Card.tsx"),
        "import type { Props } from './props'\n\
         type Other = { p: 'b' | 'a' }\n\
         export const other: Other = { p: 'a' }\n\
         export function Card(props: Props) {\n  return null\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("props.ts"), "export type Props = { p: 'a' | 'b' }\n").unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        ..Default::default()
    };
    let mut output = extract(&options);
    output.components.get_mut("Card").unwrap().props.clear();
    let prop = Arc::new(SemanticProp {
        name: "p".to_string(),
        prop_type: union(&["a", "b"]),
        required: true,
        description: String::new(),
        tags: BTreeMap::new(),
        parent: None,
        declarations: Vec::new(),
        react_declared: false,
    });
    let outcome = SemanticOutcome {
        components: vec![SemanticComponent { key: "Card".to_string(), props: vec![prop], composes: None }],
        ..Default::default()
    };

    merge_semantic(&mut output, outcome);

    assert_eq!(output.components["Card"].props["p"].prop_type, union(&["a", "b"]));
}
