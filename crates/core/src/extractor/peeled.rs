//! What [`super::component::peel`] reads beneath a callee's or a base class's wrappers, sealed: the expression beneath
//! them is reachable only as origin/main read it ([`Peeled::as_plain`]) or once its root name resolves to the one
//! declaration it can mean ([`Peeled::resolve`]). No method hands out the peeled expression or its spelling otherwise,
//! so a path that would match a name only a peel made visible does not compile.

use oxc_ast::ast::{Expression, TSTypeParameterInstantiation};

use super::component::Evidence;
use super::evidence::Scope;

/// An expression beneath every wrapper, instantiations included, the type arguments an instantiation among them gave
/// it, and what the outermost assertion says about calling it.
#[derive(Clone, Copy)]
pub(crate) struct Peeled<'b, 'a> {
    /// The expression as written.
    original: &'b Expression<'a>,
    /// The expression beneath every wrapper.
    expr: &'b Expression<'a>,
    /// The outermost instantiation's type arguments, unless an assertion above it replaced its type: `(f<T>)` and
    /// `(f<T>)!` give `T`; `(f<T> as G)` gives none, as the value then has type `G`.
    type_arguments: Option<&'b TSTypeParameterInstantiation<'a>>,
    evidence: Evidence<'b, 'a>,
}

impl<'b, 'a> Peeled<'b, 'a> {
    pub(super) fn new(
        original: &'b Expression<'a>,
        expr: &'b Expression<'a>,
        type_arguments: Option<&'b TSTypeParameterInstantiation<'a>>,
        evidence: Evidence<'b, 'a>,
    ) -> Self {
        Self { original, expr, type_arguments, evidence }
    }

    /// What the outermost assertion says about calling the expression.
    pub(super) fn evidence(&self) -> Evidence<'b, 'a> {
        self.evidence
    }

    /// The type arguments it is instantiated with.
    pub(super) fn type_arguments(&self) -> Option<&'b TSTypeParameterInstantiation<'a>> {
        self.type_arguments
    }

    /// The same, with `type_arguments` written on the site itself in place of an instantiation's.
    pub(super) fn with_type_arguments(self, type_arguments: Option<&'b TSTypeParameterInstantiation<'a>>) -> Self {
        Self { type_arguments: type_arguments.or(self.type_arguments), ..self }
    }

    /// The expression, when origin/main read it as written: only parentheses, `as`, `satisfies`, `!` and instantiations
    /// above it (`component::main_read`), and the call still says what it is (no result on the closed list).
    pub(super) fn as_plain(&self) -> Option<&'b Expression<'a>> {
        let main_read = std::ptr::eq(super::component::main_read(self.original), self.expr);
        (main_read && self.names()).then_some(self.expr)
    }

    /// The expression, read through every wrapper: as written when origin/main read it ([`Self::as_plain`]); read only
    /// through a peel, when its root name resolves to the one declaration it can mean, a unique binding or a global or
    /// import the file does not declare again (`Scope::declarations` at most one).
    pub(super) fn resolve(&self, scope: &dyn Scope) -> Option<&'b Expression<'a>> {
        if let Some(expr) = self.as_plain() {
            return Some(expr);
        }
        let root = super::component::chain_root(self.expr)?;
        (self.names() && scope.declarations(root) <= 1).then_some(self.expr)
    }

    /// The value called, whatever the assertion says the call returns (`(Helper as H)(props)` calls `Helper`): as written
    /// when origin/main read it, otherwise when its root name resolves, as [`Self::resolve`].
    pub(super) fn resolve_called(&self, scope: &dyn Scope) -> Option<&'b Expression<'a>> {
        if std::ptr::eq(super::component::main_read(self.original), self.expr) {
            return Some(self.expr);
        }
        let root = super::component::chain_root(self.expr)?;
        (scope.declarations(root) <= 1).then_some(self.expr)
    }

    /// Whether the expression is the dotted chain `chain` (a `typeof` alias's query): a comparison, not a spelling read.
    pub(super) fn is_chain(&self, chain: &str) -> bool {
        super::component::chain_names(chain, self.expr)
    }

    /// Whether what it is called still says what the call is: unless the assertion gives a result on the closed list.
    fn names(&self) -> bool {
        !matches!(self.evidence, Evidence::NotRenderable)
    }
}

#[cfg(test)]
mod tests {
    /// The methods `Peeled` exposes. Only `as_plain` and `resolve` hand out the peeled expression, each behind its check;
    /// a new method fails here until it is reviewed.
    const METHODS: &[&str] = &[
        "new",
        "evidence",
        "type_arguments",
        "with_type_arguments",
        "as_plain",
        "resolve",
        "resolve_called",
        "is_chain",
    ];

    #[test]
    fn peeled_exposes_no_accessor_that_reads_the_expression_unchecked() {
        let source = include_str!("peeled.rs");
        let start = source.find("impl<'b, 'a> Peeled<'b, 'a> {").expect("the impl block");
        let end = source[start..].find("\n}\n").expect("its end") + start;
        let methods: Vec<&str> = source[start..end]
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("pub(super) fn "))
            .filter_map(|rest| rest.split(['(', '<']).next())
            .collect();
        assert_eq!(methods, METHODS);
        // Nothing outside this module may reach the fields: they are private, so a read does not compile. And no free
        // function returns what `peel` peeled except through `Peeled`.
        for visibility in ["pub(super)", "pub(crate)", "pub"] {
            assert!(!source.contains(&format!("{visibility} {}", "expr:")), "a public field");
        }
    }
}
