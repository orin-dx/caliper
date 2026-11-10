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

fn typed(name: &str, prop_type: PropType) -> Arc<SemanticProp> {
    Arc::new(SemanticProp {
        name: name.to_string(),
        prop_type,
        required: false,
        description: String::new(),
        react_declared: false,
    })
}

#[test]
fn a_prop_the_checker_could_not_read_keeps_its_structural_type_and_says_why() {
    let mut out = output();
    let unresolved = unresolved_type("Missing");
    let outcome = SemanticOutcome {
        components: vec![button(vec![typed("label", unresolved), typed("size", PropType::String)])],
        ..Default::default()
    };

    merge_semantic(&mut out, outcome);

    let props = &out.components["Button"].props;
    // `label` is `any` structurally, read in full: the checker's placeholder does not replace it, and it is marked.
    assert_eq!(props["label"].prop_type, PropType::Any);
    assert_eq!(props["label"].degraded_by, ["unresolvableImport"]);
    assert!(!props["label"].required, "optionality is still the checker's");
    assert_eq!(props["size"].prop_type, PropType::String);
    assert!(props["size"].degraded_by.is_empty());
    let unread: Vec<_> = out.diagnostics.iter().filter(|d| d.message.contains("could not read")).collect();
    assert_eq!(unread.len(), 1, "{:?}", out.diagnostics);
    assert!(unread[0].message.starts_with("1 prop(s) of 1 component(s)"), "{}", unread[0].message);
    assert_eq!(unread[0].affects, ["Button"]);
}

#[test]
fn a_checker_type_that_is_only_cut_at_a_limit_or_where_both_are_opaque_is_the_checkers() {
    let mut out = output();
    let opaque = |raw: &str| degraded_type(raw, OpaqueReason::ConditionalType);
    out.components.get_mut("Button").unwrap().props.insert(
        "both".to_string(),
        Arc::new(ParsedProp::new(
            "both".to_string(),
            opaque("Structural<T>"),
            false,
            None,
            String::new(),
            BTreeMap::new(),
            None,
            vec![],
        )),
    );
    let cut = PropType::Union(vec![
        PropType::String,
        PropType::Truncated { text: "Wide".to_string(), limit: crate::types::TruncationLimit::Members { total: 900 } },
    ]);
    let outcome = SemanticOutcome {
        components: vec![button(vec![typed("label", cut.clone()), typed("both", opaque("Checker<T>"))])],
        ..Default::default()
    };

    merge_semantic(&mut out, outcome);

    let props = &out.components["Button"].props;
    assert_eq!(props["label"].prop_type, cut);
    assert_eq!(props["label"].degraded_by, ["truncated"]);
    assert_eq!(props["both"].prop_type, opaque("Checker<T>"));
    assert!(!out.diagnostics.iter().any(|d| d.message.contains("could not read")), "{:?}", out.diagnostics);
}

#[test]
fn a_session_merge_counts_the_props_that_kept_their_structural_type() {
    let mut out = output();
    let entry = out.components.get_mut("Button").unwrap();
    let answer = [typed("label", unresolved_type("Missing")), typed("size", PropType::String)];

    let merged = merge_semantic_props(entry, answer.iter(), &mut FxHashMap::default());

    assert_eq!(merged, MergedProps { kept: 1, unread: 1 });
    assert_eq!(entry.props["label"].prop_type, PropType::Any);
    assert_eq!(entry.props["label"].degraded_by, ["unresolvableImport"]);
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
