//! What a component's own function body does with its props, read from the syntax alone (trail 405): which props it
//! reads, the JSX elements it writes with their attributes in source order, and whether it hands its props object on
//! whole. Facts only: what they mean for which props are relevant is the resolver's to decide, and nothing here is
//! emitted.
//!
//! The function is the one a mapping's declaration holds, through the wrapper calls the extractor unwraps (`memo`,
//! `forwardRef`, a factory's first argument). Elements built with `createElement` are not recorded.
//!
//! # Conservative by construction
//!
//! Names are resolved by scope: a parameter, a `let`/`const`/`class`/`function` declaration or a `catch` binding of
//! the same name in an inner scope is another binding, and its uses are not the component's. Every use of the props
//! object (the first parameter, its rest binding, and a rest binding destructured from either) is classified by the
//! node it sits in, and only three are safe:
//!
//! - the object of a member read with a static or string-literal key (`props.x`, `props?.['x']`): `x` is read;
//! - the initializer of an object destructuring (`const { a, b = 1, ...r } = props`) or the right side of a
//!   destructuring assignment: its keys are read, and a rest binding is the props object again;
//! - a JSX spread attribute (`{...props}`): recorded as a spread.
//!
//! Any other use (a call argument, `new`, an attribute value, an object or array literal, a `return`, a computed key,
//! an assignment, `typeof`, a type query) hands the object on, so `reads` is complete only when `handed_on` is false.
//! A `var` is not taken to shadow, and a name the resolver cannot place counts as the component's: an error here
//! only ever adds reads or hand-offs.
//!
//! A function or class the body declares and renders as a JSX tag (`const Item = () => <li/>; return <Item/>`) is a
//! component of its own: its uses of the props object still count, but its elements are the file's
//! ([`JsxUsage`]), not the parent's.

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
/// those outside every function a mapping is given facts of (pages, stories, helpers, module-level JSX, class
/// components) and those of components nested in one. A mapping with no function of its own (a class, an alias of
/// another component, a declaration without a body) gets none.
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
/// elements no component's facts hold.
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
        if let std::collections::hash_map::Entry::Vacant(entry) = self.found.entry(key) {
            let (facts, nested) = facts_of(&function);
            entry.insert(facts);
            self.usages.extend(nested);
        }
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
            self.usages.push(usage_of(it));
        }
        walk::walk_jsx_opening_element(self, it);
    }
}

fn usage_of(it: &JSXOpeningElement<'_>) -> JsxUsage {
    let attributes = it
        .attributes
        .iter()
        .map(|attribute| match attribute {
            JSXAttributeItem::Attribute(attribute) => JsxUsageAttribute::Named(attribute_name(&attribute.name)),
            JSXAttributeItem::SpreadAttribute(_) => JsxUsageAttribute::Spread,
        })
        .collect();
    JsxUsage { tag: tag_of(&it.name), attributes }
}

/// The facts of a component function, and the elements of the components nested in it.
fn facts_of(function: &ComponentFn<'_, '_>) -> (BodyFacts, Vec<JsxUsage>) {
    let mut reader = BodyReader::default();
    let mut tags = TagNames::default();
    match function {
        ComponentFn::Function(function) => {
            reader.enter_params(&function.params, true);
            if let Some(body) = &function.body {
                tags.visit_function_body(body);
                reader.nested_components = tags.names;
                reader.declare_statements(&body.statements);
                reader.visit_function_body(body);
            }
        }
        ComponentFn::Arrow(arrow) => {
            reader.enter_params(&arrow.params, true);
            tags.visit_arrow_function_body(&arrow.body);
            reader.nested_components = tags.names;
            reader.declare_statements(arrow_statements(arrow));
            reader.visit_arrow_function_body(&arrow.body);
        }
    }
    (reader.facts, reader.nested_usages)
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

/// What a name in scope is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Bound {
    /// The props object: the first parameter.
    Props,
    /// The props object less some keys: a rest binding destructured from it.
    Rest,
    /// The value of one prop: a name destructured from the props object under this key.
    Prop(String),
    /// Anything else: a binding that shadows an outer one of the same name.
    Other,
}

#[derive(Default)]
struct BodyReader<'a> {
    facts: BodyFacts,
    /// The scopes from the component function's inward, each with the names it binds.
    scopes: Vec<FxHashMap<&'a str, Bound>>,
    /// The functions and classes the body declares and renders as a tag: components of their own.
    nested_components: FxHashSet<&'a str>,
    /// How many nested components enclose the current node.
    nested: u32,
    /// The elements of the nested components.
    nested_usages: Vec<JsxUsage>,
}

/// The names a binding pattern binds.
fn bound_names<'a>(pattern: &BindingPattern<'a>, out: &mut Vec<&'a str>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.push(id.name.as_str()),
        BindingPattern::AssignmentPattern(assignment) => bound_names(&assignment.left, out),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                bound_names(&property.value, out);
            }
            if let Some(rest) = &object.rest {
                bound_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                bound_names(element, out);
            }
            if let Some(rest) = &array.rest {
                bound_names(&rest.argument, out);
            }
        }
    }
}

impl<'a> BodyReader<'a> {
    /// Binds `name` in the innermost scope; the component function's scope is always open while its body is read.
    fn bind(&mut self, name: &'a str, bound: Bound) {
        if self.scopes.is_empty() {
            self.scopes.push(FxHashMap::default());
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, bound);
        }
    }

    fn shadow(&mut self, pattern: &BindingPattern<'a>) {
        let mut names = Vec::new();
        bound_names(pattern, &mut names);
        for name in names {
            self.bind(name, Bound::Other);
        }
    }

    /// Opens a function's scope with its parameters. The component's own first parameter is the props object.
    fn enter_params(&mut self, params: &FormalParameters<'a>, component: bool) {
        self.scopes.push(FxHashMap::default());
        for (index, param) in params.items.iter().enumerate() {
            if component && index == 0 {
                self.bind_props_param(&param.pattern);
            } else {
                self.shadow(&param.pattern);
            }
        }
        if let Some(rest) = &params.rest {
            self.shadow(&rest.rest.argument);
        }
    }

    fn bind_props_param(&mut self, pattern: &BindingPattern<'a>) {
        match pattern {
            BindingPattern::BindingIdentifier(id) => self.bind(id.name.as_str(), Bound::Props),
            // `function C({ a } = {})`.
            BindingPattern::AssignmentPattern(assignment) => self.bind_props_param(&assignment.left),
            BindingPattern::ObjectPattern(object) => self.destructure(object),
            // An array pattern reads indices of the props object, which says nothing about its keys.
            BindingPattern::ArrayPattern(_) => {
                self.facts.handed_on = true;
                self.shadow(pattern);
            }
        }
    }

    /// An object pattern destructured from the props object: each key is read, a name bound to a key is that prop's
    /// value, and a rest binding is the props object again. Default values are walked by the caller's visit.
    fn destructure(&mut self, object: &ObjectPattern<'a>) {
        for property in &object.properties {
            let Some(key) = property.key.static_name().map(|key| js_text::property_name(&key)) else {
                // `{ [k]: v }` reads a key the syntax does not name.
                self.facts.handed_on = true;
                self.shadow(&property.value);
                continue;
            };
            let local = match &property.value {
                BindingPattern::AssignmentPattern(assignment) => &assignment.left,
                value => value,
            };
            match local {
                BindingPattern::BindingIdentifier(id) => self.bind(id.name.as_str(), Bound::Prop(key.clone())),
                nested => self.shadow(nested),
            }
            self.read(key);
        }
        if let Some(rest) = &object.rest {
            match &rest.argument {
                BindingPattern::BindingIdentifier(id) => {
                    self.bind(id.name.as_str(), Bound::Rest);
                    if self.facts.rest.is_none() {
                        self.facts.rest = Some(id.name.to_string());
                    }
                }
                other => self.shadow(other),
            }
        }
    }

    /// Binds, as shadows, what the statements of a scope declare: `let`, `const`, `using`, `class` and `function`.
    /// A `var` belongs to the function and can redeclare a parameter, so it is not taken to shadow.
    fn declare_statements(&mut self, statements: &[Statement<'a>]) {
        for statement in statements {
            match statement {
                Statement::VariableDeclaration(declaration) if declaration.kind != VariableDeclarationKind::Var => {
                    for declarator in &declaration.declarations {
                        self.shadow(&declarator.id);
                    }
                }
                Statement::FunctionDeclaration(function) => {
                    if let Some(id) = &function.id {
                        self.bind(id.name.as_str(), Bound::Other);
                    }
                }
                Statement::ClassDeclaration(class) => {
                    if let Some(id) = &class.id {
                        self.bind(id.name.as_str(), Bound::Other);
                    }
                }
                _ => {}
            }
        }
    }

    fn scoped(&mut self, statements: &[Statement<'a>], walk: impl FnOnce(&mut Self)) {
        self.scopes.push(FxHashMap::default());
        self.declare_statements(statements);
        walk(self);
        self.scopes.pop();
    }

    fn lookup(&self, name: &str) -> Option<&Bound> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// What the props object an identifier is: `Props` or `Rest`, or `None` for anything else.
    fn whole(&self, name: &str) -> Option<SpreadOf> {
        match self.lookup(name) {
            Some(Bound::Props) => Some(SpreadOf::Props),
            Some(Bound::Rest) => Some(SpreadOf::Rest),
            _ => None,
        }
    }

    fn whole_expr(&self, expr: &Expression<'_>) -> Option<SpreadOf> {
        match expr {
            Expression::Identifier(id) => self.whole(id.name.as_str()),
            _ => None,
        }
    }

    fn read(&mut self, prop: String) {
        if !self.facts.reads.contains(&prop) {
            self.facts.reads.push(prop);
        }
    }

    /// The prop a member read names, when its object is the props object: `props.x`, `props?.['x']`, `rest.x`.
    fn member_prop(&self, expr: &Expression<'_>) -> Option<String> {
        match expr {
            Expression::StaticMemberExpression(member) if self.whole_expr(&member.object).is_some() => {
                Some(member.property.name.to_string())
            }
            Expression::ComputedMemberExpression(member) if self.whole_expr(&member.object).is_some() => {
                match &member.expression {
                    Expression::StringLiteral(key) => Some(js_text::lossy(key.value).into_owned()),
                    _ => None,
                }
            }
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::StaticMemberExpression(member) if self.whole_expr(&member.object).is_some() => {
                    Some(member.property.name.to_string())
                }
                ChainElement::ComputedMemberExpression(member) if self.whole_expr(&member.object).is_some() => {
                    match &member.expression {
                        Expression::StringLiteral(key) => Some(js_text::lossy(key.value).into_owned()),
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The prop an attribute's value is, when it is exactly one: a destructured name, `props.x` or `rest.x`.
    fn value_prop(&self, value: Option<&JSXAttributeValue<'_>>) -> Option<String> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else { return None };
        let expr = container.expression.as_expression()?;
        match expr {
            Expression::Identifier(id) => match self.lookup(id.name.as_str()) {
                Some(Bound::Prop(key)) => Some(key.clone()),
                _ => None,
            },
            _ => self.member_prop(expr),
        }
    }

    fn in_nested(&mut self, nested: bool, walk: impl FnOnce(&mut Self)) {
        self.nested += u32::from(nested);
        walk(self);
        self.nested -= u32::from(nested);
    }

    fn is_nested_component(&self, id: Option<&BindingIdentifier<'_>>) -> bool {
        id.is_some_and(|id| self.nested_components.contains(id.name.as_str()))
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
    /// The fallback: a use of the props object that no safe form below took hands it on.
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if self.whole(it.name.as_str()).is_some() {
            self.facts.handed_on = true;
        }
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if self.whole_expr(&it.object).is_some() {
            self.read(it.property.name.to_string());
            return;
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        if self.whole_expr(&it.object).is_some()
            && let Expression::StringLiteral(key) = &it.expression
        {
            self.read(js_text::lossy(key.value).into_owned());
            return;
        }
        walk::walk_computed_member_expression(self, it);
    }

    fn visit_jsx_member_expression(&mut self, it: &JSXMemberExpression<'a>) {
        if let JSXMemberExpressionObject::IdentifierReference(object) = &it.object
            && self.whole(object.name.as_str()).is_some()
        {
            self.read(it.property.name.to_string());
            return;
        }
        walk::walk_jsx_member_expression(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let (BindingPattern::ObjectPattern(object), Some(init)) = (&it.id, &it.init)
            && self.whole_expr(init).is_some()
        {
            self.destructure(object);
            // Default values and computed keys, not the initializer: it is the props object, taken apart.
            self.visit_binding_pattern(&it.id);
            return;
        }
        let nested = matches!(&it.id, BindingPattern::BindingIdentifier(id) if self.is_nested_component(Some(id)))
            && it.init.as_ref().is_some_and(|init| {
                unwrap(init).is_some() || matches!(peel_expr(init), Expression::ClassExpression(_))
            });
        self.in_nested(nested, |reader| walk::walk_variable_declarator(reader, it));
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if let AssignmentTarget::ObjectAssignmentTarget(object) = &it.left
            && self.whole_expr(&it.right).is_some()
        {
            for property in &object.properties {
                match property {
                    AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(property) => {
                        self.read(property.binding.name.to_string());
                    }
                    AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                        match property.name.static_name() {
                            Some(key) => self.read(js_text::property_name(&key)),
                            None => self.facts.handed_on = true,
                        }
                    }
                }
            }
            // The rest target is an existing binding the props object is copied into: not followed.
            if object.rest.is_some() {
                self.facts.handed_on = true;
            }
            self.visit_assignment_target(&it.left);
            return;
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let nested = it.is_declaration() && self.is_nested_component(it.id.as_ref());
        self.in_nested(nested, |reader| {
            reader.enter_params(&it.params, false);
            if let Some(body) = &it.body {
                reader.declare_statements(&body.statements);
            }
            walk::walk_function(reader, it, flags);
            reader.scopes.pop();
        });
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.enter_params(&it.params, false);
        self.declare_statements(arrow_statements(it));
        walk::walk_arrow_function_expression(self, it);
        self.scopes.pop();
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let nested = it.is_declaration() && self.is_nested_component(it.id.as_ref());
        self.in_nested(nested, |reader| walk::walk_class(reader, it));
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.scoped(&it.body, |reader| walk::walk_block_statement(reader, it));
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.scoped(&it.body, |reader| walk::walk_static_block(reader, it));
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.scopes.push(FxHashMap::default());
        for case in &it.cases {
            self.declare_statements(&case.consequent);
        }
        walk::walk_switch_statement(self, it);
        self.scopes.pop();
    }

    fn visit_catch_clause(&mut self, it: &CatchClause<'a>) {
        self.scopes.push(FxHashMap::default());
        if let Some(param) = &it.param {
            self.shadow(&param.pattern);
        }
        walk::walk_catch_clause(self, it);
        self.scopes.pop();
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.scopes.push(FxHashMap::default());
        if let Some(ForStatementInit::VariableDeclaration(declaration)) = &it.init
            && declaration.kind != VariableDeclarationKind::Var
        {
            for declarator in &declaration.declarations {
                self.shadow(&declarator.id);
            }
        }
        walk::walk_for_statement(self, it);
        self.scopes.pop();
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.scopes.push(FxHashMap::default());
        if let ForStatementLeft::VariableDeclaration(declaration) = &it.left
            && declaration.kind != VariableDeclarationKind::Var
        {
            for declarator in &declaration.declarations {
                self.shadow(&declarator.id);
            }
        }
        walk::walk_for_of_statement(self, it);
        self.scopes.pop();
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.scopes.push(FxHashMap::default());
        if let ForStatementLeft::VariableDeclaration(declaration) = &it.left
            && declaration.kind != VariableDeclarationKind::Var
        {
            for declarator in &declaration.declarations {
                self.shadow(&declarator.id);
            }
        }
        walk::walk_for_in_statement(self, it);
        self.scopes.pop();
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        if self.nested > 0 {
            self.nested_usages.push(usage_of(it));
        } else {
            let attributes = it
                .attributes
                .iter()
                .map(|attribute| match attribute {
                    JSXAttributeItem::Attribute(attribute) => JsxAttributeFact::Named {
                        name: attribute_name(&attribute.name),
                        prop: self.value_prop(attribute.value.as_ref()),
                    },
                    JSXAttributeItem::SpreadAttribute(spread) => {
                        JsxAttributeFact::Spread(self.whole_expr(&spread.argument).unwrap_or(SpreadOf::Other))
                    }
                })
                .collect();
            self.facts.elements.push(JsxElementFact { tag: tag_of(&it.name), attributes });
        }
        walk::walk_jsx_opening_element(self, it);
    }

    fn visit_jsx_spread_attribute(&mut self, it: &JSXSpreadAttribute<'a>) {
        match self.whole_expr(&it.argument) {
            // Spreading the whole props object passes every prop on.
            Some(SpreadOf::Props) => self.facts.handed_on = true,
            Some(SpreadOf::Rest | SpreadOf::Other) => {}
            None => walk::walk_jsx_spread_attribute(self, it),
        }
    }
}

/// The statements of an arrow's block body; none for an expression body.
fn arrow_statements<'b, 'a>(arrow: &'b ArrowFunctionExpression<'a>) -> &'b [Statement<'a>] {
    arrow.body.as_function_body().map_or(&[], |body| &body.statements)
}

#[cfg(test)]
mod tests;
