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
        exports: vec![ComponentExport::named(name)],
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
        tags: Default::default(),
        parent: None,
        declarations: Vec::new(),
    })
}

fn button(props: Vec<Arc<SemanticProp>>) -> SemanticComponent {
    SemanticComponent { key: "Button".to_string(), props, composes: Some(Vec::new()) }
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
fn a_description_is_the_checkers_else_the_structural_one_and_so_is_a_default_the_source_does_not_give() {
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
}

fn declared(name: &str, parent: Option<&str>, tags: &[(&str, &str)]) -> Arc<SemanticProp> {
    let parent = parent.map(|parent| PropParent { name: parent.to_string(), file_name: "/src/Base.ts".to_string() });
    Arc::new(SemanticProp {
        tags: tags.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect(),
        declarations: parent.iter().cloned().collect(),
        parent,
        ..(*semantic_prop(name, "", true)).clone()
    })
}

#[test]
fn parent_and_declarations_are_the_checkers_even_where_it_names_none() {
    let mut out = output();
    let outcome = SemanticOutcome {
        components: vec![button(vec![declared("label", Some("BaseProps"), &[]), declared("size", None, &[])])],
        ..Default::default()
    };

    merge_semantic(&mut out, outcome);

    let props = &out.components["Button"].props;
    let base = PropParent { name: "BaseProps".to_string(), file_name: "/src/Base.ts".to_string() };
    assert_eq!(props["label"].parent, Some(base.clone()));
    assert_eq!(props["label"].declarations, [base]);
    // The structural pass named `Props` for `size`; the checker found it in a type literal, which has no name.
    assert_eq!(props["size"].parent, None);
    assert_eq!(props["size"].declarations, []);
}

#[test]
fn tags_are_the_checkers_else_the_structural_ones() {
    let mut out = output();
    let tagged = |name: &str, tags: &[(&str, &str)]| {
        Arc::new(ParsedProp::new(
            name.to_string(),
            PropType::Any,
            true,
            None,
            String::new(),
            tags.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect(),
            None,
            vec![],
        ))
    };
    let props = &mut out.components.get_mut("Button").unwrap().props;
    props.insert("label".to_string(), tagged("label", &[("since", "1.0")]));
    props.insert("size".to_string(), tagged("size", &[("since", "1.0")]));
    let outcome = SemanticOutcome {
        components: vec![button(vec![
            declared("label", None, &[("deprecated", "use tone")]),
            declared("size", None, &[]),
        ])],
        ..Default::default()
    };

    merge_semantic(&mut out, outcome);

    let props = &out.components["Button"].props;
    let tags = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect()
    };
    assert_eq!(props["label"].tags, tags(&[("deprecated", "use tone")]));
    assert_eq!(props["size"].tags, tags(&[("since", "1.0")]));
}

#[test]
fn composes_is_the_checkers_in_the_whole_project_merge_and_in_a_session_merge() {
    let mut out = output();
    out.components.get_mut("Button").unwrap().composes = vec!["AriaLabelingProps".to_string()];
    let mut answered = button(vec![semantic_prop("label", "", true)]);
    answered.composes = Some(vec!["RefAttributes".to_string()]);

    merge_semantic(&mut out, SemanticOutcome { components: vec![answered.clone()], ..Default::default() });
    assert_eq!(out.components["Button"].composes, ["RefAttributes"]);

    let mut session = output();
    let entry = session.components.get_mut("Button").unwrap();
    entry.composes = vec!["AriaLabelingProps".to_string()];
    merge_semantic_props(entry, answered.props.iter(), answered.composes.as_deref(), &mut FxHashMap::default());
    assert_eq!(entry.composes, ["RefAttributes"]);
}

#[test]
fn composes_the_checker_could_not_decide_keeps_the_structural_one_and_a_decided_empty_one_replaces_it() {
    let structural = vec!["AriaLabelingProps".to_string()];
    for (checker, expected) in [(None, structural.clone()), (Some(Vec::new()), Vec::new())] {
        let mut answered = button(vec![semantic_prop("label", "", true)]);
        answered.composes = checker;

        let mut out = output();
        out.components.get_mut("Button").unwrap().composes = structural.clone();
        merge_semantic(&mut out, SemanticOutcome { components: vec![answered.clone()], ..Default::default() });
        assert_eq!(out.components["Button"].composes, expected, "whole-project merge of {:?}", answered.composes);

        let mut session = output();
        let entry = session.components.get_mut("Button").unwrap();
        entry.composes = structural.clone();
        merge_semantic_props(entry, answered.props.iter(), answered.composes.as_deref(), &mut FxHashMap::default());
        assert_eq!(entry.composes, expected, "session merge of {:?}", answered.composes);
    }
}

fn typed(name: &str, prop_type: PropType) -> Arc<SemanticProp> {
    Arc::new(SemanticProp {
        name: name.to_string(),
        prop_type,
        required: false,
        description: String::new(),
        react_declared: false,
        tags: Default::default(),
        parent: None,
        declarations: Vec::new(),
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

    let merged = merge_semantic_props(entry, answer.iter(), Some(&[]), &mut FxHashMap::default());

    assert_eq!(merged, MergedProps { kept: 1, unread: 1, diagnostics: Vec::new() });
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
        fallbacks: vec![SemanticFallback { key: "Card".to_string(), reason: "not exported".to_string() }],
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
        tags: Default::default(),
        parent: None,
        declarations: Vec::new(),
    });
    let b = interner.share(SemanticProp {
        name: "x".into(),
        prop_type: PropType::String,
        required: true,
        description: String::new(),
        react_declared: false,
        tags: Default::default(),
        parent: None,
        declarations: Vec::new(),
    });
    let c = interner.share(SemanticProp {
        name: "x".into(),
        prop_type: PropType::Number,
        required: true,
        description: String::new(),
        react_declared: false,
        tags: Default::default(),
        parent: None,
        declarations: Vec::new(),
    });

    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &c));
}

#[test]
fn merged_props_that_come_out_equal_are_one_allocation() {
    let mut out = output();
    let card = SemanticComponent {
        key: "Card".to_string(),
        props: vec![semantic_prop("tone", "", true)],
        composes: Some(Vec::new()),
    };
    let tone_on_button = button(vec![semantic_prop("tone", "", true)]);

    merge_semantic(&mut out, SemanticOutcome { components: vec![tone_on_button, card], ..Default::default() });

    assert!(Arc::ptr_eq(&out.components["Button"].props["tone"], &out.components["Card"].props["tone"]));
}

/// Two components can share a file and a display name (a `displayName` assignment, or the same name in two scopes); the
/// answer for one must not land on the other.
#[test]
fn an_answer_merges_into_the_entry_its_key_names_not_the_first_with_the_same_file_and_name() {
    let mut out = output();
    let mut twin = out.components["Card"].clone();
    twin.exports = vec![ComponentExport::named("default").member("Card")];
    out.components.insert("Card~2".to_string(), twin);
    let before = out.components["Card"].clone();

    let answer = SemanticComponent {
        key: "Card~2".to_string(),
        props: vec![semantic_prop("tone", "", true)],
        composes: Some(Vec::new()),
    };
    let fallback = SemanticFallback { key: "Card".to_string(), reason: "why".to_string() };
    merge_semantic(
        &mut out,
        SemanticOutcome { components: vec![answer], fallbacks: vec![fallback], ..Default::default() },
    );

    assert_eq!(out.components["Card"], before);
    assert_eq!(out.components["Card~2"].props.keys().collect::<Vec<_>>(), ["tone"]);
    let affected: Vec<(&str, Vec<String>)> =
        out.diagnostics.iter().map(|d| (d.message.as_str(), d.affects.clone())).collect();
    assert_eq!(
        affected,
        [
            ("1 component(s) resolved by the TypeScript checker", vec!["Card~2".to_string()]),
            ("1 component(s) kept their structural result: why", vec!["Card".to_string()]),
        ]
    );
}

#[test]
fn without_the_resolver_a_default_and_a_union_order_come_from_the_syntax_and_the_checker_alone() {
    let lit = |value: &str| PropType::StringLiteral(value.to_string());
    let declared = PropType::Union(vec![lit("sm"), lit("lg")]);
    let sorted = PropType::Union(vec![lit("lg"), lit("sm")]);
    let run = |resolver: ResolverFacts, tags: &[(&str, &str)]| {
        let mut out = output();
        let entry = out.components.get_mut("Button").unwrap();
        Arc::make_mut(entry.props.get_mut("size").unwrap()).prop_type = declared.clone();
        let mut size = (*typed("size", sorted.clone())).clone();
        size.tags = tags.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect();
        let outcome = SemanticOutcome { components: vec![button(vec![Arc::new(size)])], ..Default::default() };
        // `/src/Button.tsx` does not exist: the syntax has nothing to say.
        merge_semantic_with(&mut out, outcome, resolver);
        let size = out.components["Button"].props["size"].clone();
        (size.prop_type.clone(), size.default_value.as_ref().map(|default| default.value.clone()))
    };

    assert_eq!(run(ResolverFacts::Used, &[]), (declared.clone(), Some("\"sm\"".to_string())));
    assert_eq!(run(ResolverFacts::Off, &[]), (sorted.clone(), None));
    assert_eq!(run(ResolverFacts::Off, &[("default", " 'lg' ")]), (sorted.clone(), Some("'lg'".to_string())));
    assert_eq!(run(ResolverFacts::Used, &[("defaultValue", "'lg'")]), (declared.clone(), Some("'lg'".to_string())));
}
