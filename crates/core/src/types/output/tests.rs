use super::*;

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
    }]);
    assert_eq!(ty.raw_string(), "{ '--accent': string }");
}

#[test]
fn object_raw_string_quotes_non_identifier_keys_only() {
    let ty = PropType::Object(vec![
        ObjectField { name: "label".into(), prop_type: PropType::String, required: true, description: String::new() },
        ObjectField {
            name: "data-testid".into(),
            prop_type: PropType::String,
            required: false,
            description: String::new(),
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
        let json = pt.to_tagged_value();
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
            ObjectField { name: "a".into(), prop_type: PropType::String, required: true, description: "desc".into() },
            ObjectField { name: "b".into(), prop_type: PropType::Boolean, required: false, description: String::new() },
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
        let wire = pt.to_tagged_value();
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
        props.insert("label".to_string(), prop);

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
        props.insert("label".to_string(), prop);

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
