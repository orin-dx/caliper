//! Baked-in knowledge of React and DOM types.
//! No file I/O — these are compile-time constants derived from @types/react.

/// HTML element names recognized as component-level inheritors.
///
/// Maps `@types/react` HTML attribute type names to their corresponding DOM element names.
pub fn html_element_for(type_name: &str) -> Option<&'static str> {
    match type_name {
        "ButtonHTMLAttributes" => Some("button"),
        "InputHTMLAttributes" => Some("input"),
        "TextareaHTMLAttributes" => Some("textarea"),
        "SelectHTMLAttributes" => Some("select"),
        "AnchorHTMLAttributes" => Some("a"),
        "FormHTMLAttributes" => Some("form"),
        "LabelHTMLAttributes" => Some("label"),
        "ImgHTMLAttributes" => Some("img"),
        "VideoHTMLAttributes" => Some("video"),
        "AudioHTMLAttributes" => Some("audio"),
        "HTMLAttributes" => Some("div"),
        "DOMAttributes" => Some("div"),
        "AriaAttributes" => None,             // not an element, but recognized as built-in
        "SVGAttributes" | "SVGProps" => None, // SVG — no single element to pick from the name alone
        "HTMLProps" => Some("div"), // generic HTML props → div (overridden below when a real element arg is given)
        _ => None,
    }
}

/// `SVGAttributes<T>`/`SVGProps<T>`/`HTMLProps<T>` carry no element in their own
/// name (unlike `ButtonHTMLAttributes`, where the element is baked into the
/// name) — but a real call site always supplies a concrete DOM element type as
/// `T`, e.g. `React.SVGAttributes<SVGSVGElement>` or `React.HTMLProps<HTMLDivElement>`.
/// Derives the element tag from that argument so these generic forms get the
/// same real, structural HTML-attribute expansion (Full mode) and
/// `notableInherited` treatment (Curated mode) that the concrete forms
/// already get from `html_element_for` alone. Only covers the element types
/// that actually appear as `SVGAttributes`/`HTMLProps` type arguments in
/// practice — falls back to `None` (the prior, safe opaque behavior) for
/// anything not in this list rather than guessing at a tag name that's wrong
/// (e.g. `HTMLAnchorElement`'s real tag is `a`, not a naive strip-and-lowercase
/// of the interface name).
/// The element named by a structured type argument such as `HTMLDivElement` or `globalThis.HTMLAnchorElement`.
pub fn html_element_from_collected(type_arg: &crate::types::CollectedType) -> Option<&'static str> {
    match type_arg {
        crate::types::CollectedType::Named { name, .. } => {
            html_element_from_type_arg(name.rsplit('.').next().unwrap_or(name.as_str()))
        }
        _ => None,
    }
}

pub fn html_element_from_type_arg(type_arg: &str) -> Option<&'static str> {
    match type_arg {
        "HTMLAnchorElement" => Some("a"),
        "HTMLButtonElement" => Some("button"),
        "HTMLDivElement" => Some("div"),
        "HTMLSpanElement" => Some("span"),
        "HTMLFormElement" => Some("form"),
        "HTMLImageElement" => Some("img"),
        "HTMLInputElement" => Some("input"),
        "HTMLLabelElement" => Some("label"),
        "HTMLSelectElement" => Some("select"),
        "HTMLTextAreaElement" => Some("textarea"),
        "HTMLVideoElement" => Some("video"),
        "HTMLAudioElement" => Some("audio"),
        "HTMLParagraphElement" => Some("p"),
        "HTMLUListElement" => Some("ul"),
        "HTMLOListElement" => Some("ol"),
        "HTMLLIElement" => Some("li"),
        "HTMLTableElement" => Some("table"),
        "SVGSVGElement" => Some("svg"),
        "SVGCircleElement" => Some("circle"),
        "SVGPathElement" => Some("path"),
        "SVGRectElement" => Some("rect"),
        "SVGLineElement" => Some("line"),
        "SVGGElement" => Some("g"),
        _ => None,
    }
}

/// Types that are terminal — never need further resolution.
///
/// These are React-specific type names that we recognize as builtins and don't
/// attempt to chase through imports.
///
/// `extra` allows callers (e.g. the resolver) to pass additional builtin names
/// from `PipelineOptions.extra_builtins` without needing to change this file.
pub fn is_react_builtin(name: &str, extra: &rustc_hash::FxHashSet<compact_str::CompactString>) -> bool {
    extra.contains(name)
        || matches!(
            name,
            "ReactNode"
                | "ReactElement"
                | "JSX.Element"
                | "CSSProperties"
                | "CSSObject"
                | "SyntheticEvent"
                | "MouseEvent"
                | "KeyboardEvent"
                | "ChangeEvent"
                | "FocusEvent"
                | "FormEvent"
                | "DragEvent"
                | "TouchEvent"
                | "WheelEvent"
                | "AnimationEvent"
                | "TransitionEvent"
                | "ClipboardEvent"
                | "CompositionEvent"
                | "MouseEventHandler"
                | "KeyboardEventHandler"
                | "ChangeEventHandler"
                | "FocusEventHandler"
                | "FormEventHandler"
                | "DragEventHandler"
                | "TouchEventHandler"
                | "WheelEventHandler"
                | "AnimationEventHandler"
                | "TransitionEventHandler"
                | "ClipboardEventHandler"
                | "CompositionEventHandler"
                | "PointerEventHandler"
                | "ReactEventHandler"
                | "SubmitEventHandler"
                | "InputEventHandler"
                | "ToggleEventHandler"
                | "FC"
                | "FunctionComponent"
                | "VFC"
                | "VoidFunctionComponent"
                | "ComponentType"
                | "PropsWithChildren"
                | "PropsWithRef"
                | "RefObject"
                | "Ref"
                | "ForwardedRef"
                | "MutableRefObject"
                | "RefCallback"
                | "LegacyRef"
                | "Context"
                | "Consumer"
                | "Provider"
                | "ComponentPropsWithoutRef"
                | "ComponentPropsWithRef"
                | "ComponentProps"
                | "ElementRef"
                | "ElementType"
                | "ReactPortal"
                | "ReactFragment"
                | "ReactChild"
                | "ForwardRefExoticComponent"
                | "RefAttributes"
                | "ComponentRef"
                | "JSXElementConstructor"
                | "SVGAttributes"
                | "SVGProps"
                | "HTMLProps"
        )
}

/// Returns the curated list of notable HTML attribute prop names for a given element.
///
/// Used to populate `ComponentEntry.notable_inherited` — the subset of inherited
/// HTML props that are most relevant for documentation and prop tables.
pub fn notable_html_attrs(element: &str) -> &'static [&'static str] {
    match element {
        "button" => &[
            "onClick",
            "onKeyDown",
            "onKeyUp",
            "onFocus",
            "onBlur",
            "disabled",
            "type",
            "form",
            "name",
            "value",
            "tabIndex",
            "aria-label",
            "aria-describedby",
            "aria-expanded",
            "aria-pressed",
            "aria-haspopup",
        ],
        "input" => &[
            "onChange",
            "onInput",
            "onFocus",
            "onBlur",
            "value",
            "defaultValue",
            "placeholder",
            "type",
            "disabled",
            "readOnly",
            "required",
            "name",
            "min",
            "max",
            "pattern",
            "autoComplete",
            "checked",
            "defaultChecked",
        ],
        "a" => &["href", "target", "rel", "download", "onClick"],
        "textarea" => &[
            "onChange",
            "value",
            "defaultValue",
            "placeholder",
            "disabled",
            "readOnly",
            "required",
            "rows",
            "maxLength",
        ],
        "select" => &["onChange", "value", "defaultValue", "disabled", "multiple", "required"],
        "form" => &["onSubmit", "onReset", "action", "method", "encType", "noValidate"],
        "img" => &["src", "alt", "width", "height", "loading", "onLoad", "onError"],
        _ => &["onClick", "onFocus", "onBlur", "className", "style", "id", "tabIndex", "aria-label"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // Every match arm in html_element_for, not a sample — including both
    // members of the `SVGAttributes | SVGProps` or-pattern as distinct cases
    // (textually distinct leaves even though they share an arm), and
    // AriaAttributes as its own case since it means something different from
    // the catch-all None (recognized-but-no-element, not unrecognized).
    #[rstest]
    #[case("ButtonHTMLAttributes", Some("button"))]
    #[case("InputHTMLAttributes", Some("input"))]
    #[case("TextareaHTMLAttributes", Some("textarea"))]
    #[case("SelectHTMLAttributes", Some("select"))]
    #[case("AnchorHTMLAttributes", Some("a"))]
    #[case("FormHTMLAttributes", Some("form"))]
    #[case("LabelHTMLAttributes", Some("label"))]
    #[case("ImgHTMLAttributes", Some("img"))]
    #[case("VideoHTMLAttributes", Some("video"))]
    #[case("AudioHTMLAttributes", Some("audio"))]
    #[case("HTMLAttributes", Some("div"))]
    #[case("DOMAttributes", Some("div"))]
    #[case("AriaAttributes", None)]
    #[case("SVGAttributes", None)]
    #[case("SVGProps", None)]
    #[case("HTMLProps", Some("div"))]
    #[case("UnknownAttributes", None)]
    #[case("", None)]
    fn test_html_element_for_table(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(html_element_for(input), expected);
    }

    // Every match arm in html_element_from_type_arg, not a sample.
    #[rstest]
    #[case("HTMLAnchorElement", Some("a"))]
    #[case("HTMLButtonElement", Some("button"))]
    #[case("HTMLDivElement", Some("div"))]
    #[case("HTMLSpanElement", Some("span"))]
    #[case("HTMLFormElement", Some("form"))]
    #[case("HTMLImageElement", Some("img"))]
    #[case("HTMLInputElement", Some("input"))]
    #[case("HTMLLabelElement", Some("label"))]
    #[case("HTMLSelectElement", Some("select"))]
    #[case("HTMLTextAreaElement", Some("textarea"))]
    #[case("HTMLVideoElement", Some("video"))]
    #[case("HTMLAudioElement", Some("audio"))]
    #[case("HTMLParagraphElement", Some("p"))]
    #[case("HTMLUListElement", Some("ul"))]
    #[case("HTMLOListElement", Some("ol"))]
    #[case("HTMLLIElement", Some("li"))]
    #[case("HTMLTableElement", Some("table"))]
    #[case("SVGSVGElement", Some("svg"))]
    #[case("SVGCircleElement", Some("circle"))]
    #[case("SVGPathElement", Some("path"))]
    #[case("SVGRectElement", Some("rect"))]
    #[case("SVGLineElement", Some("line"))]
    #[case("SVGGElement", Some("g"))]
    #[case("UnknownElement", None)]
    #[case("HTMLElement", None)] // the bare, unqualified name is deliberately NOT in this table
    #[case("", None)]
    fn test_html_element_from_type_arg_table(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(html_element_from_type_arg(input), expected);
    }

    // ── is_react_builtin: every name in the hardcoded list, one case per
    // literal — not a sample — plus the `extra` HashSet's three distinct
    // behaviors (a name only in extra, a name in the hardcoded list even with
    // an unrelated extra set, and a name in neither).

    #[rstest]
    #[case("ReactNode")]
    #[case("ReactElement")]
    #[case("JSX.Element")]
    #[case("CSSProperties")]
    #[case("CSSObject")]
    #[case("SyntheticEvent")]
    #[case("MouseEvent")]
    #[case("KeyboardEvent")]
    #[case("ChangeEvent")]
    #[case("FocusEvent")]
    #[case("FormEvent")]
    #[case("DragEvent")]
    #[case("TouchEvent")]
    #[case("WheelEvent")]
    #[case("AnimationEvent")]
    #[case("TransitionEvent")]
    #[case("ClipboardEvent")]
    #[case("CompositionEvent")]
    #[case("MouseEventHandler")]
    #[case("KeyboardEventHandler")]
    #[case("ChangeEventHandler")]
    #[case("FocusEventHandler")]
    #[case("FormEventHandler")]
    #[case("DragEventHandler")]
    #[case("TouchEventHandler")]
    #[case("WheelEventHandler")]
    #[case("AnimationEventHandler")]
    #[case("TransitionEventHandler")]
    #[case("ClipboardEventHandler")]
    #[case("CompositionEventHandler")]
    #[case("PointerEventHandler")]
    #[case("ReactEventHandler")]
    #[case("SubmitEventHandler")]
    #[case("InputEventHandler")]
    #[case("ToggleEventHandler")]
    #[case("FC")]
    #[case("FunctionComponent")]
    #[case("VFC")]
    #[case("VoidFunctionComponent")]
    #[case("ComponentType")]
    #[case("PropsWithChildren")]
    #[case("PropsWithRef")]
    #[case("RefObject")]
    #[case("Ref")]
    #[case("ForwardedRef")]
    #[case("MutableRefObject")]
    #[case("RefCallback")]
    #[case("LegacyRef")]
    #[case("Context")]
    #[case("Consumer")]
    #[case("Provider")]
    #[case("ComponentPropsWithoutRef")]
    #[case("ComponentPropsWithRef")]
    #[case("ComponentProps")]
    #[case("ElementRef")]
    #[case("ElementType")]
    #[case("ReactPortal")]
    #[case("ReactFragment")]
    #[case("ReactChild")]
    #[case("ForwardRefExoticComponent")]
    #[case("RefAttributes")]
    #[case("ComponentRef")]
    #[case("JSXElementConstructor")]
    #[case("SVGAttributes")]
    #[case("SVGProps")]
    #[case("HTMLProps")]
    fn is_react_builtin_recognizes_every_hardcoded_name(#[case] name: &str) {
        let empty = rustc_hash::FxHashSet::default();
        assert!(is_react_builtin(name, &empty), "expected '{name}' to be recognized as a builtin");
    }

    #[test]
    fn is_react_builtin_rejects_a_name_in_neither_the_hardcoded_list_nor_extra() {
        let empty = rustc_hash::FxHashSet::default();
        assert!(!is_react_builtin("SomeProjectSpecificType", &empty));
    }

    #[test]
    fn is_react_builtin_recognizes_a_name_present_only_in_extra() {
        let mut extra = rustc_hash::FxHashSet::default();
        extra.insert(compact_str::CompactString::from("MyLibraryVariant"));
        assert!(is_react_builtin("MyLibraryVariant", &extra));
        // Sanity: a name NOT in extra and not hardcoded still isn't recognized,
        // proving `extra` isn't accidentally matching everything.
        assert!(!is_react_builtin("SomeOtherType", &extra));
    }

    #[test]
    fn is_react_builtin_recognizes_a_hardcoded_name_even_with_an_unrelated_extra_set() {
        let mut extra = rustc_hash::FxHashSet::default();
        extra.insert(compact_str::CompactString::from("MyLibraryVariant"));
        assert!(
            is_react_builtin("ReactNode", &extra),
            "the hardcoded list must not be masked by a non-empty extra set"
        );
    }

    // ── notable_html_attrs: exact content for every named element, not just
    // non-empty — a curated list is exactly the kind of data a "looks
    // roughly right" check would silently let drift.

    #[test]
    fn notable_html_attrs_button_is_exact() {
        assert_eq!(
            notable_html_attrs("button"),
            &[
                "onClick",
                "onKeyDown",
                "onKeyUp",
                "onFocus",
                "onBlur",
                "disabled",
                "type",
                "form",
                "name",
                "value",
                "tabIndex",
                "aria-label",
                "aria-describedby",
                "aria-expanded",
                "aria-pressed",
                "aria-haspopup",
            ]
        );
    }

    #[test]
    fn notable_html_attrs_input_is_exact() {
        assert_eq!(
            notable_html_attrs("input"),
            &[
                "onChange",
                "onInput",
                "onFocus",
                "onBlur",
                "value",
                "defaultValue",
                "placeholder",
                "type",
                "disabled",
                "readOnly",
                "required",
                "name",
                "min",
                "max",
                "pattern",
                "autoComplete",
                "checked",
                "defaultChecked",
            ]
        );
    }

    #[test]
    fn notable_html_attrs_anchor_is_exact() {
        assert_eq!(notable_html_attrs("a"), &["href", "target", "rel", "download", "onClick"]);
    }

    #[test]
    fn notable_html_attrs_textarea_is_exact() {
        assert_eq!(
            notable_html_attrs("textarea"),
            &[
                "onChange",
                "value",
                "defaultValue",
                "placeholder",
                "disabled",
                "readOnly",
                "required",
                "rows",
                "maxLength"
            ]
        );
    }

    #[test]
    fn notable_html_attrs_select_is_exact() {
        assert_eq!(
            notable_html_attrs("select"),
            &["onChange", "value", "defaultValue", "disabled", "multiple", "required"]
        );
    }

    #[test]
    fn notable_html_attrs_form_is_exact() {
        assert_eq!(notable_html_attrs("form"), &["onSubmit", "onReset", "action", "method", "encType", "noValidate"]);
    }

    #[test]
    fn notable_html_attrs_img_is_exact() {
        assert_eq!(notable_html_attrs("img"), &["src", "alt", "width", "height", "loading", "onLoad", "onError"]);
    }

    #[test]
    fn notable_html_attrs_falls_back_to_the_generic_default_for_an_unrecognized_element() {
        let generic = &["onClick", "onFocus", "onBlur", "className", "style", "id", "tabIndex", "aria-label"];
        assert_eq!(notable_html_attrs("div"), generic);
        assert_eq!(notable_html_attrs("span"), generic);
        assert_eq!(notable_html_attrs(""), generic, "an empty element name should also hit the generic default");
    }
}
