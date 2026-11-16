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
//! Names are resolved by `oxc_semantic`, which binds every reference to its symbol: a parameter, `var`, `let`,
//! function or class name, catch or loop binding of the same name is another symbol, and what is the component's is
//! decided by the symbol. Every reference to the props object (the first parameter and each rest binding taken from
//! it) is classified by the node around it, through TypeScript's `as`, `satisfies`, `!` and `<T>`; only three are
//! safe:
//!
//! - the object of a member read with a static or string-literal key (`props.x`, `props?.['x']`) that is not called
//!   (`props.f()` passes the object as `this`), written, deleted or destructured into: `x` is read;
//! - the initializer of an object destructuring (`const { a, b = 1, ...r } = props`) or the right side of a
//!   destructuring assignment: its static keys are read, and a rest binding is the props object again;
//! - a JSX spread attribute (`{...props}`): recorded as a spread, and the whole props object spread is handed on.
//!
//! Any other reference (a call argument, `new`, an attribute value, an object or array literal, a `return`, a
//! computed key, an assignment, `typeof`, a type query) hands the object on, so `reads` is complete only when
//! `handed_on` is false.
//!
//! A function or class the body binds and renders as a JSX tag, by symbol (`const Item = () => <li/>; return
//! <Item/>`), is a component of its own: its references to the props object still count, but its elements are the
//! file's ([`JsxUsage`]), not the parent's.
//!
//! # Owned by node and ordered by position
//!
//! An element belongs to the exact function or class node it is written in, never to the extent of a declaration or
//! wrapper call around it: JSX in a wrapper's other arguments (`withOptions(fn, { fallback: <Icon /> })`) is written
//! where the wrapper is called. Reads and rest bindings are ordered by source position, not by the order references
//! are classified in (symbol by symbol).

use std::collections::VecDeque;

use oxc_ast::AstKind;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::{AstNodes, Scoping, SemanticBuilder};
use oxc_span::GetSpan;
use oxc_syntax::node::NodeId;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::component::peel_expr;
use super::js_text;
use crate::types::collected::{
    BodyFacts, ComponentMapping, JsxAttributeFact, JsxElementFact, JsxTag, JsxUsage, JsxUsageAttribute, SpreadOf,
};

/// Records on each mapping the facts of the function its declaration holds, and returns the file's other JSX elements:
/// those outside every function a mapping is given facts of (pages, stories, helpers, module-level JSX, class
/// components) and those of components nested in one. A mapping with no function of its own (a class, an alias of
/// another component, a declaration without a body) gets none. The semantic pass runs only for a file with at least
/// one such function.
pub(super) fn attach<'a>(program: &'a Program<'a>, mappings: &mut [ComponentMapping], jsx: bool) -> Vec<JsxUsage> {
    if mappings.is_empty() && !jsx {
        return Vec::new();
    }
    // The nodes are what a reference's parent is read from; the scoping tells a file-level tag's binding apart from an
    // inner one of the same name.
    let semantic = SemanticBuilder::new().with_build_nodes(true).build(program).semantic;
    let (scoping, nodes) = (semantic.scoping(), semantic.nodes());
    let mut finder = Finder {
        scoping,
        wanted: mappings.iter().map(|mapping| (mapping.span_start, mapping.span_end)).collect(),
        functions: FxHashMap::default(),
        reached: FxHashMap::default(),
        owned: 0,
        usages: Vec::new(),
    };
    finder.visit_program(program);
    let Finder { functions, reached, mut usages, .. } = finder;
    if functions.is_empty() {
        return usages;
    }
    let mut found = FxHashMap::default();
    // Elements of nested components, each at the place in `usages` its component function was reached.
    let mut nested: Vec<(usize, Vec<JsxUsage>)> = Vec::new();
    for node in nodes.iter() {
        let (span, function) = match node.kind() {
            AstKind::Function(function) => (function.span, ComponentFn::Function(function)),
            AstKind::ArrowFunctionExpression(arrow) => (arrow.span, ComponentFn::Arrow(arrow)),
            _ => continue,
        };
        let Some(holders) = functions.get(&(span.start, span.end)) else { continue };
        let (facts, usages_of_nested) = Reader::new(scoping, nodes).facts_of(&function);
        for &key in holders {
            found.entry(key).or_insert_with(|| facts.clone());
        }
        // Once per function, where the function itself was reached.
        if let Some(&at) = reached.get(&(span.start, span.end)) {
            nested.push((at, usages_of_nested));
        }
    }
    nested.sort_by_key(|(at, _)| *at);
    for (at, inserted) in nested.into_iter().rev() {
        usages.splice(at..at, inserted);
    }
    for mapping in mappings {
        mapping.body = found.get(&(mapping.span_start, mapping.span_end)).cloned();
    }
    usages
}

/// A component function, by the parts the facts are read from.
enum ComponentFn<'b, 'a> {
    Function(&'b Function<'a>),
    Arrow(&'b ArrowFunctionExpression<'a>),
}

impl ComponentFn<'_, '_> {
    fn span(&self) -> oxc_span::Span {
        match self {
            Self::Function(function) => function.span,
            Self::Arrow(arrow) => arrow.span,
        }
    }
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

/// The span of the function or class a nested component's initializer is, through the same wrappers as [`unwrap`]
/// (`observer(class extends Component { … })`).
fn nested_component(expr: &Expression<'_>) -> Option<oxc_span::Span> {
    match peel_expr(expr) {
        Expression::ClassExpression(class) => Some(class.span),
        Expression::CallExpression(call) => nested_component(call.arguments.first()?.as_expression()?),
        _ => unwrap(expr).map(|function| function.span()),
    }
}

/// A span as `(start, end)`.
type SpanKey = (u32, u32);

/// Finds the nodes that start and end where a mapping does and the function each holds, and collects the JSX
/// elements outside all of those functions.
///
/// A mapping owns exactly the function [`Reader::facts_of`] reads, never the declaration or wrapper call around it:
/// JSX in a wrapper's other arguments or callee (`withOptions(fn, { fallback: <Icon /> })`) is the file's.
struct Finder<'s> {
    scoping: &'s Scoping,
    wanted: FxHashSet<SpanKey>,
    /// The span of each component function, to the spans of the mappings that hold it.
    functions: FxHashMap<SpanKey, Vec<SpanKey>>,
    /// Where in the file's usages each component function was reached.
    reached: FxHashMap<SpanKey, usize>,
    /// How many component functions enclose the current node.
    owned: u32,
    usages: Vec<JsxUsage>,
}

impl Finder<'_> {
    /// Notes the function a wanted node holds.
    fn record(&mut self, span: oxc_span::Span, function: Option<ComponentFn<'_, '_>>) {
        let key = (span.start, span.end);
        let Some(function) = function.filter(|_| self.wanted.contains(&key)) else { return };
        let function = function.span();
        let holders = self.functions.entry((function.start, function.end)).or_default();
        if !holders.contains(&key) {
            holders.push(key);
        }
    }

    /// Walks a function, owning its JSX when a mapping holds it. A wrapped function is reached after the declaration
    /// holding it, so it has been recorded by then.
    fn walk_function(&mut self, span: oxc_span::Span, walk: impl FnOnce(&mut Self)) {
        let key = (span.start, span.end);
        let owned = self.functions.contains_key(&key);
        if owned {
            self.reached.entry(key).or_insert(self.usages.len());
        }
        self.owned += u32::from(owned);
        walk(self);
        self.owned -= u32::from(owned);
    }
}

impl<'a> Visit<'a> for Finder<'_> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        self.record(it.span, it.init.as_ref().and_then(unwrap));
        walk::walk_variable_declarator(self, it);
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        self.record(it.span, it.declaration.as_expression().and_then(unwrap));
        walk::walk_export_default_declaration(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.record(it.span, Some(ComponentFn::Function(it)));
        self.walk_function(it.span, |finder| walk::walk_function(finder, it, flags));
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.record(it.span, Some(ComponentFn::Arrow(it)));
        self.walk_function(it.span, |finder| walk::walk_arrow_function_expression(finder, it));
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        if self.owned == 0 {
            self.usages.push(usage_of(it, self.scoping));
        }
        walk::walk_jsx_opening_element(self, it);
    }
}

/// One file-level element: its tag keyed by binding ([`scoped`]), and its attributes.
fn usage_of(it: &JSXOpeningElement<'_>, scoping: &Scoping) -> JsxUsage {
    let attributes = it
        .attributes
        .iter()
        .map(|attribute| match attribute {
            JSXAttributeItem::Attribute(attribute) => JsxUsageAttribute::Named(attribute_name(&attribute.name)),
            JSXAttributeItem::SpreadAttribute(_) => JsxUsageAttribute::Spread,
        })
        .collect();
    JsxUsage { tag: scoped(scoping, tag_of(&it.name), &it.name), attributes }
}

/// Reads one component function against the file's semantic model.
struct Reader<'s, 'a> {
    scoping: &'s Scoping,
    nodes: &'s AstNodes<'a>,
    facts: BodyFacts,
    /// Each read with its source position. References are classified symbol by symbol (a rest binding's after every
    /// one of the props object's), so the order of `facts.reads` is made from the positions once all are classified.
    reads: Vec<(u32, String)>,
    /// Each rest binding with its source position, ordered into `facts.rests` the same way.
    rests: Vec<(u32, String)>,
    /// The props object's symbols: the first parameter, and each rest binding taken from it.
    whole: FxHashMap<SymbolId, SpreadOf>,
    /// The symbols still to classify the references of.
    queue: VecDeque<SymbolId>,
    /// The destructured locals that hold one prop's value unchanged: no default, never written.
    locals: FxHashMap<SymbolId, String>,
}

impl<'s, 'a> Reader<'s, 'a> {
    fn new(scoping: &'s Scoping, nodes: &'s AstNodes<'a>) -> Self {
        Self {
            scoping,
            nodes,
            facts: BodyFacts::default(),
            reads: Vec::new(),
            rests: Vec::new(),
            whole: FxHashMap::default(),
            queue: VecDeque::new(),
            locals: FxHashMap::default(),
        }
    }

    fn facts_of(mut self, function: &ComponentFn<'_, 'a>) -> (BodyFacts, Vec<JsxUsage>) {
        let params = match function {
            ComponentFn::Function(function) => &*function.params,
            ComponentFn::Arrow(arrow) => &*arrow.params,
        };
        if let Some(first) = params.items.first() {
            self.props_param(&first.pattern);
        }
        while let Some(symbol) = self.queue.pop_front() {
            for reference in self.scoping.get_resolved_references(symbol) {
                self.classify(reference.node_id());
            }
        }
        // A key's first read is its earliest in the source.
        self.reads.sort_by_key(|&(at, _)| at);
        for (_, key) in std::mem::take(&mut self.reads) {
            if !self.facts.reads.contains(&key) {
                self.facts.reads.push(key);
            }
        }
        self.rests.sort_by_key(|&(at, _)| at);
        self.facts.rests = std::mem::take(&mut self.rests).into_iter().map(|(_, name)| name).collect();
        let mut elements = Elements {
            reader: &self,
            tags: FxHashSet::default(),
            nested_spans: FxHashSet::default(),
            nested: 0,
            elements: Vec::new(),
            nested_usages: Vec::new(),
            spreads_nested: false,
        };
        // The parameters too: a default or a computed key can hold JSX (`{ icon = <Icon /> }`).
        match function {
            ComponentFn::Function(function) => {
                elements.tags = tag_symbols(self.scoping, |tags| {
                    tags.visit_formal_parameters(&function.params);
                    if let Some(body) = &function.body {
                        tags.visit_function_body(body);
                    }
                });
                elements.visit_formal_parameters(&function.params);
                if let Some(body) = &function.body {
                    elements.visit_function_body(body);
                }
            }
            ComponentFn::Arrow(arrow) => {
                elements.tags = tag_symbols(self.scoping, |tags| {
                    tags.visit_formal_parameters(&arrow.params);
                    tags.visit_arrow_function_body(&arrow.body);
                });
                elements.visit_formal_parameters(&arrow.params);
                elements.visit_arrow_function_body(&arrow.body);
            }
        }
        let (found, nested_usages, spreads_nested) =
            (elements.elements, elements.nested_usages, elements.spreads_nested);
        self.facts.elements = found;
        self.facts.handed_on |= spreads_nested;
        (self.facts, nested_usages)
    }

    fn props_param(&mut self, pattern: &BindingPattern<'_>) {
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                if let Some(symbol) = id.symbol_id.get() {
                    self.track(symbol, SpreadOf::Props);
                }
            }
            // `function C({ a } = {})`.
            BindingPattern::AssignmentPattern(assignment) => self.props_param(&assignment.left),
            BindingPattern::ObjectPattern(object) => self.destructure(object),
            // An array pattern reads indices of the props object, which says nothing about its keys.
            BindingPattern::ArrayPattern(_) => self.facts.handed_on = true,
        }
    }

    fn track(&mut self, symbol: SymbolId, whole: SpreadOf) {
        if self.whole.insert(symbol, whole).is_none() {
            self.queue.push_back(symbol);
        }
    }

    fn read(&mut self, at: oxc_span::Span, prop: String) {
        self.reads.push((at.start, prop));
    }

    /// An object pattern taken from the props object: each static key is read, a plain name bound to a key that is
    /// never written holds that prop, and a rest binding is the props object again.
    fn destructure(&mut self, object: &ObjectPattern<'_>) {
        for property in &object.properties {
            let Some(key) = property.key.static_name().map(|key| js_text::property_name(&key)) else {
                // `{ [k]: v }` reads a key the syntax does not name.
                self.facts.handed_on = true;
                continue;
            };
            if let BindingPattern::BindingIdentifier(id) = &property.value
                && let Some(symbol) = id.symbol_id.get()
                && !self.scoping.symbol_is_mutated(symbol)
                // `var a` or `function a` beside `{ a }` is the same symbol, given another value.
                && self.scoping.symbol_redeclarations(symbol).is_empty()
            {
                self.locals.insert(symbol, key.clone());
            }
            self.read(property.span, key);
        }
        if let Some(rest) = &object.rest {
            match &rest.argument {
                BindingPattern::BindingIdentifier(id) => {
                    self.rests.push((rest.span.start, id.name.to_string()));
                    if let Some(symbol) = id.symbol_id.get() {
                        self.track(symbol, SpreadOf::Rest);
                    }
                }
                _ => self.facts.handed_on = true,
            }
        }
    }

    /// The nearest ancestor of `node` that is not a TypeScript assertion or parentheses, and the span of the child
    /// of it the walk came from.
    fn through_wrappers(&self, node: NodeId) -> (NodeId, oxc_span::Span) {
        let (mut child, mut parent) = (self.nodes.kind(node).span(), self.nodes.parent_id(node));
        while matches!(
            self.nodes.kind(parent),
            AstKind::TSAsExpression(_)
                | AstKind::TSSatisfiesExpression(_)
                | AstKind::TSNonNullExpression(_)
                | AstKind::TSTypeAssertion(_)
                | AstKind::TSInstantiationExpression(_)
                | AstKind::ParenthesizedExpression(_)
        ) {
            child = self.nodes.kind(parent).span();
            parent = self.nodes.parent_id(parent);
        }
        (parent, child)
    }

    /// Classifies one reference to the props object; any use not named here hands it on.
    fn classify(&mut self, reference: NodeId) {
        let (parent, child) = self.through_wrappers(reference);
        let read = match self.nodes.kind(parent) {
            AstKind::StaticMemberExpression(member) if member.object.span() == child => {
                Some(member.property.name.to_string())
            }
            AstKind::ComputedMemberExpression(member) if member.object.span() == child => match &member.expression {
                Expression::StringLiteral(key) => Some(js_text::lossy(key.value).into_owned()),
                _ => None,
            },
            AstKind::JSXMemberExpression(member) => Some(member.property.name.to_string()),
            AstKind::VariableDeclarator(declarator)
                if declarator.init.as_ref().is_some_and(|init| init.span() == child) =>
            {
                match &declarator.id {
                    BindingPattern::ObjectPattern(object) => self.destructure(object),
                    _ => self.facts.handed_on = true,
                }
                return;
            }
            AstKind::AssignmentExpression(assignment) if assignment.right.span() == child => {
                match &assignment.left {
                    AssignmentTarget::ObjectAssignmentTarget(object) if object.rest.is_none() => {
                        for property in &object.properties {
                            match property {
                                AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(property) => {
                                    self.read(property.span, property.binding.name.to_string());
                                }
                                AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                                    match property.name.static_name() {
                                        Some(key) => self.read(property.span, js_text::property_name(&key)),
                                        None => self.facts.handed_on = true,
                                    }
                                }
                            }
                        }
                    }
                    _ => self.facts.handed_on = true,
                }
                return;
            }
            // A spread of the props object or a rest binding is recorded as an element's attribute (`SpreadOf`), and what
            // it means is decided from its tag: onto a DOM tag no evidence, onto a component a forward, onto a tag
            // that does not resolve unknown. It is not an escape.
            AstKind::JSXSpreadAttribute(_) => return,
            _ => None,
        };
        let is_jsx_tag = matches!(self.nodes.kind(parent), AstKind::JSXMemberExpression(_));
        match read {
            Some(key) if is_jsx_tag || self.is_read(parent) => self.read(self.nodes.kind(reference).span(), key),
            Some(_) | None => self.facts.handed_on = true,
        }
    }

    /// Whether a member expression is only read: not called with its object as `this`, written, deleted or
    /// destructured into.
    fn is_read(&self, member: NodeId) -> bool {
        let (parent, child) = self.through_wrappers(member);
        let (parent, child) = match self.nodes.kind(parent) {
            // `props?.x`: the chain is what is called, written or deleted (`delete props?.x`, `(props?.f)()`).
            AstKind::ChainExpression(_) => self.through_wrappers(parent),
            _ => (parent, child),
        };
        match self.nodes.kind(parent) {
            AstKind::CallExpression(call) => call.callee.span() != child,
            AstKind::TaggedTemplateExpression(tagged) => tagged.tag.span() != child,
            AstKind::AssignmentExpression(assignment) => assignment.left.span() != child,
            AstKind::UnaryExpression(unary) => unary.operator != UnaryOperator::Delete,
            AstKind::ForInStatement(each) => each.left.span() != child,
            AstKind::ForOfStatement(each) => each.left.span() != child,
            AstKind::UpdateExpression(_)
            | AstKind::ArrayAssignmentTarget(_)
            | AstKind::ObjectAssignmentTarget(_)
            | AstKind::AssignmentTargetRest(_)
            | AstKind::AssignmentTargetWithDefault(_)
            | AstKind::AssignmentTargetPropertyProperty(_) => false,
            _ => true,
        }
    }

    fn symbol_of(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id.get().and_then(|reference| self.scoping.get_reference(reference).symbol_id())
    }

    fn whole_expr(&self, expr: &Expression<'_>) -> Option<SpreadOf> {
        match peel_expr(expr) {
            Expression::Identifier(id) => self.symbol_of(id).and_then(|symbol| self.whole.get(&symbol).copied()),
            _ => None,
        }
    }

    /// The prop an attribute's value is, when it is exactly one: an unchanged destructured name, `props.x`, `rest.x`.
    fn value_prop(&self, value: Option<&JSXAttributeValue<'_>>) -> Option<String> {
        let Some(JSXAttributeValue::ExpressionContainer(container)) = value else { return None };
        let (object, key) = match peel_expr(container.expression.as_expression()?) {
            Expression::Identifier(id) => {
                return self.symbol_of(id).and_then(|symbol| self.locals.get(&symbol).cloned());
            }
            Expression::StaticMemberExpression(member) => (&member.object, member.property.name.to_string()),
            Expression::ComputedMemberExpression(member) => match &member.expression {
                Expression::StringLiteral(key) => (&member.object, js_text::lossy(key.value).into_owned()),
                _ => return None,
            },
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::StaticMemberExpression(member) => (&member.object, member.property.name.to_string()),
                _ => return None,
            },
            _ => return None,
        };
        self.whole_expr(object).map(|_| key)
    }
}

/// The symbols a body renders as a component tag (`<Item/>`).
fn tag_symbols(scoping: &Scoping, visit: impl FnOnce(&mut TagSymbols<'_>)) -> FxHashSet<SymbolId> {
    let mut tags = TagSymbols { scoping, symbols: FxHashSet::default() };
    visit(&mut tags);
    tags.symbols
}

struct TagSymbols<'s> {
    scoping: &'s Scoping,
    symbols: FxHashSet<SymbolId>,
}

impl<'a> Visit<'a> for TagSymbols<'_> {
    fn visit_jsx_element_name(&mut self, it: &JSXElementName<'a>) {
        if let JSXElementName::IdentifierReference(id) = it
            && let Some(symbol) =
                id.reference_id.get().and_then(|reference| self.scoping.get_reference(reference).symbol_id())
        {
            self.symbols.insert(symbol);
        }
    }
}

/// Collects a component body's elements; those of a nested component (a function or class bound to a symbol the body
/// renders as a tag) go to `nested_usages`.
struct Elements<'r, 's, 'a> {
    reader: &'r Reader<'s, 'a>,
    tags: FxHashSet<SymbolId>,
    /// The functions and classes a declarator binds to a tag symbol, by span: only they are nested, not a wrapper call
    /// around them (`withX(() => <li />, { icon: <Icon /> })` writes `<Icon />` in the body that calls `withX`).
    nested_spans: FxHashSet<SpanKey>,
    nested: u32,
    elements: Vec<JsxElementFact>,
    nested_usages: Vec<JsxUsage>,
    /// A nested component spreads the props object or a rest binding: an escape this body's elements do not record.
    spreads_nested: bool,
}

impl Elements<'_, '_, '_> {
    fn is_tag(&self, id: Option<&BindingIdentifier<'_>>) -> bool {
        id.and_then(|id| id.symbol_id.get()).is_some_and(|symbol| self.tags.contains(&symbol))
    }

    fn is_nested_span(&self, span: oxc_span::Span) -> bool {
        self.nested_spans.contains(&(span.start, span.end))
    }

    fn in_nested(&mut self, nested: bool, walk: impl FnOnce(&mut Self)) {
        self.nested += u32::from(nested);
        walk(self);
        self.nested -= u32::from(nested);
    }
}

impl<'a> Visit<'a> for Elements<'_, '_, 'a> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if matches!(&it.id, BindingPattern::BindingIdentifier(id) if self.is_tag(Some(id)))
            && let Some(span) = it.init.as_ref().and_then(nested_component)
        {
            self.nested_spans.insert((span.start, span.end));
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let nested = (it.is_declaration() && self.is_tag(it.id.as_ref())) || self.is_nested_span(it.span);
        self.in_nested(nested, |elements| walk::walk_function(elements, it, flags));
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let nested = self.is_nested_span(it.span);
        self.in_nested(nested, |elements| walk::walk_arrow_function_expression(elements, it));
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let nested = (it.is_declaration() && self.is_tag(it.id.as_ref())) || self.is_nested_span(it.span);
        self.in_nested(nested, |elements| walk::walk_class(elements, it));
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        if self.nested > 0 {
            // A nested component's elements are not this body's: a spread of the props or a rest binding there is
            // recorded nowhere, so it hands them on.
            self.spreads_nested |= it.attributes.iter().any(|attribute| {
                matches!(attribute, JSXAttributeItem::SpreadAttribute(spread)
                    if self.reader.whole_expr(&spread.argument).is_some())
            });
            self.nested_usages.push(usage_of(it, self.reader.scoping));
        } else {
            let attributes = it
                .attributes
                .iter()
                .map(|attribute| match attribute {
                    JSXAttributeItem::Attribute(attribute) => JsxAttributeFact::Named {
                        name: attribute_name(&attribute.name),
                        prop: self.reader.value_prop(attribute.value.as_ref()),
                    },
                    JSXAttributeItem::SpreadAttribute(spread) => {
                        JsxAttributeFact::Spread(self.reader.whole_expr(&spread.argument).unwrap_or(SpreadOf::Other))
                    }
                })
                .collect();
            self.elements
                .push(JsxElementFact { tag: scoped(self.reader.scoping, tag_of(&it.name), &it.name), attributes });
        }
        walk::walk_jsx_opening_element(self, it);
    }
}

/// `tag`, as [`JsxTag::Scoped`] when the symbol its first name refers to is bound in a scope other than the module's (a
/// parameter, a local `const`, an inner function or class): not the module binding of that name, so never counted as the
/// component an import or a top-level declaration of the same name is.
fn scoped(scoping: &Scoping, tag: JsxTag, name: &JSXElementName<'_>) -> JsxTag {
    let first = match name {
        JSXElementName::IdentifierReference(id) => Some(&**id),
        JSXElementName::MemberExpression(member) => Some(&member.object).and_then(|mut object| {
            loop {
                match object {
                    JSXMemberExpressionObject::IdentifierReference(id) => break Some(&**id),
                    JSXMemberExpressionObject::MemberExpression(inner) => object = &inner.object,
                    JSXMemberExpressionObject::ThisExpression(_) => break None,
                }
            }
        }),
        _ => None,
    };
    let inner = first
        .and_then(|id| id.reference_id.get())
        .and_then(|reference| scoping.get_reference(reference).symbol_id())
        .is_some_and(|symbol| scoping.symbol_scope_id(symbol) != scoping.root_scope_id());
    match tag {
        JsxTag::Identifier(name) | JsxTag::Member(name) if inner => JsxTag::Scoped(name),
        tag => tag,
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

#[cfg(test)]
mod tests;
