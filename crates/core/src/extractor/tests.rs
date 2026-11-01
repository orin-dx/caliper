use super::*;
use crate::types::{DefaultSource, EnumValue};

#[test]
fn jsx_passed_to_a_response_constructor_does_not_make_a_route_handler_a_component() {
    let path = Utf8Path::new("/fixtures/app/og/route.tsx");
    let data = parse_file(
        path,
        "export async function GET(request: Request) { return new ImageResponse(<div>{request.url}</div>); }\n\
         export function Card(props: { title: string }) { return <div>{props.title}</div>; }",
    );

    assert_eq!(
        data.component_mappings.iter().map(|mapping| mapping.component_name.as_str()).collect::<Vec<_>>(),
        ["Card"]
    );
}

fn components_in(source: &str) -> Vec<String> {
    let data = parse_file(Utf8Path::new("/fixtures/helper.tsx"), source);
    data.component_mappings.iter().map(|mapping| mapping.component_name.clone()).collect()
}

#[test]
fn a_call_to_a_same_file_function_declared_to_return_an_element_is_element_evidence() {
    let source = "import type * as React from 'react';\n\
        function renderByProp(render: unknown, props: object, fallback: React.ReactElement): React.ReactElement { return fallback; }\n\
        export function MenuTrigger(props: { render?: unknown }) { return renderByProp(props.render, props, <button />); }\n\
        export const Arrow = (props: { render?: unknown }) => renderByProp(props.render, props, <button />);";
    assert_eq!(components_in(source), ["MenuTrigger", "Arrow"]);
}

#[test]
fn an_aliased_element_return_type_makes_the_helper_element_evidence() {
    let source = "import type { ReactElement } from 'react';\n\
        type Rendered = ReactElement;\n\
        type Output = Rendered | null;\n\
        const wrap = (child: unknown): Output => null;\n\
        export function Slot(props: { child?: unknown }) { return wrap(props.child); }";
    assert_eq!(components_in(source), ["Slot"]);
}

#[test]
fn a_call_to_a_helper_that_does_not_return_an_element_is_not_element_evidence() {
    let source = "import type { ReactElement } from 'react';\n\
        function label(props: object): string { return 'x'; }\n\
        async function later(): Promise<ReactElement> { throw new Error(); }\n\
        function untyped(props: object) { return props; }\n\
        export function Name(props: { id: string }) { return label(props); }\n\
        export function Later(props: { id: string }) { return later(); }\n\
        export function Untyped(props: { id: string }) { return untyped(props); }";
    assert_eq!(components_in(source), Vec::<String>::new());
}

#[test]
fn an_element_type_imported_from_elsewhere_does_not_make_a_helper_element_evidence() {
    let source = "import type { ReactElement } from './mine';\n\
        function build(): ReactElement { throw new Error(); }\n\
        export function Name(props: { id: string }) { return build(); }";
    assert_eq!(components_in(source), Vec::<String>::new());
}

#[test]
fn jsx_passed_to_a_constructor_stays_non_evidence_beside_an_element_helper() {
    let source = "import type { ReactElement } from 'react';\n\
        function wrap(child: ReactElement): ReactElement { return child; }\n\
        export async function GET(request: Request) { return new ImageResponse(wrap(<div />)); }\n\
        export function Card(props: { title: string }) { return wrap(<div>{props.title}</div>); }";
    assert_eq!(components_in(source), ["Card"]);
}

fn fixture_path(rel: &str) -> std::path::PathBuf {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("../../fixtures").join(rel)
}

#[test]
fn test_readonly_array_type_operator_is_kept_over_a_known_element() {
    // `readonly string[]` (the type-operator form, not the `Readonly<T>` utility
    // type — already handled separately) once captured its entire span as raw source
    // text unconditionally, found while investigating whether fully resolving
    // @types/react's real HTMLAttributes chain was tractable: ButtonHTMLAttributes'
    // real `defaultValue`/`value` fields use exactly this shape. Downstream
    // heuristics reject a raw string containing a space, so it degraded to
    // Opaque even though the actual element type (`string`) is fully knowable.
    let source = r#"
interface Props { items: readonly string[]; }
"#;
    let path = Utf8Path::new("/fixtures/readonly-array.tsx");
    let data = parse_file(path, source);

    let key = format!("{path}:Props");
    let iface = data.interfaces.get(&key).unwrap_or_else(|| panic!("expected interfaces to contain '{key}'"));
    let items = iface.props.iter().find(|p| p.name == "items").expect("'items' prop not found");

    assert!(
        matches!(&items.collected_type, CollectedType::Readonly(array)
            if matches!(&**array, CollectedType::Array(inner) if matches!(**inner, CollectedType::String))),
        "expected Readonly(Array(String)), got {:?}",
        items.collected_type
    );
}

#[test]
fn test_inline_object_type_alias_not_silently_dropped() {
    let source = r#"
type ToastVariant = { message: string; kind?: 'info' | 'error' };
"#;
    let path = Utf8Path::new("/fixtures/inline-alias.tsx");
    let data = parse_file(path, source);

    let key = format!("{path}:ToastVariant");
    let alias = data.type_aliases.get(&key).unwrap_or_else(|| {
        panic!("expected type_aliases to contain '{key}', got keys: {:?}", data.type_aliases.keys())
    });

    match alias {
        CollectedTypeAlias::Passthrough { target: CollectedType::Object(fields), .. } => {
            assert_eq!(fields.len(), 2);
            assert!(fields.iter().any(|f| f.name == "message"));
            assert!(fields.iter().any(|f| f.name == "kind"));
        }
        other => panic!("expected Passthrough{{Object}}, got {other:?}"),
    }
}

#[test]
fn test_bare_function_type_alias_not_silently_dropped() {
    // react-day-picker's real pattern: `type OnSelectHandler<T> = (selected: T, ...) => void`
    // — a bare function type as the alias body, previously falling through
    // classify_type_alias's `_ => None` and vanishing from data.type_aliases with
    // no diagnostic, so every `OnSelectHandler<Date>` reference resolved as unknown.
    let source = r#"
type OnSelectHandler<T> = (selected: T, triggerDate: Date) => void;
"#;
    let path = Utf8Path::new("/fixtures/function-alias.tsx");
    let data = parse_file(path, source);

    let key = format!("{path}:OnSelectHandler");
    let alias = data.type_aliases.get(&key).unwrap_or_else(|| {
        panic!("expected type_aliases to contain '{key}', got keys: {:?}", data.type_aliases.keys())
    });

    match alias {
        CollectedTypeAlias::Passthrough { target: CollectedType::Function { params, .. }, .. } => {
            assert_eq!(params.len(), 2, "expected 2 params, got {params:?}");
        }
        other => panic!("expected Passthrough{{Function}}, got {other:?}"),
    }
}

#[test]
fn string_literal_union_alias_stores_bare_member_values() {
    let path = Utf8Path::new("/fixtures/literal-alias.ts");
    let data = parse_file(path, r#"type Size = "sm" | "md";"#);

    let key = format!("{path}:Size");
    match data.type_aliases.get(&key) {
        Some(CollectedTypeAlias::LiteralUnion { members, .. }) => {
            assert_eq!(members, &vec!["sm".to_owned(), "md".to_owned()]);
        }
        other => panic!("expected LiteralUnion, got {other:?}"),
    }
}

#[test]
fn undefined_member_is_dropped_from_literal_union_alias() {
    let path = Utf8Path::new("/fixtures/literal-alias-undefined.ts");
    let data = parse_file(path, r#"type Size = "sm" | "md" | undefined;"#);

    let key = format!("{path}:Size");
    match data.type_aliases.get(&key) {
        Some(CollectedTypeAlias::LiteralUnion { members, .. }) => {
            assert_eq!(members, &vec!["sm".to_owned(), "md".to_owned()]);
        }
        other => panic!("expected LiteralUnion, got {other:?}"),
    }
}

#[test]
fn null_member_keeps_alias_out_of_literal_union() {
    let path = Utf8Path::new("/fixtures/literal-alias-null.ts");
    let data = parse_file(path, r#"type Size = "sm" | "md" | null;"#);

    let key = format!("{path}:Size");
    match data.type_aliases.get(&key) {
        Some(CollectedTypeAlias::Union { members, .. }) => {
            assert!(
                matches!(
                    members.as_slice(),
                    [CollectedType::StringLiteral(a), CollectedType::StringLiteral(b), CollectedType::Null]
                        if a == "sm" && b == "md"
                ),
                "expected [\"sm\", \"md\", null] members, got {members:?}"
            );
        }
        other => panic!("expected Union, got {other:?}"),
    }
}

#[test]
fn test_array_type_alias_not_silently_dropped() {
    // Storybook's real pattern: `type API_KeyCollection = string[]` — a bare
    // array type as the alias body. Same silent-vanishing bug as the function-
    // type/type-literal cases above: classify_type_alias's `_ => None` catch-all
    // dropped ANY type shape without a dedicated arm, not just those two, so
    // every `API_KeyCollection` reference resolved as unknown with a "Cannot
    // resolve type" diagnostic despite being a plain same-file declaration.
    let source = r#"
type API_KeyCollection = string[];
"#;
    let path = Utf8Path::new("/fixtures/array-alias.tsx");
    let data = parse_file(path, source);

    let key = format!("{path}:API_KeyCollection");
    let alias = data.type_aliases.get(&key).unwrap_or_else(|| {
        panic!("expected type_aliases to contain '{key}', got keys: {:?}", data.type_aliases.keys())
    });

    match alias {
        CollectedTypeAlias::Passthrough { target: CollectedType::Array(inner), .. } => {
            assert!(matches!(**inner, CollectedType::String), "expected Array<String>, got {inner:?}");
        }
        other => panic!("expected Passthrough{{Array}}, got {other:?}"),
    }
}

#[test]
fn test_tuple_type_alias_not_silently_dropped() {
    // Same class of bug as the array case above, exercised with a different
    // previously-unhandled shape to prove the fix is a general catch-all
    // fallback, not another narrowly-scoped special case for arrays only.
    let source = r#"
type Point = [number, number];
"#;
    let path = Utf8Path::new("/fixtures/tuple-alias.tsx");
    let data = parse_file(path, source);

    let key = format!("{path}:Point");
    let alias = data.type_aliases.get(&key).unwrap_or_else(|| {
        panic!("expected type_aliases to contain '{key}', got keys: {:?}", data.type_aliases.keys())
    });

    match alias {
        CollectedTypeAlias::Passthrough { target: CollectedType::Tuple(members), .. } => {
            assert_eq!(members.len(), 2, "expected 2 tuple members, got {members:?}");
        }
        other => panic!("expected Passthrough{{Tuple}}, got {other:?}"),
    }
}

#[test]
fn test_shadcn_button() {
    let fixture = fixture_path("shadcn/button.tsx");
    let source =
        std::fs::read_to_string(&fixture).unwrap_or_else(|_| panic!("fixture not found: {}", fixture.display()));
    let path = Utf8Path::new("/fixtures/shadcn/button.tsx");
    let data = parse_file(path, &source);

    assert!(
        data.component_mappings.iter().any(|m| m.component_name == "Button"),
        "Button component not found; mappings: {:?}",
        data.component_mappings.iter().map(|m| &m.component_name).collect::<Vec<_>>()
    );

    let btn = data.component_mappings.iter().find(|m| m.component_name == "Button").unwrap();
    assert_eq!(
        btn.props_type_name.as_str(),
        "ButtonProps",
        "Expected props_type_name 'ButtonProps', got '{}'",
        btn.props_type_name
    );
}

#[test]
fn test_interface_description_not_stolen_by_first_prop() {
    let source = r#"
/** Props for Button. */
interface ButtonProps {
  variant: string;
}
"#;
    let path = Utf8Path::new("/fixtures/inline.tsx");
    let data = parse_file(path, source);

    let iface = data.interfaces.values().find(|i| i.name.as_str() == "ButtonProps").unwrap();
    assert_eq!(iface.description, "Props for Button.");
    assert_eq!(iface.props[0].description, "");
}

#[test]
fn test_shadcn_input() {
    let fixture = fixture_path("shadcn/input.tsx");
    let source =
        std::fs::read_to_string(&fixture).unwrap_or_else(|_| panic!("fixture not found: {}", fixture.display()));
    let path = Utf8Path::new("/fixtures/shadcn/input.tsx");
    let data = parse_file(path, &source);

    assert!(
        data.component_mappings.iter().any(|m| m.component_name == "Input"),
        "Input component not found; mappings: {:?}",
        data.component_mappings.iter().map(|m| &m.component_name).collect::<Vec<_>>()
    );

    let inp = data.component_mappings.iter().find(|m| m.component_name == "Input").unwrap();
    assert_eq!(
        inp.props_type_name.as_str(),
        "InputProps",
        "Expected props_type_name 'InputProps', got '{}'",
        inp.props_type_name
    );
}

#[test]
fn test_radix_button_dts() {
    let fixture = fixture_path("radix/button.d.ts");
    let source =
        std::fs::read_to_string(&fixture).unwrap_or_else(|_| panic!("fixture not found: {}", fixture.display()));
    // .d.ts is NOT tsx — component detection is off, but interfaces should still collect
    let path = Utf8Path::new("/fixtures/radix/button.d.ts");
    let data = parse_file(path, &source);

    // The radix fixture declares `ButtonProps` interface
    let interface_names: Vec<_> = data.interfaces.keys().cloned().collect();
    assert!(
        interface_names.iter().any(|k| k.contains("ButtonProps")),
        "ButtonProps interface not collected; interfaces: {:?}",
        interface_names
    );
}

#[test]
fn test_interface_collected() {
    let source = r#"
        export interface FooProps {
            label: string;
            count?: number;
            onClick: () => void;
        }
    "#;
    let path = Utf8Path::new("/test/foo.tsx");
    let data = parse_file(path, source);

    assert!(data.interfaces.contains_key("/test/foo.tsx:FooProps"));
    let iface = &data.interfaces["/test/foo.tsx:FooProps"];
    assert_eq!(iface.name.as_str(), "FooProps");
    assert_eq!(iface.props.len(), 3);

    let label = iface.props.iter().find(|p| p.name == "label").unwrap();
    assert_eq!(label.collected_type.to_raw_string(), "string");
    assert!(label.required);

    let count = iface.props.iter().find(|p| p.name == "count").unwrap();
    assert!(!count.required);
}

#[test]
fn test_import_binding_collected() {
    let source = r#"
        import React, { FC, useState } from "react";
        import type { ButtonHTMLAttributes } from "react";
        import * as RadixButton from "@radix-ui/react-button";
    "#;
    let path = Utf8Path::new("/test/imports.ts");
    let data = parse_file(path, source);

    // FC should be recorded
    assert!(
        data.imports.iter().any(|i| i.local_name.as_str() == "FC"),
        "FC not in imports: {:?}",
        data.imports.iter().map(|i| i.local_name.as_str()).collect::<Vec<_>>()
    );

    // ButtonHTMLAttributes should be type-only
    let bha = data.imports.iter().find(|i| i.local_name.as_str() == "ButtonHTMLAttributes");
    assert!(bha.is_some(), "ButtonHTMLAttributes not found");
    assert!(bha.unwrap().is_type_only, "ButtonHTMLAttributes should be type-only");

    // Namespace import
    assert!(
        data.imports.iter().any(|i| i.local_name.as_str() == "RadixButton"),
        "RadixButton namespace not in imports"
    );
}

#[test]
fn test_fc_pattern() {
    let source = r#"
        import { FC } from "react";
        interface ButtonProps { label: string; }
        const Button: FC<ButtonProps> = ({ label }) => <button>{label}</button>;
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let btn = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(btn.is_some(), "Button not found via FC pattern");
    assert_eq!(btn.unwrap().props_type_name.as_str(), "ButtonProps");
}

#[test]
fn test_hoc_pattern() {
    let source = r#"
        interface CardProps { title: string; }
        const Card = memo(function Card(props: CardProps) { return null; });
    "#;
    let path = Utf8Path::new("/test/card.tsx");
    let data = parse_file(path, source);

    let card = data.component_mappings.iter().find(|m| m.component_name == "Card");
    assert!(card.is_some(), "Card not found via HOC pattern");
    assert_eq!(card.unwrap().props_type_name.as_str(), "CardProps");
}

#[test]
fn test_display_name_update() {
    let source = r#"
        interface BtnProps { text: string; }
        const Btn = React.forwardRef<HTMLButtonElement, BtnProps>(function(props, ref) {
            return null;
        });
        Btn.displayName = "Button";
    "#;
    let path = Utf8Path::new("/test/btn.tsx");
    let data = parse_file(path, source);

    // After displayName assignment, the component should be renamed
    let btn = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(
        btn.is_some(),
        "Button (renamed via displayName) not found; mappings: {:?}",
        data.component_mappings.iter().map(|m| &m.component_name).collect::<Vec<_>>()
    );
    // Content, not presence: AC-027 requires the renamed entry to still carry
    // the pre-rename props type name — checking only that "Button" exists
    // wouldn't catch a rename that dropped or corrupted props_type_name.
    assert_eq!(
        btn.unwrap().props_type_name.as_str(),
        "BtnProps",
        "renamed entry must still carry the pre-rename props type name"
    );
    assert!(
        !data.component_mappings.iter().any(|m| m.component_name == "Btn"),
        "the pre-rename key 'Btn' must no longer be present, got {:?}",
        data.component_mappings.iter().map(|m| &m.component_name).collect::<Vec<_>>()
    );
}

#[test]
fn a_generic_functions_result_is_its_return_annotation_or_its_one_asserted_return() {
    let source = r#"
        export function annotated<P>(render: unknown): React.FC<P> { return render as any; }
        export function asserted<P, R>(render: unknown) {
            const component = render;
            return component as unknown as Box<P, R>;
        }
        export function unasserted<P>(render: P) { return render; }
        export function twice<P>(flag: boolean) { return 1 as Box<P>; return 2 as Box<P>; }
        export function plain(render: unknown) { return render as Box<string>; }
    "#;
    let data = parse_file(Utf8Path::new("/test/factory.ts"), source);

    let mut results: Vec<(&str, Vec<&str>, String)> = data
        .function_results
        .iter()
        .map(|(key, result)| {
            let name = key.rsplit_once(':').unwrap().1;
            (name, result.type_params.iter().map(|p| p.as_str()).collect(), result.result.to_raw_string())
        })
        .collect();
    results.sort();
    assert_eq!(
        results,
        [("annotated", vec!["P"], "React.FC<P>".to_owned()), ("asserted", vec!["P", "R"], "Box<P, R>".to_owned())]
    );
}

#[test]
fn string_defaults_keep_their_source_quotes() {
    let source = r#"
        export function Button({ size = 'md', tone = "neutral", label = `Go` }: { size?: string; tone?: string; label?: string }) {
            return null;
        }
    "#;
    let data = parse_file(Utf8Path::new("/test/button.tsx"), source);
    let button = data.component_mappings.iter().find(|m| m.component_name == "Button").unwrap();

    let mut defaults: Vec<(&str, &str, bool)> =
        button.param_defaults.iter().map(|(name, d)| (name.as_str(), d.value.as_str(), d.computed)).collect();
    defaults.sort();
    assert_eq!(defaults, [("label", "`Go`", true), ("size", "'md'", false), ("tone", "\"neutral\"", false)]);
}

#[test]
fn test_default_props_assignment_merged_into_component_mapping() {
    // MUI-style: `Component.defaultProps = { size: 'md' }` (deprecated in
    // React 19 but still shipped in real .d.ts/.tsx). Distinct from
    // destructured defaults (`function Button({ size = 'md' })`, already
    // handled by extract_param_defaults) — this is a static assignment
    // after the component's own declaration.
    let source = r#"
        interface ButtonProps { size?: string; disabled?: boolean; }
        function Button(props: ButtonProps) {
            return null;
        }
        Button.defaultProps = { size: 'md', disabled: false };
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let button = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(button.is_some(), "Button mapping not found");
    let button = button.unwrap();

    let size_default = button.param_defaults.get("size");
    assert!(size_default.is_some(), "expected 'size' default from defaultProps, got {:?}", button.param_defaults);
    let size_default = size_default.unwrap();
    assert_eq!(size_default.value, "'md'");
    assert!(!size_default.computed);
    assert!(matches!(size_default.source, DefaultSource::DefaultProps));

    let disabled_default = button.param_defaults.get("disabled");
    assert!(disabled_default.is_some(), "expected 'disabled' default from defaultProps");
    assert_eq!(disabled_default.unwrap().value, "false");
}

#[test]
fn test_default_props_scan_survives_a_displayname_rename_to_a_different_string() {
    // Adversarial review finding: try_scan_display_name used to rename
    // mapping.component_name to the *string value* of `X.displayName = "..."`
    // immediately. When that string differs from the variable identifier
    // (a real, plausible pattern — an internal implementation name exposed
    // under a nicer public displayName), a later `X.defaultProps = {...}`
    // referencing the *original* identifier could no longer find the
    // mapping, since its component_name had already changed out from under
    // it. The rename must not affect lookups for the rest of the file.
    let source = r#"
        interface ButtonProps { size?: string; }
        function InternalButton(props: ButtonProps) {
            return null;
        }
        InternalButton.displayName = 'PublicButton';
        InternalButton.defaultProps = { size: 'md' };
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(names.contains(&"PublicButton"), "expected the displayName rename to apply, got {:?}", names);

    let button = data.component_mappings.iter().find(|m| m.component_name == "PublicButton");
    let button = button.expect("PublicButton mapping not found");
    let size_default = button.param_defaults.get("size");
    assert!(
        size_default.is_some(),
        "expected 'size' default from defaultProps to have been merged despite the earlier \
         displayName rename to a different string, got {:?}",
        button.param_defaults
    );
}

#[test]
fn test_two_aliases_to_the_same_base_component_both_survive() {
    // Adversarial review finding: try_rename_identifier_wrapped_component
    // mutated the matched mapping's component_name in place. A second,
    // different alias to the same base component (a real pattern —
    // exposing both a legacy and a new name for the same implementation)
    // could never find the base again, since the first alias had already
    // renamed it away — silently dropping the second alias with no trace.
    let source = r#"
        interface ButtonProps { label: string; }
        const InternalButton = React.forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
            return null;
        });
        const LegacyButton = InternalButton;
        const Button = InternalButton;
        export default Button;
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(names.contains(&"LegacyButton"), "expected LegacyButton to survive as its own alias, got {:?}", names);
    assert!(names.contains(&"Button"), "expected Button to survive as its own alias, got {:?}", names);
    assert!(
        !names.contains(&"InternalButton"),
        "the internal implementation name should not also appear as a separate public component, got {:?}",
        names
    );
    // Content, not presence: AC-021 requires BOTH aliases to carry the base
    // component's props type name — checking only that the names survive
    // wouldn't catch an aliasing bug that produced the right keys with the
    // wrong (or missing) props_type_name.
    let legacy = data.component_mappings.iter().find(|m| m.component_name == "LegacyButton").unwrap();
    let button = data.component_mappings.iter().find(|m| m.component_name == "Button").unwrap();
    assert_eq!(legacy.props_type_name.as_str(), "ButtonProps", "LegacyButton must carry the base's props type name");
    assert_eq!(button.props_type_name.as_str(), "ButtonProps", "Button must carry the base's props type name");
}

#[test]
fn test_identifier_wrapped_component_renamed_to_export_binding() {
    // Headless UI's real pattern: a standalone top-level function declaration
    // (already independently detected as its own component by Pattern 4) is
    // later wrapped by a same-file custom wrapper — not React's own forwardRef,
    // a library-defined one — and referenced by bare identifier, not inlined.
    // Before this fix, `ButtonFn` stayed visible under its own inner name
    // instead of being recognized as `ListboxButton`'s real implementation.
    let source = r#"
        interface ButtonProps { disabled?: boolean; }
        function ButtonFn(props: ButtonProps) {
            return null;
        }
        export let ListboxButton = forwardRefWithAs(ButtonFn) as unknown as SomeExportedType;
    "#;
    let path = Utf8Path::new("/test/listbox.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(
        names.contains(&"ListboxButton"),
        "expected ListboxButton (the real export name) among component mappings, got {:?}",
        names
    );
    assert!(
        !names.contains(&"ButtonFn"),
        "ButtonFn should be renamed to its real export name, not left visible as a separate/wrong component, got {:?}",
        names
    );
}

#[test]
fn test_bare_identifier_passthrough_alias_renames_component() {
    // antd's real Button pattern: no wrapper call at all, just a plain
    // `const NewName = OldName;` re-binding — narrower than
    // try_rename_identifier_wrapped_component's original CallExpression
    // case (Headless UI's forwardRefWithAs(ButtonFn) above), which never
    // matched a bare identifier init and left this silently unrenamed.
    let source = r#"
        interface ButtonProps { label: string; }
        const InternalCompoundedButton = React.forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
            return null;
        });
        const Button = InternalCompoundedButton;
        export default Button;
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(names.contains(&"Button"), "expected 'Button' (the real export name) among mappings, got {:?}", names);
    assert!(
        !names.contains(&"InternalCompoundedButton"),
        "InternalCompoundedButton should be renamed to its real export name, not left visible under the \
         internal name, got {:?}",
        names
    );
}

#[test]
fn test_bare_identifier_alias_with_as_cast_renames_component() {
    // The real upstream antd shape (per fixtures/antd/Button.tsx's own
    // comment): `const Button = InternalCompoundedButton as
    // CompoundedComponent;` — same bare-identifier passthrough, wrapped in
    // an `as` cast. unwrap_as_expression must see through it.
    let source = r#"
        interface ButtonProps { label: string; }
        const InternalCompoundedButton = React.forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
            return null;
        });
        type CompoundedComponent = typeof InternalCompoundedButton & { Group: unknown };
        const Button = InternalCompoundedButton as CompoundedComponent;
        export default Button;
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(names.contains(&"Button"), "expected 'Button' among mappings, got {:?}", names);
    assert!(!names.contains(&"InternalCompoundedButton"), "got {:?}", names);
}

#[test]
fn test_type_alias_omit() {
    let source = r#"
        interface FullProps { a: string; b: number; c: boolean; }
        type PartialProps = Omit<FullProps, 'b' | 'c'>;
    "#;
    let path = Utf8Path::new("/test/types.ts");
    let data = parse_file(path, source);

    let alias = data.type_aliases.get("/test/types.ts:PartialProps");
    assert!(alias.is_some(), "PartialProps alias not collected");
    match alias.unwrap() {
        CollectedTypeAlias::Omit { base, keys, .. } => {
            assert_eq!(base.to_raw_string(), "FullProps");
            assert_eq!(keys.as_string_union_keys(), vec!["b".to_owned(), "c".to_owned()]);
        }
        other => panic!("Expected Omit, got {:?}", other),
    }
}

#[test]
fn test_namespace_member_stored_under_qualified_name() {
    // Base UI's real pattern: `namespace MenuRoot { export type Props<Payload> = ... }`,
    // referenced elsewhere in the same file as `MenuRoot.Props<SomePayload>`. The
    // resolver looks up references by their fully-qualified dotted name (confirmed via
    // ts_type_name_str's TSTypeName::QualifiedName handling), so storage must key on
    // the same qualified name, not the bare member name.
    let source = r#"
        namespace MenuRoot {
            export type Props<Payload> = { open?: boolean; payload?: Payload };
        }
    "#;
    let path = Utf8Path::new("/test/menu-root.tsx");
    let data = parse_file(path, source);

    let qualified_key = format!("{path}:MenuRoot.Props");
    assert!(
        data.type_aliases.contains_key(&qualified_key),
        "expected type_aliases to contain '{qualified_key}', got keys: {:?}",
        data.type_aliases.keys().collect::<Vec<_>>()
    );
}

#[test]
fn test_deprecated_tag_does_not_bleed_to_sibling_props_without_jsdoc() {
    // Ant Design's real pattern: one prop has a real @deprecated JSDoc comment;
    // adjacent sibling props (declared close enough to fall within find_jsdoc's
    // proximity window) have no JSDoc of their own at all. Only the annotated
    // prop should carry the tag.
    let source = r#"
        interface ButtonProps {
            /** @deprecated use iconPlacement instead */
            iconPosition?: 'start' | 'end';
            shape?: 'default' | 'circle' | 'round';
            size?: 'small' | 'middle' | 'large';
        }
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);

    let key = format!("{path}:ButtonProps");
    let iface = data.interfaces.get(&key).expect("ButtonProps interface not collected");

    let icon_position = iface.props.iter().find(|p| p.name == "iconPosition").expect("iconPosition not found");
    assert!(icon_position.tags.contains_key("deprecated"), "iconPosition should carry its own @deprecated tag");

    let shape = iface.props.iter().find(|p| p.name == "shape").expect("shape not found");
    assert!(
        !shape.tags.contains_key("deprecated"),
        "shape has no JSDoc of its own and must not inherit iconPosition's @deprecated tag, got tags: {:?}",
        shape.tags
    );

    let size = iface.props.iter().find(|p| p.name == "size").expect("size not found");
    assert!(
        !size.tags.contains_key("deprecated"),
        "size has no JSDoc of its own and must not inherit iconPosition's @deprecated tag, got tags: {:?}",
        size.tags
    );
}

#[test]
fn test_exports_collected() {
    let source = r#"
        export { Button } from "./button";
        export * from "./types";
        export type { ButtonProps } from "./button";
    "#;
    let path = Utf8Path::new("/test/index.ts");
    let data = parse_file(path, source);

    assert!(!data.exports.is_empty(), "No exports collected");
    assert!(data.exports.iter().any(|e| matches!(e, LexedExport::ReExportAll { .. })), "ReExportAll not found");
    assert!(data.exports.iter().any(|e| matches!(e, LexedExport::ReExportNamed { .. })), "ReExportNamed not found");
}

#[test]
fn an_exported_namespace_import_is_a_namespace_reexport() {
    let source = "import * as menu from './menu';\nimport * as unused from './unused';\nexport { menu as Menu };\n";
    let data = parse_file(Utf8Path::new("/test/ui.ts"), source);

    let namespaces: Vec<(&str, &str)> = data
        .exports
        .iter()
        .filter_map(|e| match e {
            LexedExport::ReExportNamespace { namespace, source_specifier } => {
                Some((namespace.as_str(), source_specifier.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(namespaces, [("Menu", "./menu")]);
    assert_eq!(data.exports.len(), 1, "{:?}", data.exports);
}

#[test]
fn a_local_export_alias_records_the_declarations_own_name() {
    let source = r#"
        declare const DialogContent: unknown;
        interface DialogContentProps { a: string }
        export { DialogContent as Content, DialogContent, type DialogContentProps };
        export interface Own { b: string }
    "#;
    let data = parse_file(Utf8Path::new("/test/index.d.ts"), source);

    let locals: Vec<(&str, Option<&str>)> = data
        .exports
        .iter()
        .filter_map(|e| match e {
            LexedExport::LocalDeclaration { name, declared_as, .. } => Some((name.as_str(), declared_as.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(
        locals,
        [("Content", Some("DialogContent")), ("DialogContent", None), ("DialogContentProps", None), ("Own", None)]
    );
}

#[test]
fn every_export_form_lands_in_the_right_lexed_export_variant() {
    // Exact sets, not presence: a specifier list misclassified as a re-export would pass a presence check.
    let source = r#"
        const Local = 1;
        export { Local };
        export { Button } from "./button";
        export type { ButtonProps } from "./button";
        export * from "./types";
        export interface CardProps { a: string }
        export type Size = "sm";
        export enum Color { Red }
        export const VERSION = "1";
    "#;
    let data = parse_file(Utf8Path::new("/test/index.ts"), source);

    let mut locals: Vec<(&str, bool)> = data
        .exports
        .iter()
        .filter_map(|e| match e {
            LexedExport::LocalDeclaration { name, is_type_only, .. } => Some((name.as_str(), *is_type_only)),
            _ => None,
        })
        .collect();
    locals.sort();
    // `export_kind` only marks `export type { A }`; inline declarations are always value exports.
    assert_eq!(
        locals,
        vec![("CardProps", false), ("Color", false), ("Local", false), ("Size", false), ("VERSION", false),]
    );

    let mut re_exports: Vec<(&str, &str, bool)> = data
        .exports
        .iter()
        .filter_map(|e| match e {
            LexedExport::ReExportNamed { local_name, source_specifier, is_type_only, .. } => {
                Some((local_name.as_str(), source_specifier.as_str(), *is_type_only))
            }
            _ => None,
        })
        .collect();
    re_exports.sort();
    assert_eq!(re_exports, vec![("Button", "./button", false), ("ButtonProps", "./button", true)]);

    let star: Vec<&str> = data
        .exports
        .iter()
        .filter_map(|e| match e {
            LexedExport::ReExportAll { source_specifier, .. } => Some(source_specifier.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(star, vec!["./types"]);
}

#[test]
fn only_an_identifier_named_namespace_qualifies_the_types_declared_inside_it() {
    let source = r#"
        namespace Theme { export type Size = "sm"; }
        declare module "pkg" { export type Ambient = "a"; }
        declare global { type Global = "g"; }
    "#;
    let path = Utf8Path::new("/test/ns.ts");
    let data = parse_file(path, source);

    let mut keys: Vec<&str> = data.type_aliases.keys().map(String::as_str).collect();
    keys.sort();
    let expected = [format!("{path}:Ambient"), format!("{path}:Global"), format!("{path}:Theme.Size")];
    assert_eq!(keys, expected.iter().map(String::as_str).collect::<Vec<_>>());
}

/// csstype's shape: `namespace DataType { type ColorBase = ...; type Color = ColorBase | ... }`. A sibling is written bare
/// and stored qualified, so its reference is qualified too, unless a type parameter of the declaration shadows it.
#[test]
fn a_namespace_sibling_is_referenced_by_its_qualified_name() {
    let source = r#"
        namespace DataType {
            type Base = "red" | "blue";
            type Color<T = string> = Base | "green" | T;
            type Shadowed<Base> = Base | Color;
            interface Shape extends Frame<Base> { color: Color; shades: Array<Base> }
            interface Frame<T> { inner: T }
        }
        type Outside = Base;
    "#;
    let path = Utf8Path::new("/test/ns-siblings.ts");
    let data = parse_file(path, source);

    let alias = |name: &str| match &data.type_aliases[&format!("{path}:{name}")] {
        CollectedTypeAlias::Union { members, .. } => {
            members.iter().map(CollectedType::to_raw_string).collect::<Vec<_>>()
        }
        CollectedTypeAlias::Passthrough { target, .. } => vec![target.to_raw_string()],
        other => panic!("unexpected alias {other:?}"),
    };
    assert_eq!(alias("DataType.Color"), ["DataType.Base", "\"green\"", "T"]);
    assert_eq!(alias("DataType.Shadowed"), ["Base", "DataType.Color"]);
    assert_eq!(alias("Outside"), ["Base"]);

    let shape = &data.interfaces[&format!("{path}:DataType.Shape")];
    let props: Vec<_> = shape.props.iter().map(|p| (p.name.as_str(), p.collected_type.to_raw_string())).collect();
    assert_eq!(props, [("color", "DataType.Color".to_owned()), ("shades", "Array<DataType.Base>".to_owned())]);
    let extends: Vec<_> = shape
        .extends
        .iter()
        .map(|e| match e {
            ExtendsRef::SameFile { name, type_args } => {
                (name.to_string(), type_args.iter().map(CollectedType::to_raw_string).collect::<Vec<_>>())
            }
            other => panic!("unexpected extends {other:?}"),
        })
        .collect();
    assert_eq!(extends, [("DataType.Frame".to_owned(), vec!["DataType.Base".to_owned()])]);
}

/// antd's `Badge`: the defaults are declared in the body, `const { dot = false, overflowCount = 99 } = props`, and a
/// renamed one (`tone: t = 'a'`) is the prop `tone`, not the local `t`.
#[test]
fn defaults_destructured_from_the_props_parameter_in_the_body_are_the_components() {
    let source = r#"
        import * as React from "react";
        interface BadgeProps { dot?: boolean; overflowCount?: number; tone?: string; other?: number }
        export const Badge = React.forwardRef<HTMLSpanElement, BadgeProps>((props, ref) => {
            const { dot = false, overflowCount = 99, tone: t = 'a', other } = props;
            const { shadow = 1 } = somethingElse;
            return <span ref={ref} />;
        });
        export function Plain(props: BadgeProps) {
            const { dot = true } = props;
            function inner(props: { dot?: boolean }) { const { dot = 'nested' } = props; return dot; }
            return null;
        }
    "#;
    let data = parse_file(Utf8Path::new("/test/badge.tsx"), source);

    let defaults = |component: &str| {
        let mapping = data.component_mappings.iter().find(|m| m.component_name == component).unwrap();
        let mut found: Vec<_> = mapping.param_defaults.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect();
        found.sort();
        found
    };
    let owned =
        |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect::<Vec<_>>();
    assert_eq!(defaults("Badge"), owned(&[("dot", "false"), ("overflowCount", "99"), ("tone", "'a'")]));
    assert_eq!(defaults("Plain"), owned(&[("dot", "true")]));
}

/// `declare const Primitive: Primitives` has no initializer to read; its declared type is kept as the alias
/// `typeof Primitive`, and a tuple-typed constant (`NODES`) the same, so a member or an element type can be read.
#[test]
fn a_declared_constants_type_is_recorded_as_a_typeof_alias() {
    let source = r#"
        declare const NODES: readonly ["a", "div"];
        declare const Primitive: Primitives;
        declare let mutable: Primitives;
        declare const shape: { a: string };
    "#;
    let path = Utf8Path::new("/test/prim.d.ts");
    let data = parse_file(path, source);

    let mut keys: Vec<&str> = data.type_aliases.keys().map(String::as_str).collect();
    keys.sort();
    let expected = [format!("{path}:typeof NODES"), format!("{path}:typeof Primitive")];
    assert_eq!(keys, expected.iter().map(String::as_str).collect::<Vec<_>>());
    let CollectedTypeAlias::Passthrough { target, .. } = &data.type_aliases[&format!("{path}:typeof NODES")] else {
        panic!("expected a passthrough alias");
    };
    assert_eq!(target.to_raw_string(), "[\"a\", \"div\"]");
}

#[test]
fn test_forward_ref_component_annotation_with_as_cast() {
    // Fluent UI's real pattern: no explicit forwardRef<Ref, Props> type args, and
    // no type annotation on the render function's own props param — the props type
    // is only ever spelled out via the wrapper annotation and the matching trailing
    // `as` cast (both point at the same ForwardRefComponent<ButtonProps>).
    let source = r#"
        import * as React from "react";
        interface ButtonProps { label: string; disabled?: boolean }
        type ForwardRefComponent<P> = (props: P) => React.ReactElement | null;
        export const Button: ForwardRefComponent<ButtonProps> = React.forwardRef((props, ref) => (
            <button ref={ref}>{props.label}</button>
        )) as ForwardRefComponent<ButtonProps>;
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);
    let mapping = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(mapping.is_some(), "Button not detected via ForwardRefComponent<Props> annotation + as-cast");
    let mapping = mapping.unwrap();
    assert_eq!(mapping.props_type_name, "ButtonProps");
}

#[test]
fn test_forward_ref_explicit_generics_survives_as_cast() {
    // Defensive generalization: an explicit forwardRef<Ref, Props>(fn) call can also
    // be wrapped in a trailing `as` cast (e.g. to a component-family union type).
    let source = r#"
        import { forwardRef } from "react";
        interface ButtonProps { label: string }
        type AnyComponent = unknown;
        export const Button = forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => (
            <button ref={ref}>{props.label}</button>
        )) as AnyComponent;
    "#;
    let path = Utf8Path::new("/test/button2.tsx");
    let data = parse_file(path, source);
    let mapping = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(mapping.is_some(), "Button not detected via explicit forwardRef<Ref, Props> wrapped in an as-cast");
    let mapping = mapping.unwrap();
    assert_eq!(mapping.props_type_name, "ButtonProps");
}

#[test]
fn test_wrappers_that_do_not_change_the_value_are_peeled() {
    // Parentheses and a trailing `!` are value-preserving, so they must not hide the wrapper call.
    let source = r#"
        import { forwardRef, memo } from "react";
        interface AProps { a: string }
        interface BProps { b: string }
        interface CProps { c: string }
        export const A = (forwardRef<HTMLDivElement, AProps>((props, ref) => <div ref={ref} />));
        export const B = forwardRef<HTMLDivElement, BProps>((props, ref) => <div ref={ref} />)!;
        export const C = (memo(forwardRef<HTMLDivElement, CProps>((props, ref) => <div ref={ref} />)) as unknown)!;
    "#;
    let data = parse_file(Utf8Path::new("/test/wrapped.tsx"), source);
    let found: Vec<(&str, &str)> =
        data.component_mappings.iter().map(|m| (m.component_name.as_str(), m.props_type_name.as_str())).collect();
    assert_eq!(found, vec![("A", "AProps"), ("B", "BProps"), ("C", "CProps")]);
}

#[test]
fn each_mapping_records_the_rule_that_found_it() {
    use crate::types::DetectionRule::*;
    let source = r#"
        import React, { forwardRef, memo, Component } from "react";
        interface P { a: string }
        export const Fc: React.FC<P> = (props) => <div />;
        export const Fwd = forwardRef<HTMLDivElement, P>((props, ref) => <div ref={ref} />);
        export const Hoc = memo(function Hoc(props: P) { return <div />; });
        export function Decl(props: P) { return <div />; }
        export class Cls extends Component<P> { render() { return <div />; } }
    "#;
    let data = parse_file(Utf8Path::new("/test/rules.tsx"), source);
    let found: Vec<(&str, crate::types::DetectionRule)> =
        data.component_mappings.iter().map(|m| (m.component_name.as_str(), m.rule)).collect();
    assert_eq!(
        found,
        vec![
            ("Fc", FcAnnotation),
            ("Fwd", ForwardRef),
            ("Hoc", HocWrapper),
            ("Decl", FunctionDeclaration),
            ("Cls", ClassComponent)
        ]
    );
}

#[test]
fn typed_arrow_and_function_expression_components_are_detected() {
    use crate::types::DetectionRule::TypedFunctionExpression;
    let source = r#"
        interface AProps { a: string }
        interface BProps { b: string }
        interface CProps { c: string }
        export const A = ({ a }: AProps) => <div>{a}</div>;
        export const B = function (props: BProps) { return <div />; };
        export const C = async (props: CProps): Promise<JSX.Element> => <div />;
        export const helper = (props: AProps) => props.a;
        export const Sum = (a: number, b: number) => a + b;
        export const Untyped = (props) => <div />;
    "#;
    let data = parse_file(Utf8Path::new("/test/arrows.tsx"), source);
    let found: Vec<(&str, &str, crate::types::DetectionRule)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), m.rule))
        .collect();
    // C is an async server component: its declared `Promise<JSX.Element>` return is the evidence.
    assert_eq!(
        found,
        vec![
            ("A", "AProps", TypedFunctionExpression),
            ("B", "BProps", TypedFunctionExpression),
            ("C", "CProps", TypedFunctionExpression)
        ]
    );
}

#[test]
fn typed_functions_that_never_render_are_declined_with_the_reason() {
    let source = r#"
        interface User { first: string; last: string }
        export const FormatName = (u: User) => `${u.first} ${u.last}`;
        export const ToDto = (u: User): { id: string } => ({ id: u.first });
        export function FullName(u: User): string { return u.first; }
        export function* Saga(u: User) { yield u; }
        export const Search = debounce((q: User) => q.first, 100);
        export const Selector = createSelector((u: User) => u.first);
        export function Label(u: User) { const unused = <b>{u.first}</b>; return u.first; }
        export function Columns(u: User) { return { title: <b>{u.first}</b> }; }
        export function OnLater(u: User) { setTimeout(() => <i />); return u.last; }
        export function ResponseOfJSX(u: User) { return makeResponse(<b>{u.first}</b>); }
        import type { ReactNode } from "./my-types";
        export function Typed(u: User): ReactNode { return u.first; }
    "#;
    let data = parse_file(Utf8Path::new("/test/helpers.tsx"), source);
    assert_eq!(
        data.component_mappings.iter().map(|m| m.component_name.as_str()).collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    let mut reasons: Vec<String> = data
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::SkippedCandidate)
        .map(|d| d.message.clone())
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        vec![
            "'Columns' takes a typed props parameter but never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'FormatName' takes a typed props parameter but never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'FullName' takes a typed props parameter but declares a return type that is not React's element type",
            "'Label' takes a typed props parameter but never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'OnLater' takes a typed props parameter but never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'ResponseOfJSX' takes a typed props parameter but never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'Saga' takes a typed props parameter but is a generator",
            "'Search' wraps a typed function that never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'Selector' wraps a typed function that never returns an element: no returned JSX, element, null, children or component call, and no hook call",
            "'ToDto' takes a typed props parameter but declares a return type that is not React's element type",
            "'Typed' takes a typed props parameter but declares a return type that is not React's element type",
        ]
    );
}

#[test]
fn components_whose_returned_value_holds_an_element_are_detected() {
    let source = r#"
        import type { ReactNode } from "react";
        interface P { items: string[]; open?: boolean }
        export function Held({ items }: P) { const content = <ul>{items}</ul>; return content; }
        export function Mapped({ items }: P) { return items.map((i) => <li key={i}>{i}</li>); }
        export function Reduced({ items }: P) { return items.reduce((acc, i) => <>{acc}{i}</>, <></>); }
        export function Branch({ open }: P) { if (!open) { return null; } return open ? <div /> : <span />; }
        export function Guarded({ open }: P) { return open && <div />; }
        export function Annotated(p: P): ReactNode { return p.items[0]; }
        export function Qualified(p: P): React.ReactNode { return p.items[0]; }
    "#;
    let data = parse_file(Utf8Path::new("/test/shapes.tsx"), source);
    let mut names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Annotated", "Branch", "Guarded", "Held", "Mapped", "Qualified", "Reduced"]);
}

#[test]
fn components_that_render_without_jsx_are_still_detected() {
    let source = r#"
        import { createElement, cloneElement } from "react";
        interface P { children?: any; el?: any }
        export const Passthrough = ({ children }: P) => children;
        export function Slot(props: P) { return cloneElement(props.el, {}); }
        export const Factory = (props: P) => createElement("div", props);
        export function Hidden(props: P) { if (!props.el) return null; return props.children; }
    "#;
    let data = parse_file(Utf8Path::new("/test/no_jsx.tsx"), source);
    let mut names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Factory", "Hidden", "Passthrough", "Slot"]);
}

#[test]
fn a_component_passed_to_a_non_wrapper_call_keeps_its_name_and_the_call_is_not_a_component() {
    let source = r#"
        import { memo } from "react";
        interface CardProps { title?: string }
        export function Card(props: CardProps) { return <div />; }
        export const MemoCard = memo(Card);
        export const CardName = getDisplayName(Card);
        function InnerPill(props: CardProps) { return <span />; }
        export const Pill = withTheme(InnerPill);
    "#;
    let data = parse_file(Utf8Path::new("/test/alias.tsx"), source);
    let mut names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    names.sort_unstable();
    // `Card` is exported, so its alias does not hide it; `InnerPill` is not, so `Pill` replaces it.
    assert_eq!(names, vec!["Card", "MemoCard", "Pill"]);
    assert!(data.diagnostics.iter().any(
        |d| d.message == "'CardName' passes component 'Card' to a call that does not look like a component wrapper"
    ));
}

#[test]
fn a_wrapper_call_naming_its_props_type_does_not_take_the_props_of_the_component_it_receives() {
    use crate::types::DetectionRule::{ForwardRef, WrapperTypeArgument};
    let source = r#"
        import { forwardRef, memo } from "react";
        interface BaseProps { asChild?: boolean }
        interface OverlayProps { tone?: string }
        const BaseOverlay = forwardRef<HTMLDivElement, BaseProps>((props, ref) => <div ref={ref} />);
        export const Overlay = withContext<HTMLDivElement, OverlayProps>(BaseOverlay, "overlay");
        export const Memoized = memo(BaseOverlay);
        export const Themed = withTheme<HTMLDivElement>(BaseOverlay);
    "#;
    let data = parse_file(Utf8Path::new("/test/overlay.tsx"), source);
    let mut found: Vec<(&str, &str, crate::types::DetectionRule)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), m.rule))
        .collect();
    found.sort_by_key(|(name, _, _)| *name);
    assert_eq!(
        found,
        vec![
            ("Memoized", "BaseProps", ForwardRef),
            ("Overlay", "OverlayProps", WrapperTypeArgument),
            ("Themed", "BaseProps", ForwardRef),
        ]
    );
}

#[test]
fn anonymous_default_exports_are_named_after_their_file() {
    let cases = [
        ("/test/chip.tsx", "interface ChipProps { a?: string }\nexport default (props: ChipProps) => <span />;\n"),
        (
            "/test/badge.tsx",
            "import { memo } from 'react';\ninterface BadgeProps { a?: string }\nexport default memo(function Badge(props: BadgeProps) { return <b />; });\n",
        ),
        (
            "/test/field.tsx",
            "import { forwardRef } from 'react';\ninterface FieldProps { a?: string }\nexport default forwardRef<HTMLInputElement, FieldProps>((props, ref) => <input ref={ref} />);\n",
        ),
        (
            "/test/panel.tsx",
            "import { Component } from 'react';\ninterface PanelProps { a?: string }\nexport default class extends Component<PanelProps> { render() { return <div />; } }\n",
        ),
    ];
    let found: Vec<(String, String)> = cases
        .iter()
        .flat_map(|(path, source)| {
            parse_file(Utf8Path::new(path), source)
                .component_mappings
                .into_iter()
                .map(|m| (m.component_name, m.props_type_name.to_string()))
        })
        .collect();
    let pair = |n: &str, p: &str| (n.to_owned(), p.to_owned());
    assert_eq!(
        found,
        vec![
            pair("Chip", "ChipProps"),
            pair("Badge", "BadgeProps"),
            pair("Field", "FieldProps"),
            pair("Panel", "PanelProps")
        ]
    );
}

#[test]
fn a_declared_function_with_an_element_return_type_is_a_component() {
    let source = r#"
        interface ButtonProps { label?: string }
        export declare function Button(props: ButtonProps): JSX.Element;
        export declare function format(props: ButtonProps): string;
        export declare function Format(props: ButtonProps): string;
    "#;
    let data = parse_file(Utf8Path::new("/test/button.d.ts"), source);
    assert_eq!(data.component_mappings.iter().map(|m| m.component_name.as_str()).collect::<Vec<_>>(), vec!["Button"]);
}

#[test]
fn wrapper_calls_carrying_props_in_a_type_argument_are_detected_by_shape() {
    use crate::types::DetectionRule::WrapperTypeArgument;
    let source = r#"
        import { factory, withContext, makeThing, createContext } from "./lib";
        import type { RemoteProps, RemoteFactory } from "./remote";
        interface ButtonProps { label: string }
        interface ItemProps { item: string }
        export const Button = factory<ButtonFactory>((_props, ref) => <button ref={ref} />);
        export const Item = withContext<HTMLDivElement, ItemProps>("div", "item");
        export const Remote = withContext<HTMLDivElement, RemoteProps>("div");
        export const Late = factory<LateFactory>(() => null);
        type LateFactory = Factory<{ props: ItemProps; ref: HTMLDivElement }>;
        type ButtonFactory = Factory<{ props: ButtonProps; ref: HTMLButtonElement; stylesNames: "root" }>;

        export const Theme = createContext<ButtonProps>({ label: "" });
        export const Element = makeThing<HTMLDivElement>("div");
        export const Unknown = makeThing<RemoteFactory>("div");
        export const Constant = makeThing<ButtonProps>(42);
        export const Nothing = makeThing<ButtonProps>();
    "#;
    let data = parse_file(Utf8Path::new("/test/wrappers.tsx"), source);
    let mut found: Vec<(&str, &str, crate::types::DetectionRule)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), m.rule))
        .collect();
    found.sort_by_key(|(name, _, _)| *name);
    assert_eq!(
        found,
        vec![
            ("Button", "ButtonProps", WrapperTypeArgument),
            ("Item", "ItemProps", WrapperTypeArgument),
            ("Late", "ItemProps", WrapperTypeArgument),
            ("Remote", "RemoteProps", WrapperTypeArgument),
        ]
    );
}

#[test]
fn a_function_component_annotation_inside_an_intersection_carries_the_props() {
    use crate::types::DetectionRule::FcAnnotation;
    let source = r#"
        import * as React from "react";
        interface DrawerProps { open: boolean }
        interface PanelProps { title: string }
        export const Drawer: React.FC<DrawerProps> & { _Panel: React.FC<PanelProps> } = (props) => <div />;
        export const Sheet: { size: number } & React.FC<DrawerProps> = (props) => <div />;
        export const Plain: { size: number } & { depth: number } = { size: 1, depth: 2 };
    "#;
    let data = parse_file(Utf8Path::new("/test/drawer.tsx"), source);
    let found: Vec<(&str, &str, crate::types::DetectionRule)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), m.rule))
        .collect();
    assert_eq!(found, vec![("Drawer", "DrawerProps", FcAnnotation), ("Sheet", "DrawerProps", FcAnnotation)]);
}

#[test]
fn a_call_given_a_tag_and_a_config_object_is_a_component_when_a_props_type_beside_it_gives_that_tag() {
    use crate::types::DetectionRule::TaggedFactory;
    let source = r#"
        import { chakra, styled } from "../styled-system";
        import { createElement, h } from "react";
        export interface CenterProps extends HTMLChakraProps<"div"> {}
        export const Center = chakra("div", { base: { display: "flex" }, variants: { inline: { true: {} } } });
        type PillProps = HTMLStyledProps<"span"> & { tone: string };
        export const Pill = styled("span", { base: {} });
        interface LinkProps extends HTMLChakraProps<"a"> {}
        export const Anchor = chakra("a", { base: {} });
        type UnrelatedProps = { tone: string };
        export const Unrelated = chakra("section", { base: {} });
        export const Element = createElement("div", { className: "x" });
        export const Hyper = h("div", { className: "x" });
        export const Tagless = chakra(Center, { base: {} });
        export const Generic = chakra<CenterProps>("div", { base: {} });
        export const NoConfig = chakra("div");
    "#;
    let data = parse_file(Utf8Path::new("/test/tagged.tsx"), source);
    let mut found: Vec<(&str, &str, crate::types::DetectionRule)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), m.rule))
        .collect();
    found.sort_by_key(|(name, _, _)| *name);
    assert_eq!(
        found,
        vec![
            ("Anchor", "LinkProps", TaggedFactory),
            ("Center", "CenterProps", TaggedFactory),
            ("Pill", "PillProps", TaggedFactory)
        ]
    );
}

#[test]
fn a_wrapped_render_function_may_be_parenthesized_or_asserted() {
    use crate::types::DetectionRule::{HocWrapper, WrapperTypeArgument};
    let source = r#"
        import { genericFactory } from "./lib";
        interface GroupProps { value: string }
        type GroupFactory = Factory<{ props: GroupProps; ref: HTMLDivElement }>;
        export const Group = genericFactory<GroupFactory>(((props: GroupProps) => <div />) as any);
        export const Plain = genericFactory<GroupFactory>(((props: GroupProps) => <div />));
        export const Counter = genericFactory<GroupFactory>(((props: GroupProps) => 42) as any);
    "#;
    let data = parse_file(Utf8Path::new("/test/parenthesized.tsx"), source);
    let mut found: Vec<(&str, &str, crate::types::DetectionRule)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), m.rule))
        .collect();
    found.sort_by_key(|(name, _, _)| *name);
    assert_eq!(found, vec![("Group", "GroupProps", WrapperTypeArgument), ("Plain", "GroupProps", HocWrapper)]);
}

#[test]
fn a_factory_payload_with_generic_props_yields_those_props_never_the_payload() {
    let source = r#"
        import { genericFactory } from "./lib";
        export interface OverflowListProps<T = any> { data: T[] }
        export type OverflowListFactory = Factory<{
          props: OverflowListProps<any>;
          ref: HTMLDivElement;
          signature: <T = any>(props: OverflowListProps<T>) => React.JSX.Element;
        }>;
        export const OverflowList = genericFactory<OverflowListFactory>((_props) => <div />);
        export type InlineFactory = Factory<{ props: { label: string }; ref: HTMLDivElement }>;
        export const Inline = genericFactory<InlineFactory>((_props) => <div />);
    "#;
    let data = parse_file(Utf8Path::new("/test/overflow.tsx"), source);

    let found: Vec<(&str, &str, String)> = data
        .component_mappings
        .iter()
        .map(|m| (m.component_name.as_str(), m.props_type_name.as_str(), format!("{:?}", m.props_type_args)))
        .collect();
    assert_eq!(found, vec![("OverflowList", "OverflowListProps", "[Any]".to_owned())]);
    let skipped: Vec<&str> = data
        .diagnostics
        .iter()
        .filter(|d| d.code == crate::types::DiagnosticCode::SkippedCandidate)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        skipped,
        vec!["'Inline' is built from the factory payload 'InlineFactory', whose `props` member is not a named type"]
    );
}

#[test]
fn functions_used_only_as_functions_are_not_components() {
    let source = r#"
        type T = { type: string };
        interface P { label: string }
        function IndexedAccess(props: T) { return <span>{props.type}</span>; }
        function describe(t: T) { return IndexedAccess(t); }
        export function Exported(props: T) { return <b />; }
        const viaExport = Exported(x);
        function Row(props: P) { return <li />; }
        const rows = items.map(Row);
        function Inline(props: P) { return <i />; }
        const alsoCalled = Inline(p);
        const shown = <Inline label="x" />;
        function DropWrapper(props: P, ref: ForwardedRef<HTMLElement>) { return <div ref={ref} />; }
        const context = { render: DropWrapper };
        function Rendered(props: P, ref: ForwardedRef<HTMLElement>) { return <div />; }
        const r = <Rendered label="x" />;
    "#;
    let data = parse_file(Utf8Path::new("/test/uses.tsx"), source);

    let mut found: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    found.sort_unstable();
    assert_eq!(found, vec!["Exported", "Inline", "Rendered", "Row"]);
    let skipped: Vec<&str> = data
        .diagnostics
        .iter()
        .filter(|d| d.message.ends_with("not a component"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        skipped,
        vec![
            "'IndexedAccess' is only ever called as a function in this file, never rendered or passed on, so it is \
             a helper, not a component",
            "'DropWrapper' takes a second parameter (a ref) and is never rendered or passed to a component wrapper in \
             this file, so it is a render callback, not a component",
        ]
    );
}

#[test]
fn a_component_is_exported_when_its_module_exports_it_or_attaches_it_to_an_export() {
    let exports = |path: &str, source: &str| -> Vec<(String, bool)> {
        let data = parse_file(Utf8Path::new(path), source);
        let mut found: Vec<(String, bool)> =
            data.component_mappings.iter().map(|m| (m.component_name.to_string(), m.exported)).collect();
        found.sort_unstable();
        found
    };
    let source = r#"
        interface P { label: string }
        export function Direct(props: P) { return <b />; }
        export const Arrow = (props: P) => <i />;
        function Renamed(props: P) { return <u />; }
        export { Renamed as Public };
        function Defaulted(props: P) { return <s />; }
        export default Defaulted;
        function Item(props: P) { return <li />; }
        Direct.Item = Item;
        function Header(props: P) { return <h1 />; }
        export const Card = Object.assign((props: P) => <div />, { Header });
        function MenuRoot(props: P) { return <ul />; }
        function MenuItem(props: P) { return <li />; }
        export const Menu = Object.assign(MenuRoot, { Item: MenuItem });
        function Internal(props: P) { return <p />; }
        function Orphan(props: P) { return <em />; }
        Internal.Orphan = Orphan;
        const shown = <><Internal label="x" /><Orphan label="y" /></>;
    "#;
    assert_eq!(
        exports("/test/exports.tsx", source),
        vec![
            ("Arrow".to_string(), true),
            ("Card".to_string(), true),
            ("Defaulted".to_string(), true),
            ("Direct".to_string(), true),
            ("Header".to_string(), true),
            ("Internal".to_string(), false),
            ("Item".to_string(), true),
            ("MenuItem".to_string(), true),
            ("MenuRoot".to_string(), true),
            ("Orphan".to_string(), false),
            ("Renamed".to_string(), true),
        ]
    );
    assert_eq!(
        exports("/test/fancy-badge.tsx", "export default function (props: { tone: string }) { return <span />; }"),
        vec![("FancyBadge".to_string(), true)]
    );
}

#[test]
fn test_memo_forward_ref_detected() {
    let source = r#"
        import React, { memo, forwardRef } from "react";
        interface ButtonProps { label: string; disabled?: boolean }
        export const Button = memo(forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => (
            <button ref={ref} {...props}>{props.label}</button>
        )));
    "#;
    let path = Utf8Path::new("/test/button.tsx");
    let data = parse_file(path, source);
    let mapping = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(mapping.is_some(), "Button not detected via memo(forwardRef(...))");
    let mapping = mapping.unwrap();
    assert_eq!(mapping.props_type_name, "ButtonProps");
}

#[test]
fn test_enum_collected() {
    let source = r#"
        enum Direction {
            Up = "UP",
            Down = "DOWN",
            Left = "LEFT",
            Right = "RIGHT",
        }
    "#;
    let path = Utf8Path::new("/test/enum.ts");
    let data = parse_file(path, source);

    let entries = data.enums.get("/test/enum.ts:Direction");
    assert!(entries.is_some(), "Direction enum not collected");
    let entries = entries.unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().any(|e| e.name == "Up"));
}

#[test]
fn test_const_array_as_const_collected() {
    // antd's `type ButtonType = (typeof _ButtonTypes)[number]` pattern —
    // a flat array literal, not an object (test_enum_collected above) or a
    // cva-style variants config. Distinct storage: SourceData::const_arrays.
    let source = r#"
        const _ButtonTypes = ['default', 'primary', 'dashed', 'link', 'text'] as const;
    "#;
    let path = Utf8Path::new("/test/buttonHelpers.tsx");
    let data = parse_file(path, source);

    let values = data.const_arrays.get("/test/buttonHelpers.tsx:_ButtonTypes");
    assert!(values.is_some(), "_ButtonTypes const array not collected");
    let values = values.unwrap();
    assert_eq!(values.len(), 5);
    assert_eq!(values[0], EnumValue::String("default".into()));
    assert!(data.enums.is_empty(), "a plain const array must not be captured as an enum");
}

#[test]
fn test_excessive_nesting_guard() {
    // ~2500 levels of paren nesting — well past MAX_SOURCE_NESTING_DEPTH.
    // Before the guard, this would hand a deeply nested expression straight to
    // oxc_parser's recursive-descent parser and risk a stack overflow.
    let nested = "(".repeat(2500) + &")".repeat(2500);
    let source = format!("const x = {nested};");
    let path = Utf8Path::new("/test/deep.ts");

    let data = parse_file(path, &source);

    assert_eq!(data.diagnostics.len(), 1, "expected exactly one diagnostic, got {:?}", data.diagnostics);
    assert_eq!(data.diagnostics[0].code, DiagnosticCode::ExcessiveNesting);
    assert!(data.component_mappings.is_empty(), "no components should be extracted from a skipped file");
    assert!(data.interfaces.is_empty());
}

#[test]
fn test_nesting_guard_ignores_brackets_inside_comments() {
    // Regression test for: TypeScript's own real lib.dom.d.ts ships JSDoc
    // comments scraped from MDN containing artifacts like
    // `... MISSING: RFC(5646, '...')].` — a stray, unmatched `]` with no
    // opening `[` anywhere nearby (confirmed present verbatim ~2000 times
    // in the real file). `max_bracket_nesting_depth` counted brackets
    // byte-by-byte with no comment/string awareness, so each of these
    // drove its running depth negative; once enough had accumulated, the
    // next legitimate `(`/`{`/`[` in real code still left depth negative,
    // and casting that negative `i64` to `usize` wrapped to ~u64::MAX,
    // spuriously tripping the "exceeds maximum nesting depth" guard and
    // silently discarding the entire file — 0 interfaces extracted from a
    // 1.8MB file whose real code nesting never exceeds a handful of levels.
    let noisy_comment = "/** MISSING: RFC(5646, 'tag')]. */\n".repeat(10);
    let source = format!("{noisy_comment}interface Foo {{ bar: string; }}");
    let observed = max_bracket_nesting_depth(&source);
    assert!(observed < 5, "expected shallow depth (comment brackets should not count), got {observed}");
}

#[test]
fn test_parse_error_surfaced_as_diagnostic() {
    // Deliberately malformed: unclosed interface body.
    let source = r#"
        export interface BrokenProps {
            label: string;
    "#;
    let path = Utf8Path::new("/test/broken.tsx");

    let data = parse_file(path, source);

    assert!(
        data.diagnostics.iter().any(|d| d.code == DiagnosticCode::ParseError),
        "expected a ParseError diagnostic; got {:?}",
        data.diagnostics
    );
}

#[test]
fn record_skip_pushes_an_info_diagnostic_with_the_given_code() {
    use oxc_span::Span;
    let path = Utf8Path::new("/test/skip.tsx");
    let mut collector = SourceDataCollector::new(path, "", false);
    collector.record_skip(DiagnosticCode::SkippedCandidate, "malformed Omit<> arguments", Span::new(10, 20));

    assert_eq!(collector.data.diagnostics.len(), 1);
    let diag = &collector.data.diagnostics[0];
    assert_eq!(diag.severity, DiagnosticSeverity::Info);
    assert_eq!(diag.code, DiagnosticCode::SkippedCandidate);
    assert_eq!(diag.message, "malformed Omit<> arguments");
    assert_eq!(diag.file.as_deref(), Some("/test/skip.tsx"));
}

#[test]
fn skipped_candidates_carry_the_line_and_column_of_the_declaration() {
    let source = "import x from \"y\";\n\n  const Opaque = makeThing();\n";
    let data = parse_file(Utf8Path::new("/test/skip.tsx"), source);
    let skipped: Vec<(Option<u32>, Option<u32>)> = data
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::SkippedCandidate)
        .map(|d| (d.line, d.column))
        .collect();
    assert_eq!(skipped, vec![(Some(3), Some(9))]);
}

#[test]
fn line_and_column_counts_characters_and_clamps_past_the_end() {
    assert_eq!(line_and_column("ab\ncd", 4), (2, 2));
    assert_eq!(line_and_column("é\nx", 3), (2, 1));
    assert_eq!(line_and_column("abc", 99), (1, 4));
}

// ── SPEC-EXTRACTOR-001 AC-009: a type alias built from Omit/Pick/Partial/
// Required/Readonly with malformed type arguments is skipped, end to end
// through real parsing — the test above only proves record_skip() itself
// works in isolation, never actually parsing a malformed generic alias.

#[test]
fn malformed_omit_type_alias_is_skipped_end_to_end() {
    let source = r#"
        interface FullProps { a: string; b: number; }
        type BadOmit = Omit<FullProps>;
    "#;
    let path = Utf8Path::new("/test/malformed_omit.ts");
    let data = parse_file(path, source);

    let scoped_key = format!("{}:BadOmit", path);
    assert!(
        !data.type_aliases.contains_key(&scoped_key),
        "malformed Omit<> (fewer than 2 type args) must not be collected, got keys {:?}",
        data.type_aliases.keys().collect::<Vec<_>>()
    );
    assert!(!data.component_mappings.iter().any(|m| m.component_name == "BadOmit"));
    let diag =
        data.diagnostics.iter().find(|d| d.code == DiagnosticCode::SkippedCandidate && d.message.contains("BadOmit"));
    assert!(diag.is_some(), "expected a SkippedCandidate diagnostic naming BadOmit, got {:?}", data.diagnostics);
    assert_eq!(diag.unwrap().severity, DiagnosticSeverity::Info);
}

#[test]
fn omit_with_a_literal_base_is_collected_structurally_for_the_resolver_to_report() {
    // The base is kept as written; whether it can supply props is the resolver's call, which reports a literal base
    // when it is used as props ("cannot be used as a component's props base").
    let source = r#"
        type BadOmit2 = Omit<"not a type reference", 'a' | 'b'>;
    "#;
    let path = Utf8Path::new("/test/malformed_omit2.ts");
    let data = parse_file(path, source);

    match data.type_aliases.get(&format!("{}:BadOmit2", path)) {
        Some(CollectedTypeAlias::Omit { base, keys, .. }) => {
            assert!(matches!(base, CollectedType::StringLiteral(s) if s == "not a type reference"));
            assert_eq!(keys.as_string_union_keys(), vec!["a".to_owned(), "b".to_owned()]);
        }
        other => panic!("expected an Omit alias, got {other:?}"),
    }
}

#[test]
fn pascal_case_binding_with_no_matching_detector_records_skipped_candidate() {
    // `const Button = something()` — PascalCase binding, .tsx file, but the
    // init expression matches none of try_fc_annotation / try_forward_ref /
    // try_hoc_wrapped / try_rename_identifier_wrapped_component. Previously
    // the whole chain fell through silently with zero trace it was even
    // considered a component candidate.
    let source = r#"
        const Button = someUnrecognizedFactory();
    "#;
    let path = Utf8Path::new("/test/unrecognized.tsx");
    let data = parse_file(path, source);

    assert!(
        !data.component_mappings.iter().any(|m| m.component_name == "Button"),
        "no mapping should have been produced for an unrecognized pattern"
    );
    assert!(
        data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
        "expected a SkippedCandidate diagnostic, got: {:?}",
        data.diagnostics
    );
}

#[test]
fn pascal_case_function_declaration_with_untyped_first_param_records_skipped_candidate() {
    // `function Button(props) { ... }` — PascalCase FunctionDeclaration,
    // .tsx file, has a first param, but it carries no type annotation at
    // all. Previously the whole chain (type_annotation.as_ref()?...) fell
    // through silently.
    let source = r#"
        function Button(props) {
            return null;
        }
    "#;
    let path = Utf8Path::new("/test/untyped-param.tsx");
    let data = parse_file(path, source);

    assert!(
        !data.component_mappings.iter().any(|m| m.component_name == "Button"),
        "no mapping should have been produced for an untyped first param"
    );
    assert!(
        data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
        "expected a SkippedCandidate diagnostic, got: {:?}",
        data.diagnostics
    );
}

#[test]
fn zero_param_pascal_case_function_declaration_is_not_flagged_as_skipped() {
    // `function Button() { ... }` — a legitimate zero-props component.
    // func.params.items.first() is None here, which is "wrong shape, not
    // a candidate at all" per SkippedCandidate's own doc comment — this
    // function was never a malformed candidate for Pattern 4 (that
    // pattern exists specifically to read a first param's type
    // annotation), so it must not emit a diagnostic. It also produces no
    // mapping, since there's no props type to extract — this is a
    // pre-existing, separate limitation (no props type source at all for
    // truly prop-less components), not something this task changes.
    let source = r#"
        function Button() {
            return null;
        }
    "#;
    let path = Utf8Path::new("/test/zero-param.tsx");
    let data = parse_file(path, source);

    assert!(
        !data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
        "a parameterless component must not be flagged as a skipped candidate, got: {:?}",
        data.diagnostics
    );
}

#[test]
fn deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic() {
    // Adversarial case for the depth-tracking gap: chained conditional types
    // (`A extends B ? C extends D ? ... : ... : ...`) add one AST recursion
    // level per `? :` with only 2 brackets total (none at all, actually —
    // conditional types need no parens/braces/brackets per level), so
    // max_bracket_nesting_depth's proxy metric undercounts this shape badly.
    // Construct enough chained conditionals to exceed the depth guard while
    // staying far under MAX_SOURCE_NESTING_DEPTH's bracket-based limit (2000),
    // proving the depth counter — not the existing bracket guard — is what
    // catches this.
    let mut ty = "boolean".to_owned();
    for i in 0..600 {
        ty = format!("T{i} extends string ? {ty} : never");
    }
    let source = format!("type Deep = {ty};");

    // The bracket-nesting proxy must NOT trip on this source — proves this
    // test is closing a real gap, not duplicating existing coverage.
    assert!(
        max_bracket_nesting_depth(&source) <= MAX_SOURCE_NESTING_DEPTH,
        "test fixture is invalid: bracket heuristic already catches this, \
         defeating the purpose of the adversarial case"
    );

    let path = Utf8Path::new("/test/deep-conditional.ts");
    let data = parse_file(path, &source);

    // Multi-child TSType form (TSConditionalType descends into 4 children —
    // check_type/extends_type/true_type/false_type — simultaneously at the
    // boundary depth), so this emits one diagnostic per child branch that
    // individually crosses the threshold, not a single diagnostic the way a
    // single-child nesting chain (TSTypeReference, TSArrayType, ...) does.
    // See SPEC-EXTRACTOR-001's AC-023 for the single-child-vs-multi-child
    // distinction this exact count is pinned to.
    let count = data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::MaxDepthExceeded).count();
    assert_eq!(count, 4, "expected exactly 4 MaxDepthExceeded diagnostics (one per TSConditionalType child branch that crosses the threshold), got {count}: {:?}", data.diagnostics);
    assert!(data.diagnostics.iter().all(|d| d.severity == DiagnosticSeverity::Warning));
}

#[test]
fn deeply_nested_single_child_type_chain_emits_exactly_one_max_depth_diagnostic() {
    // SPEC-EXTRACTOR-001 AC-023: a single-child nesting form (each level has
    // exactly one nested TSType descendant, unlike TSConditionalType's four)
    // must emit exactly ONE MaxDepthExceeded diagnostic for the whole chain,
    // not one per over-deep node — descent stops at the first node that trips
    // the guard. Contrast with the TSConditionalType case above, which emits 4.
    let mut ty = "string".to_owned();
    for _ in 0..250 {
        ty = format!("Array<{ty}>");
    }
    let source = format!("type Deep = {ty};");
    let path = Utf8Path::new("/test/deep-array.ts");
    let data = parse_file(path, &source);

    let count = data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::MaxDepthExceeded).count();
    assert_eq!(
        count, 1,
        "expected exactly 1 MaxDepthExceeded diagnostic for a single-child chain, got {count}: {:?}",
        data.diagnostics
    );
}

// ── SPEC-EXTRACTOR-001 AC-004: the ForwardRefExoticComponent no-init
// pattern is the sole pattern NOT gated by file extension — assert it
// maps in a plain .ts file.

#[test]
fn forward_ref_exotic_component_no_init_maps_in_plain_ts_file() {
    let source = r#"
        interface IconProps { size: number; }
        declare const Icon: React.ForwardRefExoticComponent<IconProps>;
    "#;
    let path = Utf8Path::new("/test/icon.ts");
    let data = parse_file(path, source);

    let mapping = data.component_mappings.iter().find(|m| m.component_name == "Icon");
    assert!(mapping.is_some(), "expected an Icon mapping in a plain .ts file, got {:?}", data.component_mappings);
    assert_eq!(mapping.unwrap().props_type_name, "IconProps");
}

// ── SPEC-EXTRACTOR-001 AC-008: file-extension gating matrix — one fixture
// across three source-type contexts.

#[test]
fn file_extension_gating_matrix() {
    let source = r#"
        interface AProps { a: string; }
        export function A(props: AProps) { return null; }

        interface BProps { b: string; }
        const B: React.ForwardRefComponent<BProps> = null as any;

        interface CProps { c: string; }
        declare const C: React.ForwardRefExoticComponent<CProps>;
    "#;

    let tsx = parse_file(Utf8Path::new("/test/matrix.tsx"), source);
    let names: Vec<&str> = tsx.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(
        names.contains(&"A") && names.contains(&"B") && names.contains(&"C"),
        "expected A, B, C in .tsx, got {names:?}"
    );

    let dts = parse_file(Utf8Path::new("/test/matrix.d.ts"), source);
    let names: Vec<&str> = dts.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(
        names.contains(&"A") && names.contains(&"B") && names.contains(&"C"),
        "expected A, B, C in .d.ts, got {names:?}"
    );

    let ts = parse_file(Utf8Path::new("/test/matrix.ts"), source);
    let names: Vec<&str> = ts.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert_eq!(names, vec!["C"], "expected ONLY the AC-004 no-init pattern to map in plain .ts, got {names:?}");
}

// ── SPEC-EXTRACTOR-001 AC-014: a forward reference (target declared
// lexically LATER) must not match — the target isn't yet an
// already-collected component when the wrapping binding is visited.

#[test]
fn forward_referenced_wrap_target_is_not_yet_collected_and_is_skipped() {
    let source = r#"
        const Wrap = memo(Later);
        interface LaterProps { x: string; }
        function Later(props: LaterProps) { return null; }
    "#;
    let path = Utf8Path::new("/test/forward-ref.tsx");
    let data = parse_file(path, source);

    assert!(
        !data.component_mappings.iter().any(|m| m.component_name == "Wrap"),
        "no Wrap mapping should exist for a forward reference, got {:?}",
        data.component_mappings
    );
    assert!(
        data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate && d.message.contains("Wrap")),
        "expected a SkippedCandidate diagnostic identifying 'Wrap' by name, got {:?}",
        data.diagnostics
    );
}

// ── SPEC-EXTRACTOR-001 AC-015: a lowercase top-level binding matching an
// otherwise-recognized component shape is silently omitted — no mapping,
// no diagnostic.

#[test]
fn lowercase_top_level_binding_is_silently_omitted() {
    let source = r#"
        interface ButtonProps { label: string; }
        const button = React.forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => null);
    "#;
    let path = Utf8Path::new("/test/lowercase.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "button"));
    assert!(
        data.diagnostics.is_empty(),
        "lowercase bindings are documented silent omissions, expected no diagnostics, got {:?}",
        data.diagnostics
    );
}

// ── SPEC-EXTRACTOR-001 AC-016: a no-initializer PascalCase binding whose
// type is neither ForwardRefExoticComponent nor an FC-family annotation
// is silently omitted.

#[test]
fn unrelated_no_init_type_annotation_is_silently_omitted() {
    let source = r#"
        declare const Ghost: SomeOtherType;
    "#;
    let path = Utf8Path::new("/test/ghost.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "Ghost"));
    assert!(data.diagnostics.is_empty(), "expected no diagnostics, got {:?}", data.diagnostics);
}

#[test]
fn declared_callable_object_needs_one_element_returning_signature() {
    let source = r#"
        interface Props { value?: string }
        declare const Menu: { (props: Props): React.JSX.Element; displayName: string };
        declare const Formatter: { (props: Props): string; displayName: string };
        declare const Overloaded: { (props: Props): React.JSX.Element; (props: string): React.JSX.Element };
    "#;
    let data = parse_file(Utf8Path::new("/test/callable.d.mts"), source);
    let mappings: Vec<_> = data
        .component_mappings
        .iter()
        .map(|mapping| (mapping.component_name.as_str(), mapping.props_type_name.as_str()))
        .collect();
    assert_eq!(mappings, [("Menu", "Props")]);
}

// ── SPEC-EXTRACTOR-001 AC-018: computed/symbol keys are silently omitted;
// a numeric-literal key extracts as its decimal string form.

#[test]
fn computed_symbol_and_numeric_interface_keys() {
    let source = r#"
        interface Props {
            [key: string]: unknown;
            [Symbol.iterator]?: () => void;
            0: string;
            normal: number;
        }
        export function Widget(props: Props) { return null; }
    "#;
    let path = Utf8Path::new("/test/keys.tsx");
    let data = parse_file(path, source);

    let iface = data.interfaces.values().find(|i| i.name == "Props").expect("expected Props interface");
    let prop_names: Vec<&str> = iface.props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(prop_names, vec!["0", "normal"], "expected only the numeric and normal keys, got {prop_names:?}");
    assert!(data.diagnostics.is_empty(), "computed/symbol key omission is silent, got {:?}", data.diagnostics);
}

// ── SPEC-EXTRACTOR-001 AC-019: a zero-byte file produces empty
// collections and no diagnostics.

#[test]
fn empty_source_file_produces_empty_collections_and_no_diagnostics() {
    let path = Utf8Path::new("/test/empty.tsx");
    let data = parse_file(path, "");

    assert!(data.component_mappings.is_empty());
    assert!(data.interfaces.is_empty());
    assert!(data.type_aliases.is_empty());
    assert!(data.enums.is_empty());
    assert!(data.imports.is_empty());
    assert!(data.exports.is_empty());
    assert!(data.diagnostics.is_empty());
}

// ── SPEC-EXTRACTOR-001 AC-022: a recoverable parse error still extracts
// every other cleanly-parsed declaration — the clause distinguishing this
// from AC-026's fatal, whole-file-empty case.

#[test]
fn recoverable_parse_error_still_extracts_other_clean_declarations() {
    let source = "type Bad = Partial<>; function Ok(props: OkProps) { return null; }";
    let path = Utf8Path::new("/test/recoverable.tsx");
    let data = parse_file(path, source);

    let parse_errors: Vec<&Diagnostic> =
        data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::ParseError).collect();
    assert!(!parse_errors.is_empty(), "expected a ParseError diagnostic, got {:?}", data.diagnostics);
    assert!(
        parse_errors.iter().all(|d| d.severity == DiagnosticSeverity::Error),
        "every ParseError diagnostic must carry Error severity, got {:?}",
        parse_errors
    );
    assert!(
        data.component_mappings.iter().any(|m| m.component_name == "Ok"),
        "expected Ok's mapping to survive the earlier recoverable parse error, got {:?}",
        data.component_mappings
    );
}

// ── SPEC-EXTRACTOR-001 AC-024/AC-025: inline union/intersection/object
// props types synthesize an anonymous alias; the FC-family no-init
// pattern's own first type argument routes through the same mechanism.
// Covers all 5 shapes AC-024 names: (a) function-declaration param,
// (b) FC-family first type arg, (c) inline arrow inside a call wrap,
// (d) nested-forwardRef's 2nd type arg inside a call wrap, (e)
// forwardRef's own 2nd type arg.

#[test]
fn inline_union_intersection_object_props_synthesize_anonymous_aliases() {
    let source = r#"
        function Card(props: { title: string }) { return null; }
        function Boxy(props: A | B) { return null; }
        declare const Widget: React.FC<{ label: string }>;
        const Wrapped = withStyles((props: { title: string }) => null);
        const Nested = memo(forwardRef<HTMLDivElement, { title: string }>((props, ref) => null));
        const Direct = React.forwardRef<HTMLDivElement, { title: string }>((props, ref) => null);
    "#;
    let path = Utf8Path::new("/test/anon.tsx");
    let data = parse_file(path, source);

    for name in ["Card", "Boxy", "Widget", "Wrapped", "Nested", "Direct"] {
        let mapping = data.component_mappings.iter().find(|m| m.component_name == name);
        assert!(mapping.is_some(), "expected a mapping for {name}, got {:?}", data.component_mappings);
        let props_type_name = &mapping.unwrap().props_type_name;
        assert!(props_type_name.starts_with("__anon_"), "expected an anon alias for {name}, got {props_type_name}");
        let scoped_key = format!("{}:{}", path, props_type_name);
        assert!(
            data.type_aliases.contains_key(&scoped_key),
            "expected a type_aliases entry keyed {scoped_key}, got keys {:?}",
            data.type_aliases.keys().collect::<Vec<_>>()
        );
    }
    assert!(data.diagnostics.is_empty(), "no diagnostic expected for the anon-alias path, got {:?}", data.diagnostics);
}

// ── SPEC-EXTRACTOR-001 AC-026: a fatal parse error empties every named
// collection for the whole file.

#[test]
fn fatal_parse_error_empties_the_whole_file() {
    let source = "function Card(props: Props) { return ;;; ) }";
    let path = Utf8Path::new("/test/fatal.tsx");
    let data = parse_file(path, source);

    assert!(data.component_mappings.is_empty());
    assert!(data.interfaces.is_empty());
    assert!(data.type_aliases.is_empty());
    assert!(data.enums.is_empty());
    assert!(data.imports.is_empty());
    assert!(data.exports.is_empty());
    let parse_errors: Vec<&Diagnostic> =
        data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::ParseError).collect();
    assert!(!parse_errors.is_empty(), "expected a ParseError diagnostic, got {:?}", data.diagnostics);
    assert!(
        parse_errors.iter().all(|d| d.severity == DiagnosticSeverity::Error),
        "every ParseError diagnostic must carry Error severity, got {:?}",
        parse_errors
    );
}

// ── `satisfies`-wrapped component expressions: `unwrap_as_expression`
// peels `as X` casts so a forwardRef/HOC call cast to a hand-rolled
// wrapper type is still recognized underneath; `satisfies` is the
// modern (TS 4.9+) equivalent for the same use case and must be peeled
// the same way.

#[test]
fn satisfies_wrapped_forward_ref_is_unwrapped_like_an_as_expression() {
    let source = r#"
        interface ButtonProps { label: string; }
        interface WrapperType { (props: ButtonProps): JSX.Element; }
        const Button = React.forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => null) satisfies WrapperType;
    "#;
    let path = Utf8Path::new("/test/satisfies_forward_ref.tsx");
    let data = parse_file(path, source);

    let btn = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(
        btn.is_some(),
        "expected Button to be detected through the satisfies wrapper, got {:?}",
        data.component_mappings
    );
    assert_eq!(btn.unwrap().props_type_name.as_str(), "ButtonProps");
}

#[test]
fn satisfies_wrapped_hoc_call_is_unwrapped_like_an_as_expression() {
    let source = r#"
        interface CardProps { title: string; }
        interface WrapperType { (props: CardProps): JSX.Element; }
        const Card = memo(function Card(props: CardProps) { return null; }) satisfies WrapperType;
    "#;
    let path = Utf8Path::new("/test/satisfies_hoc.tsx");
    let data = parse_file(path, source);

    let card = data.component_mappings.iter().find(|m| m.component_name == "Card");
    assert!(
        card.is_some(),
        "expected Card to be detected through the satisfies wrapper, got {:?}",
        data.component_mappings
    );
    assert_eq!(card.unwrap().props_type_name.as_str(), "CardProps");
}

// ── Numeric-key parity: `collect_property_signature` (top-level
// interfaces) already extracts numeric-literal keys via
// `PropertyKey::static_name()` (see `computed_symbol_and_numeric_interface_keys`
// above), but the sibling `ts_signature_to_object_field` (inline object
// type literals, e.g. a nested field's own `{ ... }` type) hand-rolled a
// StaticIdentifier/StringLiteral-only match that silently drops numeric
// keys instead — sibling-path parity gap.

#[test]
fn numeric_key_in_an_inline_object_type_literal_is_not_silently_dropped() {
    let source = r#"
        interface Props {
            meta: { 0: string; normal: number };
        }
        export function Widget(props: Props) { return null; }
    "#;
    let path = Utf8Path::new("/test/inline_numeric_key.tsx");
    let data = parse_file(path, source);

    let iface = data.interfaces.values().find(|i| i.name == "Props").expect("expected Props interface");
    let meta = iface.props.iter().find(|p| p.name == "meta").expect("expected 'meta' prop");
    match &meta.collected_type {
        CollectedType::Object(fields) => {
            let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, vec!["0", "normal"], "expected the numeric key to survive, got {names:?}");
        }
        other => panic!("expected an inline Object type, got {other:?}"),
    }
}

// ── A no-initializer FC-family declaration whose props type argument is
// an exotic shape `extract_type_name_from_type` doesn't match (unlike
// `unrelated_no_init_type_annotation_is_silently_omitted` above, which is
// a genuinely unrelated type and must stay silent) must surface a
// SkippedCandidate diagnostic instead of vanishing with no trace — the
// same contract the `init.is_some()` sibling path already has.

#[test]
fn no_init_fc_family_with_an_exotic_type_argument_is_flagged_not_silently_dropped() {
    let source = r#"
        type Keys = 'a' | 'b';
        declare const Button: React.FC<{ [K in Keys]: string }>;
    "#;
    let path = Utf8Path::new("/test/no_init_exotic_fc.tsx");
    let data = parse_file(path, source);

    assert!(
        !data.component_mappings.iter().any(|m| m.component_name == "Button"),
        "an exotic type argument should not resolve, got {:?}",
        data.component_mappings
    );
    let skips: Vec<&Diagnostic> =
        data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SkippedCandidate).collect();
    assert_eq!(skips.len(), 1, "expected exactly one SkippedCandidate diagnostic, got {:?}", data.diagnostics);
    assert!(
        skips[0].message.contains("Button") && skips[0].message.contains("FC-family"),
        "expected the diagnostic to name Button and the FC-family shape, got {:?}",
        skips[0].message
    );
}

#[test]
fn no_init_forward_ref_exotic_decl_success_does_not_also_emit_a_skip_diagnostic() {
    // Regression guard for the fix above: the new guard must not fire when
    // Pattern 5's own detector (try_exotic_component_decl) already
    // succeeded — only when every no-init detector failed.
    let source = r#"
        interface ButtonProps { label: string; }
        declare const Button: React.ForwardRefExoticComponent<ButtonProps & React.RefAttributes<HTMLButtonElement>>;
    "#;
    let path = Utf8Path::new("/test/no_init_exotic_decl_success.tsx");
    let data = parse_file(path, source);

    assert!(data.component_mappings.iter().any(|m| m.component_name == "Button"));
    assert!(data.diagnostics.is_empty(), "expected no diagnostics on the success path, got {:?}", data.diagnostics);
}

// ── Anonymous default-exported function components: `export default
// function(props: Props) {}` (no identifier — Next.js page-file idiom).
// `visit_function`'s Pattern 4 requires `func.id`, so these were
// previously silently skipped entirely. Named after the PascalCased file
// stem since there's no identifier to name the component after.

#[test]
fn anonymous_default_export_function_component_named_after_the_file_stem() {
    let source = r#"
        interface ButtonGroupProps { children: unknown; }
        export default function(props: ButtonGroupProps) { return null; }
    "#;
    let path = Utf8Path::new("/test/button-group.tsx");
    let data = parse_file(path, source);

    let mapping = data.component_mappings.iter().find(|m| m.component_name == "ButtonGroup");
    assert!(
        mapping.is_some(),
        "expected a component named 'ButtonGroup' from the file stem, got {:?}",
        data.component_mappings.iter().map(|m| &m.component_name).collect::<Vec<_>>()
    );
    assert_eq!(mapping.unwrap().props_type_name.as_str(), "ButtonGroupProps");
}

#[test]
fn anonymous_default_export_with_untyped_first_param_is_flagged() {
    let source = r#"
        export default function(props) { return null; }
    "#;
    let path = Utf8Path::new("/test/card.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "Card"));
    let skips: Vec<&Diagnostic> =
        data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SkippedCandidate).collect();
    assert_eq!(skips.len(), 1, "expected exactly one SkippedCandidate diagnostic, got {:?}", data.diagnostics);
    assert!(
        skips[0].message.contains("Card") && skips[0].message.contains("untyped first param"),
        "expected the diagnostic to name Card and the untyped param, got {:?}",
        skips[0].message
    );
}

#[test]
fn anonymous_default_export_with_no_params_is_silently_omitted() {
    // Mirrors the existing named-function sibling's contract: Pattern 4
    // has no `else` branch at all when a function declares zero params,
    // so a prop-less named component is silently skipped with no
    // diagnostic today. The anonymous case must have the same contract.
    let source = r#"
        export default function() { return null; }
    "#;
    let path = Utf8Path::new("/test/empty-page.tsx");
    let data = parse_file(path, source);

    assert!(data.component_mappings.is_empty(), "got {:?}", data.component_mappings);
    assert!(data.diagnostics.is_empty(), "expected no diagnostics, got {:?}", data.diagnostics);
}

#[test]
fn named_default_export_function_component_is_unaffected_by_the_anonymous_handler() {
    let source = r#"
        interface ButtonProps { label: string; }
        export default function Button(props: ButtonProps) { return null; }
    "#;
    let path = Utf8Path::new("/test/named-default.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert_eq!(names, vec!["Button"], "expected only the named component, got {names:?}");
}

// ── Class components: `class Button extends React.Component<Props> {}`
// (or bare `Component`/`PureComponent`/`React.PureComponent`) were never
// detected as components at all — no handler existed for this shape.

#[test]
fn class_declaration_extending_react_component_is_detected() {
    let source = r#"
        interface ButtonProps { label: string; }
        class Button extends React.Component<ButtonProps> {
            render() { return null; }
        }
    "#;
    let path = Utf8Path::new("/test/class_button.tsx");
    let data = parse_file(path, source);

    let btn = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(btn.is_some(), "expected Button class component to be detected, got {:?}", data.component_mappings);
    assert_eq!(btn.unwrap().props_type_name.as_str(), "ButtonProps");
}

#[test]
fn class_declaration_extending_bare_pure_component_is_detected() {
    let source = r#"
        import { PureComponent } from "react";
        interface CardProps { title: string; }
        class Card extends PureComponent<CardProps> {
            render() { return null; }
        }
    "#;
    let path = Utf8Path::new("/test/class_card.tsx");
    let data = parse_file(path, source);

    let card = data.component_mappings.iter().find(|m| m.component_name == "Card");
    assert!(card.is_some(), "expected Card class component to be detected, got {:?}", data.component_mappings);
    assert_eq!(card.unwrap().props_type_name.as_str(), "CardProps");
}

#[test]
fn class_declaration_extending_component_with_no_type_args_is_silently_omitted() {
    // Mirrors visit_function's Pattern 4 zero-params contract: an untyped
    // legacy class component has no props type to attach to at all —
    // that's a genuinely-not-describable candidate, not a malformed one.
    let source = r#"
        class Button extends React.Component {
            render() { return null; }
        }
    "#;
    let path = Utf8Path::new("/test/class_untyped.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "Button"));
    assert!(data.diagnostics.is_empty(), "expected no diagnostics, got {:?}", data.diagnostics);
}

#[test]
fn class_declaration_with_an_exotic_props_type_argument_is_flagged() {
    let source = r#"
        type Keys = 'a' | 'b';
        class Button extends React.Component<{ [K in Keys]: string }> {
            render() { return null; }
        }
    "#;
    let path = Utf8Path::new("/test/class_exotic.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "Button"));
    let skips: Vec<&Diagnostic> =
        data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SkippedCandidate).collect();
    assert_eq!(skips.len(), 1, "expected exactly one SkippedCandidate diagnostic, got {:?}", data.diagnostics);
    assert!(
        skips[0].message.contains("Button") && skips[0].message.contains("class component"),
        "expected the diagnostic to name Button and 'class component', got {:?}",
        skips[0].message
    );
}

#[test]
fn class_declaration_extending_an_unrelated_base_is_silently_omitted() {
    let source = r#"
        interface ButtonProps { label: string; }
        class Button extends SomeOtherBase<ButtonProps> {
            render() { return null; }
        }
    "#;
    let path = Utf8Path::new("/test/class_unrelated_base.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "Button"));
    assert!(data.diagnostics.is_empty(), "expected no diagnostics, got {:?}", data.diagnostics);
}

// ── Class-expression components: `const Button = class extends
// React.Component<Props> {}` fell through every detector silently.
// Reuses the class-declaration detection logic above.

#[test]
fn anonymous_class_expression_assigned_to_a_pascal_case_binding_is_detected() {
    let source = r#"
        interface ButtonProps { label: string; }
        const Button = class extends React.Component<ButtonProps> {
            render() { return null; }
        };
    "#;
    let path = Utf8Path::new("/test/class_expr.tsx");
    let data = parse_file(path, source);

    let btn = data.component_mappings.iter().find(|m| m.component_name == "Button");
    assert!(
        btn.is_some(),
        "expected Button class-expression component to be detected, got {:?}",
        data.component_mappings
    );
    assert_eq!(btn.unwrap().props_type_name.as_str(), "ButtonProps");
}

#[test]
fn self_named_class_expression_is_detected_under_its_own_name_not_the_outer_binding() {
    let source = r#"
        interface ButtonProps { label: string; }
        const X = class Button extends React.Component<ButtonProps> {
            render() { return null; }
        };
    "#;
    let path = Utf8Path::new("/test/class_expr_self_named.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(names.contains(&"Button"), "expected 'Button' (the class's own name), got {names:?}");
    assert!(!names.contains(&"X"), "the outer binding 'X' should not itself become a component, got {names:?}");
}

// ── Object.assign(Parent, { Sub: ... }) inline-only sub-components: a
// sub-component that exists ONLY inside the object-literal argument (never
// independently exported/bound) was silently dropped — a real
// non-negotiable #6 violation, unlike the already-independently-exported
// case investigated and deliberately left alone in rdt-coverage.md.

#[test]
fn object_assign_inline_function_sub_component_is_detected() {
    let source = r#"
        interface ButtonProps { label: string; }
        interface IconProps { name: string; }
        function Button(props: ButtonProps) { return null; }
        Object.assign(Button, {
            Icon: function ButtonIcon(props: IconProps) { return null; },
        });
    "#;
    let path = Utf8Path::new("/test/object_assign_fn.tsx");
    let data = parse_file(path, source);

    let icon = data.component_mappings.iter().find(|m| m.component_name == "Icon");
    assert!(
        icon.is_some(),
        "expected an inline-only sub-component named 'Icon', got {:?}",
        data.component_mappings.iter().map(|m| &m.component_name).collect::<Vec<_>>()
    );
    assert_eq!(icon.unwrap().props_type_name.as_str(), "IconProps");
    assert!(data.component_mappings.iter().any(|m| m.component_name == "Button"));
}

#[test]
fn object_assign_inline_arrow_sub_component_is_detected() {
    let source = r#"
        interface ButtonProps { label: string; }
        interface LabelProps { text: string; }
        function Button(props: ButtonProps) { return null; }
        Object.assign(Button, {
            Label: (props: LabelProps) => null,
        });
    "#;
    let path = Utf8Path::new("/test/object_assign_arrow.tsx");
    let data = parse_file(path, source);

    let label = data.component_mappings.iter().find(|m| m.component_name == "Label");
    assert!(label.is_some(), "expected an inline-only sub-component named 'Label', got {:?}", data.component_mappings);
    assert_eq!(label.unwrap().props_type_name.as_str(), "LabelProps");
}

#[test]
fn object_assign_identifier_reference_sub_component_is_not_double_detected() {
    // The already-independently-exported case (a bare identifier
    // reference, not an inline function) — deliberately out of scope per
    // rdt-coverage.md's "compound components" investigation. IconImpl is
    // already detected under its own name via Pattern 4; Object.assign
    // must not additionally synthesize a second mapping for it.
    let source = r#"
        interface ButtonProps { label: string; }
        interface IconProps { name: string; }
        function Button(props: ButtonProps) { return null; }
        function IconImpl(props: IconProps) { return null; }
        Object.assign(Button, { Icon: IconImpl });
    "#;
    let path = Utf8Path::new("/test/object_assign_identifier.tsx");
    let data = parse_file(path, source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert!(names.contains(&"IconImpl"), "expected IconImpl under its own name, got {names:?}");
    assert!(
        !names.contains(&"Icon"),
        "must not synthesize a second mapping for an identifier reference, got {names:?}"
    );
}

#[test]
fn object_assign_sub_component_with_untyped_first_param_is_flagged() {
    let source = r#"
        interface ButtonProps { label: string; }
        function Button(props: ButtonProps) { return null; }
        Object.assign(Button, {
            Icon: function ButtonIcon(props) { return null; },
        });
    "#;
    let path = Utf8Path::new("/test/object_assign_untyped.tsx");
    let data = parse_file(path, source);

    assert!(!data.component_mappings.iter().any(|m| m.component_name == "Icon"));
    let skips: Vec<&Diagnostic> =
        data.diagnostics.iter().filter(|d| d.code == DiagnosticCode::SkippedCandidate).collect();
    assert_eq!(skips.len(), 1, "expected exactly one SkippedCandidate diagnostic, got {:?}", data.diagnostics);
    assert!(
        skips[0].message.contains("Icon") && skips[0].message.contains("untyped first param"),
        "expected the diagnostic to name Icon and the untyped param, got {:?}",
        skips[0].message
    );
}

#[test]
fn a_component_declared_inside_a_function_body_is_reported_not_mapped() {
    let source = r#"
        import { forwardRef, Component } from "react";
        interface CardProps { title: string }
        export function Card(props: CardProps) { return <div />; }
        export function createLeaf<P>(render: (props: P) => any) {
            const Result = forwardRef((props: P, ref) => <div>{render(props)}</div>);
            function Inner(props: P) { return <div />; }
            class Legacy extends Component<P> { render() { return null; } }
            const Tag = "div";
            return Result;
        }
        export const Story = (props: CardProps) => {
            const Local: React.FC<CardProps> = (props) => <div />;
            return <Local title="x" />;
        };
    "#;
    let data = parse_file(Utf8Path::new("/test/factory.tsx"), source);

    let names: Vec<&str> = data.component_mappings.iter().map(|m| m.component_name.as_str()).collect();
    assert_eq!(names, ["Card", "Story"]);
    let skips: Vec<&str> = data
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::SkippedCandidate)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        skips,
        [
            "'Result' is declared inside a function body, so no module can import it",
            "'Inner' is declared inside a function body, so no module can import it",
            "'Legacy' is declared inside a function body, so no module can import it",
            "'Local' is declared inside a function body, so no module can import it",
        ]
    );
}

#[test]
fn unrelated_object_assign_call_is_not_treated_as_a_component_pattern() {
    let source = r#"
        const config = {};
        Object.assign(config, { extra: 1 });
    "#;
    let path = Utf8Path::new("/test/object_assign_unrelated.tsx");
    let data = parse_file(path, source);

    assert!(data.component_mappings.is_empty(), "got {:?}", data.component_mappings);
    assert!(data.diagnostics.is_empty(), "expected no diagnostics, got {:?}", data.diagnostics);
}
