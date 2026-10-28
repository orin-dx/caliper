//! Evidence that a function renders, so a typed PascalCase function is a component and not a helper that happens to
//! take an object. A typed first parameter alone admits `FormatName = (u: User) => string`, `debounce((q: Row) => …)`
//! and selectors, so every rule that infers a component from a function asks this first.
//!
//! What the function returns decides, not what appears anywhere in its body: JSX built in a nested callback or never
//! returned is not evidence.

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_syntax::scope::ScopeFlags;
use rustc_hash::FxHashSet;

/// Why a function was judged not to render. Carried into the skip diagnostic.
pub(super) type NoRender = &'static str;

/// Names whose type, as a return annotation, means the function renders.
const ELEMENT_TYPE_NAMES: &[&str] = &["Element", "ReactElement", "ReactNode", "ReactPortal", "JSX"];

/// Callees that produce an element.
const ELEMENT_FACTORIES: &[&str] = &["createElement", "cloneElement", "jsx", "jsxs", "jsxDEV", "createPortal"];

/// Whether a type name, as written in this file, can be React's or the global `JSX` namespace's. Given the name's
/// leftmost segment (`React` for `React.ReactNode`), so an element-like name imported from another module does not
/// count.
pub(super) type ReactBinding<'f> = &'f dyn Scope;

/// What the file around a function says about the names it uses.
pub(super) trait Scope {
    /// Whether a type name can be React's: see [`ReactBinding`].
    fn may_be_react(&self, name: &str) -> bool;
    /// Whether a call to this name returns an element: a same-file function declared to return one.
    fn is_element_helper(&self, name: &str) -> bool;
}

fn leftmost_and_rightmost<'n>(name: &'n TSTypeName<'_>) -> Option<(&'n str, &'n str)> {
    match name {
        TSTypeName::IdentifierReference(id) => Some((id.name.as_str(), id.name.as_str())),
        TSTypeName::QualifiedName(q) => {
            let mut left = &q.left;
            while let TSTypeName::QualifiedName(inner) = left {
                left = &inner.left;
            }
            match left {
                TSTypeName::IdentifierReference(id) => Some((id.name.as_str(), q.right.name.as_str())),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether a return type annotation names a renderable result bound to React: `JSX.Element`, `ReactNode`,
/// `ReactElement`, a union containing one, or `Promise` of one (async server components).
pub(super) fn type_renders(ty: &TSType<'_>, react: ReactBinding<'_>) -> bool {
    match ty {
        TSType::TSTypeReference(r) => {
            let Some((left, right)) = leftmost_and_rightmost(&r.type_name) else { return false };
            if right == "Promise" {
                return r
                    .type_arguments
                    .as_ref()
                    .and_then(|a| a.params.first())
                    .is_some_and(|t| type_renders(t, react));
            }
            ELEMENT_TYPE_NAMES.contains(&right) && react.may_be_react(left)
        }
        TSType::TSUnionType(u) => u.types.iter().any(|t| type_renders(t, react)),
        TSType::TSParenthesizedType(p) => type_renders(&p.type_annotation, react),
        _ => false,
    }
}

fn callee_name<'n>(callee: &'n Expression<'_>) -> Option<&'n str> {
    match callee {
        Expression::Identifier(id) => Some(id.name.as_str()),
        Expression::StaticMemberExpression(m) => Some(m.property.name.as_str()),
        _ => None,
    }
}

fn is_hook_name(name: &str) -> bool {
    name.strip_prefix("use").is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_uppercase()))
}

fn is_children(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::Identifier(id) => id.name == "children",
        Expression::StaticMemberExpression(m) => m.property.name == "children",
        _ => false,
    }
}

/// Finds JSX or an element factory call that can be part of an expression's value: through conditionals, logical
/// operators, arrays and call arguments (`items.map(i => <li/>)`, `createPortal(<div/>, el)`), but not inside an object
/// literal, whose JSX is a property's value rather than the expression's.
struct ElementInValue<'s> {
    found: bool,
    scope: &'s dyn Scope,
}

impl<'a> Visit<'a> for ElementInValue<'_> {
    fn visit_jsx_element(&mut self, _it: &JSXElement<'a>) {
        self.found = true;
    }

    fn visit_jsx_fragment(&mut self, _it: &JSXFragment<'a>) {
        self.found = true;
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let name = callee_name(&it.callee);
        let calls_helper = matches!(&it.callee, Expression::Identifier(id) if self.scope.is_element_helper(&id.name));
        if calls_helper || name.is_some_and(|c| ELEMENT_FACTORIES.contains(&c)) {
            self.found = true;
        // These calls preserve the JSX value; an arbitrary call may turn it into a Response or other data.
        } else if name.is_some_and(|name| matches!(name, "map" | "flatMap" | "reduce"))
            || matches!(&it.callee, Expression::StaticMemberExpression(member)
                if matches!(&member.object, Expression::Identifier(id) if id.name == "Promise")
                    && matches!(member.property.name.as_str(), "resolve" | "all"))
        {
            walk::walk_call_expression(self, it);
        }
    }

    // JSX passed to a constructor, such as `new ImageResponse(<div />)`, is not the function's return value.
    fn visit_new_expression(&mut self, _it: &NewExpression<'a>) {}

    fn visit_object_expression(&mut self, _it: &ObjectExpression<'a>) {}
}

fn value_has_element(expr: &Expression<'_>, scope: &dyn Scope) -> bool {
    let mut finder = ElementInValue { found: false, scope };
    finder.visit_expression(expr);
    finder.found
}

/// What one returned value says about the function.
fn returns_element(expr: &Expression<'_>, element_locals: &FxHashSet<String>, scope: &dyn Scope) -> bool {
    match expr.without_parentheses() {
        // Rendering nothing is a valid render.
        Expression::NullLiteral(_) => true,
        Expression::Identifier(id) if element_locals.contains(id.name.as_str()) => true,
        // A component invoked directly: `return JoinList({ ... })`.
        Expression::CallExpression(call)
            if callee_name(&call.callee).is_some_and(|n| n.starts_with(|c: char| c.is_ascii_uppercase())) =>
        {
            true
        }
        e if is_children(e) => true,
        e => value_has_element(e, scope),
    }
}

/// Walks one function's own body, not nested functions: its `return`s, its top-level hook calls, and the local
/// variables it initializes with an element (`const content = <div/>; return content`).
/// A local is declared before the `return` that uses it, so each `return` is judged as it is reached.
struct OwnBody<'s> {
    returns_element: bool,
    element_locals: FxHashSet<String>,
    calls_hook: bool,
    nested: u32,
    scope: &'s dyn Scope,
}

impl<'s> OwnBody<'s> {
    fn new(scope: &'s dyn Scope) -> Self {
        Self { returns_element: false, element_locals: FxHashSet::default(), calls_hook: false, nested: 0, scope }
    }
}

impl<'a> Visit<'a> for OwnBody<'_> {
    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if self.nested == 0
            && it.argument.as_ref().is_some_and(|r| returns_element(r, &self.element_locals, self.scope))
        {
            self.returns_element = true;
        }
        walk::walk_return_statement(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if self.nested == 0 {
            if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init) {
                if value_has_element(init, self.scope) {
                    self.element_locals.insert(id.name.to_string());
                }
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if self.nested == 0 && callee_name(&it.callee).is_some_and(is_hook_name) {
            self.calls_hook = true;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.nested += 1;
        walk::walk_function(self, it, flags);
        self.nested -= 1;
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.nested += 1;
        walk::walk_arrow_function_expression(self, it);
        self.nested -= 1;
    }
}

/// A function body: a block, or an arrow's single expression or block.
pub(super) enum Body<'b, 'a> {
    Block(&'b FunctionBody<'a>),
    /// An arrow's body, and whether it is a single expression (`=> <div/>`) rather than a block.
    Arrow(&'b ArrowFunctionBody<'a>, bool),
}

/// The parts of a function that decide whether it renders.
pub(super) struct FunctionShape<'b, 'a> {
    pub generator: bool,
    pub return_type: Option<&'b TSTypeAnnotation<'a>>,
    pub body: Option<Body<'b, 'a>>,
}

impl<'b, 'a> FunctionShape<'b, 'a> {
    pub(super) fn of_arrow(f: &'b ArrowFunctionExpression<'a>) -> Self {
        Self {
            generator: false,
            return_type: f.return_type.as_deref(),
            body: Some(Body::Arrow(&f.body, f.is_expression())),
        }
    }

    pub(super) fn of_function(f: &'b Function<'a>) -> Self {
        Self { generator: f.generator, return_type: f.return_type.as_deref(), body: f.body.as_deref().map(Body::Block) }
    }
}

/// `Ok` when the function renders; `Err` with the reason it does not.
pub(super) fn renders(shape: &FunctionShape<'_, '_>, react: ReactBinding<'_>) -> Result<(), NoRender> {
    if shape.generator {
        return Err("is a generator");
    }
    if let Some(annotation) = shape.return_type {
        return if type_renders(&annotation.type_annotation, react) {
            Ok(())
        } else {
            Err("declares a return type that is not React's element type")
        };
    }
    let renders = match &shape.body {
        None => return Err("has no body and no element return type"),
        Some(Body::Arrow(body, true)) => {
            body.as_expression().is_some_and(|expr| returns_element(expr, &FxHashSet::default(), react))
        }
        Some(Body::Arrow(body, false)) => {
            let mut own = OwnBody::new(react);
            own.visit_arrow_function_body(body);
            own.calls_hook || own.returns_element
        }
        Some(Body::Block(body)) => {
            let mut own = OwnBody::new(react);
            own.visit_function_body(body);
            own.calls_hook || own.returns_element
        }
    };
    if renders {
        Ok(())
    } else {
        Err("never returns an element: no returned JSX, element, null, children or component call, and no hook call")
    }
}

/// Names of module-level functions whose declared return type is an element, which makes a call to them element
/// evidence: `renderByProp(render, props, <button />)` with `renderByProp(...): React.ReactElement`. Judged on the
/// declaration alone, so a helper that returns something else without saying so is not counted.
pub(super) fn element_helpers(program: &Program<'_>) -> FxHashSet<String> {
    let mut foreign = FxHashSet::default();
    for stmt in &program.body {
        if let Statement::ImportDeclaration(import) = stmt {
            if import.source.value != "react" && !import.source.value.starts_with("react/") {
                for spec in import.specifiers.iter().flatten() {
                    foreign.insert(spec.local().name.to_string());
                }
            }
        }
    }
    let react = Foreign(&foreign);
    let declarations: Vec<&Declaration<'_>> = program.body.iter().filter_map(|stmt| stmt.as_declaration()).collect();

    // `type Output = React.ReactElement` and aliases of aliases.
    let mut aliases = FxHashSet::default();
    loop {
        let before = aliases.len();
        for declaration in &declarations {
            if let Declaration::TSTypeAliasDeclaration(alias) = declaration {
                if names_element(&alias.type_annotation, &react, &aliases) {
                    aliases.insert(alias.id.name.to_string());
                }
            }
        }
        if aliases.len() == before {
            break;
        }
    }

    let returns_element = |annotation: Option<&TSTypeAnnotation<'_>>| {
        annotation.is_some_and(|a| names_element(&a.type_annotation, &react, &aliases))
    };
    let mut helpers = FxHashSet::default();
    for declaration in declarations {
        match declaration {
            Declaration::FunctionDeclaration(f) => {
                if let Some(id) = f.id.as_ref().filter(|_| returns_element(f.return_type.as_deref())) {
                    helpers.insert(id.name.to_string());
                }
            }
            Declaration::VariableDeclaration(vars) => {
                for declarator in &vars.declarations {
                    let (BindingPattern::BindingIdentifier(id), Some(init)) = (&declarator.id, &declarator.init) else {
                        continue;
                    };
                    let annotation = match init.without_parentheses() {
                        Expression::ArrowFunctionExpression(f) => f.return_type.as_deref(),
                        Expression::FunctionExpression(f) => f.return_type.as_deref(),
                        _ => None,
                    };
                    if returns_element(annotation) {
                        helpers.insert(id.name.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    helpers
}

/// The imports that are not React's, seen before the file is walked.
struct Foreign<'f>(&'f FxHashSet<String>);

impl Scope for Foreign<'_> {
    fn may_be_react(&self, name: &str) -> bool {
        !self.0.contains(name)
    }

    fn is_element_helper(&self, _name: &str) -> bool {
        false
    }
}

/// Whether a type is an element type or a local alias of one. A `Promise` of one is not: the call returns the promise.
fn names_element(ty: &TSType<'_>, react: ReactBinding<'_>, aliases: &FxHashSet<String>) -> bool {
    match ty {
        TSType::TSTypeReference(r) => match &r.type_name {
            TSTypeName::IdentifierReference(id) if aliases.contains(id.name.as_str()) => true,
            name => {
                leftmost_and_rightmost(name).is_some_and(|(_, right)| right != "Promise") && type_renders(ty, react)
            }
        },
        TSType::TSUnionType(u) => u.types.iter().any(|t| names_element(t, react, aliases)),
        TSType::TSParenthesizedType(p) => names_element(&p.type_annotation, react, aliases),
        _ => false,
    }
}
