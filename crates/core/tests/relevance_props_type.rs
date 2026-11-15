#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The lane facts the props-type evidence of ADR 0028 (slice T2, `relevance::props_type::members`) reads, pinned at
//! the public output: the evidence itself is crate-private and tested beside it. A member the props type redeclares is
//! both a listed prop and a member of its base's collapsed layer (the structural own-vs-inherited split, which marks
//! it overriding); a `Pick`'s keys are listed with their base as `parent`; an `Omit`'s key is in neither; and a
//! collapsed layer names its members only, so a required member in it is known by name alone in either lane.

use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::{ComponentEntry, ExtractionOutput};
use camino::Utf8PathBuf;
use tempfile::TempDir;

fn run() -> ExtractionOutput {
    let fields: String = (0..31).map(|i| format!("p{i}?: string; ")).collect();
    let source = format!(
        "export interface Big {{ size?: string; tone: string; {fields}}}\n\
         interface Narrows extends Big {{ size?: 'sm' | 'md' }}\n\
         export function ByExtends(props: Narrows) {{ return null; }}\n\
         export function ByIntersection(props: Big & {{ size?: 'sm' }}) {{ return null; }}\n\
         export function ByPick(props: Pick<Big, 'p1' | 'p2'> & {{ own?: boolean }}) {{ return null; }}\n\
         export function ByOmit(props: Omit<Big, 'p1'> & {{ own?: boolean }}) {{ return null; }}\n"
    );
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    std::fs::write(root.join("Components.tsx"), source).unwrap();
    let options = PipelineOptions {
        src_dirs: vec![root.clone()],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: CollapseInheritedProps::All,
        ..Default::default()
    };
    extract(&options)
}

fn layered(entry: &ComponentEntry, name: &str) -> bool {
    entry.inheritance.iter().any(|layer| layer.members.iter().any(|member| member == name))
}

fn parent(entry: &ComponentEntry, name: &str) -> Option<String> {
    entry.props[name].parent.as_ref().map(|parent| parent.name.clone())
}

#[test]
fn a_redeclared_member_is_listed_and_held_by_its_collapsed_base() {
    let output = run();
    let extends = &output.components["ByExtends"];
    assert_eq!(parent(extends, "size").as_deref(), Some("Narrows"));
    assert!(layered(extends, "size"));
    let intersection = &output.components["ByIntersection"];
    assert_eq!(parent(intersection, "size"), None);
    assert!(layered(intersection, "size"));
}

#[test]
fn picked_keys_are_listed_from_their_base_and_an_omitted_key_is_nowhere() {
    let output = run();
    let pick = &output.components["ByPick"];
    assert_eq!(pick.props.keys().collect::<Vec<_>>(), ["own", "p1", "p2"]);
    assert_eq!(parent(pick, "p1").as_deref(), Some("Big"));
    assert!(pick.inheritance.is_empty());
    let omit = &output.components["ByOmit"];
    assert_eq!(omit.inheritance[0].omitted, ["p1"]);
    assert!(!layered(omit, "p1") && !omit.props.contains_key("p1"));
}

#[test]
fn a_required_member_of_a_collapsed_source_is_known_by_name_alone() {
    let output = run();
    let extends = &output.components["ByExtends"];
    assert!(layered(extends, "tone"));
    assert!(!extends.props.contains_key("tone") && !extends.notable_inherited.contains_key("tone"));
}
