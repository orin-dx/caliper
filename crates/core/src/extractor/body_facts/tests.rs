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
    assert_eq!(facts.rests, ["rest"]);
    assert_eq!(
        facts.elements,
        [intrinsic(
            "div",
            vec![
                named("data-tone", Some("tone")),
                // A defaulted name is the prop or its default, not the prop.
                named("data-size", None),
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
           const onClick = () => use(props['onPress']);\n\
           return <button title={props.title} aria-label={label} onClick={onClick} />\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["label", "onPress", "title"]);
    assert_eq!(facts.rests, Vec::<String>::new());
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
    assert_eq!(facts.rests, ["r"]);
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
    // Both are bound in the body's scope, not the module's: recorded as scoped, never as a module binding's name.
    let scoped = |tag: &str, attributes| JsxElementFact { tag: JsxTag::Scoped(tag.into()), attributes };
    assert_eq!(
        facts.elements,
        [intrinsic("ul", vec![]), scoped("Row", vec![named("item", None)]), scoped("Empty", vec![])]
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

#[test]
fn parameter_defaults_and_computed_keys_are_references_too() {
    let read = facts("export function C(props: P, { x = props.b } = {}) { return <div /> }", "C");
    assert_eq!(reads(&read), ["b"]);
    assert!(!read.handed_on);
    let handed = facts("export function C(props: P, store = use(props)) { return <div /> }", "C");
    assert!(handed.handed_on);
    let key = facts("export function C(props: P) { const { [props.k]: v } = other; return <div /> }", "C");
    assert_eq!(reads(&key), ["k"]);
}

#[test]
fn a_body_declaration_does_not_capture_a_parameter_default() {
    // The default is evaluated in the parameters' scope: `props` there is the parameter, whatever the body declares.
    let facts = facts("export function C(props: P, d = use(props)) { const use = 1; return <div /> }", "C");
    assert!(facts.handed_on);
}

#[test]
fn a_var_in_a_nested_function_is_another_binding() {
    let facts = facts(
        "export function C({ size }: P) {\n\
           const f = () => { var size = 2; return <i data-s={size} /> };\n\
           return <b data-s={size}>{f()}</b>\n\
         }",
        "C",
    );
    assert_eq!(
        facts.elements,
        [intrinsic("i", vec![named("data-s", None)]), intrinsic("b", vec![named("data-s", Some("size"))])]
    );
}

#[test]
fn a_named_function_expression_binds_its_own_name() {
    let facts = facts(
        "export function C(props: P) { const g = function props() { return props.x }; return <div>{g()}</div> }",
        "C",
    );
    assert_eq!(facts.reads, Vec::<String>::new());
    assert!(!facts.handed_on);
}

#[test]
fn every_rest_binding_is_the_props_object_and_is_named() {
    let facts = facts(
        "export function C(props: P) {\n\
           const { a, ...r1 } = props; const { b, ...r2 } = r1;\n\
           return <div {...r2} data-z={r2.z} />\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["a", "b", "z"]);
    assert_eq!(facts.rests, ["r1", "r2"]);
    assert_eq!(
        facts.elements,
        [intrinsic("div", vec![JsxAttributeFact::Spread(SpreadOf::Rest), named("data-z", Some("z"))])]
    );
    assert!(!facts.handed_on);
}

#[test]
fn a_switch_discriminant_is_outside_the_case_bindings() {
    let facts = facts(
        "export function C(props: P) { switch (props.kind) { case 'a': const props = 1; return <i /> } return null }",
        "C",
    );
    assert_eq!(reads(&facts), ["kind"]);
    assert!(!facts.handed_on);
}

#[test]
fn typescript_wrappers_around_the_props_object_are_seen_through() {
    let facts = facts(
        "export function C(props: P) {\n\
           const x = props!.x; const y = (props as P).y;\n\
           const { a } = props as P; const { b } = props satisfies P;\n\
           return <div />\n\
         }",
        "C",
    );
    assert_eq!(reads(&facts), ["x", "y", "a", "b"]);
    assert!(!facts.handed_on);
}

#[test]
fn a_method_call_a_write_or_a_delete_through_the_props_object_hands_it_on() {
    let uses = [
        "props.onClick()",
        "props.onClick?.()",
        "props.hasOwnProperty('a')",
        "props.x = 1",
        "delete props.x",
        "props.n++",
    ];
    for used in uses {
        let source = format!("export function C(props: P) {{ {used}; return <div /> }}");
        assert!(facts(&source, "C").handed_on, "`{used}` hands the props object on");
    }
}

#[test]
fn a_reassigned_or_defaulted_local_is_not_the_prop() {
    let facts = facts(
        "export function C(props: P) {\n\
           let { a } = props; a = f(a);\n\
           const { b = 5, c } = props;\n\
           return <i data-a={a} data-b={b} data-c={c} />\n\
         }",
        "C",
    );
    assert_eq!(
        facts.elements,
        [intrinsic("i", vec![named("data-a", None), named("data-b", None), named("data-c", Some("c"))])]
    );
}

#[test]
fn a_nested_component_is_found_by_symbol_not_by_name() {
    let facts = facts(
        "import { Row } from './row';\n\
         export function C(props: P) {\n\
           { const Row = memoize(() => <li {...props} />); use(Row); }\n\
           return <Row />\n\
         }",
        "C",
    );
    assert_eq!(
        facts.elements,
        [intrinsic("li", vec![JsxAttributeFact::Spread(SpreadOf::Props)]), component("Row", vec![])]
    );
}

#[test]
fn deleting_or_calling_through_an_optional_chain_hands_the_props_object_on() {
    let uses = ["delete props.x", "delete props?.x", "delete rest?.x", "(props?.f)()", "(rest?.f)()", "props?.f()"];
    for used in uses {
        let source =
            format!("export function C(props: P) {{ const {{ a, ...rest }} = props; {used}; return <div /> }}");
        assert!(facts(&source, "C").handed_on, "`{used}` hands the props object on");
    }
    let read = facts("export function C(props: P) { const x = props?.x; return <div /> }", "C");
    assert!(!read.handed_on);
}

#[test]
fn an_optional_chain_as_an_assignment_target_hands_the_props_object_on() {
    // Not valid JavaScript; the parser recovers, and the classification must stay conservative.
    let data = parse("export function C(props: P) { props?.x = 1; return <div /> }");
    if let Some(body) = data.component_mappings.first().and_then(|mapping| mapping.body.clone()) {
        assert!(body.handed_on);
    }
}

#[test]
fn a_function_held_by_two_mappings_files_its_nested_elements_once() {
    let source = "export const A = function B(props: P) { const Row = () => <li />; return <Row /> };";
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, oxc_span::SourceType::tsx()).parse();
    let start = |text: &str| u32::try_from(source.find(text).unwrap()).unwrap();
    let declarator = (start("A ="), start(" };") + 2);
    let function = (start("function B"), start(" };") + 2);
    let mapping = |name: &str, (from, to): (u32, u32)| {
        ComponentMapping::new(
            name.to_owned(),
            crate::types::TypeName::default(),
            camino::Utf8PathBuf::from("/p/c.tsx"),
            from,
            to,
            crate::types::DetectionRule::FunctionDeclaration,
        )
    };
    let mut mappings = vec![mapping("A", declarator), mapping("B", function)];
    let usages = attach(&parsed.program, &mut mappings, true);
    assert_eq!(usages, [usage(JsxTag::Intrinsic("li".into()))]);
    assert!(mappings.iter().all(|mapping| mapping.body.is_some()));
}

#[test]
fn a_destructured_name_redeclared_by_var_is_not_the_prop() {
    let param = facts("export function C({ a }: P) { var a = compute(); return <X a={a} /> }", "C");
    assert_eq!(param.elements, [component("X", vec![named("a", None)])]);
    let body = facts("export function C(props: P) { var { a } = props; var a = other; return <X a={a} /> }", "C");
    assert_eq!(body.elements, [component("X", vec![named("a", None)])]);
}

#[test]
fn rest_bindings_in_different_scopes_are_all_recorded() {
    let facts = facts(
        "export function C(props: P) {\n\
           const { a, ...r1 } = props;\n\
           const f = () => { const { b, ...r2 } = props; return <i {...r2} /> };\n\
           return <b {...r1}>{f()}</b>\n\
         }",
        "C",
    );
    assert_eq!(facts.rests, ["r1", "r2"]);
    assert!(!facts.handed_on);
}

#[test]
fn jsx_in_a_parameter_default_is_the_components() {
    let data = parse("export const C = ({ icon = <Icon size=\"sm\" /> }: P) => <span>{icon}</span>;");
    let facts = data.component_mappings[0].body.clone().unwrap();
    assert_eq!(facts.elements, [component("Icon", vec![named("size", None)]), intrinsic("span", vec![])]);
    assert_eq!(data.jsx_usages, []);
}

#[test]
fn reads_through_a_rest_binding_keep_source_order() {
    // Every reference to `props` used to be classified before any to `rest`, which put `b` first.
    let rest = facts("export function C(props: P) { const { ...rest } = props; rest.a; props.b; return <div /> }", "C");
    assert_eq!(reads(&rest), ["a", "b"]);
    let chain = facts(
        "export function C(props: P) { const { ...r1 } = props; const { ...r2 } = r1; r2.a; r1.b; props.c; \
         const { ...r3 } = props; return <div /> }",
        "C",
    );
    assert_eq!(reads(&chain), ["a", "b", "c"]);
    assert_eq!(chain.rests, ["r1", "r2", "r3"]);
}

#[test]
fn jsx_in_a_wrappers_other_arguments_is_the_files() {
    // The whole declarator used to be owned, so `<Icon />` was neither the component's nor the file's.
    let icon = || [usage(JsxTag::Identifier("Icon".into()))];
    let options = parse("export const C = withOptions((props: P) => <div />, { fallback: <Icon /> });");
    assert_eq!(options.component_mappings[0].body.as_ref().unwrap().elements, [intrinsic("div", vec![])]);
    assert_eq!(options.jsx_usages, icon());
    let default = parse("export default withOptions(function C(props: P) { return <div /> }, <Icon />);");
    assert_eq!(default.jsx_usages, icon());
    let callee = parse("export const C = withIcon(<Icon />)((props: P) => <div />);");
    assert_eq!(callee.jsx_usages, icon());
}

#[test]
fn a_nested_components_wrapper_arguments_are_the_parents() {
    let facts = facts(
        "export function C(props: P) { const Item = withX(() => <li />, { a: <Icon /> }); return <Item /> }",
        "C",
    );
    // `Item` is bound inside `C`, so its tag is scoped, not the module's binding of that name.
    let item = || JsxElementFact { tag: JsxTag::Scoped("Item".into()), attributes: vec![] };
    assert_eq!(facts.elements, [component("Icon", vec![]), item()]);
    let class = parse(
        "export function C(props: P) { const Item = observer(class extends Component { render() { return <li /> } }); \
         return <Item /> }",
    );
    assert_eq!(class.component_mappings[0].body.as_ref().unwrap().elements, [item()]);
    assert_eq!(class.jsx_usages, [usage(JsxTag::Intrinsic("li".into()))]);
}

/// A small deterministic generator of one-component files (a linear congruential sequence, no dependency): a wrapper
/// around the component function, a body of reads through the props object and rest chains, and JSX in every place a
/// fact could be owned by extent rather than by node (wrapper arguments and callees, nested components and their
/// wrappers, module level).
struct Gen {
    state: u64,
    keys: u32,
    tags: Vec<String>,
}

impl Gen {
    fn next(&mut self, bound: usize) -> usize {
        self.state = self.state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.state >> 33).unwrap() % bound
    }

    /// A fresh prop name; names are numbered in source order, since only the body holds them.
    fn key(&mut self) -> String {
        self.keys += 1;
        format!("k{}x", self.keys)
    }

    fn tag(&mut self, stem: &str) -> String {
        let tag = format!("{stem}{}", self.tags.len());
        self.tags.push(tag.clone());
        tag
    }

    fn jsx(&mut self) -> String {
        format!("<{} />", self.tag("J"))
    }

    fn body(&mut self) -> String {
        let mut objects = vec!["props".to_owned()];
        let mut statements = Vec::new();
        for _ in 0..2 + self.next(6) {
            let object = objects[self.next(objects.len())].clone();
            let statement = match self.next(9) {
                0 | 1 => format!("{object}.{};", self.key()),
                2 => format!("{object}?.['{}'];", self.key()),
                3 | 4 => {
                    let rest = format!("r{}", objects.len());
                    objects.push(rest.clone());
                    if self.next(2) == 0 {
                        format!("const {{ ...{rest} }} = {object};")
                    } else {
                        format!("const {{ {}, ...{rest} }} = {object};", self.key())
                    }
                }
                5 => format!("const x{} = {};", self.tags.len(), self.jsx()),
                6 => {
                    let item = self.tag("Item");
                    format!("const {item} = () => {}; const y{item} = <{item} />;", self.jsx())
                }
                7 => {
                    let item = self.tag("Item");
                    let (inner, option) = (self.jsx(), self.jsx());
                    format!("const {item} = withX(() => {inner}, {{ a: {option} }}); const y{item} = <{item} />;")
                }
                _ => format!("use({object}.{}, {});", self.key(), self.jsx()),
            };
            statements.push(statement);
        }
        statements.push(format!("return {};", self.jsx()));
        statements.join(" ")
    }

    /// The file's source, its prop names and every component tag it writes (each once).
    fn file(seed: u64) -> (String, Vec<String>, Vec<String>) {
        let mut generator = Gen { state: seed, keys: 0, tags: Vec::new() };
        let (wrapper, arrow) = (generator.next(9), generator.next(2) == 0);
        let before =
            if generator.next(3) == 0 { format!("const page = {};\n", generator.jsx()) } else { String::new() };
        let callee = if wrapper == 6 { generator.jsx() } else { String::new() };
        let body = generator.body();
        let function =
            if arrow { format!("(props: P) => {{ {body} }}") } else { format!("function C(props: P) {{ {body} }}") };
        let declaration = match wrapper {
            0 => format!("export function C(props: P) {{ {body} }}"),
            1 => format!("export const C = {function};"),
            2 => format!("export const C = memo({function});"),
            3 => format!("export const C = memo(forwardRef({function}));"),
            4 => format!("export const C = withOptions({function}, {{ fallback: {} }});", generator.jsx()),
            5 => format!("export const C = hoc({function}, {}, other);", generator.jsx()),
            6 => format!("export const C = withIcon({callee})({function});"),
            7 => format!("export default withOptions({function}, {{ fallback: {} }});", generator.jsx()),
            _ => format!("export default memo({function});"),
        };
        let after =
            if generator.next(3) == 0 { format!("\nconst story = {};", generator.jsx()) } else { String::new() };
        let keys = (1..=generator.keys).map(|key| format!("k{key}x")).collect();
        (format!("{before}{declaration}{after}"), keys, generator.tags)
    }
}

fn tag_name(tag: &JsxTag) -> String {
    match tag {
        JsxTag::Identifier(name) | JsxTag::Intrinsic(name) | JsxTag::Member(name) | JsxTag::Scoped(name) => {
            name.clone()
        }
    }
}

/// Invariants over generated files: every JSX element is in exactly one of the component's elements and the file's
/// usages; the component's own elements are in source order; every prop read is recorded, in source order of first
/// read, and nothing generated hands the props object on.
#[test]
fn generated_components_place_every_element_once_and_read_in_source_order() {
    let mut with_body = 0;
    for seed in 0..800 {
        let (source, keys, tags) = Gen::file(seed);
        let data = parse(&source);
        let bodies: Vec<&BodyFacts> =
            data.component_mappings.iter().filter_map(|mapping| mapping.body.as_ref()).collect();
        assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]), "one function, one set of facts:\n{source}");
        let mut seen: Vec<String> = data.jsx_usages.iter().map(|usage| tag_name(&usage.tag)).collect();
        if let Some(body) = bodies.first() {
            with_body += 1;
            assert_eq!(body.reads, keys, "every read, in source order:\n{source}");
            assert!(!body.handed_on, "{source}");
            let own: Vec<String> = body.elements.iter().map(|element| tag_name(&element.tag)).collect();
            let numbered: Vec<usize> = own.iter().filter_map(|tag| tag.strip_prefix('J')?.parse().ok()).collect();
            assert!(numbered.is_sorted(), "elements in source order:\n{source}");
            seen.extend(own);
        }
        seen.sort_unstable();
        let mut expected = tags;
        expected.sort_unstable();
        assert_eq!(seen, expected, "every element in exactly one collection:\n{source}");
    }
    assert!(with_body > 600, "the generator mostly yields components: {with_body}");
}

#[test]
fn an_attribute_value_read_through_any_chain_form_names_the_prop() {
    for value in ["props.a", "props['a']", "props?.a", "props?.['a']", r#"props?.["a"]"#, "props!.a", "(props?.a)!"] {
        let source = format!("export function C(props: P) {{ return <Child value={{{value}}} /> }}");
        let body = facts(&source, "C");
        assert_eq!(reads(&body), ["a"], "{value}");
        assert_eq!(body.elements, [component("Child", vec![named("value", Some("a"))])], "{value}");
    }
    // An optional call is not one prop's value.
    let called = facts("export function C(props: P) { return <Child value={props?.f()} /> }", "C");
    assert_eq!(called.elements, [component("Child", vec![named("value", None)])]);
}

#[test]
fn a_declaration_without_an_implementation_has_no_body_facts() {
    assert_eq!(body_of("export declare function Button(props: P): JSX.Element;", "Button"), None);
    for source in ["declare const Button: FC<P>;", "export declare const Button: FC<P>;"] {
        assert!(parse(source).component_mappings.iter().all(|mapping| mapping.body.is_none()), "{source}");
    }
    // Overload signatures carry no facts; the implementation does.
    let overloaded = "export function Button(props: A): JSX.Element;\n\
                      export function Button(props: P) { return <i /> }";
    let data = parse(overloaded);
    assert!(data.component_mappings.iter().any(|mapping| mapping.body.is_some()));
    assert!(data.component_mappings.iter().all(|mapping| mapping.body.as_ref().is_none_or(|body| !body.handed_on)));
}

#[test]
fn props_named_by_no_first_parameter_are_handed_on() {
    assert!(facts("export function C(props: P) { use(arguments[0]); return <i /> }", "C").handed_on);
    assert!(!facts("export function C(props: P) { return <i /> }", "C").handed_on);
}

/// `arguments` is the component's only in the component itself or in arrows nested directly in it; any nested
/// non-arrow function, method, accessor or class member has its own.
#[test]
fn arguments_is_the_components_only_through_arrows() {
    let handed = |body: &str| facts(&format!("export function C(props: P) {{ {body} return <i /> }}"), "C").handed_on;
    for own in [
        "const f = () => arguments[0];",
        "const f = () => () => arguments[0];",
        "const f = () => { const g = () => arguments; };",
        "const f = (a = arguments[0]) => a;",
    ] {
        assert!(handed(own), "{own}");
    }
    for other in [
        "function first() { return arguments[0] }",
        "const first = function () { return arguments[0] };",
        "const o = { first() { return arguments[0] } };",
        "const o = { get first() { return arguments[0] } };",
        "class K { first() { return arguments[0] } }",
        "function f() { return () => arguments[0] }",
        "function f(a = arguments[0]) { return a }",
    ] {
        assert!(!handed(other), "{other}");
    }
    assert!(facts("export function C(props: P, a = arguments[0]) { return <i /> }", "C").handed_on);
}

/// Every syntactic context a props member can sit in: a read is complete only where the context is known to only
/// read; every other form hands the object on (unknown), never a silent "read".
#[test]
fn a_member_in_each_syntax_context_is_a_read_only_where_known_and_never_silently_complete() {
    let reads_only = [
        "use(props.a)",
        "const v = props.a",
        "x = props.a",
        "[props.a]",
        "({ k: props.a })",
        "`${props.a}`",
        "props.a ? 1 : 2",
        "props.a && 1",
        "props.a + 1",
        "typeof props.a",
        "await props.a",
        "!props.a",
        "new F(props.a)",
        "(props.a as any)",
        "props.a!",
        "(props.a satisfies any)",
        "props?.a",
        "props?.['a']",
        "props['a']",
        "if (props.a) {}",
        "for (;props.a;) {}",
        "const { x = props.a } = o",
        "const { [props.a]: x } = o",
        "<i {...props.a} />",
        "<i a={props.a} />",
    ];
    for context in reads_only {
        let source = format!("export async function C(props: P) {{ let o: any, x: any; {context}; return <i /> }}");
        let body = facts(&source, "C");
        assert!(!body.handed_on && reads(&body).first() == Some(&"a"), "{context}: {body:?}");
    }
    let hands_on = [
        "props.f()",
        "props?.f()",
        "props.f?.()",
        "(props.f)()",
        "(props?.f)()",
        "props.a = 1",
        "props.a += 1",
        "props.a++",
        "delete props.a",
        "delete props?.a",
        "for (props.a of o) {}",
        "for (props.a in o) {}",
        "[props.a] = o",
        "({ k: props.a } = o)",
        "({ ...props.a } = o)",
        "props.f`t`",
        "props[k]",
        "props?.[k]",
        "use(props)",
        "const p = props",
        "const [p] = props",
        "({ ...props })",
        "[...props]",
        // Contexts not known to only read: unknown, even though they do only read.
        "@props.a class X {}",
        "enum E { A = props.a }",
    ];
    for context in hands_on {
        let source = format!("export function C(props: P) {{ let o: any, k: any; {context}; return <i /> }}");
        let body = facts(&source, "C");
        assert!(body.handed_on, "{context}: {body:?}");
    }
}

#[test]
fn an_arguments_use_in_another_function_is_not_this_ones() {
    let other = "function other() { return arguments[0] }\nexport function C(props: P) { return <i /> }";
    assert!(!facts(other, "C").handed_on);
}

/// A props binding that is ever written holds no known value where it is read: the body is unknown for it.
#[test]
fn a_written_props_binding_hands_the_object_on_and_records_nothing_through_it() {
    for body in [
        "props = other; return <Child value={props.value} />",
        "props += x; return <Child value={props.value} />",
        "props++; return <Child value={props.value} />",
        "for (props of xs) {} return <Child value={props.value} />",
        "props = { ...props }; return <Child value={props.value} />",
        "({ props } = obj); return <Child value={props.value} />",
        "[props] = xs; return <Child value={props.value} />",
        "var props = other; return <Child value={props.value} />",
        "delete props.a; return <Child />",
        "x = props; return <Child />",
        "[a] = props; return <Child />",
    ] {
        let source = format!("export function C(props: P) {{ let a, x; {body} }}");
        let facts = facts(&source, "C");
        assert!(facts.handed_on, "{body}");
        assert!(!facts.reads.iter().any(|read| read == "value"), "{body}");
        for attribute in facts.elements.iter().flat_map(|element| &element.attributes) {
            assert!(!matches!(attribute, JsxAttributeFact::Named { prop: Some(_), .. }), "{body}");
        }
    }
}

#[test]
fn a_written_rest_binding_is_not_the_props_object() {
    let facts = facts("export function C({ a, ...r }: P) { r = other; return <Child {...r} v={r.v} /> }", "C");
    assert!(facts.handed_on);
    for attribute in facts.elements.iter().flat_map(|element| &element.attributes) {
        assert!(!matches!(
            attribute,
            JsxAttributeFact::Spread(SpreadOf::Props | SpreadOf::Rest) | JsxAttributeFact::Named { prop: Some(_), .. }
        ));
    }
}

#[test]
fn a_destructuring_assignment_reads_its_keys_only_when_its_value_is_discarded() {
    let discarded = facts("export function C(props: P) { let a; ({ a } = props); return <i /> }", "C");
    assert!(!discarded.handed_on);
    assert_eq!(reads(&discarded), ["a"]);
    for source in [
        "let a; use(({ a } = props)); return <i />",
        "let a; const q = ({ a } = props); return <i />",
        "let a; return <i x={({ a } = props)} />;",
    ] {
        let facts = facts(&format!("export function C(props: P) {{ {source} }}"), "C");
        assert!(facts.handed_on, "{source}");
    }
}

#[test]
fn a_default_parameter_and_a_declaration_destructuring_are_not_writes() {
    let facts =
        facts("export function C(props: P = defaults) { const { a } = props; return <i title={props.b} /> }", "C");
    assert!(!facts.handed_on);
    assert_eq!(reads(&facts), ["a", "b"]);
}
