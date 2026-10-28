use super::*;
use std::sync::Arc;

// ── Helpers ───────────────────────────────────────────────────────────────

fn empty_ctx() -> ResolutionContext {
    let global = Arc::new(GlobalSourceData::default());
    let options = PipelineOptions::default();
    ResolutionContext::new(global, &options)
}

fn resolve_type(ct: &CollectedType, ctx: &ResolutionContext) -> PropType {
    use super::collected::resolve_collected_type;
    let mut state = ResolveState::default();
    resolve_collected_type(ct, Utf8Path::new("/test/button.tsx"), ctx, &mut state, 0)
}

// ── Test 1: Simple literal union ──────────────────────────────────────────

#[test]
fn test_string_literal_union() {
    let ctx = empty_ctx();
    let ct = CollectedType::Union(vec![
        CollectedType::StringLiteral("default".into()),
        CollectedType::StringLiteral("destructive".into()),
        CollectedType::StringLiteral("outline".into()),
    ]);
    let result = resolve_type(&ct, &ctx);
    assert!(
        matches!(&result, PropType::Union(members) if members.len() == 3),
        "Expected Union with 3 members, got {:?}",
        result
    );
}

// ── Test 2: Simple interface resolution ───────────────────────────────────

#[test]
fn test_simple_interface_resolution() {
    let file_path = Utf8PathBuf::from("/test/types.ts");
    let scoped_key = format!("{}:ButtonProps", file_path);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        scoped_key.clone(),
        CollectedInterface {
            scoped_key: scoped_key.clone(),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "variant".into(),
                collected_type: CollectedType::Union(vec![
                    CollectedType::StringLiteral("default".into()),
                    CollectedType::StringLiteral("destructive".into()),
                ]),
                required: false,
                description: "The visual variant".into(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);
    // The unresolvable import diagnostic fires because ButtonProps is in the same file
    // but we do find it via scoped_key lookup.
    let variant_prop = entry.props.get("variant").expect("variant prop should exist");
    assert!(matches!(&variant_prop.prop_type, PropType::Union(m) if m.len() == 2));
    let _ = diagnostics; // may have info diagnostics
}

// ── Test 3: (string & {}) normalization ───────────────────────────────────

#[test]
fn test_string_and_empty_object_normalizes_to_string() {
    let ctx = empty_ctx();
    // (string & {}) → PropType::String
    let ct = CollectedType::Intersection(vec![CollectedType::String, CollectedType::Object(vec![])]);
    let result = resolve_type(&ct, &ctx);
    assert_eq!(result, PropType::String, "Expected String, got {:?}", result);
}

// ── Test 4: Known pattern - SxProps ──────────────────────────────────────

#[test]
fn test_sx_props_known_pattern() {
    let ctx = empty_ctx();
    // No shortcut (precedence): without MUI's declaration it stays the named reference RDT also prints.
    let ct = CollectedType::Named { name: "SxProps".into(), args: vec![] };
    let result = resolve_type(&ct, &ctx);
    assert_eq!(result, PropType::Named { name: "SxProps".into(), args: vec![] });
}

// ── Test: P0-1 regression — a project-defined SxProps extended via
// `extends` must not be silently replaced by the hardcoded MUI shortcut ────
// chain.rs's extends-clause path (reached via ExtendsRef::SameFile /
// ExtendsRef::Imported) independently reimplemented named.rs's "source
// before known-pattern" order and got it backwards — checking the known
// SxProps shortcut before ever looking at the project's own `interface
// SxProps`. This is the confirmed, demonstrable bug from the original audit.

#[test]
fn test_extends_clause_prefers_project_defined_sx_props_over_known_shortcut() {
    let file_path = Utf8PathBuf::from("/test/theme-button.tsx");

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        format!("{}:SxProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:SxProps", file_path),
            name: "SxProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "customSx".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:ThemeButtonProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:ThemeButtonProps", file_path),
            name: "ThemeButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::SameFile { name: "SxProps".into(), type_args: vec![] }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "ThemeButton".into(),
        "ThemeButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        entry.props.contains_key("customSx"),
        "Expected the project's own SxProps interface's 'customSx' field to be inherited, \
         got props {:?} — this means the hardcoded MUI SxProps known-pattern shortcut won \
         instead of the project's own source-defined interface",
        entry.props.keys().collect::<Vec<_>>()
    );
}

// ── SPEC-RESOLVER-001 AC-1b: the same source-before-known precedence
// applies at the resolve_named call site (a field's own type reference),
// not just chain.rs's extends-clause path tested above ──────────────────

#[test]
fn test_field_typed_as_project_defined_sx_props_is_not_replaced_by_known_shortcut() {
    let file_path = Utf8PathBuf::from("/test/widget.tsx");

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        format!("{}:SxProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:SxProps", file_path),
            name: "SxProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "customSx".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:WidgetProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:WidgetProps", file_path),
            name: "WidgetProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "sx".into(),
                collected_type: CollectedType::Named { name: "SxProps".into(), args: vec![] },
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "WidgetProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    let sx_prop = entry.props.get("sx").expect("expected a resolved 'sx' prop");
    assert!(
        matches!(&sx_prop.prop_type, PropType::Named { name, .. } if name == "SxProps"),
        "expected the field's type to resolve to the project's own SxProps interface \
         (PropType::Named), got {:?} — this means resolve_named's known-pattern shortcut \
         won instead of the project's own source-defined interface",
        sx_prop.prop_type
    );
}

// ── Test 5: Indexed access - CSSProperties ────────────────────────────────

#[test]
fn test_indexed_access_css_properties_string_key() {
    let ctx = empty_ctx();
    let ct = CollectedType::IndexedAccess {
        obj: Box::new(CollectedType::Named { name: "CSSProperties".into(), args: vec![] }),
        key: Box::new(CollectedType::StringLiteral("justifyContent".into())),
    };
    let result = resolve_type(&ct, &ctx);
    assert_eq!(result, PropType::String, "Expected String for CSSProperties[string key], got {:?}", result);
}

#[test]
fn test_indexed_access_css_properties_numeric_key() {
    let ctx = empty_ctx();
    let ct = CollectedType::IndexedAccess {
        obj: Box::new(CollectedType::Named { name: "CSSProperties".into(), args: vec![] }),
        key: Box::new(CollectedType::StringLiteral("zIndex".into())),
    };
    let result = resolve_type(&ct, &ctx);
    assert_eq!(result, PropType::Number, "Expected Number for CSSProperties[zIndex], got {:?}", result);
}

// ── Test 5b: Indexed access into a generic interface's own field, with the
// call site's concrete type argument substituted in ─────────────────────
// Regression test for: react-final-form's
// `RenderableProps<FieldRenderProps<FieldValue, T>>["children"]` degraded to
// Opaque — `resolve_indexed_access`'s generic fallback only handled a type
// ALIAS wrapping an inline object literal (resolves to `PropType::Object`),
// never an INTERFACE (which resolves to a bare `PropType::Named` at the type
// level, so the fallback's `if let PropType::Object(fields) = ...` never matched).

#[test]
fn test_indexed_access_into_generic_interface_field_substitutes_type_arg() {
    let file_path = Utf8PathBuf::from("/test/button.tsx");
    let mut global = GlobalSourceData::default();

    global.interfaces.insert(
        format!("{}:RenderableProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:RenderableProps", file_path),
            name: "RenderableProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "children".into(),
                collected_type: CollectedType::Named { name: "T".into(), args: vec![] },
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interface_type_params.insert(format!("{}:RenderableProps", file_path), vec!["T".into()]);

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let ct = CollectedType::IndexedAccess {
        obj: Box::new(CollectedType::Named { name: "RenderableProps".into(), args: vec![CollectedType::String] }),
        key: Box::new(CollectedType::StringLiteral("children".into())),
    };
    let result = resolve_type(&ct, &ctx);
    assert_eq!(
        result,
        PropType::String,
        "Expected T substituted with the call site's String argument, got {:?}",
        result
    );
}

// ── Test 5c: Indexed access into an ambient DOM interface, walking its
// extends chain ───────────────────────────────────────────────────────────
// Regression test for: day-picker's `HTMLDivElement["dir"]` (also "nonce",
// "title", "lang") degraded to Opaque. `dir` is not declared directly on
// `HTMLDivElement` — it's inherited via `extends HTMLElement`, and both
// interfaces are ambient (declared in TypeScript's own `lib.dom.d.ts`, never
// imported by the consuming file). `resolve_indexed_access`'s interface
// lookup only checked the consuming file's own imports/declarations (never
// `ctx.ambient_global_files`), and only searched an interface's own `props`
// (never its `extends` chain).

#[test]
fn test_indexed_access_into_ambient_dom_interface_walks_extends_chain() {
    let lib_file = Utf8PathBuf::from("/fake/lib.dom.d.ts");
    let mut global = GlobalSourceData::default();

    global.interfaces.insert(
        format!("{}:HTMLDivElement", lib_file),
        CollectedInterface {
            scoped_key: format!("{}:HTMLDivElement", lib_file),
            name: "HTMLDivElement".into(),
            file_path: lib_file.clone(),
            props: vec![],
            extends: vec![ExtendsRef::SameFile { name: "HTMLElement".into(), type_args: vec![] }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:HTMLElement", lib_file),
        CollectedInterface {
            scoped_key: format!("{}:HTMLElement", lib_file),
            name: "HTMLElement".into(),
            file_path: lib_file.clone(),
            props: vec![RawProp {
                name: "dir".into(),
                collected_type: CollectedType::String,
                required: true,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let mut ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    ctx.ambient_global_files = vec![lib_file];

    let ct = CollectedType::IndexedAccess {
        obj: Box::new(CollectedType::Named { name: "HTMLDivElement".into(), args: vec![] }),
        key: Box::new(CollectedType::StringLiteral("dir".into())),
    };
    let result = resolve_type(&ct, &ctx);
    assert_eq!(result, PropType::String, "Expected String for HTMLDivElement[\"dir\"], got {:?}", result);
}

// ── Test 5d: Indexed access via `(typeof X)[number]` on a flat
// `const X = [...] as const` array ───────────────────────────────────────
// Regression test for: antd's `type ButtonType = (typeof _ButtonTypes)[number]`
// degraded to Opaque — nothing captured plain `as const` array literals
// (only cva/tv's `variants` object shape), and indexed access never handled
// a `[number]` key (a `CollectedType::Number`, not a string literal).

#[test]
fn test_indexed_access_typeof_number_on_const_array_builds_literal_union() {
    let file_path = Utf8PathBuf::from("/test/buttonHelpers.tsx");
    let mut global = GlobalSourceData::default();

    global.const_arrays.insert(
        format!("{}:_ButtonTypes", file_path),
        vec![
            EnumValue::String("default".into()),
            EnumValue::String("primary".into()),
            EnumValue::String("dashed".into()),
        ],
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let ct = CollectedType::IndexedAccess {
        obj: Box::new(CollectedType::TypeOf("_ButtonTypes".into())),
        key: Box::new(CollectedType::Number),
    };
    let result = resolve_type(&ct, &ctx);
    assert_eq!(
        result,
        PropType::LiteralUnion {
            members: vec!["default".into(), "primary".into(), "dashed".into()],
            has_default: false,
        },
        "Expected LiteralUnion of the array's values, got {:?}",
        result
    );
}

// ── Test 6: Primitives pass through ──────────────────────────────────────

#[test]
fn test_primitive_types() {
    let ctx = empty_ctx();
    assert_eq!(resolve_type(&CollectedType::String, &ctx), PropType::String);
    assert_eq!(resolve_type(&CollectedType::Number, &ctx), PropType::Number);
    assert_eq!(resolve_type(&CollectedType::Boolean, &ctx), PropType::Boolean);
    assert_eq!(resolve_type(&CollectedType::Null, &ctx), PropType::Null);
    assert_eq!(resolve_type(&CollectedType::Undefined, &ctx), PropType::Undefined);
    assert_eq!(resolve_type(&CollectedType::Any, &ctx), PropType::Any);
    assert_eq!(resolve_type(&CollectedType::Never, &ctx), PropType::Never);
    assert_eq!(resolve_type(&CollectedType::Unknown, &ctx), PropType::Unknown);
    assert_eq!(resolve_type(&CollectedType::Void, &ctx), PropType::Void);
}

// ── Test 7: React builtins ────────────────────────────────────────────────

#[test]
fn test_react_node() {
    let ctx = empty_ctx();
    let ct = CollectedType::Named { name: "ReactNode".into(), args: vec![] };
    assert_eq!(resolve_type(&ct, &ctx), PropType::ReactNode);
}

#[test]
fn test_css_properties() {
    let ctx = empty_ctx();
    let ct = CollectedType::Named { name: "CSSProperties".into(), args: vec![] };
    assert_eq!(resolve_type(&ct, &ctx), PropType::CssProperties);
}

#[test]
fn test_ref_type() {
    let ctx = empty_ctx();
    let ct = CollectedType::Named {
        name: "Ref".into(),
        args: vec![CollectedType::Named { name: "HTMLButtonElement".into(), args: vec![] }],
    };
    let result = resolve_type(&ct, &ctx);
    assert!(
        matches!(&result, PropType::Ref { element: Some(e) } if e == "HTMLButtonElement"),
        "Expected Ref<HTMLButtonElement>, got {:?}",
        result
    );
}

#[test]
fn test_less_common_event_handler_names_recognized_as_builtin() {
    // @types/react defines these identically to the ~19 *EventHandler names
    // already recognized (`type XEventHandler<T> = EventHandler<XEvent<T>>`) —
    // found missing from is_react_builtin while investigating whether fully
    // resolving @types/react's real HTMLAttributes/DOMAttributes chain was
    // tractable. Without this, each one falls through to same-file/imported
    // type resolution, fails (they're real @types/react types, not local), and
    // degrades to opaque. A handler type keeps its name, as TypeScript prints it.
    let ctx = empty_ctx();
    for handler_name in ["ReactEventHandler", "SubmitEventHandler", "InputEventHandler", "ToggleEventHandler"] {
        let ct = CollectedType::Named {
            name: handler_name.into(),
            args: vec![CollectedType::Named { name: "HTMLDivElement".into(), args: vec![] }],
        };
        let result = resolve_type(&ct, &ctx);
        assert_eq!(result.raw_string(), format!("{handler_name}<HTMLDivElement>"));
    }
}

#[test]
fn test_element_type() {
    let ctx = empty_ctx();
    let ct = CollectedType::Named { name: "ElementType".into(), args: vec![] };
    assert_eq!(resolve_type(&ct, &ctx), PropType::ElementType);
}

// ── Test 8: Default value code vs JSDoc mismatch ─────────────────────────

#[test]
fn test_default_value_jsdoc_mismatch_emits_diagnostic() {
    let file_path = Utf8PathBuf::from("/test/types.ts");
    let scoped_key = format!("{}:ButtonProps", file_path);

    let mut global = GlobalSourceData::default();
    let mut prop_tags = BTreeMap::new();
    prop_tags.insert("default".into(), "outline".into());

    global.interfaces.insert(
        scoped_key.clone(),
        CollectedInterface {
            scoped_key: scoped_key.clone(),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "variant".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: prop_tags,
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());

    let mut param_defaults = FxHashMap::default();
    param_defaults.insert(
        "variant".to_string(),
        RawDefault { value: "\"default\"".into(), computed: false, source: DefaultSource::Destructuring },
    );

    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    )
    .with_param_defaults(param_defaults);

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);
    let variant_prop = entry.props.get("variant").expect("variant prop");
    // Code default should win.
    assert_eq!(variant_prop.default_value.as_ref().map(|d| d.value.as_str()), Some("\"default\""));
    // A JsDocDefaultMismatch diagnostic should have been emitted.
    assert!(
        diagnostics.iter().any(|d| d.code == DiagnosticCode::JsDocDefaultMismatch),
        "Expected JsDocDefaultMismatch diagnostic"
    );
}

// ── Test 9: Discriminated union detection ─────────────────────────────────

#[test]
fn test_discriminated_union_detection() {
    let members: Vec<(&str, Vec<ParsedProp>)> = vec![
        (
            "ButtonBaseProps",
            vec![ParsedProp::new(
                "variant".into(),
                PropType::StringLiteral("default".into()),
                true,
                None,
                String::new(),
                BTreeMap::new(),
                None,
                vec![],
            )],
        ),
        (
            "ButtonOutlineProps",
            vec![ParsedProp::new(
                "variant".into(),
                PropType::StringLiteral("outline".into()),
                true,
                None,
                String::new(),
                BTreeMap::new(),
                None,
                vec![],
            )],
        ),
    ];
    let members: Vec<(&str, Vec<std::sync::Arc<ParsedProp>>)> =
        members.into_iter().map(|(name, props)| (name, props.into_iter().map(std::sync::Arc::new).collect())).collect();
    let discriminant = chain::find_discriminant_prop(&members);
    assert_eq!(discriminant, Some("variant".to_string()));
}

// ── Test 9b: Discriminated union wrapped in an intersection ───────────────
// Regression test for the Day Picker pattern:
//   type DayPickerProps = PropsBase & (PropsSingle | PropsMulti)
// Discriminant detection only ran for a type alias whose RHS is *directly*
// a union; wrapped in an intersection, the union member fell into a naive
// merge-first-wins path with no discriminant detection at all.

#[test]
fn test_discriminant_detected_through_intersection_wrapped_union() {
    let file_path = Utf8PathBuf::from("/test/day-picker.tsx");

    let mut global = GlobalSourceData::default();

    global.interfaces.insert(
        format!("{}:PropsBase", file_path),
        CollectedInterface {
            scoped_key: format!("{}:PropsBase", file_path),
            name: "PropsBase".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "id".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:PropsSingle", file_path),
        CollectedInterface {
            scoped_key: format!("{}:PropsSingle", file_path),
            name: "PropsSingle".into(),
            file_path: file_path.clone(),
            props: vec![
                RawProp {
                    name: "mode".into(),
                    collected_type: CollectedType::StringLiteral("single".into()),
                    required: true,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
                RawProp {
                    name: "selected".into(),
                    collected_type: CollectedType::String,
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
            ],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:PropsMulti", file_path),
        CollectedInterface {
            scoped_key: format!("{}:PropsMulti", file_path),
            name: "PropsMulti".into(),
            file_path: file_path.clone(),
            props: vec![
                RawProp {
                    name: "mode".into(),
                    collected_type: CollectedType::StringLiteral("multi".into()),
                    required: true,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
                RawProp {
                    name: "selected".into(),
                    collected_type: CollectedType::Array(Box::new(CollectedType::String)),
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
            ],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    // type DayPickerProps = PropsBase & (PropsSingle | PropsMulti)
    global.type_aliases.insert(
        format!("{}:DayPickerProps", file_path),
        CollectedTypeAlias::Intersection {
            members: vec![
                CollectedType::Named { name: "PropsBase".into(), args: vec![] },
                CollectedType::Union(vec![
                    CollectedType::Named { name: "PropsSingle".into(), args: vec![] },
                    CollectedType::Named { name: "PropsMulti".into(), args: vec![] },
                ]),
            ],
            file_path: file_path.clone(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "DayPicker".into(),
        "DayPickerProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    assert_eq!(
        entry.discriminant_prop,
        Some("mode".to_string()),
        "Expected 'mode' to be detected as the discriminant even though the union is wrapped in an intersection"
    );

    let selected = entry.props.get("selected").expect("'selected' prop not found");
    assert!(
        matches!(&selected.prop_type, PropType::Union(members) if members.len() == 2),
        "Expected 'selected' to merge both branches' types into a union, got {:?} (this is the exact bug: \
         the union fell into a naive merge that keeps only the first branch's type)",
        selected.prop_type
    );
}

// ── Test 9c: Double-discriminated union (repeated single-field values) ────
// Real Day Picker's union is discriminated jointly on `mode` AND `required` —
// `PropsSingle` and `PropsSingleRequired` both have `mode: "single"`, and are
// only distinguished by `required`. `mode` alone can't identify the variant
// (`"single"` is ambiguous between the two), so declining to name it as *the*
// discriminant is correct, not a bug — verify that's still true, and that the
// real bug (prop types collapsing to a single branch instead of merging into a
// union across every branch) stays fixed for this messier, real-world shape too.
// Wrapped in an intersection with a trivial base, matching Day Picker's real
// `PropsBase & (union)` shape, so this actually exercises the fix rather than
// the already-working direct-union path.

#[test]
fn test_repeated_discriminant_values_decline_gracefully_but_still_merge_types() {
    let file_path = Utf8PathBuf::from("/test/day-picker-repeated.tsx");

    let mut global = GlobalSourceData::default();

    for (iface_name, mode, required) in [("PropsSingle", "single", false), ("PropsSingleRequired", "single", true)] {
        global.interfaces.insert(
            format!("{}:{}", file_path, iface_name),
            CollectedInterface {
                scoped_key: format!("{}:{}", file_path, iface_name),
                name: iface_name.into(),
                file_path: file_path.clone(),
                props: vec![
                    RawProp {
                        name: "mode".into(),
                        collected_type: CollectedType::StringLiteral(mode.into()),
                        required: true,
                        description: String::new(),
                        tags: BTreeMap::new(),
                        span_start: 0,
                        span_end: 0,
                    },
                    RawProp {
                        name: "required".into(),
                        collected_type: CollectedType::BoolLiteral(required),
                        required: true,
                        description: String::new(),
                        tags: BTreeMap::new(),
                        span_start: 0,
                        span_end: 0,
                    },
                    RawProp {
                        name: "selected".into(),
                        collected_type: if required { CollectedType::String } else { CollectedType::Undefined },
                        required,
                        description: String::new(),
                        tags: BTreeMap::new(),
                        span_start: 0,
                        span_end: 0,
                    },
                ],
                extends: vec![],
                description: String::new(),
                tags: BTreeMap::new(),
            },
        );
    }

    global.interfaces.insert(
        format!("{}:PropsBase", file_path),
        CollectedInterface {
            scoped_key: format!("{}:PropsBase", file_path),
            name: "PropsBase".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "id".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    // type DayPickerProps = PropsBase & (PropsSingle | PropsSingleRequired)
    global.type_aliases.insert(
        format!("{}:DayPickerProps", file_path),
        CollectedTypeAlias::Intersection {
            members: vec![
                CollectedType::Named { name: "PropsBase".into(), args: vec![] },
                CollectedType::Union(vec![
                    CollectedType::Named { name: "PropsSingle".into(), args: vec![] },
                    CollectedType::Named { name: "PropsSingleRequired".into(), args: vec![] },
                ]),
            ],
            file_path: file_path.clone(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "DayPicker".into(),
        "DayPickerProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    assert_eq!(
        entry.discriminant_prop, None,
        "'mode' repeats the same value across branches and can't identify a variant alone — \
         declining to name a discriminant here is correct"
    );

    let selected = entry.props.get("selected").expect("'selected' prop not found");
    assert!(
        matches!(&selected.prop_type, PropType::Union(members) if members.len() == 2),
        "Expected 'selected' to still merge both branches' distinct types into a union even \
         without a usable discriminant, got {:?}",
        selected.prop_type
    );
}

// ── Test 10: Extends resolution — InheritedLayer populated ───────────────

#[test]
fn test_extends_builtin_html_attrs_creates_inherited_layer() {
    let file_path = Utf8PathBuf::from("/test/button.tsx");
    let scoped_key = format!("{}:ButtonProps", file_path);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        scoped_key.clone(),
        CollectedInterface {
            scoped_key: scoped_key.clone(),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::Builtin {
                name: "ButtonHTMLAttributes".into(),
                element: Some("button".into()),
                type_args: vec![],
            }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);
    assert!(
        entry.inheritance.iter().any(|l| l.html_element.as_deref() == Some("button")),
        "Expected 'button' in inheritance layers, got {:?}",
        entry.inheritance
    );
}

#[test]
fn test_html_attribute_mode_curated_does_not_expand_real_button_attrs() {
    // Baseline: with the default Curated mode, a real (pre-merged, simulating
    // an actually-parsed @types/react) ButtonHTMLAttributes interface is NOT
    // consulted at all — own props stay empty, matching today's behavior.
    let file_path = Utf8PathBuf::from("/test/button.tsx");
    let mut global = GlobalSourceData::default();
    let react_file = react::resolve_react_types_file(
        &file_path,
        &ResolutionContext::new(Arc::new(GlobalSourceData::default()), &PipelineOptions::default()),
    );

    global.interfaces.insert(
        format!("{}:ButtonHTMLAttributes", react_file),
        CollectedInterface {
            scoped_key: format!("{}:ButtonHTMLAttributes", react_file),
            name: "ButtonHTMLAttributes".into(),
            file_path: react_file.clone().into(),
            props: vec![RawProp {
                name: "formAction".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:ButtonProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:ButtonProps", file_path),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::Builtin {
                name: "ButtonHTMLAttributes".into(),
                element: Some("button".into()),
                type_args: vec![],
            }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let options =
        PipelineOptions { html_attributes: crate::pipeline::HtmlAttributeMode::Curated, ..Default::default() };
    let ctx = ResolutionContext::new(Arc::new(global), &options);
    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);
    assert!(
        !entry.props.contains_key("formAction"),
        "Curated mode should not expand real ButtonHTMLAttributes fields into own props, got {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
}

#[test]
fn test_html_attribute_mode_full_expands_real_button_attrs() {
    // Full mode: the same pre-merged, real ButtonHTMLAttributes interface IS
    // consulted, and its real fields become genuine own props — matching how
    // RDT flattens inherited HTML attributes directly into its props map.
    let file_path = Utf8PathBuf::from("/test/button.tsx");
    let mut global = GlobalSourceData::default();
    let react_file = react::resolve_react_types_file(
        &file_path,
        &ResolutionContext::new(Arc::new(GlobalSourceData::default()), &PipelineOptions::default()),
    );

    global.interfaces.insert(
        format!("{}:ButtonHTMLAttributes", react_file),
        CollectedInterface {
            scoped_key: format!("{}:ButtonHTMLAttributes", react_file),
            name: "ButtonHTMLAttributes".into(),
            file_path: react_file.clone().into(),
            props: vec![RawProp {
                name: "formAction".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:ButtonProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:ButtonProps", file_path),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::Builtin {
                name: "ButtonHTMLAttributes".into(),
                element: Some("button".into()),
                type_args: vec![],
            }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let options = PipelineOptions { html_attributes: crate::pipeline::HtmlAttributeMode::Full, ..Default::default() };
    let ctx = ResolutionContext::new(Arc::new(global), &options);
    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);
    assert!(
        entry.props.contains_key("formAction"),
        "Full mode should expand real ButtonHTMLAttributes fields into own props, got {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
}

#[test]
fn test_html_attribute_mode_none_suppresses_notable_inherited() {
    // None mode: no curated attrs (onClick, disabled, etc.) should be
    // synthesized into notable_inherited at all — own props only.
    let file_path = Utf8PathBuf::from("/test/button.tsx");
    let scoped_key = format!("{}:ButtonProps", file_path);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        scoped_key.clone(),
        CollectedInterface {
            scoped_key: scoped_key.clone(),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::Builtin {
                name: "ButtonHTMLAttributes".into(),
                element: Some("button".into()),
                type_args: vec![],
            }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let options = PipelineOptions { html_attributes: crate::pipeline::HtmlAttributeMode::None, ..Default::default() };
    let ctx = ResolutionContext::new(Arc::new(global), &options);
    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);
    assert!(
        entry.notable_inherited.is_empty(),
        "None mode should not synthesize any curated HTML attrs, got {:?}",
        entry.notable_inherited.keys().collect::<Vec<_>>()
    );
}

// ── Test 10b: ComponentPropsWithoutRef in intersection type alias ─────────
// Regression test for the Radix UI pattern:
//   type PrimitiveButtonProps = React.ComponentPropsWithoutRef<"button"> & { asChild?: boolean }
//   interface ButtonProps extends PrimitiveButtonProps {}

#[test]
fn test_component_props_without_ref_in_intersection_alias() {
    let file_path = Utf8PathBuf::from("/test/button.tsx");

    let mut global = GlobalSourceData::default();

    // type PrimitiveButtonProps = React.ComponentPropsWithoutRef<"button"> & { asChild?: boolean }
    let alias_key = format!("{}:PrimitiveButtonProps", file_path);
    global.type_aliases.insert(
        alias_key,
        CollectedTypeAlias::Intersection {
            members: vec![
                CollectedType::Named {
                    name: "React.ComponentPropsWithoutRef".into(),
                    args: vec![CollectedType::StringLiteral("button".into())],
                },
                CollectedType::Object(vec![CollectedObjectField {
                    name: "asChild".into(),
                    collected_type: CollectedType::Boolean,
                    required: false,
                    description: String::new(),
                }]),
            ],
            file_path: file_path.clone(),
        },
    );

    // interface ButtonProps extends PrimitiveButtonProps {}
    let iface_key = format!("{}:ButtonProps", file_path);
    global.interfaces.insert(
        iface_key,
        CollectedInterface {
            scoped_key: format!("{}:ButtonProps", file_path),
            name: "ButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::SameFile { name: "PrimitiveButtonProps".into(), type_args: vec![] }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Button".into(),
        "ButtonProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    // Should have asChild as an own prop
    assert!(
        entry.props.contains_key("asChild"),
        "Expected 'asChild' prop, got props: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );

    // Should have button in the inheritance chain
    assert!(
        entry.inheritance.iter().any(|l| l.html_element.as_deref() == Some("button")),
        "Expected 'button' in inheritance layers, got {:?}",
        entry.inheritance
    );

    // Should not have ComponentPropsWithoutRef in composes
    assert!(
        !entry.composes.contains(&"React.ComponentPropsWithoutRef".to_owned()),
        "ComponentPropsWithoutRef should not appear in composes, got {:?}",
        entry.composes
    );

    let warnings: Vec<_> = diagnostics.iter().filter(|d| matches!(d.severity, DiagnosticSeverity::Warning)).collect();
    assert!(warnings.is_empty(), "Expected no warnings, got {:?}", warnings);
}

// ── Test 10b2: Two distinct Partial<X> extends targets on one interface ───
// Regression test for the Blueprint Table pattern:
//   interface TableProps extends Partial<RowHeights>, Partial<ColumnWidths> {}
// resolve_props_chain's cycle-detection visited-key was built from the bare
// type name alone ("Partial"), ignoring type_args — so the second Partial<X>
// extends target collided with the first's visited-key and silently resolved
// to nothing, with zero diagnostic.

#[test]
fn test_two_distinct_partial_extends_targets_both_resolve() {
    let file_path = Utf8PathBuf::from("/test/table.tsx");

    let mut global = GlobalSourceData::default();

    global.interfaces.insert(
        format!("{}:RowHeights", file_path),
        CollectedInterface {
            scoped_key: format!("{}:RowHeights", file_path),
            name: "RowHeights".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "defaultRowHeight".into(),
                collected_type: CollectedType::Number,
                required: true,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:ColumnWidths", file_path),
        CollectedInterface {
            scoped_key: format!("{}:ColumnWidths", file_path),
            name: "ColumnWidths".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "defaultColumnWidth".into(),
                collected_type: CollectedType::Number,
                required: true,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:TableProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:TableProps", file_path),
            name: "TableProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![
                ExtendsRef::SameFile {
                    name: "Partial".into(),
                    type_args: vec![CollectedType::Named { name: "RowHeights".into(), args: vec![] }],
                },
                ExtendsRef::SameFile {
                    name: "Partial".into(),
                    type_args: vec![CollectedType::Named { name: "ColumnWidths".into(), args: vec![] }],
                },
            ],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Table".into(),
        "TableProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        entry.props.contains_key("defaultRowHeight"),
        "Expected 'defaultRowHeight' from the first Partial<X> target, got props: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
    assert!(
        entry.props.contains_key("defaultColumnWidth"),
        "Expected 'defaultColumnWidth' from the second Partial<X> target — it must not be dropped due to a visited-key collision with the first, got props: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
}

// ── Test 9b: Union alias members resolve relative to the alias's own file ──
// Regression test for: TanStack Table's `ColumnDef<TData, TValue> = DisplayColumnDef<...>
// | GroupColumnDef<...> | AccessorColumnDef<...>` (all same-file siblings in
// types.ts). A consumer file that only imports `ColumnDef` (not the union
// members) was spuriously getting "Cannot resolve type 'DisplayColumnDef'"
// diagnostics, because `resolve_type_alias_type` forwarded the ORIGINAL
// caller's `consuming_file` into the recursive member resolution instead of
// the alias's own `file_path` — so the same-file sibling lookup was attempted
// against the wrong file.

#[test]
fn test_union_alias_members_resolve_relative_to_alias_own_file_not_caller() {
    let types_file = Utf8PathBuf::from("/test/types.ts");
    let consumer_file = Utf8PathBuf::from("/test/consumer.tsx");

    let mut global = GlobalSourceData::default();

    global.interfaces.insert(
        format!("{}:SiblingA", types_file),
        CollectedInterface {
            scoped_key: format!("{}:SiblingA", types_file),
            name: "SiblingA".into(),
            file_path: types_file.clone(),
            props: vec![RawProp {
                name: "x".into(),
                collected_type: CollectedType::String,
                required: true,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:SiblingB", types_file),
        CollectedInterface {
            scoped_key: format!("{}:SiblingB", types_file),
            name: "SiblingB".into(),
            file_path: types_file.clone(),
            props: vec![RawProp {
                name: "y".into(),
                collected_type: CollectedType::Number,
                required: true,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    // type Combined = SiblingA | SiblingB — re-exported into consumer.tsx (keyed
    // under consumer_file so the outer `value: Combined` reference resolves via
    // the same-file fallback without needing real import-map/oxc_resolver
    // plumbing), but its own `file_path` correctly records where it — and its
    // union members — are actually declared: types.ts.
    global.type_aliases.insert(
        format!("{}:Combined", consumer_file),
        CollectedTypeAlias::Union {
            members: vec![
                CollectedType::Named { name: "SiblingA".into(), args: vec![] },
                CollectedType::Named { name: "SiblingB".into(), args: vec![] },
            ],
            file_path: types_file.clone(),
        },
    );

    // interface WidgetProps { value: Combined } — declared in consumer.tsx,
    // which never imports SiblingA/SiblingB directly.
    global.interfaces.insert(
        format!("{}:WidgetProps", consumer_file),
        CollectedInterface {
            scoped_key: format!("{}:WidgetProps", consumer_file),
            name: "WidgetProps".into(),
            file_path: consumer_file.clone(),
            props: vec![RawProp {
                name: "value".into(),
                collected_type: CollectedType::Named { name: "Combined".into(), args: vec![] },
                required: true,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "WidgetProps".into(),
        consumer_file.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    let value = entry.props.get("value").expect("expected 'value' prop");
    match &value.prop_type {
        PropType::Union(members) => assert_eq!(members.len(), 2, "expected 2 union members, got {:?}", members),
        other => panic!("expected Union, got {:?}", other),
    }

    assert!(
        diagnostics.is_empty(),
        "SiblingA/SiblingB are real same-file siblings of Combined in types.ts — expected no diagnostics, got: {:?}",
        diagnostics
    );
}

// ── Test 9c: Generic interface's own type params don't trigger unresolvable-
// type diagnostics ──────────────────────────────────────────────────────────
// Regression test for: TanStack Table's `DataTableProps<TData, TValue>`
// referencing its own `TData`/`TValue` generic parameters in its body
// (`columns: ColumnDef<TData, TValue>[]`, `data: TData[]`) was spuriously
// getting "Cannot resolve type 'TData'" diagnostics — the resolver had no
// concept of an interface's own declared type parameters, so it tried (and
// "failed") to look them up as if they were real named types.

#[test]
fn test_generic_interface_own_type_params_suppress_diagnostic() {
    let file_path = Utf8PathBuf::from("/test/data-table.tsx");

    let mut global = GlobalSourceData::default();

    global.interfaces.insert(
        format!("{}:WidgetProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:WidgetProps", file_path),
            name: "WidgetProps".into(),
            file_path: file_path.clone(),
            props: vec![
                RawProp {
                    name: "data".into(),
                    collected_type: CollectedType::Array(Box::new(CollectedType::Named {
                        name: "TData".into(),
                        args: vec![],
                    })),
                    required: true,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
                RawProp {
                    name: "getValue".into(),
                    collected_type: CollectedType::Named { name: "TValue".into(), args: vec![] },
                    required: true,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
            ],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interface_type_params.insert(format!("{}:WidgetProps", file_path), vec!["TData".into(), "TValue".into()]);

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "WidgetProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        entry.props.contains_key("data"),
        "expected 'data' prop, got: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
    assert!(
        entry.props.contains_key("getValue"),
        "expected 'getValue' prop, got: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
    assert!(
        diagnostics.is_empty(),
        "TData/TValue are WidgetProps's own declared type parameters — expected no diagnostics, got: {:?}",
        diagnostics
    );
}

// ── Test 10c: User-defined generic type alias substitution (Ark UI `Assign<T, U>`) ─
// Regression test for: `type Assign<T, U> = Omit<T, keyof U> & U` used with
// concrete call-site arguments. Before substitution was implemented, `T`/`U`
// resolved as literal (unresolvable) type names and the component's props came
// back completely empty. Expected result: `U`'s own fields are present, `T`'s
// fields are present minus whatever `U` overrides (here, `title`).

#[test]
fn test_generic_type_alias_substitution() {
    let file_path = Utf8PathBuf::from("/test/types.ts");

    let mut global = GlobalSourceData::default();

    // interface Base { id: string; title: string }  (stands in for HTMLProps<'div'>)
    let base_key = format!("{}:Base", file_path);
    global.interfaces.insert(
        base_key,
        CollectedInterface {
            scoped_key: format!("{}:Base", file_path),
            name: "Base".into(),
            file_path: file_path.clone(),
            props: vec![
                RawProp {
                    name: "id".into(),
                    collected_type: CollectedType::String,
                    required: true,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
                RawProp {
                    name: "title".into(),
                    collected_type: CollectedType::String,
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
            ],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    // interface Overrides { title: number; extra: boolean }  (stands in for SelectRootBaseProps<T>)
    let overrides_key = format!("{}:Overrides", file_path);
    global.interfaces.insert(
        overrides_key,
        CollectedInterface {
            scoped_key: format!("{}:Overrides", file_path),
            name: "Overrides".into(),
            file_path: file_path.clone(),
            props: vec![
                RawProp {
                    name: "title".into(),
                    collected_type: CollectedType::Number,
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
                RawProp {
                    name: "extra".into(),
                    collected_type: CollectedType::Boolean,
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
            ],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    // type Assign<T, U> = Omit<T, keyof U> & U
    let assign_key = format!("{}:Assign", file_path);
    global.type_aliases.insert(
        assign_key.clone(),
        CollectedTypeAlias::Intersection {
            members: vec![
                CollectedType::Named {
                    name: "Omit".into(),
                    args: vec![
                        CollectedType::Named { name: "T".into(), args: vec![] },
                        CollectedType::KeyOf(Box::new(CollectedType::Named { name: "U".into(), args: vec![] })),
                    ],
                },
                CollectedType::Named { name: "U".into(), args: vec![] },
            ],
            file_path: file_path.clone(),
        },
    );
    global.type_alias_params.insert(assign_key, vec!["T".into(), "U".into()]);

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());

    // A component whose props type is `Assign<Base, Overrides>` directly.
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "Assign".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    )
    .with_props_type_args(vec![
        CollectedType::Named { name: "Base".into(), args: vec![] },
        CollectedType::Named { name: "Overrides".into(), args: vec![] },
    ]);

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    // `id` survives from Base/T (not omitted — Overrides/U doesn't declare it).
    assert!(
        entry.props.contains_key("id"),
        "Expected 'id' prop from T, got: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );

    // `title` comes from Overrides/U (number), not Base/T (string) — U wins,
    // and Omit<T, keyof U> must have actually removed T's `title`.
    let title = entry.props.get("title").expect("expected 'title' prop");
    assert_eq!(title.prop_type, PropType::Number, "Expected U's 'title: number' to win over T's 'title: string'");

    // `extra` comes from Overrides/U.
    assert!(
        entry.props.contains_key("extra"),
        "Expected 'extra' prop from U, got: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );

    // Exactly 3 props: T's title was omitted via `keyof U`, not duplicated.
    assert_eq!(
        entry.props.len(),
        3,
        "Expected exactly 3 props (id, title, extra), got: {:?}",
        entry.props.keys().collect::<Vec<_>>()
    );
}

// ── Test 11: Array type ───────────────────────────────────────────────────

#[test]
fn test_array_type() {
    let ctx = empty_ctx();
    let ct = CollectedType::Array(Box::new(CollectedType::String));
    let result = resolve_type(&ct, &ctx);
    assert!(matches!(result, PropType::Array(inner) if *inner == PropType::String));
}

// ── SPEC-SERIALIZATION-001 AC-9: ResolvedChain::give_up, called directly,
// pushes the given diagnostic and populates composes — nothing in the
// suite called this shared "give up" entry point directly before.

#[test]
fn resolved_chain_give_up_pushes_diagnostic_and_populates_composes() {
    let mut state = ResolveState::default();
    let diag = Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: "gave up".into(),
        file: None,
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::MaxDepthExceeded,
        affects: Vec::new(),
    };

    let chain = ResolvedChain::give_up("SomeUnresolved".to_string(), Some(diag.clone()), &mut state);

    assert_eq!(chain.composes, vec!["SomeUnresolved".to_string()]);
    assert_eq!(state.diagnostics, vec![diag]);
}

// ── Test 12: Template literal — opaque on unresolvable parts ─────────────

#[test]
fn test_template_literal_opaque_on_unknown_type() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    // `compact-${UnknownSize}` — UnknownSize is not in global, so opaque.
    let ct = CollectedType::TemplateLiteral(vec![
        CollectedType::StringLiteral("compact-".into()),
        CollectedType::Named { name: "UnknownSize".into(), args: vec![] },
    ]);
    let result = super::template::resolve_template_literal(
        match &ct {
            CollectedType::TemplateLiteral(parts) => parts,
            _ => unreachable!(),
        },
        Utf8Path::new("/test/button.tsx"),
        &ctx,
        &mut state,
        0,
    );
    assert!(
        matches!(&result, PropType::Opaque(d) if matches!(d.reason(), OpaqueReason::TemplateLiteral { .. })),
        "Expected Opaque TemplateLiteral, got {:?}",
        result
    );
    assert!(
        state.diagnostics.iter().any(|d| d.code == DiagnosticCode::TemplateLiteralOpaque),
        "expected a TemplateLiteralOpaque diagnostic, got {:?}",
        state.diagnostics
    );
}

// ── Test 13: Template literal expansion ───────────────────────────────────

#[test]
fn test_template_literal_expansion() {
    let file_path = Utf8PathBuf::from("/test/types.ts");
    let mut global = GlobalSourceData::default();
    let scoped_key = format!("{}:Size", file_path);
    global.type_aliases.insert(
        scoped_key,
        CollectedTypeAlias::LiteralUnion {
            members: vec!["sm".into(), "md".into(), "lg".into()],
            file_path: file_path.clone(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mut state = ResolveState::default();

    // `compact-${Size}` where Size is in the same file.
    // Note: because Size is in /test/types.ts and consuming_file is also /test/types.ts,
    // the resolve_to_canonical will return None (not imported), so we won't find the alias.
    // This is expected — cross-file resolution requires imports to be present.
    // Instead test with a raw string literal union.
    let parts = vec![CollectedType::StringLiteral("compact-".into()), CollectedType::StringLiteral("sm".into())];
    let result = template::try_expand_template_literal(&parts, Utf8Path::new("/test/types.ts"), &ctx, &mut state, 0);
    assert_eq!(result, Some(vec!["compact-sm".to_string()]));
}

// ── Test 14: Conditional type → Opaque ───────────────────────────────────

#[test]
fn test_conditional_type_is_opaque() {
    // An unbound type parameter as the check: nothing decides it without a checker.
    let ctx = empty_ctx();
    let ct = CollectedType::Conditional {
        check: Box::new(CollectedType::Named { name: "T".into(), args: vec![] }),
        extends_type: Box::new(CollectedType::String),
        true_type: Box::new(CollectedType::Boolean),
        false_type: Box::new(CollectedType::Number),
    };
    let result = resolve_type(&ct, &ctx);
    assert!(
        matches!(&result, PropType::Opaque(d) if matches!(d.reason(), OpaqueReason::ConditionalType)),
        "Expected ConditionalType opaque, got {:?}",
        result
    );
}

#[test]
fn a_conditional_over_known_types_takes_its_branch() {
    let ctx = empty_ctx();
    let conditional = |check: CollectedType, extends_type: CollectedType| CollectedType::Conditional {
        check: Box::new(check),
        extends_type: Box::new(extends_type),
        true_type: Box::new(CollectedType::Boolean),
        false_type: Box::new(CollectedType::Number),
    };
    let literal = |s: &str| CollectedType::StringLiteral(s.into());
    assert_eq!(resolve_type(&conditional(CollectedType::String, CollectedType::String), &ctx), PropType::Boolean);
    assert_eq!(resolve_type(&conditional(literal("a"), CollectedType::String), &ctx), PropType::Boolean);
    assert_eq!(resolve_type(&conditional(CollectedType::String, literal("a")), &ctx), PropType::Number);
    assert_eq!(
        resolve_type(
            &conditional(
                CollectedType::Null,
                CollectedType::Union(vec![CollectedType::Null, CollectedType::Undefined])
            ),
            &ctx
        ),
        PropType::Boolean
    );
}

// ── Test 15: Mapped type → Opaque ─────────────────────────────────────────

#[test]
fn test_mapped_type_is_opaque() {
    let ctx = empty_ctx();
    let ct = CollectedType::Mapped {
        param: "K".into(),
        key_type: Box::new(CollectedType::String),
        value_type: Box::new(CollectedType::Boolean),
        optional: None,
    };
    let result = resolve_type(&ct, &ctx);
    assert!(
        matches!(&result, PropType::Opaque(d) if matches!(d.reason(), OpaqueReason::MappedType)),
        "Expected MappedType opaque, got {:?}",
        result
    );
}

// ── Test 16: Function types keep their parameters and return type ────────

fn function_type(params: Vec<(&str, CollectedType)>, return_type: CollectedType) -> CollectedType {
    CollectedType::Function {
        type_params: vec![],
        param_names: params.iter().map(|(name, _)| Some((*name).into())).collect(),
        param_kinds: vec![],
        params: params.into_iter().map(|(_, t)| t).collect(),
        return_type: Box::new(return_type),
    }
}

#[test]
fn a_function_type_prints_every_parameter_and_its_return_type() {
    let ctx = empty_ctx();
    let named = |name: &str| CollectedType::Named { name: name.into(), args: vec![] };
    let cases = [
        (function_type(vec![("e", named("MouseEvent"))], CollectedType::Void), "(e: MouseEvent) => void"),
        (function_type(vec![], CollectedType::Void), "() => void"),
        (function_type(vec![("value", CollectedType::Number)], named("ReactNode")), "(value: number) => ReactNode"),
        (
            function_type(vec![("a", CollectedType::String), ("b", CollectedType::Number)], CollectedType::Boolean),
            "(a: string, b: number) => boolean",
        ),
    ];
    for (ct, expected) in cases {
        let mut state = ResolveState::default();
        let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/b.tsx"), &ctx, &mut state, 0);
        assert_eq!(result.raw_string(), expected);
        assert_eq!(state.diagnostics, vec![], "{expected}");
    }
}

#[test]
fn a_generic_function_types_own_parameters_are_placeholders_not_types_to_look_up() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let item = CollectedType::Named { name: "T".into(), args: vec![] };
    let ct = CollectedType::Function {
        type_params: vec!["T".into()],
        params: vec![item.clone()],
        param_names: vec![Some("item".into())],
        param_kinds: vec![],
        return_type: Box::new(item),
    };
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/b.tsx"), &ctx, &mut state, 0);
    assert_eq!(result.raw_string(), "<T>(item: T) => T");
    assert_eq!(state.diagnostics, vec![]);
    assert!(state.in_scope_type_params.is_empty(), "the function's own parameters leave scope with it");
}

#[test]
fn a_signature_names_its_types_as_written_without_looking_them_up() {
    // `CalendarMode` is an alias of a literal union here; the signature still says `CalendarMode`, as TypeScript prints it.
    let mut global = GlobalSourceData::default();
    global.type_aliases.insert(
        "/test/b.tsx:CalendarMode".into(),
        CollectedTypeAlias::LiteralUnion {
            members: vec!["year".into(), "month".into()],
            file_path: "/test/b.tsx".into(),
        },
    );
    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mut state = ResolveState::default();
    let named = |name: &str| CollectedType::Named { name: name.into(), args: vec![] };
    let ct = function_type(
        vec![("mode", named("CalendarMode")), ("rest", CollectedType::Array(Box::new(named("Missing"))))],
        CollectedType::Union(vec![named("HTMLElement"), named("Window")]),
    );
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/b.tsx"), &ctx, &mut state, 0);
    assert_eq!(result.raw_string(), "(mode: CalendarMode, rest: Missing[]) => HTMLElement | Window");
    assert_eq!(state.diagnostics, vec![]);
}

#[test]
fn test_union_filters_undefined() {
    let ctx = empty_ctx();
    // string | undefined → just string (undefined is filtered out from meaningful)
    let ct = CollectedType::Union(vec![CollectedType::String, CollectedType::Undefined]);
    let result = resolve_type(&ct, &ctx);
    // With undefined filtered, only one meaningful member → string
    assert_eq!(result, PropType::String, "Expected String after filtering undefined, got {:?}", result);
}

// ── Test 19: resolve_base_as_chain must not silently drop unhandled variants ─
// Regression test for: adversarial review finding — `resolve_base_as_chain`'s
// wildcard arm silently returned an empty chain for any CollectedType variant
// it didn't explicitly list (IndexedAccess, Function, Conditional, Mapped,
// TemplateLiteral, KeyOf, TypeOf, Raw, every primitive/literal). A real
// pattern like `type Props = SomeGeneric["variant"] & { asChild?: boolean }`
// silently lost the IndexedAccess member's contribution with zero diagnostic.

#[test]
fn test_unhandled_intersection_member_gets_diagnostic_not_silent_drop() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let file_path = Utf8PathBuf::from("/test/foo.tsx");
    let mapping = ComponentMapping::new(
        "Foo".into(),
        "FooProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    // type FooProps = SomeGeneric["variant"] & { asChild?: boolean }
    let base = CollectedType::Intersection(vec![
        CollectedType::IndexedAccess {
            obj: Box::new(CollectedType::Named { name: "SomeGeneric".into(), args: vec![] }),
            key: Box::new(CollectedType::StringLiteral("variant".into())),
        },
        CollectedType::Object(vec![CollectedObjectField {
            name: "asChild".into(),
            collected_type: CollectedType::Boolean,
            required: false,
            description: String::new(),
        }]),
    ]);

    let chain = super::alias::resolve_base_as_chain(&base, &file_path, &mapping, &ctx, &mut state, 0);

    assert!(
        chain.props.iter().any(|p| p.name == "asChild"),
        "expected 'asChild' from the Object member to survive, got {:?}",
        chain.props.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    assert!(
        !state.diagnostics.is_empty(),
        "expected a diagnostic for the unresolvable IndexedAccess intersection member, got none"
    );
}

// ── Test 20: resolve_known's Opaque results must emit a diagnostic ─────────
// Regression test for: adversarial review finding — a KnownPatternResult::Type
// that's actually PropType::Opaque (cva/tv without discoverable variants)
// never got a diagnostic pushed at either call site (named.rs step 5,
// chain.rs step 2), even though resolve_known has no diagnostics channel of
// its own to push through.

fn variants_of_missing() -> CollectedType {
    CollectedType::Named { name: "VariantProps".into(), args: vec![CollectedType::TypeOf("missingVariants".into())] }
}

#[test]
fn test_known_opaque_result_emits_diagnostic_at_type_level() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let result = super::collected::resolve_collected_type(
        &variants_of_missing(),
        Utf8Path::new("/test/button.tsx"),
        &ctx,
        &mut state,
        0,
    );

    assert!(matches!(result, PropType::Opaque(_)), "expected Opaque, got {:?}", result);
    // A named type's diagnostics are shared with its recording (`memo`), not copied into the state.
    let diagnostics: Vec<_> =
        state.diagnostics.iter().chain(state.shared_diagnostics.iter().flat_map(|chunk| chunk.iter())).collect();
    assert!(!diagnostics.is_empty(), "expected a diagnostic for VariantProps without variants, got none");
}

#[test]
fn test_known_opaque_result_emits_diagnostic_at_chain_level() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let file_path = Utf8PathBuf::from("/test/box.tsx");
    let mapping = ComponentMapping::new(
        "Box".into(),
        "VariantProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );
    let CollectedType::Named { args, .. } = variants_of_missing() else { unreachable!() };

    let chain = super::chain::resolve_props_chain("VariantProps", &args, &file_path, &mapping, &ctx, &mut state, 0);

    assert!(!chain.composes.is_empty(), "expected VariantProps to still surface via composes");
    assert!(!state.diagnostics.is_empty(), "expected a diagnostic for VariantProps without variants, got none");
}

// `VariantProps<typeof itemVariants>` reads the variants of the `itemVariants` its own file declares: shadcn keeps a
// copy of every component per registry, each with variants of its own, and the bare name alone gave all of them the
// first file's by path.
#[test]
fn test_variant_props_reads_the_variants_its_own_file_declares() {
    use crate::types::{EnumEntry, EnumValue};

    let entry = |name: &str, value: &str| EnumEntry {
        name: name.into(),
        value: EnumValue::String(value.into()),
        description: String::new(),
    };
    let mut global = GlobalSourceData::default();
    global.enums.insert("/a/item.tsx:itemVariants".into(), vec![entry("size", "default"), entry("size", "sm")]);
    global.enums.insert(
        "/b/item.tsx:itemVariants".into(),
        vec![entry("size", "default"), entry("size", "sm"), entry("size", "xs")],
    );
    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let args = [CollectedType::TypeOf("itemVariants".into())];

    let sizes_in = |path: &str| {
        let file_path = Utf8PathBuf::from(path);
        let mapping = ComponentMapping::new(
            "Item".into(),
            "VariantProps".into(),
            file_path.clone(),
            0,
            0,
            crate::types::DetectionRule::FunctionDeclaration,
        );
        let mut state = ResolveState::default();
        let chain = super::chain::resolve_props_chain("VariantProps", &args, &file_path, &mapping, &ctx, &mut state, 0);
        let size = chain.props.iter().find(|p| p.name == "size").unwrap_or_else(|| panic!("no size prop in {path}"));
        let PropType::Union(members) = &size.prop_type else { panic!("size is {:?}", size.prop_type) };
        let PropType::LiteralUnion { members, .. } = &members[0] else { panic!("size is {:?}", size.prop_type) };
        members.clone()
    };

    assert_eq!(sizes_in("/a/item.tsx"), ["default", "sm"]);
    assert_eq!(sizes_in("/b/item.tsx"), ["default", "sm", "xs"]);
}

// ── Test 21: collected.rs's Conditional/Mapped/KeyOf/Raw-fallback Opaque
// constructions must each emit a diagnostic ─────────────────────────────────

#[test]
fn test_conditional_type_opaque_emits_diagnostic() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let ct = CollectedType::Conditional {
        check: Box::new(CollectedType::Named { name: "T".into(), args: vec![] }),
        extends_type: Box::new(CollectedType::String),
        true_type: Box::new(CollectedType::Number),
        false_type: Box::new(CollectedType::Boolean),
    };
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);
    assert!(matches!(result, PropType::Opaque(_)));
    assert!(!state.diagnostics.is_empty(), "expected a diagnostic for a Conditional type, got none");
}

#[test]
fn test_mapped_type_opaque_emits_diagnostic() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let ct = CollectedType::Mapped {
        param: "P".into(),
        key_type: Box::new(CollectedType::Named { name: "K".into(), args: vec![] }),
        value_type: Box::new(CollectedType::String),
        optional: None,
    };
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);
    assert!(matches!(result, PropType::Opaque(_)));
    assert!(!state.diagnostics.is_empty(), "expected a diagnostic for a Mapped type, got none");
}

#[test]
fn test_keyof_opaque_emits_diagnostic() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let ct = CollectedType::KeyOf(Box::new(CollectedType::Named { name: "SomeType".into(), args: vec![] }));
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);
    assert!(matches!(result, PropType::Opaque(_)));
    assert!(!state.diagnostics.is_empty(), "expected a diagnostic for a standalone keyof, got none");
}

#[test]
fn test_complex_raw_fallback_opaque_emits_diagnostic() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    // Not an identifier, not a literal, not a recognized keyword, not a union, array or generic reference — falls to
    // the final Raw-fallback Opaque branch.
    let ct = CollectedType::Raw("{ a: string }".into());
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);
    assert!(matches!(result, PropType::Opaque(_)));
    assert!(!state.diagnostics.is_empty(), "expected a diagnostic for an unparsable Raw fallback, got none");
}

#[test]
fn test_depth_exceeded_opaque_carries_the_max_depth_diagnostic() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let result = super::collected::resolve_collected_type(
        &CollectedType::String,
        Utf8Path::new("/test/button.tsx"),
        &ctx,
        &mut state,
        MAX_DEPTH + 1,
    );
    let PropType::Opaque(detail) = &result else { panic!("expected Opaque, got {:?}", result) };
    assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
    assert!(
        state.diagnostics.iter().any(|d| d.code == DiagnosticCode::MaxDepthExceeded),
        "expected a MaxDepthExceeded diagnostic, got {:?}",
        state.diagnostics
    );
}

// ── Test: self-referential extends must emit a diagnostic, not silently
// return an empty chain ──────────────────────────────────────────────────
// Regression test for: chain.rs's cycle-detected path returned a bare
// `ResolvedChain::default()` with zero diagnostic when a type's own extends
// chain referenced itself, silently degrading with no trace — unlike every
// other give-up path in the resolver.

#[test]
fn test_self_referential_extends_emits_diagnostic_instead_of_silent_default() {
    let file_path = Utf8PathBuf::from("/test/cycle.tsx");
    let scoped_key = format!("{}:LoopProps", file_path);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        scoped_key.clone(),
        CollectedInterface {
            scoped_key: scoped_key.clone(),
            name: "LoopProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "id".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            // Self-referential — LoopProps extends itself.
            extends: vec![ExtendsRef::SameFile { name: "LoopProps".into(), type_args: vec![] }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Loop".into(),
        "LoopProps".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    // Own prop still resolves fine — the cycle is only in the extends chain.
    assert!(entry.props.contains_key("id"));
    // Exact contract, not a loose substring: AC-2 specifies Info severity,
    // DiagnosticCode::MaxDepthExceeded, and the exact phrase "Circular type
    // reference detected" — a case-insensitive .contains("circular") would
    // also match unrelated diagnostics (e.g. help text on a future rewording).
    let cycle_diag = diagnostics.iter().find(|d| d.message.contains("Circular type reference detected"));
    assert!(
        cycle_diag.is_some(),
        "expected a diagnostic containing 'Circular type reference detected', got {:?}",
        diagnostics
    );
    assert_eq!(cycle_diag.unwrap().severity, DiagnosticSeverity::Info);
    assert_eq!(cycle_diag.unwrap().code, DiagnosticCode::CircularReference);
    // give_up's other observable effect: the cycle target ends up in composes.
    assert!(
        entry.composes.contains(&"LoopProps".to_string()),
        "expected the give_up path's composes entry to survive into the resolved component, got {:?}",
        entry.composes
    );
}

// ── Ordinary diamond inheritance must not be misreported as a cycle.
// `interface C extends A, B` where both A and B extend `Base` is a normal,
// non-cyclic pattern — but chain.rs's extends loop used to share one
// `visited` set across sibling branches (A's branch, B's branch), so
// whichever branch resolved second found `Base` already marked visited by
// the first and reported a false "Circular type reference detected".
// Found via an adversarial drift-check probe, not from any spec criterion.

#[test]
fn diamond_inheritance_does_not_falsely_report_a_cycle() {
    let file_path = Utf8PathBuf::from("/test/diamond.tsx");
    let key = |name: &str| format!("{}:{}", file_path, name);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        key("Base"),
        CollectedInterface {
            scoped_key: key("Base"),
            name: "Base".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "shared".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    for (name, prop) in [("A", "a"), ("B", "b")] {
        global.interfaces.insert(
            key(name),
            CollectedInterface {
                scoped_key: key(name),
                name: name.into(),
                file_path: file_path.clone(),
                props: vec![RawProp {
                    name: prop.into(),
                    collected_type: CollectedType::String,
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                }],
                extends: vec![ExtendsRef::SameFile { name: "Base".into(), type_args: vec![] }],
                description: String::new(),
                tags: BTreeMap::new(),
            },
        );
    }
    global.interfaces.insert(
        key("C"),
        CollectedInterface {
            scoped_key: key("C"),
            name: "C".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "c".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![
                ExtendsRef::SameFile { name: "A".into(), type_args: vec![] },
                ExtendsRef::SameFile { name: "B".into(), type_args: vec![] },
            ],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Diamond".into(),
        "C".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        !diagnostics.iter().any(|d| d.message.contains("Circular type reference detected")),
        "diamond inheritance (both A and B independently extending Base) must not report a false cycle, got {:?}",
        diagnostics
    );
    for name in ["shared", "a", "b", "c"] {
        assert!(
            entry.props.contains_key(name),
            "expected prop '{name}' present, got {:?}",
            entry.props.keys().collect::<Vec<_>>()
        );
    }
}

// ── resolve_named had no cycle detection at all, unlike its chain-level
// sibling resolve_props_chain — a self-referential type alias reached
// directly as a field's TYPE (not via `extends`) recursed 20 frames deep
// (MAX_DEPTH) before bailing with a generic depth-exceeded diagnostic,
// instead of being caught immediately with a friendly "circular
// reference" message the way the chain-level path already was.
// docs/edge-cases.md's previously-untracked-gap entry, not a spec
// criterion — SPEC-RESOLVER-001 explicitly non_goals this out.

#[test]
fn self_referential_type_alias_used_directly_as_a_field_type_is_caught_immediately() {
    let file_path = Utf8PathBuf::from("/test/loop.tsx");
    let key = |name: &str| format!("{}:{}", file_path, name);

    let mut global = GlobalSourceData::default();
    // type Loop = Loop; — a bare self-reference, the fallback Passthrough
    // shape the extractor produces for any unrecognized type reference.
    global.type_aliases.insert(
        key("Loop"),
        CollectedTypeAlias::Passthrough {
            target: CollectedType::Named { name: "Loop".into(), args: vec![] },
            file_path: file_path.clone(),
        },
    );
    global.interfaces.insert(
        key("Props"),
        CollectedInterface {
            scoped_key: key("Props"),
            name: "Props".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "x".into(),
                collected_type: CollectedType::Named { name: "Loop".into(), args: vec![] },
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "Props".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    let cycle_diag = diagnostics.iter().find(|d| d.message.contains("Circular type reference detected"));
    assert!(
        cycle_diag.is_some(),
        "expected an immediate circular-reference diagnostic, not a 20-frame depth exhaustion, got {:?}",
        diagnostics
    );
    assert_eq!(cycle_diag.unwrap().severity, DiagnosticSeverity::Info);
    // MaxDepthExceeded is deliberately reused as the code (see the doc
    // comment on ResolveState::named_in_progress and named.rs's own
    // depth>MAX_DEPTH path, which already uses this same code) — there's
    // no dedicated circular-reference code at the type level, matching
    // the chain-level cycle case's own choice of code.
    assert_eq!(cycle_diag.unwrap().code, DiagnosticCode::CircularReference);

    let x = entry.props.get("x").expect("expected prop 'x' to still be present, degraded to Opaque");
    assert!(matches!(x.prop_type, PropType::Opaque(_)), "expected 'x' to degrade to Opaque, got {:?}", x.prop_type);
}

// ── Guard against the exact false-positive class this fix could
// introduce if it reused a permanent (not per-call-stack) visited set:
// two UNRELATED, non-cyclic fields that happen to reference the SAME
// type must both resolve cleanly, not have the second one falsely
// flagged as "already visited".

#[test]
fn two_unrelated_fields_sharing_the_same_type_are_not_falsely_flagged_as_a_cycle() {
    let file_path = Utf8PathBuf::from("/test/shared.tsx");
    let key = |name: &str| format!("{}:{}", file_path, name);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        key("Shared"),
        CollectedInterface {
            scoped_key: key("Shared"),
            name: "Shared".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "value".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        key("Props"),
        CollectedInterface {
            scoped_key: key("Props"),
            name: "Props".into(),
            file_path: file_path.clone(),
            props: vec![
                RawProp {
                    name: "a".into(),
                    collected_type: CollectedType::Named { name: "Shared".into(), args: vec![] },
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
                RawProp {
                    name: "b".into(),
                    collected_type: CollectedType::Named { name: "Shared".into(), args: vec![] },
                    required: false,
                    description: String::new(),
                    tags: BTreeMap::new(),
                    span_start: 0,
                    span_end: 0,
                },
            ],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "Props".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        !diagnostics.iter().any(|d| d.message.contains("Circular")),
        "two unrelated fields sharing a type must not be flagged as circular, got {:?}",
        diagnostics
    );
    for field in ["a", "b"] {
        let prop = entry.props.get(field).unwrap_or_else(|| panic!("expected prop '{field}' present"));
        assert!(
            matches!(&prop.prop_type, PropType::Named { name, .. } if name == "Shared"),
            "expected '{field}' to resolve to PropType::Named(\"Shared\"), got {:?}",
            prop.prop_type
        );
    }
}

// ── SPEC-RESOLVER-001 AC-2b: a chain of DISTINCT (non-repeating) interfaces
// exceeding MAX_DEPTH must give up via ResolvedChain::give_up (same
// construction path as the cycle-detected case above), producing a
// Warning-severity MaxDepthExceeded diagnostic — distinct from the cycle
// case's Info-severity "Circular type reference detected" message ────────

#[test]
fn test_non_cyclic_depth_exhaustion_emits_max_depth_diagnostic() {
    let file_path = Utf8PathBuf::from("/test/deep_chain.tsx");
    let depth: u32 = u32::from(MAX_DEPTH) + 5;

    let mut global = GlobalSourceData::default();
    for level in 0..=depth {
        let name = format!("Level{level}");
        let extends = if level == depth {
            vec![]
        } else {
            vec![ExtendsRef::SameFile { name: format!("Level{}", level + 1).into(), type_args: vec![] }]
        };
        global.interfaces.insert(
            format!("{}:{}", file_path, name),
            CollectedInterface {
                scoped_key: format!("{}:{}", file_path, name),
                name: name.clone().into(),
                file_path: file_path.clone(),
                props: vec![],
                extends,
                description: String::new(),
                tags: BTreeMap::new(),
            },
        );
    }

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Deep".into(),
        "Level0".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        diagnostics.iter().any(|d| {
            d.severity == DiagnosticSeverity::Warning
                && d.code == DiagnosticCode::MaxDepthExceeded
                && d.message.contains("Max resolution depth exceeded")
        }),
        "expected a Warning-severity MaxDepthExceeded diagnostic containing \
         'Max resolution depth exceeded', got {:?}",
        diagnostics
    );
    // give_up's other observable effect — never checked before: the level
    // where the guard tripped ends up in composes, propagated up through
    // every merge_parent call along the extends chain back to Level0.
    assert!(
        entry.composes.iter().any(|c| c.starts_with("Level")),
        "expected the give_up path's composes entry (the level where MAX_DEPTH was exceeded) to survive \
         into the resolved component, got {:?}",
        entry.composes
    );
}

// ── Test: a literal union used directly as a component's props base must
// emit a diagnostic, not silently return an empty chain ─────────────────

#[test]
fn test_literal_union_as_props_base_emits_diagnostic_instead_of_silent_default() {
    let file_path = Utf8PathBuf::from("/test/literal_union.tsx");
    let scoped_key = format!("{}:Props", file_path);

    let mut global = GlobalSourceData::default();
    global.type_aliases.insert(
        scoped_key.clone(),
        CollectedTypeAlias::LiteralUnion {
            members: vec!["a".into(), "b".into(), "c".into()],
            file_path: file_path.clone(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping::new(
        "Widget".into(),
        "Props".into(),
        file_path.clone(),
        0,
        0,
        crate::types::DetectionRule::FunctionDeclaration,
    );

    let (_entry, diagnostics) = resolve_component(&mapping, &ctx);

    // Exact contract: AC-4 specifies two exact substrings, in order, plus
    // Warning severity and DiagnosticCode::OpaqueType — a two-word fragment
    // match like .contains("literal union") is satisfied by a much shorter,
    // weaker message than what the criterion actually requires.
    let diag = diagnostics.iter().find(|d| {
        let first = d.message.find("is a literal union and can't be used as a component's props base");
        let second = d.message.find("expected an interface, intersection, union, or inline object type");
        matches!((first, second), (Some(f), Some(s)) if f < s)
    });
    assert!(
        diag.is_some(),
        "expected a diagnostic containing both required substrings in order, got {:?}",
        diagnostics
    );
    assert_eq!(diag.unwrap().severity, DiagnosticSeverity::Warning);
    assert_eq!(diag.unwrap().code, DiagnosticCode::OpaqueType);
}

// ── resolve_ts_lib_paths: TypeScript <= 6 and 7 package layouts ──────────

fn write(root: &std::path::Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn lib_paths_from(root: &std::path::Path) -> TsLibs {
    let root = Utf8PathBuf::from_path_buf(root.canonicalize().unwrap()).unwrap();
    resolve_ts_lib_paths(&root)
}

/// TS 7 as published: `exports` exposes only `package.json`; `optionalDependencies` lists the platforms.
fn write_typescript_7(root: &std::path::Path, platforms: &[&str]) {
    let optional = platforms.iter().map(|p| format!(r#""@typescript/typescript-{p}":"7.0.2""#)).collect::<Vec<_>>();
    write(
        root,
        "node_modules/typescript/package.json",
        &format!(
            r#"{{"name":"typescript","version":"7.0.2","exports":{{"./package.json":"./package.json"}},
                "optionalDependencies":{{{}}}}}"#,
            optional.join(",")
        ),
    );
}

fn write_platform_package(root: &std::path::Path, platform: &str, libs: &[&str]) {
    let dir = format!("node_modules/@typescript/typescript-{platform}");
    write(
        root,
        &format!("{dir}/package.json"),
        &format!(
            r#"{{"name":"@typescript/typescript-{platform}","version":"7.0.2",
                "exports":{{"./package.json":"./package.json"}}}}"#
        ),
    );
    for lib in libs {
        write(root, &format!("{dir}/lib/{lib}"), "");
    }
}

fn canonical(root: &std::path::Path, rel: &str) -> String {
    root.canonicalize().unwrap().join(rel).to_string_lossy().into_owned()
}

#[test]
fn ts_lib_paths_resolve_from_typescript_6_lib_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    write(tmp.path(), "node_modules/typescript/package.json", r#"{"name":"typescript","version":"5.9.3"}"#);
    write(tmp.path(), "node_modules/typescript/lib/lib.es5.d.ts", "");
    write(tmp.path(), "node_modules/typescript/lib/lib.dom.d.ts", "");

    assert_eq!(
        lib_paths_from(tmp.path()),
        TsLibs {
            paths: vec![
                canonical(tmp.path(), "node_modules/typescript/lib/lib.es5.d.ts"),
                canonical(tmp.path(), "node_modules/typescript/lib/lib.dom.d.ts"),
            ],
            diagnostic: None,
        }
    );
}

#[test]
fn ts_lib_paths_resolve_from_typescript_7_platform_package() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Two declared platforms, only one installed.
    write_typescript_7(tmp.path(), &["missing", "here"]);
    write_platform_package(tmp.path(), "here", &["lib.es5.d.ts", "lib.dom.d.ts"]);

    assert_eq!(
        lib_paths_from(tmp.path()),
        TsLibs {
            paths: vec![
                canonical(tmp.path(), "node_modules/@typescript/typescript-here/lib/lib.es5.d.ts"),
                canonical(tmp.path(), "node_modules/@typescript/typescript-here/lib/lib.dom.d.ts"),
            ],
            diagnostic: None,
        }
    );
}

#[test]
fn ts_lib_paths_skip_platform_packages_without_the_file_and_ignore_declaration_order() {
    let tmp = tempfile::TempDir::new().unwrap();
    // `a` has a `lib/` but not the files (a stale or partial install); `b` and `c` both have them.
    write_typescript_7(tmp.path(), &["c", "b", "a"]);
    write_platform_package(tmp.path(), "a", &["tsc"]);
    write_platform_package(tmp.path(), "b", &["lib.es5.d.ts", "lib.dom.d.ts"]);
    write_platform_package(tmp.path(), "c", &["lib.es5.d.ts", "lib.dom.d.ts"]);

    assert_eq!(
        lib_paths_from(tmp.path()).paths,
        vec![
            canonical(tmp.path(), "node_modules/@typescript/typescript-b/lib/lib.es5.d.ts"),
            canonical(tmp.path(), "node_modules/@typescript/typescript-b/lib/lib.dom.d.ts"),
        ]
    );
}

#[test]
fn ts_lib_paths_report_typescript_7_without_its_platform_package() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_typescript_7(tmp.path(), &["not-installed"]);

    let libs = lib_paths_from(tmp.path());

    assert_eq!(libs.paths, Vec::<String>::new());
    assert_eq!(
        libs.diagnostic,
        Some(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "TypeScript's lib.es5.d.ts, lib.dom.d.ts not found in '{}' or any installed platform package — \
                 native and DOM globals (Date, Element, …) will appear as unexpanded named references",
                canonical(tmp.path(), "node_modules/typescript")
            ),
            file: None,
            line: None,
            column: None,
            help: Some(
                "TypeScript 7 ships its lib files in an optional dependency (@typescript/typescript-<os>-<arch>). \
                 Install with optional dependencies enabled (no --no-optional / --omit=optional)."
                    .into()
            ),
            code: DiagnosticCode::MissingTypeLibrary,
            affects: Vec::new(),
        })
    );
}

#[test]
fn ts_lib_paths_keep_the_files_found_and_name_only_the_missing_one() {
    let tmp = tempfile::TempDir::new().unwrap();
    write(tmp.path(), "node_modules/typescript/package.json", r#"{"name":"typescript","version":"5.9.3"}"#);
    write(tmp.path(), "node_modules/typescript/lib/lib.es5.d.ts", "");

    let libs = lib_paths_from(tmp.path());

    assert_eq!(libs.paths, vec![canonical(tmp.path(), "node_modules/typescript/lib/lib.es5.d.ts")]);
    assert_eq!(
        libs.diagnostic.map(|d| d.message),
        Some(format!(
            "TypeScript's lib.dom.d.ts not found in '{}' or any installed platform package — native and DOM \
             globals (Date, Element, …) will appear as unexpanded named references",
            canonical(tmp.path(), "node_modules/typescript")
        ))
    );
}

#[test]
fn ts_lib_paths_are_empty_and_silent_without_typescript() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert_eq!(lib_paths_from(tmp.path()), TsLibs::default());
}

// ── interface chains resolved once per run (`memo`) ─────────────────────────

const SHARED_BASE: &str = r#"
import type { Tone } from "./tone";
export interface Base {
  /** shared tone */
  tone?: Tone;
  size?: number;
  broken?: NotDeclaredAnywhere;
}
"#;

const SHARED_COMPONENTS: &str = r#"
import type { Base } from "./base";
interface AProps extends Base { a?: string }
interface BProps extends Base { b?: string }
interface CProps extends Base { c?: string }
export function A(props: AProps) { return null; }
export function B(props: BProps) { return null; }
export function C({ tone = "b" }: CProps) { return null; }
"#;

/// A context over real files in a temp dir, so imports resolve as in a run.
fn parsed_ctx(dir: &Utf8Path, files: &[(&str, &str)], options: &PipelineOptions) -> ResolutionContext {
    let mut global = GlobalSourceData::default();
    for (name, source) in files {
        let path = dir.join(name);
        std::fs::write(&path, source).unwrap();
        global.merge(&path, crate::extractor::parse_file(&path, source));
    }
    ResolutionContext::new(Arc::new(global), options)
}

type SharedResults = BTreeMap<String, (ComponentEntry, Vec<Diagnostic>, deps::Dependencies)>;

/// Resolve `components` in order on one context, asserting each gives exactly what it gives on a context of its own:
/// entry, dependencies, and each distinct diagnostic in the order first reported (a reused chain reports each once).
fn resolve_shared(dir: &Utf8Path, files: &[(&str, &str)], components: &[&str]) -> (ResolutionContext, SharedResults) {
    resolve_shared_with(dir, files, components, &PipelineOptions::default())
}

fn resolve_shared_with(
    dir: &Utf8Path,
    files: &[(&str, &str)],
    components: &[&str],
    options: &PipelineOptions,
) -> (ResolutionContext, SharedResults) {
    let shared = parsed_ctx(dir, files, options);
    let mapping = |ctx: &ResolutionContext, name: &str| {
        ctx.global.component_mappings.iter().find(|m| m.component_name == name).unwrap().clone()
    };
    let mut results = BTreeMap::new();
    for name in components {
        let fresh = parsed_ctx(dir, files, options);
        let flat = |(entry, diagnostics, deps): (ComponentEntry, ResolvedDiagnostics, deps::Dependencies)| {
            (entry, diagnostics.flatten(), deps)
        };
        let alone = flat(resolve_component_tracked(&mapping(&fresh, name), &fresh));
        let reused = flat(resolve_component_tracked(&mapping(&shared, name), &shared));
        assert_eq!(serde_json::to_value(&reused.0).unwrap(), serde_json::to_value(&alone.0).unwrap(), "{name}");
        let firsts = |diagnostics: &[Diagnostic]| {
            let mut seen = FxHashSet::default();
            diagnostics.iter().filter(|d| seen.insert(*d)).cloned().collect::<Vec<_>>()
        };
        assert_eq!(firsts(&reused.1), firsts(&alone.1), "{name}: diagnostics");
        assert_eq!(reused.2, alone.2, "{name}: dependencies");
        results.insert((*name).to_owned(), reused);
    }
    (shared, results)
}

fn canonical_temp_dir() -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    (tmp, dir)
}

#[test]
fn equal_props_built_for_different_components_are_one_allocation() {
    // A generic interface's chain is resolved per instantiation, so each component builds its own `value` prop; the two
    // `string` ones are equal and share an allocation, the `number` one is not.
    let (_tmp, dir) = canonical_temp_dir();
    let source = r#"
interface Box<T> { /** held */ value: T; label?: string }
interface AProps extends Box<string> {}
interface BProps extends Box<string> {}
interface CProps extends Box<number> {}
export function A(props: AProps) { return null; }
export function B(props: BProps) { return null; }
export function C(props: CProps) { return null; }
"#;
    let (_shared, results) = resolve_shared(&dir, &[("Comp.tsx", source)], &["A", "B", "C"]);

    let prop = |component: &str, name: &str| &results[component].0.props[name];
    assert!(Arc::ptr_eq(prop("A", "value"), prop("B", "value")));
    assert!(Arc::ptr_eq(prop("A", "label"), prop("C", "label")));
    assert!(!Arc::ptr_eq(prop("A", "value"), prop("C", "value")));
    assert_eq!(prop("C", "value").prop_type, PropType::Number);
}

#[test]
fn a_shared_interface_chain_resolves_like_a_fresh_one_for_every_component() {
    let (_tmp, dir) = canonical_temp_dir();
    let tone = "export type Tone = \"a\" | \"b\";\n";
    let files = [("tone.ts", tone), ("base.ts", SHARED_BASE), ("Comp.tsx", SHARED_COMPONENTS)];
    let (shared, results) = resolve_shared(&dir, &files, &["A", "B", "C"]);

    for (name, (_, diagnostics, deps)) in &results {
        let unresolved: Vec<_> = diagnostics.iter().filter(|d| d.message.contains("NotDeclaredAnywhere")).collect();
        assert_eq!(unresolved.len(), 1, "{name}: {diagnostics:?}");
        // Only Base's chain reads tone.ts, so a component that reuses it must still depend on it.
        assert!(deps.files.contains(dir.join("tone.ts").as_str()), "{name}: {deps:?}");
    }

    // B reuses A's recording of Base; C's default for `tone` makes it resolve Base again.
    let comp = |name: &str| format!("{dir}/Comp.tsx:{name}");
    let expected: FxHashMap<String, usize> =
        [(format!("{dir}/base.ts:Base"), 2), (comp("AProps"), 1), (comp("BProps"), 1), (comp("CProps"), 1)]
            .into_iter()
            .collect();
    assert_eq!(memo::recording_counts(&shared.interface_chains), expected);

    let prop = |component: &str, name: &str| &results[component].0.props[name];
    assert!(Arc::ptr_eq(prop("A", "tone"), prop("B", "tone")));
    assert!(Arc::ptr_eq(prop("A", "size"), prop("B", "size")));
    assert!(!Arc::ptr_eq(prop("A", "tone"), prop("C", "tone")));
    let default_of = |component: &str| prop(component, "tone").default_value.as_ref().map(|d| d.value.clone());
    assert_eq!((default_of("A"), default_of("B"), default_of("C")), (None, None, Some("\"b\"".to_owned())));
}

#[test]
fn a_named_type_is_resolved_once_and_every_component_that_names_it_gets_the_same_answer() {
    // `resolve_shared` asserts that each component resolves identically with and without the shared recordings,
    // diagnostics and dependencies included; A and B name `Tone`, and A also names a self-referential `Loop`.
    let (_tmp, dir) = canonical_temp_dir();
    let source = r#"
type Tone = "a" | "b" | NotDeclaredAnywhere;
type Loop = { next?: Loop } | "end";
export function A(props: { tone?: Tone; loop?: Loop }) { return null; }
export function B(props: { tone?: Tone }) { return null; }
"#;
    let (shared, results) = resolve_shared(&dir, &[("Comp.tsx", source)], &["A", "B"]);

    for name in ["A", "B"] {
        let unresolved = results[name].1.iter().filter(|d| d.message.contains("NotDeclaredAnywhere")).count();
        assert_eq!(unresolved, 1, "{name}: {:?}", results[name].1);
    }
    let tone_recordings: usize = memo::recording_counts(&shared.named_results)
        .into_iter()
        .filter(|(key, _)| key.contains("Comp.tsx:Tone<"))
        .map(|(_, count)| count)
        .sum();
    assert_eq!(tone_recordings, 1);
}

#[test]
fn an_interface_chain_read_for_its_keys_is_not_reused_where_its_props_are_listed() {
    // K reads only Base's names (`keyof Base`), so its resolution of Base skips member types; N lists Base's props.
    // Both reach Base at depth 2, so only the mode tells the recordings apart.
    let (_tmp, dir) = canonical_temp_dir();
    let source = r#"
type Tone = "a" | "b";
interface Base { tone?: Tone; label: string }
interface Own { tone?: number; extra?: boolean }
interface Mid extends Base {}
interface NProps extends Mid {}
export function K(props: Omit<Own, keyof Base>) { return null; }
export function N(props: NProps) { return null; }
"#;
    let (shared, results) = resolve_shared(&dir, &[("Comp.tsx", source)], &["K", "N"]);

    let types = |component: &str| {
        results[component].0.props.iter().map(|(name, p)| (name.as_str(), p.prop_type.clone())).collect::<Vec<_>>()
    };
    assert_eq!(types("K"), [("extra", PropType::Boolean)]);
    let tone = PropType::LiteralUnion { members: vec!["a".into(), "b".into()], has_default: false };
    assert_eq!(types("N"), [("label", PropType::String), ("tone", tone)]);
    assert_eq!(memo::recording_counts(&shared.interface_chains)[&format!("{dir}/Comp.tsx:Base")], 2);
}

#[test]
fn an_interface_chain_expanded_for_a_pick_is_not_reused_where_it_collapses() {
    // `Pick` expands its base so it can pick from the props; Q inherits the same 31-prop interface, which collapses
    // to a layer. Both reach Big at depth 1, so only the mode tells the recordings apart.
    let (_tmp, dir) = canonical_temp_dir();
    let fields: String = (0..31).map(|i| format!("p{i}?: string; ")).collect();
    let source = format!(
        "export interface Big {{ {fields}}}\ninterface QProps extends Big {{ own?: boolean }}\n\
         export function P(props: Pick<Big, \"p0\">) {{ return null; }}\n\
         export function Q(props: QProps) {{ return null; }}\n"
    );
    let options = PipelineOptions {
        collapse_inherited_props: crate::pipeline::CollapseInheritedProps::All,
        ..Default::default()
    };
    let (shared, results) = resolve_shared_with(&dir, &[("Comp.tsx", &source)], &["P", "Q"], &options);

    let names = |component: &str| results[component].0.props.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!((names("P"), names("Q")), (vec!["p0"], vec!["own"]));
    let layers: Vec<&str> = results["Q"].0.inheritance.iter().map(|layer| layer.type_name.as_str()).collect();
    assert_eq!(layers, ["Big"]);
    assert_eq!(memo::recording_counts(&shared.interface_chains)[&format!("{dir}/Comp.tsx:Big")], 2);
}

#[test]
fn an_interface_chain_cut_short_by_a_cycle_is_not_reused_where_the_cycle_is_not_open() {
    // P reaches Base through Loop, so Base's own `extends Loop` closes a cycle there; Q reaches Base at the same depth
    // through Z, where Loop has not been entered, so Base's chain takes Loop's props.
    let (_tmp, dir) = canonical_temp_dir();
    let source = r#"
interface Base extends Loop { b?: string }
interface Loop extends Base { l?: string }
interface X extends Loop {}
interface Z extends Base { z?: string }
interface Y extends Z {}
export function P(props: X) { return null; }
export function Q(props: Y) { return null; }
"#;
    let (shared, results) = resolve_shared(&dir, &[("Comp.tsx", source)], &["P", "Q"]);

    let names = |component: &str| results[component].0.props.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!((names("P"), names("Q")), (vec!["b", "l"], vec!["b", "l", "z"]));
    assert_eq!(memo::recording_counts(&shared.interface_chains)[&format!("{dir}/Comp.tsx:Base")], 2);
}

#[test]
fn an_interface_chain_reached_at_different_depths_is_one_recording_with_the_same_answer() {
    // A reaches Base at depth 1 and B at depth 3; nothing in Base's resolution is near the depth limit, so one recording
    // answers both (the key once held the depth, and Base was resolved again at each one).
    let (_tmp, dir) = canonical_temp_dir();
    let source = r#"
interface Base { label: string; size?: number }
interface M2 extends Base { m2?: boolean }
interface M1 extends M2 { m1?: boolean }
interface AProps extends Base { a?: string }
interface BProps extends M1 { b?: string }
export function A(props: AProps) { return null; }
export function B(props: BProps) { return null; }
"#;
    let (shared, results) = resolve_shared(&dir, &[("Comp.tsx", source)], &["A", "B"]);

    let names = |component: &str| results[component].0.props.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(names("A"), ["a", "label", "size"]);
    assert_eq!(names("B"), ["b", "label", "m1", "m2", "size"]);
    assert_eq!(memo::recording_counts(&shared.interface_chains)[&format!("{dir}/Comp.tsx:Base")], 1);
}

#[test]
fn an_interface_chain_that_hit_the_depth_limit_is_not_reused_from_a_shallower_start() {
    // P walks a 25-interface extends chain from the top and stops at the limit, so its recording of the chain from L10
    // holds a depth-exceeded stop; Q starts at L10 itself and reaches the bottom. `resolve_shared` checks that each
    // resolves as it does alone.
    let (_tmp, dir) = canonical_temp_dir();
    let interfaces: String = (0..25)
        .map(|i| match i {
            24 => format!("interface L{i} {{ p{i}?: string }}\n"),
            _ => format!("interface L{i} extends L{} {{ p{i}?: string }}\n", i + 1),
        })
        .collect();
    let source = format!(
        "{interfaces}export function P(props: L0) {{ return null; }}\nexport function Q(props: L10) {{ return null; }}\n"
    );
    let (_shared, results) = resolve_shared(&dir, &[("Comp.tsx", &source)], &["P", "Q"]);

    let count = |component: &str| results[component].0.props.len();
    assert_eq!((count("P"), count("Q")), (21, 15));
    assert!(results["P"].1.iter().any(|d| d.message.contains("Max resolution depth")), "{:?}", results["P"].1);
    assert!(results["Q"].1.iter().all(|d| !d.message.contains("Max resolution depth")), "{:?}", results["Q"].1);
}

// `import React from "react"` of an `export =` module binds the module's namespace, as `import * as React` does, so
// `React.CSSProperties` names the same member under either spelling (antd imports React the first way).
#[test]
fn a_default_import_names_a_namespace_member_as_a_namespace_import_does() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    write(root.as_std_path(), "lib.ts", "export interface Options { size: string }\n");
    let canonical_of = |import: &str| {
        let source = format!("{import}\nexport const x = 1;\n");
        let consumer = root.join("c.tsx");
        write(root.as_std_path(), "c.tsx", &source);
        let mut global = GlobalSourceData::default();
        for path in [root.join("lib.ts"), consumer.clone()] {
            let text = std::fs::read_to_string(&path).unwrap();
            global.merge(&path, crate::extractor::parse_file(&path, &text));
        }
        let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
        super::import::resolve_to_canonical("Lib.Options", &consumer, &ctx, &mut Vec::new())
    };
    let expected = Some((root.join("lib.ts"), "Options".to_owned()));
    assert_eq!(canonical_of(r#"import * as Lib from "./lib";"#), expected, "the namespace spelling");
    assert_eq!(canonical_of(r#"import Lib from "./lib";"#), expected, "the default spelling");
}

// ── Component lookup by file and name ─────────────────────────────────────

/// The lookup answers as a scan of the mappings does: the first mapping with that file and name, nothing for a name
/// the file does not declare, whatever other files declare under it.
#[test]
fn component_mapping_is_the_first_match_a_scan_finds() {
    let mapping = |name: &str, props: &str, file: &str| {
        ComponentMapping::new(
            name.into(),
            props.into(),
            Utf8PathBuf::from(file),
            0,
            0,
            crate::types::DetectionRule::FunctionDeclaration,
        )
    };
    let mut global = GlobalSourceData::default();
    global.component_mappings = vec![
        mapping("Button", "FirstProps", "/a/button.tsx"),
        mapping("Button", "SecondProps", "/b/button.tsx"),
        mapping("Button", "ShadowedProps", "/a/button.tsx"),
        mapping("Card", "CardProps", "/a/card.tsx"),
    ];
    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let props_of = |file: &str, name: &str| {
        ctx.component_mapping(Utf8Path::new(file), name).map(|found| found.props_type_name.to_string())
    };

    assert_eq!(props_of("/a/button.tsx", "Button").as_deref(), Some("FirstProps"));
    assert_eq!(props_of("/b/button.tsx", "Button").as_deref(), Some("SecondProps"));
    assert_eq!(props_of("/a/card.tsx", "Card").as_deref(), Some("CardProps"));
    assert_eq!(props_of("/b/button.tsx", "Card"), None, "another file's component");
    assert_eq!(props_of("/a/button.tsx", "Missing"), None);
    assert_eq!(props_of("/c/button.tsx", "Button"), None, "a file that declares none");
    // A context derived for a dependency layer answers from the same index.
    let derived = ctx.over_layer(ctx.named_types.clone(), None);
    assert_eq!(
        derived
            .component_mapping(Utf8Path::new("/b/button.tsx"), "Button")
            .map(|found| found.props_type_name.to_string()),
        Some("SecondProps".to_owned())
    );
}
