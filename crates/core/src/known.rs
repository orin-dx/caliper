//! Resolution of known generic type patterns.
//!
//! A pattern stands in for a declaration that isn't available (its package isn't loaded), or that evaluates to nothing
//! structurally; a declaration that resolves always wins (ADR 0014). Add one only when that fallback measurably beats
//! leaving the name unresolved, against react-docgen-typescript on code that uses the library.

use crate::types::*;

/// Result of recognizing a known type pattern.
pub enum KnownPatternResult {
    /// Pattern resolves to a set of props
    Props(Vec<ParsedProp>),
    /// Pattern is opaque — use this PropType directly
    Type(PropType),
    /// Pattern transparently delegates to another type name
    Alias { name: String },
}

/// Push a diagnostic for a `KnownPatternResult::Type` that resolved to
/// `PropType::Opaque`. `resolve_known` has no diagnostics channel of its own
/// (this module must not depend on `resolver`), so callers push this
/// themselves right after matching `KnownPatternResult::Type(PropType::Opaque
/// { .. })` — otherwise a recognized-but-unexpandable pattern (ThemingProps,
/// StylesApiProps, cva/tv without discoverable variants, …) degrades with no
/// trace at all, unlike every other opaque-producing path in the resolver.
pub fn push_known_opaque_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    reason: &OpaqueReason,
    type_name: &str,
    file: &camino::Utf8Path,
) {
    let why = match reason {
        OpaqueReason::RuntimeDependent { function_name } => {
            format!("depends on '{function_name}' at runtime")
        }
        OpaqueReason::ModuleAugmentation => "depends on a module augmentation this tool doesn't evaluate".to_string(),
        OpaqueReason::ConditionalType => "is a conditional type".to_string(),
        OpaqueReason::MappedType => "is a mapped type".to_string(),
        OpaqueReason::UnresolvableImport { specifier } => format!("could not be resolved from '{specifier}'"),
        OpaqueReason::PandaCodegenMissing => "depends on PandaCSS's generated styled-system output".to_string(),
        OpaqueReason::DepthExceeded => "exceeded the maximum resolution depth".to_string(),
        OpaqueReason::IndexedAccess { expression } => format!("is an indexed access type ('{expression}')"),
        OpaqueReason::TemplateLiteral { expression } => format!("is a template literal type ('{expression}')"),
        OpaqueReason::UnsupportedExpression => "is a type expression this tool doesn't recognize".to_string(),
    };
    diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("'{type_name}' {why} and can't be statically resolved — it will appear as opaque"),
        file: Some(file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
        affects: Vec::new(),
    });
}

/// Attempt to resolve a named generic type as a known pattern.
///
/// Called by the resolver when it encounters a type like `SxProps<Theme>`
/// or `VariantProps<typeof buttonVariants>`.
///
/// Returns `None` if this type is not recognized — caller should continue
/// with normal resolution.
///
/// `enum_key` maps a name to its scoped key in `global.enums`, as the file that wrote the name sees it
/// (`ResolutionContext::enum_key_for`) — passed in rather than a resolver-owned context type, since this module must
/// not depend on `resolver` (no reverse deps). Two files can each declare a `buttonVariants`.
pub fn resolve_known(
    name: &str,
    args: &[PropType],
    global: &GlobalSourceData,
    enum_key: &dyn Fn(&str) -> Option<String>,
) -> Option<KnownPatternResult> {
    match name {
        // ── Variant systems ──────────────────────────────────────────────────
        // class-variance-authority: VariantProps<typeof buttonVariants>
        // PandaCSS: RecipeVariantProps<typeof buttonStyle>
        // vanilla-extract: RecipeVariants<typeof buttonRecipe>
        // tailwind-variants: VariantProps<typeof tv(...)>
        // The real declarations infer from the function's parameters (`Parameters<T>[0]`), which needs a checker, so
        // this also stands in when they are loaded (ADR 0014).
        "VariantProps" | "RecipeVariantProps" | "RecipeVariants" => resolve_cva_variant_props(args, global, enum_key),
        "InferRecipeProps" => resolve_inferred_recipe_props(args, global, enum_key),

        // ── React Aria ───────────────────────────────────────────────────────
        // RenderProps<ButtonRenderProps> → className, style and children, each a value or a function of the state
        "RenderProps" => Some(KnownPatternResult::Props(render_props(args))),
        // SlotProps → { slot?: string | null }
        "SlotProps" => Some(KnownPatternResult::Props(vec![slot_prop()])),

        // ── Chakra / Ark / Styled System ────────────────────────────────────
        // HTMLChakraProps<'button'> → same as ComponentPropsWithoutRef<'button'>
        "HTMLChakraProps" | "HTMLArkProps" | "HTMLStyledProps" => html_attrs_from_first_arg(args),

        // ── React standard ───────────────────────────────────────────────────
        "PropsWithChildren" => props_with_children(args),
        "PropsWithRef" => props_with_ref(args),

        // ComponentPropsWithoutRef<'button'> or ComponentPropsWithoutRef<typeof X>
        "ComponentPropsWithoutRef" | "ComponentProps" | "ComponentPropsWithRef" => component_props(args),

        // ElementRef<typeof X> → opaque Ref
        "ElementRef" => Some(KnownPatternResult::Type(PropType::Ref { element: None })),

        // ── Unknown type names (including TS utility types like Partial/Omit/Pick —
        // structurally handled in resolver/alias.rs, silently no-op'd via
        // resolver::is_ts_utility_type otherwise) — caller continues normal resolution.
        _ => None,
    }
}

fn resolve_cva_variant_props(
    args: &[PropType],
    global: &GlobalSourceData,
    enum_key: &dyn Fn(&str) -> Option<String>,
) -> Option<KnownPatternResult> {
    // The arg is typeof buttonVariants — a Named type reference to a cva() call result.
    // We stored the cva() call variants in global.enums during extraction.
    // Look them up and return as individual props.
    //
    // If we can't find the variants (e.g. they're imported from elsewhere),
    // return an Opaque rather than failing.

    match args.first() {
        Some(PropType::Named { name, .. }) => {
            let name_str = name.as_str();
            let found = enum_key(name_str).and_then(|key| global.enums.get_key_value(key.as_str()));

            match found {
                Some((_key, enum_entries)) => {
                    // Build one LiteralUnion prop per variant key found in enum entries.
                    // Group by variant key (the enum entry name acts as variant key).
                    // Each EnumEntry maps to a variant value; group all values for
                    // the same variant key into a LiteralUnion prop.
                    use std::collections::BTreeMap;
                    let mut variant_map: BTreeMap<String, Vec<String>> = BTreeMap::new();

                    for entry in enum_entries {
                        variant_map.entry(entry.name.clone()).or_default().push(entry.value.to_display_string());
                    }

                    if variant_map.is_empty() {
                        return Some(KnownPatternResult::Type(OpaqueDetail::new(
                            format!("VariantProps<typeof {}>", name_str),
                            OpaqueReason::RuntimeDependent { function_name: "cva".into() },
                        )));
                    }

                    // CVA/tv/defineRecipe VariantProps adds `| null` to every
                    // variant key (passing null opts out of the variant, using
                    // the default). Match what the TypeScript type system produces.
                    let props = variant_map
                        .into_iter()
                        .map(|(variant_key, values)| {
                            simple_prop(
                                &variant_key,
                                PropType::Union(vec![
                                    PropType::LiteralUnion { members: values, has_default: false },
                                    PropType::Null,
                                ]),
                                false,
                                "",
                            )
                        })
                        .collect();

                    Some(KnownPatternResult::Props(props))
                }
                None => {
                    // Variants not found in global data — degrade to opaque
                    Some(KnownPatternResult::Type(OpaqueDetail::new(
                        format!("VariantProps<typeof {}>", name_str),
                        OpaqueReason::RuntimeDependent { function_name: "cva".into() },
                    )))
                }
            }
        }
        _ => Some(KnownPatternResult::Type(OpaqueDetail::new(
            "VariantProps<...>",
            OpaqueReason::RuntimeDependent { function_name: "cva".into() },
        ))),
    }
}

fn resolve_inferred_recipe_props(
    args: &[PropType],
    global: &GlobalSourceData,
    enum_key: &dyn Fn(&str) -> Option<String>,
) -> Option<KnownPatternResult> {
    let Some(PropType::Named { name, .. }) = args.first() else { return None };
    let entries = enum_key(name).and_then(|key| global.enums.get(&key))?;
    let mut variants = std::collections::BTreeMap::<String, Vec<String>>::new();
    for entry in entries {
        variants.entry(entry.name.clone()).or_default().push(entry.value.to_display_string());
    }
    let props = variants
        .into_iter()
        .map(|(key, values)| {
            let mut literals = Vec::new();
            let mut boolean = false;
            for value in values {
                if matches!(value.as_str(), "true" | "false") {
                    boolean = true;
                } else if !literals.contains(&value) {
                    literals.push(value);
                }
            }
            let mut members = Vec::new();
            if !literals.is_empty() {
                members.push(PropType::LiteralUnion { members: literals, has_default: false });
            }
            if boolean {
                members.push(PropType::Boolean);
            }
            members.push(PropType::Undefined);
            simple_prop(
                &key,
                PropType::Named { name: "ConditionalValue".into(), args: vec![PropType::Union(members)] },
                false,
                "",
            )
        })
        .collect();
    Some(KnownPatternResult::Props(props))
}

/// React Aria's `RenderProps<T>`: `className`, `style` and `children` each take a value or a function of the render
/// state `T`.
fn render_props(args: &[PropType]) -> Vec<ParsedProp> {
    let state = args.first().map_or_else(|| "RenderProps".to_owned(), PropType::raw_string);
    let or_function = |value: PropType| {
        PropType::Union(vec![
            value.clone(),
            PropType::Function {
                type_params: vec![],
                params: vec![FunctionParam::new(
                    Some("values".into()),
                    PropType::Named { name: state.clone().into(), args: vec![] },
                )],
                return_type: Box::new(value),
            },
        ])
    };
    vec![
        simple_prop(
            "className",
            or_function(PropType::String),
            false,
            "CSS class name, or a function of the render state.",
        ),
        simple_prop(
            "style",
            or_function(PropType::CssProperties),
            false,
            "Inline styles, or a function of the render state.",
        ),
        simple_prop("children", or_function(PropType::ReactNode), false, "Content, or a function of the render state."),
    ]
}

fn slot_prop() -> ParsedProp {
    simple_prop(
        "slot",
        PropType::Union(vec![PropType::String, PropType::Null]),
        false,
        "Slot name for component context composition.",
    )
}

fn props_with_children(args: &[PropType]) -> Option<KnownPatternResult> {
    // PropsWithChildren<T> = T & { children?: ReactNode }
    // Delegate to T (first arg), children prop comes from resolver's HTML table
    args.first().map(|inner| KnownPatternResult::Alias { name: inner.raw_string() })
}

fn props_with_ref(args: &[PropType]) -> Option<KnownPatternResult> {
    args.first().map(|inner| KnownPatternResult::Alias { name: inner.raw_string() })
}

fn html_attrs_from_first_arg(args: &[PropType]) -> Option<KnownPatternResult> {
    // HTMLChakraProps<'button'> → HtmlAttributes { element: "button" }
    match args.first() {
        Some(PropType::StringLiteral(element)) => Some(KnownPatternResult::Type(PropType::HtmlAttributes {
            element: element.to_lowercase(),
            omitted: vec![],
        })),
        _ => None,
    }
}

fn component_props(args: &[PropType]) -> Option<KnownPatternResult> {
    match args.first() {
        Some(PropType::StringLiteral(element)) => {
            // ComponentPropsWithoutRef<'button'>
            Some(KnownPatternResult::Type(PropType::HtmlAttributes {
                element: element.to_lowercase(),
                omitted: vec![],
            }))
        }
        Some(PropType::Named { name, .. }) => {
            // ComponentPropsWithoutRef<typeof X> — delegate to X's props
            Some(KnownPatternResult::Alias { name: name.to_string() })
        }
        _ => None,
    }
}

fn simple_prop(name: &str, prop_type: PropType, required: bool, description: &str) -> ParsedProp {
    ParsedProp::new(
        name.to_owned(),
        prop_type,
        required,
        None,
        description.to_owned(),
        Default::default(),
        None,
        vec![],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashMap;

    /// Library types whose declaration measured better than a shortcut, or whose shortcut only hid them, resolve like
    /// any other name: their declaration when it is loaded, otherwise an unresolved named reference (ADR 0014).
    #[test]
    fn library_types_with_a_usable_declaration_are_not_shortcuts() {
        for name in [
            "SxProps",
            "SystemStyleObject",
            "SystemCssProperties",
            "OverridableStringUnion",
            "ThemingProps",
            "StylesApiProps",
            "MantineColor",
            "MantineSize",
            "MantineRadius",
        ] {
            assert!(resolve_known(name, &[], &GlobalSourceData::default(), &|_| None).is_none(), "{name}");
        }
    }

    #[test]
    fn test_component_props_without_ref_string_literal() {
        let args = vec![PropType::StringLiteral("button".into())];
        let result = resolve_known("ComponentPropsWithoutRef", &args, &GlobalSourceData::default(), &|_| None);
        assert!(
            matches!(result, Some(KnownPatternResult::Type(PropType::HtmlAttributes { ref element, .. })) if element == "button")
        );
    }

    #[test]
    fn test_html_chakra_props() {
        let args = vec![PropType::StringLiteral("input".into())];
        let result = resolve_known("HTMLChakraProps", &args, &GlobalSourceData::default(), &|_| None);
        assert!(
            matches!(result, Some(KnownPatternResult::Type(PropType::HtmlAttributes { ref element, .. })) if element == "input")
        );
    }

    #[test]
    fn test_props_with_children_aliases() {
        let args = vec![PropType::Named { name: "ButtonProps".into(), args: vec![] }];
        let result = resolve_known("PropsWithChildren", &args, &GlobalSourceData::default(), &|_| None);
        assert!(matches!(result, Some(KnownPatternResult::Alias { ref name, .. }) if name == "ButtonProps"));
    }

    #[test]
    fn test_partial_returns_none() {
        let result = resolve_known("Partial", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_render_props_returns_two_props() {
        let result = resolve_known("RenderProps", &[], &GlobalSourceData::default(), &|_| None);
        let Some(KnownPatternResult::Props(props)) = result else { panic!("expected Props result") };
        assert!(props.iter().any(|p| p.name == "className"));
        assert!(props.iter().any(|p| p.name == "style"));
    }

    #[test]
    fn test_slot_props_returns_slot_prop() {
        let result = resolve_known("SlotProps", &[], &GlobalSourceData::default(), &|_| None);
        let Some(KnownPatternResult::Props(props)) = result else { panic!("expected Props result") };
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].name, "slot");
        assert!(!props[0].required);
    }

    #[test]
    fn test_html_ark_props() {
        let args = vec![PropType::StringLiteral("div".into())];
        let result = resolve_known("HTMLArkProps", &args, &GlobalSourceData::default(), &|_| None);
        assert!(
            matches!(result, Some(KnownPatternResult::Type(PropType::HtmlAttributes { ref element, .. })) if element == "div")
        );
    }

    #[test]
    fn test_html_styled_props() {
        let args = vec![PropType::StringLiteral("span".into())];
        let result = resolve_known("HTMLStyledProps", &args, &GlobalSourceData::default(), &|_| None);
        assert!(
            matches!(result, Some(KnownPatternResult::Type(PropType::HtmlAttributes { ref element, .. })) if element == "span")
        );
    }

    #[test]
    fn test_props_with_ref_aliases() {
        let args = vec![PropType::Named { name: "InputProps".into(), args: vec![] }];
        let result = resolve_known("PropsWithRef", &args, &GlobalSourceData::default(), &|_| None);
        assert!(matches!(result, Some(KnownPatternResult::Alias { ref name, .. }) if name == "InputProps"));
    }

    #[test]
    fn test_component_props_without_ref_named_type() {
        let args = vec![PropType::Named { name: "MyComponent".into(), args: vec![] }];
        let result = resolve_known("ComponentPropsWithoutRef", &args, &GlobalSourceData::default(), &|_| None);
        assert!(matches!(result, Some(KnownPatternResult::Alias { ref name, .. }) if name == "MyComponent"));
    }

    #[test]
    fn test_component_props_with_ref() {
        let args = vec![PropType::StringLiteral("a".into())];
        let result = resolve_known("ComponentPropsWithRef", &args, &GlobalSourceData::default(), &|_| None);
        assert!(
            matches!(result, Some(KnownPatternResult::Type(PropType::HtmlAttributes { ref element, .. })) if element == "a")
        );
    }

    #[test]
    fn test_element_ref_returns_ref() {
        let result = resolve_known("ElementRef", &[], &GlobalSourceData::default(), &|_| None);
        assert!(matches!(result, Some(KnownPatternResult::Type(PropType::Ref { element: None }))));
    }

    #[test]
    fn test_required_returns_none() {
        let result = resolve_known("Required", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_readonly_returns_none() {
        let result = resolve_known("Readonly", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_non_nullable_returns_none() {
        let result = resolve_known("NonNullable", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_omit_returns_none() {
        let result = resolve_known("Omit", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_pick_returns_none() {
        let result = resolve_known("Pick", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_unknown_type_returns_none() {
        let result = resolve_known("SomeRandomType", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_recipe_variant_props_no_args_is_opaque() {
        let result = resolve_known("RecipeVariantProps", &[], &GlobalSourceData::default(), &|_| None);
        assert!(matches!(
            result,
            Some(KnownPatternResult::Type(PropType::Opaque(ref d)))
                if matches!(d.reason(), OpaqueReason::RuntimeDependent { .. })
        ));
    }

    #[test]
    fn test_variant_props_named_not_in_global_is_opaque() {
        let args = vec![PropType::Named { name: "buttonVariants".into(), args: vec![] }];
        let result = resolve_known("VariantProps", &args, &GlobalSourceData::default(), &|_| None);
        assert!(matches!(
            result,
            Some(KnownPatternResult::Type(PropType::Opaque(ref d)))
                if matches!(d.reason(), OpaqueReason::RuntimeDependent { .. })
        ));
    }

    #[test]
    fn test_variant_props_found_in_global_returns_props() {
        let mut enums: FxHashMap<String, Vec<EnumEntry>> = FxHashMap::default();
        enums.insert(
            "/src/button.ts:buttonVariants".to_string(),
            vec![
                EnumEntry {
                    name: "variant".into(),
                    value: EnumValue::String("default".into()),
                    description: String::new(),
                },
                EnumEntry {
                    name: "variant".into(),
                    value: EnumValue::String("destructive".into()),
                    description: String::new(),
                },
                EnumEntry { name: "size".into(), value: EnumValue::String("sm".into()), description: String::new() },
            ],
        );

        let global = GlobalSourceData { enums, ..Default::default() };

        let enum_key = |name: &str| (name == "buttonVariants").then(|| "/src/button.ts:buttonVariants".to_owned());

        let args = vec![PropType::Named { name: "buttonVariants".into(), args: vec![] }];
        let result = resolve_known("VariantProps", &args, &global, &enum_key);
        let Some(KnownPatternResult::Props(props)) = result else {
            panic!("expected Props result, got something else")
        };
        // Should have two variant keys: "variant" and "size"
        assert_eq!(props.len(), 2);
        assert!(props.iter().any(|p| p.name == "variant"));
        assert!(props.iter().any(|p| p.name == "size"));
    }

    #[test]
    fn test_component_props_no_args_returns_none() {
        let result = resolve_known("ComponentProps", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }

    #[test]
    fn test_html_chakra_props_no_args_returns_none() {
        let result = resolve_known("HTMLChakraProps", &[], &GlobalSourceData::default(), &|_| None);
        assert!(result.is_none());
    }
}
