//! Evidence that a function renders, so a typed PascalCase function is a component and not a helper that happens to
//! take an object. A typed first parameter alone admits `FormatName = (u: User) => string`, `debounce((q: Row) => …)`
//! and selectors, so every rule that infers a component from a function asks this first.

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_syntax::scope::ScopeFlags;

/// Why a function was judged not to render. Carried into the skip diagnostic.
pub(super) type NoRender = &'static str;

/// Names whose type, as a return annotation, means the function renders.
const ELEMENT_TYPE_NAMES: &[&str] = &["Element", "ReactElement", "ReactNode", "ReactPortal", "JSX"];

/// Callees that produce an element.
const ELEMENT_FACTORIES: &[&str] = &["createElement", "cloneElement", "jsx", "jsxs", "jsxDEV"];

fn rightmost_name<'n>(name: &'n TSTypeName<'_>) -> Option<&'n str> {
    match name {
        TSTypeName::IdentifierReference(id) => Some(id.name.as_str()),
        TSTypeName::QualifiedName(q) => Some(q.right.name.as_str()),
        _ => None,
    }
}

/// Whether a return type annotation names a renderable result: `JSX.Element`, `ReactNode`, `ReactElement`, a union
/// containing one, or `Promise` of one (async server components).
pub(super) fn type_renders(ty: &TSType<'_>) -> bool {
    match ty {
        TSType::TSTypeReference(r) => {
            let Some(name) = rightmost_name(&r.type_name) else { return false };
            if name == "Promise" {
                return r.type_arguments.as_ref().and_then(|a| a.params.first()).is_some_and(type_renders);
            }
            ELEMENT_TYPE_NAMES.contains(&name)
        }
        TSType::TSUnionType(u) => u.types.iter().any(type_renders),
        TSType::TSParenthesizedType(p) => type_renders(&p.type_annotation),
        _ => false,
    }
}

/// Looks for JSX, an element factory call, a top-level hook call (only components and hooks may call hooks, and a
/// PascalCase function is not a hook), or a top-level `return` of `null`, `children` or a call to a PascalCase function
/// (a component invoked directly, `return JoinList({...})`). Nested functions still count for JSX (a render callback),
/// but not for the top-level checks.
#[derive(Default)]
struct RenderFinder {
    renders: bool,
    nested: u32,
}

fn is_children(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::Identifier(id) => id.name == "children",
        Expression::StaticMemberExpression(m) => m.property.name == "children",
        Expression::ParenthesizedExpression(p) => is_children(&p.expression),
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

fn returns_element_like(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::NullLiteral(_) => true,
        Expression::CallExpression(call) => callee_name(&call.callee)
            .is_some_and(|n| n.starts_with(|c: char| c.is_ascii_uppercase()) || is_hook_name(n)),
        Expression::ParenthesizedExpression(p) => returns_element_like(&p.expression),
        _ => is_children(expr),
    }
}

impl<'a> Visit<'a> for RenderFinder {
    fn visit_jsx_element(&mut self, _it: &JSXElement<'a>) {
        self.renders = true;
    }

    fn visit_jsx_fragment(&mut self, _it: &JSXFragment<'a>) {
        self.renders = true;
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let callee = callee_name(&it.callee);
        if callee.is_some_and(|c| ELEMENT_FACTORIES.contains(&c) || (self.nested == 0 && is_hook_name(c))) {
            self.renders = true;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if self.nested == 0 && it.argument.as_ref().is_some_and(returns_element_like) {
            self.renders = true;
        }
        walk::walk_return_statement(self, it);
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
    Arrow(&'b ArrowFunctionBody<'a>),
}

/// The parts of a function that decide whether it renders.
pub(super) struct FunctionShape<'b, 'a> {
    pub generator: bool,
    pub return_type: Option<&'b TSTypeAnnotation<'a>>,
    pub body: Option<Body<'b, 'a>>,
}

impl<'b, 'a> FunctionShape<'b, 'a> {
    pub(super) fn of_arrow(f: &'b ArrowFunctionExpression<'a>) -> Self {
        Self { generator: false, return_type: f.return_type.as_deref(), body: Some(Body::Arrow(&f.body)) }
    }

    pub(super) fn of_function(f: &'b Function<'a>) -> Self {
        Self { generator: f.generator, return_type: f.return_type.as_deref(), body: f.body.as_deref().map(Body::Block) }
    }
}

/// `Ok` when the function renders; `Err` with the reason it does not.
pub(super) fn renders(shape: &FunctionShape<'_, '_>) -> Result<(), NoRender> {
    if shape.generator {
        return Err("is a generator");
    }
    if let Some(annotation) = shape.return_type {
        return if type_renders(&annotation.type_annotation) {
            Ok(())
        } else {
            Err("declares a return type that is not an element")
        };
    }
    let mut finder = RenderFinder::default();
    match &shape.body {
        None => return Err("has no body and no element return type"),
        Some(Body::Block(body)) => finder.visit_function_body(body),
        Some(Body::Arrow(body)) => {
            if let Some(expr) = body.as_expression() {
                if returns_element_like(expr) {
                    return Ok(());
                }
            }
            finder.visit_arrow_function_body(body);
        }
    }
    if finder.renders {
        Ok(())
    } else {
        Err("never renders: no JSX, element, hook call, or returned null, children or component call")
    }
}
