use super::*;

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
            "members": members.iter().map(|m| to_tagged_value(m)).collect::<Vec<_>>()
        }),
        PropType::Intersection(members) => serde_json::json!({
            "kind": "intersection",
            "members": members.iter().map(|m| to_tagged_value(m)).collect::<Vec<_>>()
        }),
        PropType::Array(inner) => serde_json::json!({
            "kind": "array",
            "element": to_tagged_value(inner)
        }),
        PropType::Tuple(members) => serde_json::json!({
            "kind": "tuple",
            "elements": members.iter().map(|m| to_tagged_value(m)).collect::<Vec<_>>()
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
            "args": args.iter().map(|a| to_tagged_value(a)).collect::<Vec<_>>()
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
                OpaqueReason::TemplateLiteral { expression } => {
                    serde_json::json!({"type": "templateLiteral", "expression": expression})
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
    let ty = PropType::Tuple(vec![PropType::Number, PropType::Number]);
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

    #[test]
    fn opaque_round_trips_through_the_tagged_json_wire_format() {
        let pt = OpaqueDetail::new("A<B> | C", OpaqueReason::UnsupportedExpression);
        let json = to_tagged_value(&pt);
        let restored = PropType::from_tagged_value(&json).expect("should deserialize");
        assert_eq!(pt, restored);
    }
}

mod number_literal_roundtrip_tests {
    use super::*;

    #[test]
    fn nan_number_literal_round_trips_as_nan_not_zero() {
        let original = PropType::NumberLiteral(f64::NAN);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert!(n.is_nan(), "expected NaN to survive the round-trip, got {n}"),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    // ── SPEC-TYPES-001 AC-004: a NaN payload reachable NESTED inside Union,
    // Intersection, Array, Tuple, Object (via ObjectField.prop_type), or
    // Named's args must also survive the round-trip as NaN — AC-006 only
    // proves the top-level case; this covers every position the criterion
    // explicitly enumerates in one fixture rather than leaving them unasserted.

    #[test]
    fn nan_number_literal_survives_the_round_trip_when_nested() {
        let original = PropType::Union(vec![
            PropType::Intersection(vec![PropType::NumberLiteral(f64::NAN), PropType::String]),
            PropType::Array(Box::new(PropType::NumberLiteral(f64::NAN))),
            PropType::Tuple(vec![PropType::NumberLiteral(f64::NAN), PropType::String]),
            PropType::Object(vec![ObjectField {
                name: "n".into(),
                prop_type: PropType::NumberLiteral(f64::NAN),
                required: true,
                description: String::new(),
                readonly: false,
                index_key: None,
            }]),
            PropType::Named { name: "Foo".into(), args: vec![PropType::NumberLiteral(f64::NAN)] },
        ]);

        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        let PropType::Union(members) = &restored else { panic!("expected Union, got {restored:?}") };

        let PropType::Intersection(items) = &members[0] else { panic!("expected Intersection, got {:?}", members[0]) };
        assert!(
            matches!(items[0], PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Intersection did not survive, got {:?}",
            items[0]
        );

        let PropType::Array(inner) = &members[1] else { panic!("expected Array, got {:?}", members[1]) };
        assert!(
            matches!(**inner, PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Array did not survive, got {inner:?}"
        );

        let PropType::Tuple(items) = &members[2] else { panic!("expected Tuple, got {:?}", members[2]) };
        assert!(
            matches!(items[0], PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Tuple did not survive, got {:?}",
            items[0]
        );

        let PropType::Object(fields) = &members[3] else { panic!("expected Object, got {:?}", members[3]) };
        assert!(
            matches!(fields[0].prop_type, PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Object's ObjectField.prop_type did not survive, got {:?}",
            fields[0].prop_type
        );

        let PropType::Named { args, .. } = &members[4] else { panic!("expected Named, got {:?}", members[4]) };
        assert!(
            matches!(args[0], PropType::NumberLiteral(n) if n.is_nan()),
            "NaN nested in Named's args did not survive, got {:?}",
            args[0]
        );
    }

    #[test]
    fn infinity_number_literal_round_trips_as_infinity() {
        let original = PropType::NumberLiteral(f64::INFINITY);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert_eq!(n, f64::INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn finite_number_literal_still_round_trips_normally() {
        let original = PropType::NumberLiteral(42.5);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        assert_eq!(restored, PropType::NumberLiteral(42.5));
    }

    // ── SPEC-TYPES-001 AC-007: PropType::NumberLiteral(NEG_INFINITY) must also
    // round-trip exactly — only +INFINITY was previously tested, an asymmetry
    // against CollectedType's own test for the negative case.

    #[test]
    fn neg_infinity_number_literal_round_trips_as_neg_infinity() {
        let original = PropType::NumberLiteral(f64::NEG_INFINITY);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert_eq!(n, f64::NEG_INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }
}

mod prop_type_composite_roundtrip_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-004: manual serde impls round-trip nested/composite
    // PropType shapes, not just the leaf Opaque/NumberLiteral cases.

    #[test]
    fn union_of_composite_members_round_trips_exactly() {
        let original = PropType::Union(vec![
            PropType::StringLiteral("a".into()),
            PropType::Array(Box::new(PropType::Number)),
            PropType::Named { name: "Foo".into(), args: vec![PropType::String] },
        ]);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    #[test]
    fn object_with_fields_round_trips_exactly() {
        let original = PropType::Object(vec![
            ObjectField {
                name: "a".into(),
                prop_type: PropType::String,
                required: true,
                description: "desc".into(),
                readonly: false,
                index_key: None,
            },
            ObjectField {
                name: "b".into(),
                prop_type: PropType::Boolean,
                required: false,
                description: String::new(),
                readonly: false,
                index_key: None,
            },
        ]);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    #[test]
    fn tuple_and_named_with_args_round_trip_exactly() {
        let original = PropType::Tuple(vec![PropType::String, PropType::Named { name: "Bar".into(), args: vec![] }]);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    // ── SPEC-TYPES-001 AC-004C: from_tagged_value returns Err when "kind" is
    // absent or present-but-not-a-string.

    #[test]
    fn from_tagged_value_errs_on_missing_kind() {
        let v = serde_json::json!({"notKind": "string"});
        assert!(PropType::from_tagged_value(&v).is_err());
    }

    #[test]
    fn from_tagged_value_errs_on_non_string_kind() {
        let v = serde_json::json!({"kind": 42});
        assert!(PropType::from_tagged_value(&v).is_err());
    }

    // ── SPEC-TYPES-001 AC-004C2: an unrecognized "kind" degrades to
    // Opaque(DepthExceeded) with the raw string naming the bad tag, not an Err.

    #[test]
    fn from_tagged_value_unrecognized_kind_degrades_to_opaque() {
        let v = serde_json::json!({"kind": "bogus"});
        let pt = PropType::from_tagged_value(&v).expect("unrecognized kind should degrade, not error");
        match pt {
            PropType::Opaque(detail) => {
                assert_eq!(detail.raw(), "unknown PropType kind: bogus");
                assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
            }
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

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

mod component_entry_roundtrip_tests {
    use super::*;

    // ── SPEC-TYPES-001 AC-014: ComponentEntry derives both serde traits
    // directly (no manual impl) — there was previously NO round-trip test for
    // ComponentEntry at all.

    #[test]
    fn component_entry_round_trips_through_json() {
        let prop = ParsedProp::new(
            "label".to_string(),
            PropType::String,
            true,
            None,
            "the label".to_string(),
            Default::default(),
            None,
            vec![],
        );
        let mut props = BTreeMap::new();
        props.insert("label".to_string(), std::sync::Arc::new(prop));

        let original = ComponentEntry {
            display_name: "Widget".to_string(),
            file_path: "Widget.tsx".into(),
            description: "A widget".to_string(),
            props,
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: Some("variant".to_string()),
            composes: vec!["Base".to_string()],
            tags: Default::default(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
        };

        let json = serde_json::to_value(&original).expect("serialize");
        let restored: ComponentEntry = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }
}

mod parsed_prop_mutation_and_deserialize_escape_hatch_tests {
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

    // ── SPEC-TYPES-001 AC-014C: the derived Deserialize impl (with _seal
    // skipped) performs no normalization — deserializing a hand-crafted JSON
    // object with required:true and a non-null defaultValue produces the
    // contradictory state directly.

    #[test]
    fn derived_deserialize_does_not_normalize_required_and_default_value() {
        let json = serde_json::json!({
            "name": "p",
            "type": {"kind": "string"},
            "required": true,
            "defaultValue": {"value": "a", "computed": false},
            "description": "",
            "tags": {},
            "parent": null,
            "declarations": []
        });

        let prop: ParsedProp = serde_json::from_value(json).expect("deserialize");
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
    fn single_member_literal_union_is_not_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec!["only".to_string()], has_default: false };
        assert!(!pt.is_literal_union());
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
            OpaqueReason::TemplateLiteral { expression: "`a-${B}`".into() },
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
            PropType::Tuple(vec![PropType::Number, PropType::String]),
            PropType::Tuple(vec![]),
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
            PropType::Readonly(Box::new(PropType::Tuple(vec![PropType::String, PropType::Number]))),
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
        ];
        shapes.extend(reasons.into_iter().map(|reason| OpaqueDetail::new("Raw<T>", reason)));
        shapes
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
    fn the_flags_are_written_only_when_set_and_read_back() {
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
            PropType::from_tagged_value(&json).map(|p| p.raw_string()),
            Ok("(a: string, b?: number, ...rest: boolean[]) => void".to_owned())
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
            assert_eq!(
                PropType::from_tagged_value(&serde_json::from_str(&streamed).unwrap()).map(|p| format!("{p:?}")),
                Ok(format!("{shape:?}"))
            );
        }
    }
}
