use super::*;
use crate::extractor::parse_file;
use crate::types::collected::SourceData;

fn parse(source: &str) -> SourceData {
    parse_file(camino::Utf8Path::new("/p/c.tsx"), source)
}

fn body_of(source: &str, component: &str) -> Option<BodyFacts> {
    let data = parse(source);
    let mapping = data.component_mappings.iter().find(|mapping| mapping.component_name == component);
    mapping.unwrap_or_else(|| panic!("no component {component}")).body.clone()
}

fn facts(source: &str, component: &str) -> BodyFacts {
    body_of(source, component).expect("the component has a function body")
}

fn named(name: &str, prop: Option<&str>) -> JsxAttributeFact {
    JsxAttributeFact::Named { name: name.into(), prop: prop.map(Into::into) }
}

fn intrinsic(tag: &str, attributes: Vec<JsxAttributeFact>) -> JsxElementFact {
    JsxElementFact { tag: JsxTag::Intrinsic(tag.into()), attributes }
}

fn component(tag: &str, attributes: Vec<JsxAttributeFact>) -> JsxElementFact {
    JsxElementFact { tag: JsxTag::Identifier(tag.into()), attributes }
}

fn usage(tag: JsxTag) -> JsxUsage {
    JsxUsage { tag, attributes: vec![] }
}

fn reads(facts: &BodyFacts) -> Vec<&str> {
    facts.reads.iter().map(String::as_str).collect()
}

#[test]
fn destructuring_reads_renamed_defaulted_nested_keys_and_the_rest_binding() {
    let facts = facts(
        "export function C({ tone: t, size = 'md', style: { color }, ...rest }: P) {\n\
           return <div data-tone={t} data-size={size} data-color={color} {...rest} />\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["tone", "size", "style"]);
    assert_eq!(facts.rest.as_deref(), Some("rest"));
    assert_eq!(
        facts.elements,
        [intrinsic(
            "div",
            vec![
                named("data-tone", Some("tone")),
                named("data-size", Some("size")),
                named("data-color", None),
                JsxAttributeFact::Spread(SpreadOf::Rest),
            ]
        )]
    );
    assert!(!facts.handed_on);
}

#[test]
fn member_reads_of_the_props_identifier_are_reads_anywhere_in_the_body() {
    let facts = facts(
        "export function C(props: P) {\n\
           const label = props.label;\n\
           const onClick = () => props['onPress']?.();\n\
           return <button title={props.title} aria-label={label} onClick={onClick} />\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["label", "onPress", "title"]);
    assert_eq!(facts.rest, None);
    assert_eq!(
        facts.elements,
        [intrinsic("button", vec![named("title", Some("title")), named("aria-label", None), named("onClick", None)])]
    );
    assert!(!facts.handed_on);
}

#[test]
fn the_three_safe_uses_read_keys_and_do_not_hand_the_props_on() {
    let facts = facts(
        "export function C(props: P) {\n\
           const a = props?.a; const b = props?.['b'];\n\
           const { tone, size = 1, ...r } = props;\n\
           let c; ({ c } = props);\n\
           return <div data-x={r.extra} {...r} />\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["a", "b", "tone", "size", "c", "extra"]);
    assert_eq!(facts.rest.as_deref(), Some("r"));
    assert_eq!(
        facts.elements,
        [intrinsic("div", vec![named("data-x", Some("extra")), JsxAttributeFact::Spread(SpreadOf::Rest)])]
    );
    assert!(!facts.handed_on);
}

#[test]
fn every_other_use_of_the_props_object_hands_it_on() {
    let uses = [
        "useX(props)",
        "new Store(props)",
        "<Child p={props} />",
        "({ value: props })",
        "({ props })",
        "({ ...props })",
        "[props]",
        "useMemo(() => 1, [props])",
        "props[key]",
        "typeof props",
        "const copy = props",
        "const [first] = props",
        "useY(rest)",
        "({ ...rest })",
    ];
    for used in uses {
        let source =
            format!("export function C(props: P) {{ const {{ a, ...rest }} = props; {used}; return <div /> }}");
        assert!(facts(&source, "C").handed_on, "`{used}` hands the props object on");
    }
    let returned = facts("export function C(props: P) { if (props.a) return <i />; return props }", "C");
    assert!(returned.handed_on);
    let spread = facts("export function C(props: P) { return <Inner {...props} /> }", "C");
    assert_eq!(spread.elements, [component("Inner", vec![JsxAttributeFact::Spread(SpreadOf::Props)])]);
    assert!(spread.handed_on);
}

#[test]
fn a_shadowing_parameter_or_local_is_another_binding() {
    let callback = facts(
        "export const C = ({ tone, items }: P) => (\n\
           <ul>{items.map((tone) => <i data-x={tone} />)}</ul>\n\
         )",
        "C",
    );
    assert_eq!(callback.elements[1], intrinsic("i", vec![named("data-x", None)]));

    let shadowed = facts(
        "export function C(props: P) {\n\
           const list = props.list.map((props) => <b {...props} />);\n\
           { const props = other; use(props); }\n\
           try { x() } catch (props) { use(props) }\n\
           return <ul>{list}</ul>\n\
         }",
        "C",
    );
    assert_eq!(reads(&shadowed), ["list"]);
    assert_eq!(shadowed.elements[0], intrinsic("b", vec![JsxAttributeFact::Spread(SpreadOf::Other)]));
    assert!(!shadowed.handed_on);
}

#[test]
fn a_rest_member_is_a_read_and_the_attribute_names_the_same_prop() {
    let facts = facts("export const C = ({ a, ...rest }: P) => <b data-x={rest.x} data-y={rest['y']} />", "C");
    assert_eq!(reads(&facts), ["a", "x", "y"]);
    assert_eq!(facts.elements, [intrinsic("b", vec![named("data-x", Some("x")), named("data-y", Some("y"))])]);
    assert!(!facts.handed_on);
}

#[test]
fn a_tag_taken_from_a_prop_is_read_not_skipped() {
    let facts = facts(
        "export function C(props: P) {\n\
           const Tag = props.as ?? 'div';\n\
           const Icon = props.icon;\n\
           return <Tag><Icon /><props.Slot /></Tag>\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["as", "icon", "Slot"]);
    assert!(!facts.handed_on);
    assert_eq!(facts.elements.len(), 3);
}

#[test]
fn attributes_keep_their_order_around_a_spread_and_the_spread_says_what_it_spreads() {
    let facts = facts(
        "export const C = ({ id, ...rest }: P) => (\n\
           <section>\n\
             <input id={id} {...rest} type=\"text\" />\n\
             <Field {...rest} id={rest.id} />\n\
             <span {...other} />\n\
           </section>\n\
         )",
        "C",
    );
    assert_eq!(
        facts.elements,
        [
            intrinsic("section", vec![]),
            intrinsic(
                "input",
                vec![named("id", Some("id")), JsxAttributeFact::Spread(SpreadOf::Rest), named("type", None)]
            ),
            component("Field", vec![JsxAttributeFact::Spread(SpreadOf::Rest), named("id", Some("id"))]),
            intrinsic("span", vec![JsxAttributeFact::Spread(SpreadOf::Other)]),
        ]
    );
    assert!(!facts.handed_on);
}

#[test]
fn a_member_of_the_props_object_spread_is_not_a_hand_off() {
    let facts = facts("export function C(props: P) { useX(props.a); return <div {...props.b} /> }", "C");
    assert!(!facts.handed_on);
    assert_eq!(reads(&facts), ["a", "b"]);
    assert_eq!(facts.elements, [intrinsic("div", vec![JsxAttributeFact::Spread(SpreadOf::Other)])]);
}

#[test]
fn tags_follow_the_jsx_rule_not_a_list_of_names() {
    let facts = facts(
        "export function C(props: P) {\n\
           return <><chakra.button /><Menu.Item.Label /><my-element /><svg:path /><Box /><div /></>\n\
         }",
        "C",
    );
    let tags: Vec<&JsxTag> = facts.elements.iter().map(|element| &element.tag).collect();
    assert_eq!(
        tags,
        [
            &JsxTag::Member("chakra.button".into()),
            &JsxTag::Member("Menu.Item.Label".into()),
            &JsxTag::Intrinsic("my-element".into()),
            &JsxTag::Intrinsic("svg:path".into()),
            &JsxTag::Identifier("Box".into()),
            &JsxTag::Intrinsic("div".into()),
        ]
    );
}

#[test]
fn forward_ref_and_memo_wrapped_functions_are_read_through_the_wrapper() {
    let source = "import { forwardRef, memo } from 'react';\n\
         export const A = forwardRef<HTMLButtonElement, P>(function A({ size }, ref) {\n\
           return <button ref={ref} data-size={size} />\n\
         });\n\
         export const B = memo(forwardRef((props: P, ref) => <a ref={ref} href={props.href} />));\n\
         export const M = memo(function M({ label }: P) { return <span>{label}</span> });";
    let a = facts(source, "A");
    assert_eq!(reads(&a), ["size"]);
    assert_eq!(a.elements, [intrinsic("button", vec![named("ref", None), named("data-size", Some("size"))])]);
    let b = facts(source, "B");
    assert_eq!(reads(&b), ["href"]);
    assert_eq!(b.elements, [intrinsic("a", vec![named("ref", None), named("href", Some("href"))])]);
    let m = facts(source, "M");
    assert_eq!(reads(&m), ["label"]);
    assert_eq!(m.elements, [intrinsic("span", vec![])]);
}

#[test]
fn every_return_branch_and_both_sides_of_a_conditional_are_recorded() {
    let facts = facts(
        "export function C({ href, open }: P) {\n\
           if (href) return <a href={href} />;\n\
           return open ? <Dialog open={open} /> : (open === false && <span />);\n\
         }",
        "C",
    );
    assert_eq!(
        facts.elements,
        [
            intrinsic("a", vec![named("href", Some("href"))]),
            component("Dialog", vec![named("open", Some("open"))]),
            intrinsic("span", vec![]),
        ]
    );
}

#[test]
fn a_nested_component_keeps_its_props_uses_and_its_elements_go_to_the_file() {
    let data = parse(
        "export function C(props: P) {\n\
           const Row = ({ item }: { item: string }) => <li data-item={item} title={props.label} />;\n\
           function Empty() { return <em /> }\n\
           return props.items.length ? <ul>{props.items.map((item) => <Row item={item} />)}</ul> : <Empty />;\n\
         }",
    );
    let facts = data.component_mappings[0].body.clone().unwrap();
    assert_eq!(
        facts.elements,
        [intrinsic("ul", vec![]), component("Row", vec![named("item", None)]), component("Empty", vec![])]
    );
    assert_eq!(reads(&facts), ["label", "items"]);
    let tags: Vec<&JsxTag> = data.jsx_usages.iter().map(|usage| &usage.tag).collect();
    assert_eq!(tags, [&JsxTag::Intrinsic("li".into()), &JsxTag::Intrinsic("em".into())]);
}

#[test]
fn a_class_components_elements_go_to_the_file() {
    let data = parse(
        "import { Component } from 'react';\n\
         export class K extends Component<P> { render() { return <div /> } }",
    );
    assert_eq!(data.component_mappings.iter().find(|m| m.component_name == "K").unwrap().body, None);
    assert_eq!(data.jsx_usages, [usage(JsxTag::Intrinsic("div".into()))]);
}

#[test]
fn create_element_calls_record_no_elements() {
    let facts = facts(
        "import React from 'react';\n\
         export function C({ onPress }: P): React.ReactElement {\n\
           return React.createElement('button', { onClick: onPress });\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["onPress"]);
    assert_eq!(facts.elements, []);
    assert!(!facts.handed_on);
}

#[test]
fn jsx_outside_every_component_body_is_a_usage_of_the_file() {
    let data = parse_file(
        camino::Utf8Path::new("/p/c.stories.tsx"),
        "export function C(props: P) { return <div {...props} /> }\n\
         export const icon = <svg:g><Icon.Star size=\"sm\" {...rest} /></svg:g>;\n\
         export const Primary = { render: (args) => <C {...args} tone=\"loud\" /> };",
    );
    assert_eq!(
        data.jsx_usages,
        [
            usage(JsxTag::Intrinsic("svg:g".into())),
            JsxUsage {
                tag: JsxTag::Member("Icon.Star".into()),
                attributes: vec![JsxUsageAttribute::Named("size".into()), JsxUsageAttribute::Spread],
            },
            JsxUsage {
                tag: JsxTag::Identifier("C".into()),
                attributes: vec![JsxUsageAttribute::Spread, JsxUsageAttribute::Named("tone".into())],
            },
        ]
    );
}
