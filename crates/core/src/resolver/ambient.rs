//! Which inherited dependency sources are ambient: what every element has, or a style system, which a reader would not
//! lead with. The resolver collapses the large ones into a layer (names only) and lists every other dependency source,
//! which is the API a dependency's component exposes (its modes, state and composition).
//!
//! Which clause fits reads only the source's file, its type name and a prop's name; the resolver adds its size (below). It
//! is the rule measured in the "Layer-only props" section of `docs/measurement.md`; the test
//! `the_rule_gives_the_clause_the_oracles_classifier_gives` keeps this module equal to that classifier's recorded
//! verdicts. The first clause that fits decides:
//!
//! 1. **dom file**: the source's file is `@types/react`, or react-aria's `@react-types/shared/src/dom.d.ts`
//! 2. **dom type**: the source's type is a DOM family: `GlobalDOMEvents`, `DOMAttributes`, `AriaAttributes`,
//!    `AllHTMLAttributes`, `SVGAttributes`, `*HTMLAttributes`, `Aria*Attributes`
//! 3. **style type**: the source's type is a style system: `*StyleProps`, `*StyleProp`, `SystemProps`,
//!    `Space|Layout|Color|Typography|Border|Flex|Grid|Position|Shadow|Background` + `Props`
//! 4. **global attribute**: the prop is one every element has, or starts `aria-` or `data-`
//! 5. **dependency api**: anything else
//!
//! Clauses 1 to 3 decide a whole source: an ambient source is one the resolver may collapse, and only when it is also
//! large (`ResolutionContext::collapses`: a layer record costs more than a handful of props, and listing a small
//! ambient source gives `aria-label` and `id` their types). Clause 4 decides a prop of a source the first three did not
//! match; it never collapses a source on its own. A source that is API (clause 5) collapses only past
//! `LARGE_API_SOURCE` props, which no API measured on the pinned corpora and on apps reaches.
//!
//! Clause 4 is not part of the production rule: the resolver decides per source (`is_ambient_source`, clauses 1 to 3)
//! and never reads a prop's name. It is kept under `#[cfg(test)]` so the parity test can compare the whole classifier,
//! not only the source verdicts, with the oracle's.

/// The attributes every element has, which no component's API is about. `defaultValue`, `value`, `type`, `name` and
/// the like are not here: they are element-specific, and a dependency's component often uses them as its own API.
#[cfg(test)]
const GLOBAL_ATTRIBUTES: &[&str] = &[
    "className",
    "style",
    "id",
    "title",
    "lang",
    "dir",
    "hidden",
    "tabIndex",
    "role",
    "slot",
    "accessKey",
    "draggable",
    "contentEditable",
    "spellCheck",
    "translate",
    "inputMode",
    "nonce",
];

/// Type names of the DOM families that are named exactly (the rest end in `HTMLAttributes` or are `Aria*Attributes`).
const DOM_TYPES: &[&str] =
    &["GlobalDOMEvents", "DOMAttributes", "AriaAttributes", "AllHTMLAttributes", "SVGAttributes"];

/// The style-system prefixes that, with `Props`, name a style source (`SpaceProps`, `LayoutProps`, ...).
const STYLE_PREFIXES: &[&str] =
    &["Space", "Layout", "Color", "Typography", "Border", "Flex", "Grid", "Position", "Shadow", "Background"];

/// Which clause of the rule decided. Only the parity test with the oracle's classifier reads the per-prop verdict; the
/// resolver needs the whole-source clauses (`is_ambient_source`).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// The source's file is a DOM declaration file (clause 1).
    DomFile,
    /// The source's type is a DOM family (clause 2).
    DomType,
    /// The source's type is a style system (clause 3).
    StyleType,
    /// The prop is one every element has (clause 4).
    GlobalAttribute,
    /// None of the above: a dependency's own API (clause 5).
    DependencyApi,
}

#[cfg(test)]
impl Why {
    /// Whether the prop is ambient (clauses 1 to 4) and not API (clause 5).
    fn is_ambient(self) -> bool {
        self != Self::DependencyApi
    }
}

/// `\w*` in the rule: letters, digits and underscore.
fn is_word(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_dom_file(file: &str) -> bool {
    file.contains("/@types/react/")
        || file
            .rsplit_once("/@react-types/shared/src/")
            .is_some_and(|(_, name)| matches!(name, "dom.d.ts" | "dom.d.mts" | "dom.d.cts"))
}

fn is_dom_type(type_name: &str) -> bool {
    DOM_TYPES.contains(&type_name)
        || type_name.strip_suffix("HTMLAttributes").is_some_and(is_word)
        || type_name.strip_prefix("Aria").and_then(|rest| rest.strip_suffix("Attributes")).is_some_and(is_word)
}

fn is_style_type(type_name: &str) -> bool {
    type_name == "SystemProps"
        || type_name.strip_suffix("StyleProps").or_else(|| type_name.strip_suffix("StyleProp")).is_some_and(is_word)
        || STYLE_PREFIXES.iter().any(|prefix| type_name.strip_prefix(prefix) == Some("Props"))
}

/// Whether `prop` is one every element has, or an `aria-` or `data-` attribute (clause 4).
#[cfg(test)]
fn is_global_attribute(prop: &str) -> bool {
    GLOBAL_ATTRIBUTES.contains(&prop) || prop.starts_with("aria-") || prop.starts_with("data-")
}

/// Whether a source in `file` declaring the type `type_name` is ambient as a whole (clauses 1 to 3), so the resolver may
/// record it as a layer of names instead of listing its props (it does when the source is also large).
pub(crate) fn is_ambient_source(file: &str, type_name: &str) -> bool {
    is_dom_file(file) || is_dom_type(type_name) || is_style_type(type_name)
}

/// The rule for one prop a collapsed source names: the clause that decided and so whether it is ambient or API.
#[cfg(test)]
fn classify(file: &str, type_name: &str, prop: &str) -> Why {
    if is_dom_file(file) {
        Why::DomFile
    } else if is_dom_type(type_name) {
        Why::DomType
    } else if is_style_type(type_name) {
        Why::StyleType
    } else if is_global_attribute(prop) {
        Why::GlobalAttribute
    } else {
        Why::DependencyApi
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const REACT: &str = "/p/node_modules/@types/react/index.d.ts";
    const ARIA_DOM: &str = "/p/node_modules/@react-types/shared/src/dom.d.ts";
    const ARIA_DOM_MTS: &str = "/p/node_modules/@react-types/shared/src/dom.d.mts";
    const ARIA_EVENTS: &str = "/p/node_modules/@react-types/shared/src/events.d.ts";
    const PKG: &str = "/p/node_modules/x/index.d.ts";

    #[rstest]
    #[case(REACT, "Anything", "foo", Why::DomFile)]
    #[case(ARIA_DOM, "GlobalDOMEvents", "onClick", Why::DomFile)]
    #[case(ARIA_DOM_MTS, "Other", "x", Why::DomFile)]
    #[case(ARIA_EVENTS, "KeyboardEvents", "onKeyDown", Why::DependencyApi)]
    #[case(PKG, "GlobalDOMEvents", "onClick", Why::DomType)]
    #[case(PKG, "DOMAttributes", "onClick", Why::DomType)]
    #[case(PKG, "AriaAttributes", "aria-label", Why::DomType)]
    #[case(PKG, "AllHTMLAttributes", "href", Why::DomType)]
    #[case(PKG, "SVGAttributes", "d", Why::DomType)]
    #[case(PKG, "HTMLAttributes", "dir", Why::DomType)]
    #[case(PKG, "ButtonHTMLAttributes", "type", Why::DomType)]
    #[case(PKG, "AriaLabelingAttributes", "aria-label", Why::DomType)]
    #[case(PKG, "AriaAttributesExtra", "foo", Why::DependencyApi)]
    #[case(PKG, "Aria_Attributes", "foo", Why::DomType)]
    #[case(PKG, "Aria-Attributes", "foo", Why::DependencyApi)]
    #[case(PKG, "StyleProps", "margin", Why::StyleType)]
    #[case(PKG, "JsxStyleProps", "margin", Why::StyleType)]
    #[case(PKG, "ViewStyleProp", "margin", Why::StyleType)]
    #[case(PKG, "SystemProps", "margin", Why::StyleType)]
    #[case(PKG, "SpaceProps", "m", Why::StyleType)]
    #[case(PKG, "BackgroundProps", "bg", Why::StyleType)]
    #[case(PKG, "Props", "m", Why::DependencyApi)]
    #[case(PKG, "ColorsProps", "m", Why::DependencyApi)]
    #[case(PKG, "StylePropsExtra", "m", Why::DependencyApi)]
    #[case(PKG, "DialogProps", "className", Why::GlobalAttribute)]
    #[case(PKG, "DialogProps", "aria-label", Why::GlobalAttribute)]
    #[case(PKG, "DialogProps", "data-testid", Why::GlobalAttribute)]
    #[case(PKG, "DialogProps", "tabIndex", Why::GlobalAttribute)]
    #[case(PKG, "DialogProps", "value", Why::DependencyApi)]
    #[case(PKG, "DialogProps", "defaultValue", Why::DependencyApi)]
    #[case(PKG, "DialogProps", "type", Why::DependencyApi)]
    #[case(PKG, "DialogProps", "name", Why::DependencyApi)]
    #[case(PKG, "DialogProps", "disabled", Why::DependencyApi)]
    #[case(PKG, "DialogProps", "Aria-label", Why::DependencyApi)]
    fn the_clause_that_decides_is_the_first_that_fits(
        #[case] file: &str,
        #[case] type_name: &str,
        #[case] prop: &str,
        #[case] expected: Why,
    ) {
        assert_eq!(classify(file, type_name, prop), expected);
    }

    /// `ambient_parity.json` holds the verdicts of the TypeScript classifier of the "Layer-only props" measurement
    /// (`docs/measurement.md`) for every distinct (file, type, prop) a collapsed layer names on antd and shadcn, plus
    /// synthetic edge cases (`/p/node_modules/x/index.d.ts`) for the style-system and DOM-type clauses, which those two
    /// corpora do not exercise (the verdicts are the classifier's, not written by hand). Chakra, Mantine and Spectrum
    /// are not in the data. This module must give the same clause for each.
    #[test]
    fn the_rule_gives_the_clause_the_oracles_classifier_gives() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!("ambient_parity.json")).unwrap();
        assert!(rows.len() > 500, "{} rows", rows.len());
        let mut seen = std::collections::BTreeSet::new();
        for row in &rows {
            let text = |key: &str| row[key].as_str().unwrap_or_else(|| panic!("{key} in {row}"));
            let (file, type_name, prop, label) = (text("file"), text("type"), text("prop"), text("why"));
            let expected = match label {
                "dom file" => Why::DomFile,
                "dom type" => Why::DomType,
                "style type" => Why::StyleType,
                "global attribute" => Why::GlobalAttribute,
                "dependency api" => Why::DependencyApi,
                other => panic!("unknown clause {other}"),
            };
            let got = classify(file, type_name, prop);
            assert_eq!(got, expected, "{file} {type_name} {prop}");
            assert_eq!(text("kind") == "ambient", got.is_ambient(), "{file} {type_name} {prop}");
            seen.insert(label.to_owned());
            // Clauses one to three decide a whole source, and so whether the resolver may collapse it.
            if matches!(expected, Why::DomFile | Why::DomType | Why::StyleType) {
                assert!(is_ambient_source(file, type_name), "{file} {type_name}");
            }
        }
        assert_eq!(seen.len(), 5, "every clause is covered: {seen:?}");
    }

    #[test]
    fn only_clauses_one_to_three_collapse_a_source() {
        assert!(is_ambient_source(REACT, "X"));
        assert!(is_ambient_source(PKG, "GlobalDOMEvents"));
        assert!(is_ambient_source(PKG, "SystemProps"));
        // A source of global attributes only is not collapsed on its own: clause 4 decides a prop, not a source.
        assert!(!is_ambient_source(PKG, "DialogProps"));
    }
}
