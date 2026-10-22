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
pub(super) type ReactBinding<'f> = &'f dyn Fn(&str) -> bool;

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
            ELEMENT_TYPE_NAMES.contains(&right) && react(left)
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
#[derive(Default)]
struct ElementInValue {
    found: bool,
}

impl<'a> Visit<'a> for ElementInValue {
    fn visit_jsx_element(&mut self, _it: &JSXElement<'a>) {
        self.found = true;
    }

    fn visit_jsx_fragment(&mut self, _it: &JSXFragment<'a>) {
        self.found = true;
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if callee_name(&it.callee).is_some_and(|c| ELEMENT_FACTORIES.contains(&c)) {
            self.found = true;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_object_expression(&mut self, _it: &ObjectExpression<'a>) {}
}

fn value_has_element(expr: &Expression<'_>) -> bool {
    let mut finder = ElementInValue::default();
    finder.visit_expression(expr);
    finder.found
}

/// What one returned value says about the function.
fn returns_element(expr: &Expression<'_>, element_locals: &FxHashSet<String>) -> bool {
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
        e => value_has_element(e),
    }
}

/// Walks one function's own body, not nested functions: its `return`s, its top-level hook calls, and the local
/// variables it initializes with an element (`const content = <div/>; return content`).
/// A local is declared before the `return` that uses it, so each `return` is judged as it is reached.
#[derive(Default)]
struct OwnBody {
    returns_element: bool,
    element_locals: FxHashSet<String>,
    calls_hook: bool,
    nested: u32,
}

impl<'a> Visit<'a> for OwnBody {
    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if self.nested == 0 && it.argument.as_ref().is_some_and(|r| returns_element(r, &self.element_locals)) {
            self.returns_element = true;
        }
        walk::walk_return_statement(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if self.nested == 0 {
            if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init) {
                if value_has_element(init) {
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
            body.as_expression().is_some_and(|expr| returns_element(expr, &FxHashSet::default()))
        }
        Some(Body::Arrow(body, false)) => {
            let mut own = OwnBody::default();
            own.visit_arrow_function_body(body);
            own.calls_hook || own.returns_element
        }
        Some(Body::Block(body)) => {
            let mut own = OwnBody::default();
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
