#![allow(clippy::disallowed_methods, reason = "tests build degraded types directly")]

use super::*;

/// A tuple of required, unlabeled elements.
fn tuple(types: Vec<PropType>) -> PropType {
    PropType::Tuple(types.into_iter().map(TupleElement::required).collect())
}

/// The wire shape built as a `serde_json::Value`, as `Serialize` did before it streamed; tests check the two agree.
fn to_tagged_value(pt: &PropType) -> serde_json::Value {
    match pt {
        // Unit variants (primitives)
        PropType::String => serde_json::json!({"kind": "string"}),
        PropType::Number => serde_json::json!({"kind": "number"}),
        PropType::Boolean => serde_json::json!({"kind": "boolean"}),
        PropType::Null => serde_json::json!({"kind": "null"}),
        PropType::Undefined => serde_json::json!({"kind": "undefined"}),
        PropType::Any => serde_json::json!({"kind": "any"}),
        PropType::Never => serde_json::json!({"kind": "never"}),
        PropType::Unknown => serde_json::json!({"kind": "unknown"}),
        PropType::Void => serde_json::json!({"kind": "void"}),
        PropType::BigInt => serde_json::json!({"kind": "bigint"}),
        PropType::ReactNode => serde_json::json!({"kind": "reactNode"}),
        PropType::CssProperties => serde_json::json!({"kind": "cssProperties"}),
        PropType::ElementType => serde_json::json!({"kind": "elementType"}),
        PropType::SxProps => serde_json::json!({"kind": "sxProps"}),
        // Newtype/tuple variants — give each a real field name instead of a
        // positional "0" key, matching the struct-style variants below.
        PropType::StringLiteral(s) => serde_json::json!({"kind": "stringLiteral", "value": s}),
        PropType::NumberLiteral(n) => {
            // `serde_json::Number` cannot represent NaN/Infinity (they'd
            // silently become JSON `null`, then round-trip back as `0.0`
            // — see `from_tagged_value` below). Tag non-finite values as
            // strings instead so the read side can tell them apart from a
            // genuinely-absent value.
            let value = if n.is_finite() {
                serde_json::json!(n)
            } else if n.is_nan() {
                serde_json::json!("NaN")
            } else if *n > 0.0 {
                serde_json::json!("Infinity")
            } else {
                serde_json::json!("-Infinity")
            };
            serde_json::json!({"kind": "numberLiteral", "value": value})
        }
        PropType::BoolLiteral(b) => serde_json::json!({"kind": "boolLiteral", "value": b}),
        PropType::Union(members) => serde_json::json!({
            "kind": "union",
            "members": members.iter().map(to_tagged_value).collect::<Vec<_>>()
        }),
        PropType::Intersection(members) => serde_json::json!({
            "kind": "intersection",
            "members": members.iter().map(to_tagged_value).collect::<Vec<_>>()
        }),
        PropType::Array(inner) => serde_json::json!({
            "kind": "array",
            "element": to_tagged_value(inner)
        }),
        PropType::Tuple(elements) => serde_json::json!({
            "kind": "tuple",
            "elements": elements.iter().map(|e| {
                let mut element = serde_json::json!({ "type": to_tagged_value(&e.element_type) });
                if let Some(label) = &e.label {
                    element["label"] = serde_json::json!(label.as_str());
                }
                let modifier = match e.kind {
                    TupleElementKind::Required => None,
                    TupleElementKind::Optional => Some("optional"),
                    TupleElementKind::Rest => Some("rest"),
                    TupleElementKind::Variadic => Some("variadic"),
                };
                if let Some(modifier) = modifier {
                    element["modifier"] = serde_json::json!(modifier);
                }
                element
            }).collect::<Vec<_>>()
        }),
        PropType::BigIntLiteral(value) => serde_json::json!({"kind": "bigintLiteral", "value": value.as_str()}),
        PropType::UniqueSymbol { name } => serde_json::json!({"kind": "uniqueSymbol", "name": name.as_str()}),
        PropType::EnumMember { enum_name, member, value } => serde_json::json!({
            "kind": "enumMember",
            "enum": enum_name.as_str(),
            "member": member.as_str(),
            "value": to_tagged_value(&match value {
                EnumMemberValue::String(s) => PropType::StringLiteral(s.clone()),
                EnumMemberValue::Number(n) => PropType::NumberLiteral(*n),
            }),
        }),
        PropType::TemplateLiteral(parts) => serde_json::json!({
            "kind": "templateLiteral",
            "parts": parts.iter().map(|part| match part {
                TemplatePart::Text(text) => serde_json::json!({ "text": text }),
                TemplatePart::Type(ty) => serde_json::json!({ "type": to_tagged_value(ty) }),
            }).collect::<Vec<_>>()
        }),
        PropType::Callable { signatures, fields } => serde_json::json!({
            "kind": "callable",
            "signatures": signatures.iter().map(|s| {
                let function = to_tagged_value(&PropType::Function {
                    type_params: s.type_params.clone(),
                    params: s.params.clone(),
                    return_type: Box::new(s.return_type.clone()),
                });
                let mut signature = serde_json::Map::new();
                if s.kind == SignatureKind::Construct {
                    signature.insert("construct".into(), serde_json::Value::Bool(true));
                }
                for key in ["typeParams", "params", "returnType"] {
                    signature.insert(key.into(), function[key].clone());
                }
                serde_json::Value::Object(signature)
            }).collect::<Vec<_>>(),
            "fields": to_tagged_value(&PropType::Object(fields.clone()))["fields"],
        }),
        PropType::Recursive { name, args } => serde_json::json!({
            "kind": "recursive",
            "name": name.as_str(),
            "args": args.iter().map(to_tagged_value).collect::<Vec<_>>()
        }),
        PropType::Truncated { text, limit } => serde_json::json!({
            "kind": "truncated",
            "text": text,
            "limit": match limit {
                TruncationLimit::Depth => serde_json::json!({"type": "depth"}),
                TruncationLimit::Members { total } => serde_json::json!({"type": "members", "total": total}),
            },
        }),
        PropType::Readonly(inner) => serde_json::json!({
            "kind": "readonly",
            "element": to_tagged_value(inner)
        }),
        PropType::Object(fields) => serde_json::json!({
            "kind": "object",
            "fields": fields.iter().map(|f| {
                let mut field = serde_json::json!({
                    "name": f.name,
                    "propType": to_tagged_value(&f.prop_type),
                    "required": f.required,
                    "description": f.description,
                });
                // Written only when set, so a field without them is the shape it always had.
                if f.readonly {
                    field["readonly"] = serde_json::Value::Bool(true);
                }
                if let Some(key) = &f.index_key {
                    field["indexKey"] = to_tagged_value(key);
                }
                field
            }).collect::<Vec<_>>()
        }),
        // Struct variants — fields merge directly with kind
        PropType::Named { name, args } => serde_json::json!({
            "kind": "named",
            "name": name.as_str(),
            "args": args.iter().map(to_tagged_value).collect::<Vec<_>>()
        }),
        PropType::Function { type_params, params, return_type } => serde_json::json!({
            "kind": "function",
            "typeParams": type_params,
            "params": params.iter().map(|p| {
                let mut param = serde_json::json!({ "name": p.name, "type": to_tagged_value(&p.param_type) });
                // The flags are written only when set, so a signature without them is the shape it always had.
                for (key, set) in [("optional", p.optional), ("rest", p.rest)] {
                    if set {
                        param[key] = serde_json::Value::Bool(true);
                    }
                }
                param
            }).collect::<Vec<_>>(),
            "returnType": to_tagged_value(return_type)
        }),
        PropType::Ref { name, element } => serde_json::json!({
            "kind": "ref",
            "name": name,
            "element": element
        }),
        PropType::HtmlAttributes { element, omitted } => serde_json::json!({
            "kind": "htmlAttributes",
            "element": element,
            "omitted": omitted
        }),
        PropType::LiteralUnion { members, has_default } => serde_json::json!({
            "kind": "literalUnion",
            "members": members,
            "hasDefault": has_default
        }),
        PropType::Opaque(OpaqueDetail { raw, reason }) => {
            let reason_val = match reason {
                OpaqueReason::ConditionalType => serde_json::json!({"type": "conditionalType"}),
                OpaqueReason::MappedType => serde_json::json!({"type": "mappedType"}),
                OpaqueReason::ModuleAugmentation => {
                    serde_json::json!({"type": "moduleAugmentation"})
                }
                OpaqueReason::RuntimeDependent { function_name } => {
                    serde_json::json!({"type": "runtimeDependent", "functionName": function_name})
                }
                OpaqueReason::UnresolvableImport { specifier } => {
                    serde_json::json!({"type": "unresolvableImport", "specifier": specifier})
                }
                OpaqueReason::PandaCodegenMissing => {
                    serde_json::json!({"type": "pandaCodegenMissing"})
                }
                OpaqueReason::DepthExceeded => serde_json::json!({"type": "depthExceeded"}),
                OpaqueReason::IndexedAccess { expression } => {
                    serde_json::json!({"type": "indexedAccess", "expression": expression})
                }
                OpaqueReason::UnsupportedExpression => {
                    serde_json::json!({"type": "unsupportedExpression"})
                }
            };
            serde_json::json!({"kind": "opaque", "raw": raw, "reason": reason_val})
        }
    }
}

#[test]
fn an_operand_is_parenthesized_where_typescript_needs_it() {
    let callback = PropType::Function {
        type_params: vec![],
        params: vec![FunctionParam::new(Some("theme".into()), PropType::Named { name: "Theme".into(), args: vec![] })],
        return_type: Box::new(PropType::CssProperties),
    };
    let either = PropType::Union(vec![PropType::String, PropType::Number]);
    let cases = [
        (
            PropType::Union(vec![PropType::CssProperties, callback.clone()]),
            "CSSProperties | ((theme: Theme) => CSSProperties)",
        ),
        (PropType::Array(Box::new(callback.clone())), "((theme: Theme) => CSSProperties)[]"),
        (PropType::Array(Box::new(either.clone())), "(string | number)[]"),
        (
            PropType::Intersection(vec![either.clone(), PropType::Named { name: "A".into(), args: vec![] }]),
            "(string | number) & A",
        ),
        (PropType::Union(vec![either, PropType::Null]), "string | number | null"),
        (callback, "(theme: Theme) => CSSProperties"),
    ];
    for (ty, expected) in cases {
        assert_eq!(ty.raw_string(), expected);
    }
}

#[test]
fn tuple_raw_string_renders_real_elements_not_a_placeholder() {
    let ty = tuple(vec![PropType::Number, PropType::Number]);
    assert_eq!(ty.raw_string(), "[number, number]");
}

#[test]
fn object_raw_string_renders_real_fields_not_a_placeholder() {
    // Matches the real rdt-compat/types.tsx `tokenStyle` fixture shape:
    // `CSSProperties & { '--accent': string }` — the object side must render
    // its real field, not the literal placeholder "object".
    let ty = PropType::Object(vec![ObjectField {
        name: "--accent".into(),
        prop_type: PropType::String,
        required: true,
        description: String::new(),
        readonly: false,
        index_key: None,
    }]);
    assert_eq!(ty.raw_string(), "{ '--accent': string }");
}

#[test]
fn object_raw_string_quotes_non_identifier_keys_only() {
    let ty = PropType::Object(vec![
        ObjectField {
            name: "label".into(),
            prop_type: PropType::String,
            required: true,
            description: String::new(),
            readonly: false,
            index_key: None,
        },
        ObjectField {
            name: "data-testid".into(),
            prop_type: PropType::String,
            required: false,
            description: String::new(),
            readonly: false,
            index_key: None,
        },
    ]);
    assert_eq!(ty.raw_string(), "{ label: string; 'data-testid'?: string }");
}

#[test]
fn exit_code_is_zero_with_no_diagnostics() {
    let output = ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: vec![],
        stats: ExtractionStats::default(),
    };
    assert_eq!(output.exit_code(false), 0);
    assert_eq!(output.exit_code(true), 0);
}

#[test]
fn exit_code_is_two_when_any_diagnostic_is_error_severity() {
    let output = ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: vec![crate::types::diagnostic::Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "boom".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: crate::types::diagnostic::DiagnosticCode::Unknown,
            affects: Vec::new(),
        }],
        stats: ExtractionStats::default(),
    };
    assert_eq!(output.exit_code(false), 2);
    assert_eq!(output.exit_code(true), 2);
}

#[test]
fn exit_code_is_one_only_when_strict_and_a_warning_is_present() {
    let output = ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: vec![crate::types::diagnostic::Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: "heads up".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: crate::types::diagnostic::DiagnosticCode::Unknown,
            affects: Vec::new(),
        }],
        stats: ExtractionStats::default(),
    };
    assert_eq!(output.exit_code(false), 0, "non-strict must not fail on warnings");
    assert_eq!(output.exit_code(true), 1, "strict must fail on warnings");
}

mod opaque_detail_tests {
    use super::*;
    use crate::types::diagnostic::DiagnosticCode;
    use crate::types::diagnostic::DiagnosticSeverity;
    use crate::types::global::ResolveState;

    #[test]
    fn give_up_pushes_the_diagnostic_and_builds_the_opaque_payload() {
        let mut state = ResolveState::default();
        let diagnostic = Diagnostic {
            severity: DiagnosticSeverity::Info,
            message: "gave up".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::OpaqueType,
            affects: Vec::new(),
        };

        let pt = OpaqueDetail::give_up(&mut state, "SomeType", OpaqueReason::DepthExceeded, diagnostic.clone());

        assert_eq!(state.diagnostics.len(), 1);
        // Full equality, not just .message: the criterion's own stated test
        // contract is "the pushed diagnostic equals what was passed in" —
        // Diagnostic derives PartialEq+Clone, so this costs nothing extra and
        // catches give_up silently mutating severity/code/file/line/column/help
        // before pushing, which a single-field check couldn't.
        assert_eq!(state.diagnostics[0], diagnostic);
        let PropType::Opaque(detail) = &pt else { panic!("expected PropType::Opaque, got {:?}", pt) };
        assert_eq!(detail.raw(), "SomeType");
        assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
    }
}

mod number_literal_wire_tests {
    use super::*;

    #[test]
    fn non_finite_number_literals_are_written_as_strings_and_finite_ones_as_numbers() {
        let wire = |n: f64| serde_json::to_value(PropType::NumberLiteral(n)).expect("serialize")["value"].clone();
        assert_eq!(wire(f64::NAN), "NaN");
        assert_eq!(wire(f64::INFINITY), "Infinity");
        assert_eq!(wire(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(wire(42.5), 42.5);
    }
}

mod opaque_reason_wire_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-005: OpaqueReason's OWN derived serialize output
    // differs from the hand-built {"type": ...} shape PropType::to_tagged_value
    // produces for the same reason — proving to_tagged_value intercepts the
    // reason before serde's derived impl ever runs on a bare OpaqueReason.

    #[test]
    fn opaque_reason_bare_derived_serialization_differs_from_the_intercepted_wire_form() {
        let reason = OpaqueReason::RuntimeDependent { function_name: "getVariant".into() };

        // The bare, derived serde output (serde's default externally-tagged form).
        let bare = serde_json::to_value(&reason).expect("OpaqueReason derives Serialize directly");

        // The wire form actually used when the reason is embedded in an Opaque PropType.
        // OpaqueDetail::new returns PropType::Opaque(..) directly, not a bare OpaqueDetail.
        let pt = OpaqueDetail::new("raw", reason);
        let wire = to_tagged_value(&pt);
        let wire_reason = wire.get("reason").expect("expected a 'reason' field in the tagged Opaque JSON");

        assert_ne!(
            &bare, wire_reason,
            "OpaqueReason's bare derived form must differ from to_tagged_value's hand-built {{\"type\": ...}} \
             shape — if these ever match, to_tagged_value's manual construction has become redundant \
             or, worse, a bare OpaqueReason is leaking through unintercepted"
        );
        assert_eq!(wire_reason["type"], "runtimeDependent");
        assert_eq!(wire_reason["functionName"], "getVariant");
    }
}

mod serde_key_casing_tests {
    use super::*;
    use crate::types::diagnostic::DiagnosticCode;

    // ── SPEC-TYPES-001 AC-012B: Diagnostic's `severity` renders camelCase,
    // `code` renders SCREAMING_SNAKE_CASE — only the `code` half was
    // previously asserted anywhere in this crate.

    #[test]
    fn diagnostic_severity_serializes_as_camel_case() {
        let d = Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "boom".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::Unknown,
            affects: Vec::new(),
        };
        let v = serde_json::to_value(&d).expect("serialize");
        assert_eq!(v["severity"], "error", "expected camelCase 'error', got {:?}", v["severity"]);
    }

    // ── SPEC-TYPES-001 AC-012C: ComponentEntry/ParsedProp field renames —
    // display_name->displayName, file_path->filePath, default_value->defaultValue,
    // and prop_type->"type" specifically (not "propType", which rename_all would
    // otherwise have produced).

    #[test]
    fn component_entry_and_parsed_prop_serialize_with_the_documented_key_renames() {
        let prop = ParsedProp::new(
            "label".to_string(),
            PropType::String,
            true,
            None,
            String::new(),
            Default::default(),
            None,
            vec![],
        );
        let mut props = BTreeMap::new();
        props.insert("label".to_string(), std::sync::Arc::new(prop));

        let entry = ComponentEntry {
            display_name: "Widget".to_string(),
            file_path: "Widget.tsx".into(),
            description: String::new(),
            props,
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
            exports: Vec::new(),
        };

        let v = serde_json::to_value(&entry).expect("serialize");
        assert!(v.get("displayName").is_some(), "expected 'displayName' key, got {v}");
        assert!(v.get("display_name").is_none(), "unexpected snake_case 'display_name' key, got {v}");
        assert!(v.get("filePath").is_some(), "expected 'filePath' key, got {v}");

        let prop_json = &v["props"]["label"];
        assert!(prop_json.get("type").is_some(), "expected the prop_type field under key 'type', got {prop_json}");
        assert!(prop_json.get("propType").is_none(), "prop_type must not serialize as 'propType', got {prop_json}");
    }

    #[test]
    fn parsed_prop_default_value_serializes_as_camel_case() {
        let prop = ParsedProp::new(
            "size".to_string(),
            PropType::String,
            false,
            Some(DefaultValue { value: "md".to_string(), computed: false }),
            String::new(),
            Default::default(),
            None,
            vec![],
        );
        let v = serde_json::to_value(&prop).expect("serialize");
        assert!(v.get("defaultValue").is_some(), "expected 'defaultValue' key, got {v}");
    }
}

mod parsed_prop_mutation_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-014B: ParsedProp::new normalizes required/default_value
    // at construction time only — direct field mutation after construction can
    // still produce the contradictory state. This is the documented, decided
    // exception (see non_goals), pinned here as expected behavior, not a gap.

    #[test]
    fn direct_field_mutation_can_reintroduce_the_contradictory_state() {
        let mut prop = ParsedProp::new(
            "p".to_string(),
            PropType::String,
            true,
            Some(DefaultValue { value: "a".to_string(), computed: false }),
            String::new(),
            Default::default(),
            None,
            vec![],
        );
        assert!(!prop.required, "sanity check: new() should have normalized required to false");

        prop.required = true;

        assert!(prop.required);
        assert!(prop.default_value.is_some());
    }
}

mod is_literal_union_tests {
    use super::*;

    #[test]
    fn empty_literal_union_is_not_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec![], has_default: false };
        assert!(!pt.is_literal_union());
    }

    #[test]
    fn single_member_literal_union_is_treated_as_a_one_option_enum() {
        let pt = PropType::LiteralUnion { members: vec!["only".to_string()], has_default: false };
        assert!(pt.is_literal_union());
        assert!(PropType::StringLiteral("button".into()).is_literal_union());
        assert!(PropType::NumberLiteral(1.0).is_literal_union());
        assert!(PropType::Union(vec![PropType::StringLiteral("x".into())]).is_literal_union());
    }

    #[test]
    fn a_union_storybook_reads_from_its_name_and_an_empty_union_are_not_enums() {
        let mixed = PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::BoolLiteral(true)]);
        assert!(!mixed.is_literal_union());
        // Text Storybook cannot split and parse from the type name keeps the options as an enum.
        let piped = PropType::Union(vec![PropType::StringLiteral("a|b".into()), PropType::BoolLiteral(true)]);
        assert!(piped.is_literal_union());
        assert!(!PropType::BoolLiteral(true).is_literal_union());
        assert!(!PropType::Union(vec![PropType::BoolLiteral(true)]).is_literal_union());
        assert!(!PropType::Union(vec![]).is_literal_union());
        assert!(!PropType::String.is_literal_union());
    }

    #[test]
    fn two_member_literal_union_is_still_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec!["a".to_string(), "b".to_string()], has_default: false };
        assert!(pt.is_literal_union());
    }
}

mod streaming_serialize_tests {
    use super::*;

    /// Every variant, every `OpaqueReason`, and the non-finite numbers.
    fn every_shape() -> Vec<PropType> {
        let reasons = vec![
            OpaqueReason::ConditionalType,
            OpaqueReason::MappedType,
            OpaqueReason::ModuleAugmentation,
            OpaqueReason::RuntimeDependent { function_name: "cva".into() },
            OpaqueReason::UnresolvableImport { specifier: "./missing".into() },
            OpaqueReason::PandaCodegenMissing,
            OpaqueReason::DepthExceeded,
            OpaqueReason::IndexedAccess { expression: "T[\"k\"]".into() },
            OpaqueReason::UnsupportedExpression,
        ];
        let mut shapes = vec![
            PropType::String,
            PropType::Number,
            PropType::Boolean,
            PropType::Null,
            PropType::Undefined,
            PropType::Any,
            PropType::Never,
            PropType::Unknown,
            PropType::Void,
            PropType::BigInt,
            PropType::ReactNode,
            PropType::CssProperties,
            PropType::ElementType,
            PropType::SxProps,
            PropType::StringLiteral("a \"quoted\" value".into()),
            PropType::NumberLiteral(1.5),
            PropType::NumberLiteral(2.0),
            PropType::NumberLiteral(f64::NAN),
            PropType::NumberLiteral(f64::INFINITY),
            PropType::NumberLiteral(f64::NEG_INFINITY),
            PropType::BoolLiteral(true),
            PropType::Union(vec![PropType::String, PropType::Null]),
            PropType::Intersection(vec![PropType::Number, PropType::Unknown]),
            PropType::Array(Box::new(PropType::Boolean)),
            tuple(vec![PropType::Number, PropType::String]),
            tuple(vec![]),
            PropType::Object(vec![ObjectField {
                name: "x".into(),
                prop_type: PropType::Union(vec![PropType::String, PropType::Number]),
                required: true,
                description: "The x.".into(),
                readonly: false,
                index_key: None,
            }]),
            PropType::Named { name: "Box".into(), args: vec![PropType::StringLiteral("div".into())] },
            PropType::Function {
                type_params: vec!["T".into()],
                params: vec![
                    FunctionParam::new(
                        Some("event".into()),
                        PropType::Named { name: "MouseEvent".into(), args: vec![] },
                    ),
                    FunctionParam::new(None, PropType::String),
                ],
                return_type: Box::new(PropType::ReactNode),
            },
            PropType::Function { type_params: vec![], params: vec![], return_type: Box::new(PropType::Void) },
            signature_with_optional_and_rest(),
            PropType::Ref { name: "Ref".into(), element: Some("HTMLDivElement".into()) },
            PropType::Ref { name: "RefObject".into(), element: Some("HTMLDivElement".into()) },
            PropType::Ref { name: "ElementRef".into(), element: None },
            PropType::Readonly(Box::new(PropType::Array(Box::new(PropType::String)))),
            PropType::Readonly(Box::new(tuple(vec![PropType::String, PropType::Number]))),
            PropType::Object(vec![
                ObjectField {
                    name: "id".into(),
                    prop_type: PropType::String,
                    required: true,
                    description: String::new(),
                    readonly: true,
                    index_key: None,
                },
                ObjectField {
                    name: "key".into(),
                    prop_type: PropType::Number,
                    required: true,
                    description: String::new(),
                    readonly: false,
                    index_key: Some(Box::new(PropType::String)),
                },
            ]),
            PropType::HtmlAttributes { element: "button".into(), omitted: vec!["type".into()] },
            PropType::LiteralUnion { members: vec!["sm".into(), "lg".into()], has_default: true },
            every_tuple_element(),
            PropType::BigIntLiteral(BigIntValue::parse("-10").unwrap()),
            PropType::UniqueSymbol { name: "brand".into() },
            size_small(),
            PropType::EnumMember {
                enum_name: "Level".into(),
                member: "High".into(),
                value: EnumMemberValue::Number(2.0),
            },
            pixels(),
            overloads(),
            PropType::Recursive { name: "Json".into(), args: vec![] },
            PropType::Recursive { name: "Tree".into(), args: vec![PropType::String] },
            PropType::Truncated { text: "SystemStyleObject".into(), limit: TruncationLimit::Depth },
            PropType::Truncated { text: "HTMLAttributes".into(), limit: TruncationLimit::Members { total: 880 } },
        ];
        shapes.extend(reasons.into_iter().map(|reason| OpaqueDetail::new("Raw<T>", reason)));
        shapes
    }

    /// `[string, number?, ...rest: boolean[], ...T]`, a label on the second.
    fn every_tuple_element() -> PropType {
        PropType::Tuple(vec![
            TupleElement::required(PropType::String),
            TupleElement {
                kind: TupleElementKind::Optional,
                label: TupleLabel::new("count"),
                ..TupleElement::required(PropType::Number)
            },
            TupleElement {
                kind: TupleElementKind::Rest,
                label: TupleLabel::new("rest"),
                ..TupleElement::required(PropType::Boolean)
            },
            TupleElement {
                kind: TupleElementKind::Variadic,
                ..TupleElement::required(PropType::Named { name: "T".into(), args: vec![] })
            },
        ])
    }

    /// `Size.Small`, whose value is `"small"`.
    fn size_small() -> PropType {
        PropType::EnumMember {
            enum_name: "Size".into(),
            member: "Small".into(),
            value: EnumMemberValue::String("small".into()),
        }
    }

    /// `` `${number}px` ``
    fn pixels() -> PropType {
        PropType::TemplateLiteral(vec![TemplatePart::Type(PropType::Number), TemplatePart::Text("px".into())])
    }

    /// `{ (value: string): string; new (value: number): Date; displayName?: string }`
    fn overloads() -> PropType {
        PropType::Callable {
            signatures: vec![
                CallSignature {
                    kind: SignatureKind::Call,
                    type_params: vec![],
                    params: vec![FunctionParam::new(Some("value".into()), PropType::String)],
                    return_type: PropType::String,
                },
                CallSignature {
                    kind: SignatureKind::Construct,
                    type_params: vec![],
                    params: vec![FunctionParam::new(Some("value".into()), PropType::Number)],
                    return_type: PropType::Named { name: "Date".into(), args: vec![] },
                },
            ],
            fields: vec![ObjectField::new("displayName", PropType::String, false, "")],
        }
    }

    #[test]
    fn each_new_kind_prints_as_typescript_writes_it() {
        let cases = [
            (every_tuple_element(), "[string, count?: number, ...rest: boolean[], ...T]"),
            (PropType::BigInt, "bigint"),
            (PropType::BigIntLiteral(BigIntValue::parse("-10").unwrap()), "-10n"),
            (PropType::UniqueSymbol { name: "brand".into() }, "typeof brand"),
            (size_small(), "Size.Small"),
            (pixels(), "`${number}px`"),
            (overloads(), "{ (value: string): string; new (value: number): Date; displayName?: string }"),
            (PropType::Recursive { name: "Tree".into(), args: vec![PropType::String] }, "Tree<string>"),
            (
                PropType::Truncated { text: "SystemStyleObject".into(), limit: TruncationLimit::Depth },
                "SystemStyleObject",
            ),
        ];
        for (ty, expected) in cases {
            assert_eq!(ty.raw_string(), expected, "{ty:?}");
        }
        // A rest element of a union is parenthesized before its `[]`.
        let rest_union = PropType::Tuple(vec![TupleElement {
            kind: TupleElementKind::Rest,
            ..TupleElement::required(PropType::Union(vec![PropType::String, PropType::Number]))
        }]);
        assert_eq!(rest_union.raw_string(), "[...(string | number)[]]");
    }

    #[test]
    fn each_new_kind_is_written_with_its_fields() {
        let wire = |ty: PropType| serde_json::to_value(ty).expect("serialize");
        assert_eq!(
            wire(every_tuple_element())["elements"],
            serde_json::json!([
                { "type": { "kind": "string" } },
                { "type": { "kind": "number" }, "label": "count", "modifier": "optional" },
                { "type": { "kind": "boolean" }, "label": "rest", "modifier": "rest" },
                { "type": { "kind": "named", "name": "T", "args": [] }, "modifier": "variadic" },
            ])
        );
        assert_eq!(
            wire(size_small()),
            serde_json::json!({
                "kind": "enumMember", "enum": "Size", "member": "Small",
                "value": { "kind": "stringLiteral", "value": "small" },
            })
        );
        assert_eq!(
            wire(pixels()),
            serde_json::json!({ "kind": "templateLiteral", "parts": [{ "type": { "kind": "number" } }, { "text": "px" }] })
        );
        assert_eq!(wire(overloads())["signatures"][1]["construct"], serde_json::json!(true));
        assert_eq!(wire(overloads())["signatures"][0].get("construct"), None);
        assert_eq!(
            wire(PropType::Truncated { text: "A".into(), limit: TruncationLimit::Members { total: 3 } })["limit"],
            serde_json::json!({ "type": "members", "total": 3 })
        );
    }

    #[test]
    fn only_a_truncated_member_degrades_among_the_new_kinds() {
        for shape in [every_tuple_element(), size_small(), pixels(), overloads()] {
            assert!(shape.degraded_by().is_empty(), "{shape:?}");
            assert!(!shape.holds_opaque(), "{shape:?}");
        }
        assert!(PropType::Recursive { name: "Json".into(), args: vec![] }.degraded_by().is_empty());
        let cut = PropType::Array(Box::new(PropType::Truncated { text: "A".into(), limit: TruncationLimit::Depth }));
        assert_eq!(cut.degraded_by(), vec!["truncated"]);
        assert!(cut.holds_opaque(), "the checker may answer a truncated type");
        // Inside a tuple, a template hole and a signature, an opaque member is still found.
        let opaque = || OpaqueDetail::new("X", OpaqueReason::ConditionalType);
        let holders = [
            PropType::Tuple(vec![TupleElement { kind: TupleElementKind::Rest, ..TupleElement::required(opaque()) }]),
            PropType::TemplateLiteral(vec![TemplatePart::Type(opaque())]),
            PropType::Callable {
                signatures: vec![CallSignature {
                    kind: SignatureKind::Call,
                    type_params: vec![],
                    params: vec![],
                    return_type: opaque(),
                }],
                fields: vec![],
            },
        ];
        for holder in holders {
            assert_eq!(holder.degraded_by(), vec!["conditionalType"], "{holder:?}");
            assert!(holder.holds_unevaluated_type(), "{holder:?}");
        }
    }

    #[test]
    fn enum_members_are_options_and_offer_their_values() {
        let large = PropType::EnumMember {
            enum_name: "Size".into(),
            member: "Large".into(),
            value: EnumMemberValue::String("large".into()),
        };
        let union = PropType::Union(vec![size_small(), large]);
        assert!(union.is_literal_union());
        assert_eq!(
            crate::pipeline::rdt::type_json(&union),
            serde_json::json!({
                "name": "enum",
                "raw": "Size.Small | Size.Large",
                "value": [{ "value": "\"small\"" }, { "value": "\"large\"" }],
            })
        );
        // With a boolean the members' names are not JSON, so it stays an enum, as a string with a `|` does.
        assert!(PropType::Union(vec![size_small(), PropType::BoolLiteral(true)]).is_literal_union());
        // A bigint is not an option Storybook can pass back.
        let bigints = PropType::Union(vec![
            PropType::BigIntLiteral(BigIntValue::parse("1").unwrap()),
            PropType::BigIntLiteral(BigIntValue::parse("2").unwrap()),
        ]);
        assert!(!bigints.is_literal_union());
        assert_eq!(crate::pipeline::rdt::type_json(&bigints), serde_json::json!({ "name": "1n | 2n" }));
    }

    #[test]
    fn the_new_kinds_hash_apart() {
        use std::hash::{DefaultHasher, Hasher};
        let hash = |ty: &PropType| {
            let mut hasher = DefaultHasher::new();
            ty.structural_hash(&mut hasher);
            hasher.finish()
        };
        let optional = PropType::Tuple(vec![TupleElement {
            kind: TupleElementKind::Optional,
            ..TupleElement::required(PropType::String)
        }]);
        let labeled = PropType::Tuple(vec![TupleElement {
            label: TupleLabel::new("a"),
            ..TupleElement::required(PropType::String)
        }]);
        let plain = tuple(vec![PropType::String]);
        let named = PropType::Named { name: "Json".into(), args: vec![] };
        let recursive = PropType::Recursive { name: "Json".into(), args: vec![] };
        let hashes = [hash(&optional), hash(&labeled), hash(&plain), hash(&named), hash(&recursive)];
        for (i, a) in hashes.iter().enumerate() {
            for b in &hashes[i + 1..] {
                assert_ne!(a, b, "{hashes:?}");
            }
        }
    }

    /// `(a: string, b?: number, ...rest: boolean[]) => void`
    fn signature_with_optional_and_rest() -> PropType {
        PropType::Function {
            type_params: vec![],
            params: vec![
                FunctionParam::new(Some("a".into()), PropType::String),
                FunctionParam { optional: true, ..FunctionParam::new(Some("b".into()), PropType::Number) },
                FunctionParam {
                    rest: true,
                    ..FunctionParam::new(Some("rest".into()), PropType::Array(Box::new(PropType::Boolean)))
                },
            ],
            return_type: Box::new(PropType::Void),
        }
    }

    #[test]
    fn a_signature_prints_its_optional_and_rest_parameters() {
        assert_eq!(
            signature_with_optional_and_rest().raw_string(),
            "(a: string, b?: number, ...rest: boolean[]) => void"
        );
        let unnamed = PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam { optional: true, ..FunctionParam::new(None, PropType::String) }],
            return_type: Box::new(PropType::Void),
        };
        assert_eq!(unnamed.raw_string(), "(__0?: string) => void");
    }

    #[test]
    fn the_flags_are_written_only_when_set() {
        let json = serde_json::to_value(signature_with_optional_and_rest()).expect("serialize");
        assert_eq!(
            json["params"],
            serde_json::json!([
                { "name": "a", "type": { "kind": "string" } },
                { "name": "b", "type": { "kind": "number" }, "optional": true },
                { "name": "rest", "type": { "kind": "array", "element": { "kind": "boolean" } }, "rest": true },
            ])
        );
        assert_eq!(
            signature_with_optional_and_rest().raw_string(),
            "(a: string, b?: number, ...rest: boolean[]) => void"
        );
    }

    #[test]
    fn signatures_that_differ_only_by_a_flag_do_not_share_a_hash() {
        use std::hash::{DefaultHasher, Hasher};
        let hash = |ty: &PropType| {
            let mut hasher = DefaultHasher::new();
            ty.structural_hash(&mut hasher);
            hasher.finish()
        };
        let required = |optional, rest| PropType::Function {
            type_params: vec![],
            params: vec![FunctionParam { optional, rest, ..FunctionParam::new(Some("a".into()), PropType::String) }],
            return_type: Box::new(PropType::Void),
        };
        let hashes = [hash(&required(false, false)), hash(&required(true, false)), hash(&required(false, true))];
        assert!(hashes[0] != hashes[1] && hashes[0] != hashes[2] && hashes[1] != hashes[2], "{hashes:?}");
    }

    #[test]
    fn streamed_bytes_equal_the_value_built_wire_shape_for_every_variant() {
        for shape in every_shape() {
            let streamed = serde_json::to_string(&shape).expect("serialize");
            let reference = serde_json::to_string(&to_tagged_value(&shape)).expect("serialize value");
            assert_eq!(streamed, reference, "{shape:?}");
        }
    }
}

#[test]
fn opaque_reason_kinds_lists_every_variant_in_declaration_order() {
    // The match has no wildcard arm: a new variant stops compiling here until it is added to `KINDS` and to the samples.
    fn position(reason: &OpaqueReason) -> usize {
        match reason {
            OpaqueReason::ConditionalType => 0,
            OpaqueReason::MappedType => 1,
            OpaqueReason::ModuleAugmentation => 2,
            OpaqueReason::RuntimeDependent { .. } => 3,
            OpaqueReason::UnresolvableImport { .. } => 4,
            OpaqueReason::PandaCodegenMissing => 5,
            OpaqueReason::DepthExceeded => 6,
            OpaqueReason::IndexedAccess { .. } => 7,
            OpaqueReason::UnsupportedExpression => 8,
        }
    }
    let samples = [
        OpaqueReason::ConditionalType,
        OpaqueReason::MappedType,
        OpaqueReason::ModuleAugmentation,
        OpaqueReason::RuntimeDependent { function_name: "cva".into() },
        OpaqueReason::UnresolvableImport { specifier: "@acme/ui".into() },
        OpaqueReason::PandaCodegenMissing,
        OpaqueReason::DepthExceeded,
        OpaqueReason::IndexedAccess { expression: "P[\"a\"]".into() },
        OpaqueReason::UnsupportedExpression,
    ];
    for (index, reason) in samples.iter().enumerate() {
        assert_eq!(position(reason), index);
        assert_eq!(OpaqueReason::KINDS[index], reason.kind());
    }
    assert_eq!(OpaqueReason::KINDS.len(), samples.len());
}
