use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::types::output::{ComponentEntry, ExtractionStats};
use crate::types::{DefaultValue, DetectionRule, PropParent};

fn structural(name: &str, description: &str, default: Option<&str>) -> Arc<ParsedProp> {
    Arc::new(ParsedProp::new(
        name.to_string(),
        PropType::Any,
        true,
        default.map(|value| DefaultValue { value: value.to_string(), computed: false }),
        description.to_string(),
        BTreeMap::new(),
        Some(PropParent { name: "Props".to_string(), file_name: "/src/Button.tsx".to_string() }),
        vec![],
    ))
}

fn output() -> ExtractionOutput {
    let props = BTreeMap::from([
        ("label".to_string(), structural("label", "Structural label.", None)),
        ("size".to_string(), structural("size", "", Some("\"sm\""))),
        ("gone".to_string(), structural("gone", "Not in the checker's answer.", None)),
    ]);
    let entry = |name: &str, props| ComponentEntry {
        display_name: name.to_string(),
        file_path: format!("/src/{name}.tsx").into(),
        description: String::new(),
        props,
        inheritance: vec![],
        notable_inherited: BTreeMap::new(),
        discriminant_prop: None,
        composes: vec![],
        tags: BTreeMap::new(),
        methods: vec![],
        detected_by: DetectionRule::FunctionDeclaration,
        exported: true,
    };
    ExtractionOutput {
        components: BTreeMap::from([
            ("Button".to_string(), entry("Button", props)),
            ("Card".to_string(), entry("Card", BTreeMap::new())),
        ]),
        enums: BTreeMap::new(),
        diagnostics: vec![],
        stats: ExtractionStats::default(),
    }
}

fn semantic_prop(name: &str, description: &str, required: bool) -> Arc<SemanticProp> {
    Arc::new(SemanticProp {
        name: name.to_string(),
        prop_type: PropType::String,
        required,
        description: description.to_string(),
        react_declared: false,
    })
}

fn button(props: Vec<Arc<SemanticProp>>) -> SemanticComponent {
    SemanticComponent { file_path: "/src/Button.tsx".into(), display_name: "Button".to_string(), props }
}

#[test]
fn the_checker_decides_which_props_exist_their_type_and_whether_they_are_required() {
    let mut out = output();
    let outcome = SemanticOutcome {
        components: vec![button(vec![semantic_prop("label", "", false), semantic_prop("extra", "From tsc.", true)])],
        ..Default::default()
    };

    merge_semantic(&mut out, outcome);

    let props = &out.components["Button"].props;
    assert_eq!(props.keys().map(String::as_str).collect::<Vec<_>>(), ["extra", "label"]);
    assert_eq!(props["label"].prop_type, PropType::String);
    assert!(!props["label"].required);
    assert!(props["extra"].required);
}

#[test]
fn a_description_is_the_checkers_else_the_structural_one_and_the_default_and_parent_stay_structural() {
    let mut out = output();
    let outcome = SemanticOutcome {
        components: vec![button(vec![semantic_prop("label", "", true), semantic_prop("size", "Checker size.", true)])],
        ..Default::default()
    };

    merge_semantic(&mut out, outcome);

    let props = &out.components["Button"].props;
    assert_eq!(props["label"].description, "Structural label.");
    assert_eq!(props["size"].description, "Checker size.");
    assert_eq!(props["size"].default_value, Some(DefaultValue { value: "\"sm\"".to_string(), computed: false }));
    assert!(!props["size"].required, "a default value makes the prop optional");
    assert_eq!(props["label"].parent.as_ref().map(|p| p.name.as_str()), Some("Props"));
}

#[test]
fn a_component_the_checker_did_not_answer_for_keeps_its_structural_result() {
    let mut out = output();
    let before = out.components["Card"].clone();

    merge_semantic(&mut out, SemanticOutcome { components: vec![button(vec![])], ..Default::default() });

    assert_eq!(out.components["Card"], before);
}

#[test]
fn provenance_names_the_resolved_components_and_each_fallback_reason() {
    let mut out = output();
    let outcome = SemanticOutcome {
        components: vec![button(vec![])],
        fallbacks: vec![SemanticFallback {
            file_path: "/src/Card.tsx".into(),
            display_name: "Card".to_string(),
            reason: "not exported".to_string(),
        }],
        diagnostics: vec![],
    };

    merge_semantic(&mut out, outcome);

    let summary: Vec<_> =
        out.diagnostics.iter().map(|d| (d.severity.clone(), d.message.as_str(), d.affects.clone())).collect();
    assert_eq!(
        summary,
        [
            (DiagnosticSeverity::Info, "1 component(s) resolved by the TypeScript checker", vec!["Button".to_string()]),
            (
                DiagnosticSeverity::Info,
                "1 component(s) kept their structural result: not exported",
                vec!["Card".to_string()]
            ),
        ]
    );
}

#[test]
fn an_unavailable_backend_changes_nothing_but_adds_one_warning() {
    let mut out = output();
    let before = out.components.clone();

    merge_semantic(&mut out, SemanticOutcome::unavailable("no tsc"));

    assert_eq!(out.components, before);
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].code, DiagnosticCode::SemanticBackend);
    assert_eq!(out.diagnostics[0].severity, DiagnosticSeverity::Warning);
}

#[test]
fn degrade_diagnostics_of_a_resolved_component_are_dropped() {
    let mut out = output();
    let mut stale = Diagnostic::new(DiagnosticSeverity::Warning, DiagnosticCode::OpaqueType, "opaque");
    stale.affects = vec!["Button".to_string()];
    let mut kept = Diagnostic::new(DiagnosticSeverity::Warning, DiagnosticCode::OpaqueType, "other");
    kept.affects = vec!["Card".to_string()];
    out.diagnostics = vec![stale, kept.clone()];

    merge_semantic(&mut out, SemanticOutcome { components: vec![button(vec![])], ..Default::default() });

    assert!(out.diagnostics.contains(&kept));
    assert!(!out.diagnostics.iter().any(|d| d.message == "opaque"));
}

#[test]
fn identical_props_are_shared_between_components_and_a_different_one_is_not() {
    let interner = PropInterner::default();
    let a = interner.share(SemanticProp {
        name: "x".into(),
        prop_type: PropType::String,
        required: true,
        description: String::new(),
        react_declared: false,
    });
    let b = interner.share(SemanticProp {
        name: "x".into(),
        prop_type: PropType::String,
        required: true,
        description: String::new(),
        react_declared: false,
    });
    let c = interner.share(SemanticProp {
        name: "x".into(),
        prop_type: PropType::Number,
        required: true,
        description: String::new(),
        react_declared: false,
    });

    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &c));
}

#[test]
fn merged_props_that_come_out_equal_are_one_allocation() {
    let mut out = output();
    let card = SemanticComponent {
        file_path: "/src/Card.tsx".into(),
        display_name: "Card".to_string(),
        props: vec![semantic_prop("tone", "", true)],
    };
    let tone_on_button = button(vec![semantic_prop("tone", "", true)]);

    merge_semantic(&mut out, SemanticOutcome { components: vec![tone_on_button, card], ..Default::default() });

    assert!(Arc::ptr_eq(&out.components["Button"].props["tone"], &out.components["Card"].props["tone"]));
}
