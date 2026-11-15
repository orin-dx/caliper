//! What a component's own function body does with its props, read from the syntax alone (trail 405): which props it
//! reads, the JSX elements it writes with their attributes in source order, and whether it hands its props object on
//! whole. Facts only: what they mean for which props are relevant is the resolver's to decide, and nothing here is
//! emitted.
//!
//! The function is the one a mapping's declaration holds, through the wrapper calls the extractor unwraps (`memo`,
//! `forwardRef`, a factory's first argument). A component the body declares and renders as a JSX tag
//! (`const Item = () => <li/>; return <Item/>`) is a component of its own: its body is not read. Elements built with
//! `createElement` are not recorded.

use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::scope::ScopeFlags;
use rustc_hash::{FxHashMap, FxHashSet};

use super::component::peel_expr;
use super::js_text;
use crate::types::collected::{
    BodyFacts, ComponentMapping, JsxAttributeFact, JsxElementFact, JsxTag, JsxUsage, JsxUsageAttribute, SpreadOf,
};

/// Records on each mapping the facts of the function its declaration holds, and returns the file's other JSX elements:
/// those outside every function a mapping is given facts of (pages, stories, helpers, module-level JSX). A mapping with
/// no function of its own (a class, an alias of another component, a declaration without a body) gets none.
pub(super) fn attach(program: &Program<'_>, mappings: &mut [ComponentMapping], jsx: bool) -> Vec<JsxUsage> {
    if mappings.is_empty() && !jsx {
        return Vec::new();
    }
    let mut finder = Finder {
        wanted: mappings.iter().map(|mapping| (mapping.span_start, mapping.span_end)).collect(),
        found: FxHashMap::default(),
        owned: 0,
        usages: Vec::new(),
    };
    finder.visit_program(program);
    for mapping in mappings {
        mapping.body = finder.found.get(&(mapping.span_start, mapping.span_end)).cloned();
    }
    finder.usages
}

/// A component function, by the parts the facts are read from.
enum ComponentFn<'b, 'a> {
    Function(&'b Function<'a>),
    Arrow(&'b ArrowFunctionExpression<'a>),
}

/// The function an expression is, through parentheses, type assertions and the first argument of wrapper calls
/// (`memo(forwardRef((props, ref) => …))`).
fn unwrap<'b, 'a>(expr: &'b Expression<'a>) -> Option<ComponentFn<'b, 'a>> {
    match peel_expr(expr) {
        Expression::FunctionExpression(function) => Some(ComponentFn::Function(function)),
        Expression::ArrowFunctionExpression(arrow) => Some(ComponentFn::Arrow(arrow)),
        Expression::CallExpression(call) => unwrap(call.arguments.first()?.as_expression()?),
        _ => None,
    }
}

/// Finds the nodes that start and end where a mapping does, reads the function each holds, and collects the JSX
/// elements outside all of them.
struct Finder {
    wanted: FxHashSet<(u32, u32)>,
    found: FxHashMap<(u32, u32), BodyFacts>,
    /// How many nodes whose function was read enclose the current one.
    owned: u32,
    usages: Vec<JsxUsage>,
}

impl Finder {
    /// Reads the function a wanted node holds; `true` when the node is one, so its JSX is the component's.
    fn record(&mut self, span: oxc_span::Span, function: Option<ComponentFn<'_, '_>>) -> bool {
        let key = (span.start, span.end);
        let Some(function) = function.filter(|_| self.wanted.contains(&key)) else { return false };
        self.found.entry(key).or_insert_with(|| facts_of(&function));
        true
    }

    fn walk_owned(&mut self, owned: bool, walk: impl FnOnce(&mut Self)) {
        self.owned += u32::from(owned);
        walk(self);
        self.owned -= u32::from(owned);
    }
}

impl<'a> Visit<'a> for Finder {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        let owned = self.record(it.span, it.init.as_ref().and_then(unwrap));
        self.walk_owned(owned, |finder| walk::walk_variable_declarator(finder, it));
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        let owned = self.record(it.span, it.declaration.as_expression().and_then(unwrap));
        self.walk_owned(owned, |finder| walk::walk_export_default_declaration(finder, it));
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let owned = self.record(it.span, Some(ComponentFn::Function(it)));
        self.walk_owned(owned, |finder| walk::walk_function(finder, it, flags));
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let owned = self.record(it.span, Some(ComponentFn::Arrow(it)));
        self.walk_owned(owned, |finder| walk::walk_arrow_function_expression(finder, it));
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        if self.owned == 0 {
            let attributes = it
                .attributes
                .iter()
                .map(|attribute| match attribute {
                    JSXAttributeItem::Attribute(attribute) => JsxUsageAttribute::Named(attribute_name(&attribute.name)),
                    JSXAttributeItem::SpreadAttribute(_) => JsxUsageAttribute::Spread,
                })
                .collect();
            self.usages.push(JsxUsage { tag: tag_of(&it.name), attributes });
        }
        walk::walk_jsx_opening_element(self, it);
    }
}

fn facts_of(function: &ComponentFn<'_, '_>) -> BodyFacts {
    let params = match function {
        ComponentFn::Function(function) => &*function.params,
        ComponentFn::Arrow(arrow) => &*arrow.params,
    };
    let mut reader = BodyReader::default();
    if let Some(first) = params.items.first() {
        reader.read_param(&first.pattern);
    }
    let mut tags = TagNames::default();
    match function {
        ComponentFn::Function(function) => {
            if let Some(body) = &function.body {
                tags.visit_function_body(body);
                reader.nested_components = tags.names;
                reader.visit_function_body(body);
            }
        }
        ComponentFn::Arrow(arrow) => {
            tags.visit_arrow_function_body(&arrow.body);
            reader.nested_components = tags.names;
            reader.visit_arrow_function_body(&arrow.body);
        }
    }
    reader.facts
}

/// The names a body uses as a component tag (`<Item/>`): a function or class the body declares under one of them is a
/// component of its own.
#[derive(Default)]
struct TagNames<'a> {
    names: FxHashSet<&'a str>,
}

impl<'a> Visit<'a> for TagNames<'a> {
    fn visit_jsx_element_name(&mut self, it: &JSXElementName<'a>) {
        if let JSXElementName::IdentifierReference(id) = it {
            self.names.insert(id.name.as_str());
        }
    }
}

#[derive(Default)]
struct BodyReader<'a> {
    facts: BodyFacts,
    /// The first parameter, when it is a plain identifier (`props`).
    props: Option<&'a str>,
    /// The rest binding of a destructured first parameter (`...rest`).
    rest: Option<&'a str>,
    /// Each local name a destructured first parameter binds, to the prop it binds (`{ tone: t }` binds `t` to `tone`).
    locals: FxHashMap<&'a str, String>,
    /// The functions and classes the body declares and renders as a tag: their bodies are not this component's.
    nested_components: FxHashSet<&'a str>,
}

impl<'a> BodyReader<'a> {
    fn read_param(&mut self, pattern: &BindingPattern<'a>) {
        match pattern {
            BindingPattern::BindingIdentifier(id) => self.props = Some(id.name.as_str()),
            // `function C({ a } = {})`.
            BindingPattern::AssignmentPattern(assignment) => self.read_param(&assignment.left),
            BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    let Some(key) = property.key.static_name() else { continue };
                    let prop = js_text::property_name(&key);
                    let local = match &property.value {
                        BindingPattern::BindingIdentifier(id) => Some(id),
                        BindingPattern::AssignmentPattern(assignment) => match &assignment.left {
                            BindingPattern::BindingIdentifier(id) => Some(id),
                            _ => None,
                        },
                        BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_) => None,
                    };
                    if let Some(local) = local {
                        self.locals.insert(local.name.as_str(), prop.clone());
                    }
                    self.read(prop);
                }
                if let Some(rest) = &object.rest
                    && let BindingPattern::BindingIdentifier(id) = &rest.argument
                {
                    self.rest = Some(id.name.as_str());
                    self.facts.rest = Some(id.name.to_string());
                }
            }
            BindingPattern::ArrayPattern(_) => {}
        }
    }

    fn read(&mut self, prop: String) {
        if !self.facts.reads.contains(&prop) {
            self.facts.reads.push(prop);
        }
    }

    fn is_props(&self, name: &str) -> bool {
        self.props == Some(name)
    }

    fn is_props_or_rest(&self, name: &str) -> bool {
        self.props == Some(name) || self.rest == Some(name)
    }

    /// The prop a member read of `object` names: `props.x`, `props['x']`, or the same on the rest binding when
    /// `rest_too`.
    fn member_prop(&self, expr: &Expression<'_>, rest_too: bool) -> Option<String> {
        let (object, name) = match expr {
            Expression::StaticMemberExpression(member) => (&member.object, member.property.name.to_string()),
            Expression::ComputedMemberExpression(member) => match &member.expression {
                Expression::StringLiteral(key) => (&member.object, js_text::lossy(key.value).into_owned()),
                _ => return None,
            },
            _ => return None,
        };
        let Expression::Identifier(object) = object else { return None };
        let name_matches =
            if rest_too { self.is_props_or_rest(object.name.as_str()) } else { self.is_props(object.name.as_str()) };
        name_matches.then_some(name)
    }

    /// The prop an attribute's value is, when it is exactly one: a destructured name, `props.x` or `rest.x`.
    fn value_prop(&self, value: Option<&JSXAttributeValue<'_>>) -> Option<String> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else { return None };
        let expr = container.expression.as_expression()?;
        match expr {
            Expression::Identifier(id) => self.locals.get(id.name.as_str()).cloned(),
            _ => self.member_prop(expr, true),
        }
    }

    fn spread_of(&self, argument: &Expression<'_>) -> SpreadOf {
        match argument {
            Expression::Identifier(id) if self.is_props(id.name.as_str()) => SpreadOf::Props,
            Expression::Identifier(id) if self.rest == Some(id.name.as_str()) => SpreadOf::Rest,
            _ => SpreadOf::Other,
        }
    }
}

/// A tag as the JSX rule reads it: a lowercase first letter, a dash or a namespace makes it intrinsic.
fn tag_of(name: &JSXElementName<'_>) -> JsxTag {
    let identifier = |name: &str| {
        if name.starts_with(|c: char| c.is_ascii_lowercase()) || name.contains('-') {
            JsxTag::Intrinsic(name.to_owned())
        } else {
            JsxTag::Identifier(name.to_owned())
        }
    };
    match name {
        JSXElementName::Identifier(id) => identifier(id.name.as_str()),
        JSXElementName::IdentifierReference(id) => identifier(id.name.as_str()),
        JSXElementName::NamespacedName(namespaced) => {
            JsxTag::Intrinsic(format!("{}:{}", namespaced.namespace.name, namespaced.name.name))
        }
        JSXElementName::MemberExpression(member) => JsxTag::Member(member_path(member)),
        JSXElementName::ThisExpression(_) => JsxTag::Identifier("this".to_owned()),
    }
}

fn member_path(member: &JSXMemberExpression<'_>) -> String {
    let object = match &member.object {
        JSXMemberExpressionObject::IdentifierReference(id) => id.name.to_string(),
        JSXMemberExpressionObject::MemberExpression(inner) => member_path(inner),
        JSXMemberExpressionObject::ThisExpression(_) => "this".to_owned(),
    };
    format!("{object}.{}", member.property.name)
}

fn attribute_name(name: &JSXAttributeName<'_>) -> String {
    match name {
        JSXAttributeName::Identifier(id) => id.name.to_string(),
        JSXAttributeName::NamespacedName(namespaced) => {
            format!("{}:{}", namespaced.namespace.name, namespaced.name.name)
        }
    }
}

impl<'a> Visit<'a> for BodyReader<'a> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &it.id
            && self.nested_components.contains(id.name.as_str())
        {
            return;
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if it.is_declaration() && it.id.as_ref().is_some_and(|id| self.nested_components.contains(id.name.as_str())) {
            return;
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if it.is_declaration() && it.id.as_ref().is_some_and(|id| self.nested_components.contains(id.name.as_str())) {
            return;
        }
        walk::walk_class(self, it);
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        let attributes = it
            .attributes
            .iter()
            .map(|attribute| match attribute {
                JSXAttributeItem::Attribute(attribute) => JsxAttributeFact::Named {
                    name: attribute_name(&attribute.name),
                    prop: self.value_prop(attribute.value.as_ref()),
                },
                JSXAttributeItem::SpreadAttribute(spread) => {
                    let of = self.spread_of(&spread.argument);
                    if of == SpreadOf::Props {
                        self.facts.handed_on = true;
                    }
                    JsxAttributeFact::Spread(of)
                }
            })
            .collect();
        self.facts.elements.push(JsxElementFact { tag: tag_of(&it.name), attributes });
        walk::walk_jsx_opening_element(self, it);
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if let Expression::Identifier(object) = &it.object
            && self.is_props(object.name.as_str())
        {
            self.read(it.property.name.to_string());
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        if let (Expression::Identifier(object), Expression::StringLiteral(key)) = (&it.object, &it.expression)
            && self.is_props(object.name.as_str())
        {
            self.read(js_text::lossy(key.value).into_owned());
        }
        walk::walk_computed_member_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if it.arguments.iter().any(|argument| {
            matches!(argument.as_expression(), Some(Expression::Identifier(id)) if self.is_props_or_rest(id.name.as_str()))
        }) {
            self.facts.handed_on = true;
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::parse_file;

    fn body_of(source: &str, component: &str) -> Option<BodyFacts> {
        let data = parse_file(camino::Utf8Path::new("/p/c.tsx"), source);
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
            [intrinsic(
                "button",
                vec![named("title", Some("title")), named("aria-label", None), named("onClick", None)]
            )]
        );
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
    fn spreading_the_whole_props_object_or_passing_it_to_a_call_hands_it_on() {
        let spread = facts("export function C(props: P) { return <Inner {...props} /> }", "C");
        assert_eq!(spread.elements, [component("Inner", vec![JsxAttributeFact::Spread(SpreadOf::Props)])]);
        assert!(spread.handed_on);

        let hook = facts("export function C(props: P) { const state = useX(props); return <div /> }", "C");
        assert!(hook.handed_on);

        let rest = facts("export function C({ a, ...rest }: P) { useY(rest); return <div /> }", "C");
        assert!(rest.handed_on);

        let neither = facts("export function C(props: P) { useX(props.a); return <div {...props.b} /> }", "C");
        assert!(!neither.handed_on);
        assert_eq!(neither.elements, [intrinsic("div", vec![JsxAttributeFact::Spread(SpreadOf::Other)])]);
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
    fn a_nested_component_rendered_as_a_tag_keeps_its_elements_and_callbacks_do_not() {
        let facts = facts(
            "export function C({ items }: P) {\n\
               const Row = ({ item }: { item: string }) => <li data-item={item} />;\n\
               function Empty() { return <em /> }\n\
               return items.length ? <ul>{items.map((item) => <Row item={item} />)}</ul> : <Empty />;\n\
             }",
            "C",
        );
        assert_eq!(
            facts.elements,
            [intrinsic("ul", vec![]), component("Row", vec![named("item", None)]), component("Empty", vec![])]
        );
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
                JsxUsage { tag: JsxTag::Intrinsic("svg:g".into()), attributes: vec![] },
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
    fn a_component_with_no_function_of_its_own_has_no_body_facts() {
        let source = "import { Component } from 'react';\n\
             export class K extends Component<P> { render() { return <div /> } }";
        assert_eq!(body_of(source, "K"), None);
    }
}
