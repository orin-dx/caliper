//! Component detection: FC annotation, forwardRef, HOC-wrapped, ForwardRefExoticComponent.

use std::collections::BTreeMap;

use camino::Utf8Path;
use compact_str::CompactString;
use oxc_ast::ast::*;
use rustc_hash::FxHashMap;

use crate::types::{CollectedTypeAlias, ComponentMapping, DetectionRule, DiagnosticCode};

use super::evidence::{FunctionShape, Scope, renders};
use super::{ReactApi, SourceDataCollector, is_pascal_case};

/// One layer of the wrappers TypeScript erases from a value.
enum Wrapper<'b, 'a> {
    /// Parentheses, `satisfies` or `!`: the value and its type pass through.
    Transparent(&'b Expression<'a>),
    /// `as` or `<T>x`: the value passes through, its type is replaced by the one asserted.
    Asserted(&'b Expression<'a>, &'b TSType<'a>),
    /// `f<T>`: the value passes through with its type arguments given.
    Instantiation(&'b TSInstantiationExpression<'a>),
}

/// Whether a callee is written through a wrapper origin/main (oxc 0.158) read no callee name through: an `as` or
/// `<T>x` assertion, `satisfies` or `!`. Parentheses are not one, and neither is an instantiation, whose type
/// arguments OXC 0.158 moved onto the call.
pub(super) fn wrapped_beyond_main(mut expr: &Expression<'_>) -> bool {
    loop {
        expr = match (expr, wrapper(expr)) {
            (Expression::ParenthesizedExpression(parenthesized), _) => &parenthesized.expression,
            (_, Some(Wrapper::Instantiation(instantiation))) => &instantiation.expression,
            (_, Some(Wrapper::Transparent(_) | Wrapper::Asserted(..))) => return true,
            (_, None) => return false,
        };
    }
}

/// The wrapper `expr` is, if it is one. The one list of what a wrapper is, and exhaustive so that a new expression kind
/// in an OXC release has to be placed here before it compiles.
fn wrapper<'a, 'b>(expr: &'b Expression<'a>) -> Option<Wrapper<'b, 'a>> {
    match expr {
        Expression::ParenthesizedExpression(inner) => Some(Wrapper::Transparent(&inner.expression)),
        Expression::TSSatisfiesExpression(inner) => Some(Wrapper::Transparent(&inner.expression)),
        Expression::TSNonNullExpression(inner) => Some(Wrapper::Transparent(&inner.expression)),
        Expression::TSAsExpression(inner) => Some(Wrapper::Asserted(&inner.expression, &inner.type_annotation)),
        Expression::TSTypeAssertion(inner) => Some(Wrapper::Asserted(&inner.expression, &inner.type_annotation)),
        Expression::TSInstantiationExpression(inner) => Some(Wrapper::Instantiation(inner)),
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::TemplateLiteral(_)
        | Expression::Identifier(_)
        | Expression::Super(_)
        | Expression::ArrayExpression(_)
        | Expression::ArrowFunctionExpression(_)
        | Expression::AssignmentExpression(_)
        | Expression::AwaitExpression(_)
        | Expression::BinaryExpression(_)
        | Expression::CallExpression(_)
        | Expression::ChainExpression(_)
        | Expression::ClassExpression(_)
        | Expression::ConditionalExpression(_)
        | Expression::FunctionExpression(_)
        | Expression::ImportExpression(_)
        | Expression::LogicalExpression(_)
        | Expression::NewExpression(_)
        | Expression::ObjectExpression(_)
        | Expression::SequenceExpression(_)
        | Expression::TaggedTemplateExpression(_)
        | Expression::ThisExpression(_)
        | Expression::UnaryExpression(_)
        | Expression::UpdateExpression(_)
        | Expression::YieldExpression(_)
        | Expression::PrivateInExpression(_)
        | Expression::ImportMeta(_)
        | Expression::NewTarget(_)
        | Expression::JSXElement(_)
        | Expression::JSXFragment(_)
        | Expression::V8IntrinsicExpression(_)
        | Expression::ComputedMemberExpression(_)
        | Expression::StaticMemberExpression(_)
        | Expression::PrivateFieldExpression(_) => None,
    }
}

/// A value beneath its wrappers, for an initializer, an argument or an object's member: component libraries cast a
/// forwardRef or HOC call to a hand-rolled component type (Fluent UI: "required due to lack of distributive union to
/// support unions on @types/react") or check it with `satisfies`. The value is the same beneath those. An
/// instantiation (`List<Row>`) is where the read stops, as on OXC 0.158, whose value positions kept it the same way:
/// what its type arguments bind is not applied yet, so the instantiated value is no component here. Only a callee or an
/// `extends` target reads through one ([`peel`]), as OXC 0.159 leaves it there where 0.158 moved it onto the call.
pub(super) fn peel_expr<'a, 'b>(mut expr: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        expr = match wrapper(expr) {
            Some(Wrapper::Transparent(inner) | Wrapper::Asserted(inner, _)) => inner,
            Some(Wrapper::Instantiation(_)) | None => return expr,
        };
    }
}

/// What the outermost assertion on a callee, or on the namespace it is read from, says about calling it. Only the
/// outermost one counts: `(e as A) as B` has type `B` whatever `A` is. An assertion declines a candidate only on
/// affirmative evidence ([`Evidence::NotRenderable`], which only [`never_renders`] gives); every other type leaves the
/// call the candidate its name makes it ([`Evidence::Unknown`]), as before assertions were read at all.
#[derive(Clone, Copy)]
pub(super) enum Evidence<'b, 'a> {
    /// No assertion, or one that keeps the callee's own type: `typeof` the same expression, `any` and `unknown`. The
    /// callee's declaration describes the call.
    Identity,
    /// A type reference (`(forwardRef as forwardRefType)`). The file may alias it to `typeof` the callee
    /// (`SourceDataCollector::describes_call`); otherwise it reads as [`Evidence::Unknown`].
    Reference(&'b TSTypeReference<'a>),
    /// Every call signature of the asserted type returns a type on [`never_renders`]'s closed list:
    /// `(Object as (v: object) => object)(…)` neither is a component nor renders, whatever its name.
    NotRenderable,
    /// Anything else: `typeof` another expression, a signature whose result is not on the closed list or is not
    /// written, a type with no call signature. The name still says what the call is; the callee's declaration no
    /// longer describes it.
    Unknown,
}

/// An expression beneath every wrapper, instantiations included, the type arguments an instantiation among them gave
/// it, and what the outermost assertion says about calling it.
#[derive(Clone, Copy)]
pub(super) struct Peeled<'b, 'a> {
    pub(super) expr: &'b Expression<'a>,
    /// The outermost instantiation's type arguments, unless an assertion above it replaced its type: `(f<T>)` and
    /// `(f<T>)!` give `T`; `(f<T> as G)` gives none, as the value then has type `G`.
    pub(super) type_arguments: Option<&'b TSTypeParameterInstantiation<'a>>,
    pub(super) evidence: Evidence<'b, 'a>,
}

impl<'b, 'a> Peeled<'b, 'a> {
    /// `expr`, when what it is called still says what the call is (`forwardRef`, `memo`, `Object.assign`, a HOC, an
    /// element factory, `map` over elements, a component invoked directly): always, unless the assertion gives the
    /// call a result on [`never_renders`]'s closed list.
    pub(super) fn named(&self) -> Option<&'b Expression<'a>> {
        match self.evidence {
            Evidence::NotRenderable => None,
            Evidence::Identity | Evidence::Reference(_) | Evidence::Unknown => Some(self.expr),
        }
    }
}

/// The value beneath `expr`'s wrappers, the outermost instantiation's type arguments when no assertion is above them,
/// and the outermost assertion. One pass over the wrappers, so a chain of them costs its length.
fn layers<'a, 'b>(
    mut expr: &'b Expression<'a>,
) -> (&'b Expression<'a>, Option<&'b TSTypeParameterInstantiation<'a>>, Option<&'b TSType<'a>>) {
    let mut type_arguments = None;
    let mut outermost = None;
    loop {
        #[cfg(test)]
        steps::count();
        expr = match wrapper(expr) {
            Some(Wrapper::Transparent(inner)) => inner,
            Some(Wrapper::Asserted(inner, ty)) => {
                outermost = outermost.or(Some(ty));
                inner
            }
            Some(Wrapper::Instantiation(instantiation)) => {
                if outermost.is_none() && type_arguments.is_none() {
                    type_arguments = Some(&*instantiation.type_arguments);
                }
                &instantiation.expression
            }
            None => return (expr, type_arguments, outermost),
        };
    }
}

/// How a site uses the value it peels, and so which signatures of an asserted type answer it.
#[derive(Clone, Copy)]
pub(super) enum Signatures {
    /// A callee, `f(…)`: its call signatures.
    Call,
    /// An `extends` target, which the class constructs: its construct signatures. The closed list is the same: an
    /// `object`, `symbol` or `void` instance is no `React.Component`. Other primitives are not instances either, but
    /// they are left as no evidence, like every type off the list.
    Construct,
}

/// See [`Peeled`]. For what a callee (`Signatures::Call`) or an `extends` target (`Signatures::Construct`) is: OXC
/// 0.159 keeps `(f<T>)(x)` and `extends (B<T>)` as an instantiation node beneath the call or heritage clause, where
/// 0.158 moved its type arguments onto them. A namespace a member is read from is never called itself: the member is
/// read with the site's signatures, and only for that member.
pub(super) fn peel<'a, 'b>(expr: &'b Expression<'a>, signatures: Signatures) -> Peeled<'b, 'a> {
    let (expr, type_arguments, outermost) = layers(expr);
    let evidence = match (outermost, expr) {
        (Some(ty), _) => asserted(ty, expr, signatures),
        // `(React as { forwardRef: typeof React.forwardRef }).forwardRef`: the namespace's assertion gives the member
        // the type it declares for it.
        (None, Expression::StaticMemberExpression(member)) => match receiver(&member.object, signatures) {
            Typed::Own => Evidence::Identity,
            Typed::Asserted(ty) => member_evidence(ty, member.property.name.as_str(), expr, signatures),
            Typed::Unread => Evidence::Unknown,
        },
        (None, _) => Evidence::Identity,
    };
    Peeled { expr, type_arguments, evidence }
}

/// The type a namespace has, as far as the assertions on it and on the namespaces it is read from say.
enum Typed<'b, 'a> {
    /// Its own: no assertion, or one that keeps it.
    Own,
    /// The asserted type, or the type an asserted namespace's type literal declares for the member.
    Asserted(&'b TSType<'a>),
    /// A member the asserted type does not declare where the extractor looks (a type literal's property).
    Unread,
}

/// `signatures` is the site's: only whether an assertion keeps the namespace's own type is read with it, which does not
/// depend on the signatures.
fn receiver<'a, 'b>(expr: &'b Expression<'a>, signatures: Signatures) -> Typed<'b, 'a> {
    let (expr, _, outermost) = layers(expr);
    let declared = match (outermost, expr) {
        (Some(ty), _) => Some(ty),
        (None, Expression::StaticMemberExpression(object)) => match receiver(&object.object, signatures) {
            Typed::Own => None,
            Typed::Asserted(ty) => match member(ty, object.property.name.as_str()) {
                Member::Property(ty) => Some(ty),
                Member::Declared(_) | Member::Unread => return Typed::Unread,
            },
            Typed::Unread => return Typed::Unread,
        },
        (None, _) => None,
    };
    match declared {
        None => Typed::Own,
        Some(ty) if matches!(asserted(ty, expr, signatures), Evidence::Identity) => Typed::Own,
        Some(ty) => Typed::Asserted(ty),
    }
}

/// What a type literal declares for a member.
enum Member<'b, 'a> {
    /// One property or getter, and nothing else of that name: its type is the member's.
    Property(&'b TSType<'a>),
    /// Anything else of that name (`{ memo(c: C): R }`, overloads, a property beside a method): every declaration of
    /// the name in the literal counts ([`declarations`]).
    Declared(&'b TSTypeLiteral<'a>),
    Unread,
}

/// One declaration of a member's name in a type literal.
enum MemberDeclaration<'b, 'a> {
    /// A property or getter, and the type it gives the member, if written.
    Typed(Option<&'b TSType<'a>>),
    /// A method, and the result it declares, if written.
    Method(Option<&'b TSType<'a>>),
}

fn names_key(key: &PropertyKey<'_>, name: &str) -> bool {
    key.static_name().is_some_and(|key| super::js_text::property_name(&key) == name)
}

/// Every declaration of `name` among a type literal's members, in order.
fn declarations<'a, 'b>(literal: &'b TSTypeLiteral<'a>, name: &str) -> impl Iterator<Item = MemberDeclaration<'b, 'a>> {
    literal.members.iter().filter_map(move |signature| match signature {
        TSSignature::TSPropertySignature(property) if names_key(&property.key, name) => Some(MemberDeclaration::Typed(
            property.type_annotation.as_ref().map(|annotation| &annotation.type_annotation),
        )),
        TSSignature::TSMethodSignature(method) if names_key(&method.key, name) => {
            let written = method.return_type.as_ref().map(|annotation| &annotation.type_annotation);
            Some(match method.kind {
                TSMethodSignatureKind::Method => MemberDeclaration::Method(written),
                TSMethodSignatureKind::Get => MemberDeclaration::Typed(written),
                // A setter's parameter is what is assigned; reading the member never calls it, so it answers nothing.
                TSMethodSignatureKind::Set => return None,
            })
        }
        TSSignature::TSPropertySignature(_)
        | TSSignature::TSMethodSignature(_)
        | TSSignature::TSIndexSignature(_)
        | TSSignature::TSCallSignatureDeclaration(_)
        | TSSignature::TSConstructSignatureDeclaration(_) => None,
    })
}

fn member<'a, 'b>(ty: &'b TSType<'a>, name: &str) -> Member<'b, 'a> {
    match ty {
        TSType::TSParenthesizedType(inner) => member(&inner.type_annotation, name),
        TSType::TSTypeLiteral(literal) => {
            let mut all = declarations(literal, name);
            match (all.next(), all.next()) {
                (None, _) => Member::Unread,
                (Some(MemberDeclaration::Typed(Some(ty))), None) => Member::Property(ty),
                (Some(MemberDeclaration::Typed(None)), None) => Member::Unread,
                (Some(MemberDeclaration::Typed(_) | MemberDeclaration::Method(_)), Some(_))
                | (Some(MemberDeclaration::Method(_)), None) => Member::Declared(literal),
            }
        }
        TSType::TSAnyKeyword(_)
        | TSType::TSBigIntKeyword(_)
        | TSType::TSBooleanKeyword(_)
        | TSType::TSIntrinsicKeyword(_)
        | TSType::TSNeverKeyword(_)
        | TSType::TSNullKeyword(_)
        | TSType::TSNumberKeyword(_)
        | TSType::TSObjectKeyword(_)
        | TSType::TSStringKeyword(_)
        | TSType::TSSymbolKeyword(_)
        | TSType::TSUndefinedKeyword(_)
        | TSType::TSUnknownKeyword(_)
        | TSType::TSVoidKeyword(_)
        | TSType::TSArrayType(_)
        | TSType::TSConditionalType(_)
        | TSType::TSConstructorType(_)
        | TSType::TSFunctionType(_)
        | TSType::TSImportType(_)
        | TSType::TSIndexedAccessType(_)
        | TSType::TSInferType(_)
        | TSType::TSIntersectionType(_)
        | TSType::TSLiteralType(_)
        | TSType::TSMappedType(_)
        | TSType::TSNamedTupleMember(_)
        | TSType::TSTemplateLiteralType(_)
        | TSType::TSThisType(_)
        | TSType::TSTupleType(_)
        | TSType::TSTypeOperatorType(_)
        | TSType::TSTypePredicate(_)
        | TSType::TSTypeQuery(_)
        | TSType::TSTypeReference(_)
        | TSType::TSUnionType(_)
        | TSType::JSDocNullableType(_)
        | TSType::JSDocNonNullableType(_)
        | TSType::JSDocUnknownType(_) => Member::Unread,
    }
}

/// What calling or constructing `expr`, a member read from a namespace asserted to `ty`, is.
fn member_evidence<'a, 'b>(
    ty: &'b TSType<'a>,
    name: &str,
    expr: &Expression<'_>,
    signatures: Signatures,
) -> Evidence<'b, 'a> {
    match member(ty, name) {
        Member::Property(ty) => asserted(ty, expr, signatures),
        // The member has every type declared for it: no component only when each declaration says so.
        Member::Declared(literal) => combine(declarations(literal, name).map(|declaration| match declaration {
            MemberDeclaration::Typed(Some(ty)) => asserted(ty, expr, signatures),
            MemberDeclaration::Typed(None) => Evidence::Unknown,
            // A method declares a call signature, which says nothing about constructing the member.
            MemberDeclaration::Method(result) => match signatures {
                Signatures::Call => results(std::iter::once(result)),
                Signatures::Construct => Evidence::Unknown,
            },
        })),
        Member::Unread => Evidence::Unknown,
    }
}

/// What asserting `ty` on `beneath` says about calling or constructing it, as `signatures` says the site does. One
/// exhaustive match: a new type kind in an OXC release has to be placed here before it compiles.
fn asserted<'a, 'b>(ty: &'b TSType<'a>, beneath: &Expression<'_>, signatures: Signatures) -> Evidence<'b, 'a> {
    let result = |annotation: &'b TSTypeAnnotation<'a>| Some(&annotation.type_annotation);
    match ty {
        TSType::TSAnyKeyword(_) | TSType::TSUnknownKeyword(_) => Evidence::Identity,
        TSType::TSParenthesizedType(inner) => asserted(&inner.type_annotation, beneath, signatures),
        TSType::TSTypeQuery(query) if query.type_arguments.is_none() && query_names(&query.expr_name, beneath) => {
            Evidence::Identity
        }
        TSType::TSTypeReference(reference) => Evidence::Reference(reference),
        // A function type has only a call signature, a constructor type only a construct signature.
        TSType::TSFunctionType(function) => match signatures {
            Signatures::Call => results(std::iter::once(result(&function.return_type))),
            Signatures::Construct => Evidence::Unknown,
        },
        TSType::TSConstructorType(constructor) => match signatures {
            Signatures::Construct => results(std::iter::once(result(&constructor.return_type))),
            Signatures::Call => Evidence::Unknown,
        },
        TSType::TSTypeLiteral(literal) => {
            results(literal.members.iter().filter_map(|signature| match (signature, signatures) {
                (TSSignature::TSCallSignatureDeclaration(call), Signatures::Call) => {
                    Some(call.return_type.as_deref().and_then(result))
                }
                (TSSignature::TSConstructSignatureDeclaration(construct), Signatures::Construct) => {
                    Some(construct.return_type.as_deref().and_then(result))
                }
                (TSSignature::TSCallSignatureDeclaration(_), Signatures::Construct)
                | (TSSignature::TSConstructSignatureDeclaration(_), Signatures::Call)
                | (
                    TSSignature::TSPropertySignature(_)
                    | TSSignature::TSMethodSignature(_)
                    | TSSignature::TSIndexSignature(_),
                    Signatures::Call | Signatures::Construct,
                ) => None,
            }))
        }
        TSType::TSTypeQuery(_)
        | TSType::TSBigIntKeyword(_)
        | TSType::TSBooleanKeyword(_)
        | TSType::TSIntrinsicKeyword(_)
        | TSType::TSNeverKeyword(_)
        | TSType::TSNullKeyword(_)
        | TSType::TSNumberKeyword(_)
        | TSType::TSObjectKeyword(_)
        | TSType::TSStringKeyword(_)
        | TSType::TSSymbolKeyword(_)
        | TSType::TSUndefinedKeyword(_)
        | TSType::TSVoidKeyword(_)
        | TSType::TSArrayType(_)
        | TSType::TSConditionalType(_)
        | TSType::TSImportType(_)
        | TSType::TSIndexedAccessType(_)
        | TSType::TSInferType(_)
        | TSType::TSIntersectionType(_)
        | TSType::TSLiteralType(_)
        | TSType::TSMappedType(_)
        | TSType::TSNamedTupleMember(_)
        | TSType::TSTemplateLiteralType(_)
        | TSType::TSThisType(_)
        | TSType::TSTupleType(_)
        | TSType::TSTypeOperatorType(_)
        | TSType::TSTypePredicate(_)
        | TSType::TSUnionType(_)
        | TSType::JSDocNullableType(_)
        | TSType::JSDocNonNullableType(_)
        | TSType::JSDocUnknownType(_) => Evidence::Unknown,
    }
}

/// The evidence of a type's call signatures, given their declared results (`None` when one declares none): see
/// [`combine`].
fn results<'a, 'b>(signatures: impl Iterator<Item = Option<&'b TSType<'a>>>) -> Evidence<'b, 'a> {
    combine(signatures.map(|result| match result {
        Some(result) if never_renders(result) => Evidence::NotRenderable,
        Some(_) | None => Evidence::Unknown,
    }))
}

/// The evidence of several signatures or declarations of one callee, each read alone: [`Evidence::NotRenderable`] when
/// there is one and every one is, [`Evidence::Unknown`] otherwise. One that may give a node, an element or a component
/// keeps the whole callee a candidate.
fn combine<'a, 'b>(all: impl Iterator<Item = Evidence<'b, 'a>>) -> Evidence<'b, 'a> {
    let mut any = false;
    for evidence in all {
        match evidence {
            Evidence::NotRenderable => any = true,
            Evidence::Identity | Evidence::Reference(_) | Evidence::Unknown => return Evidence::Unknown,
        }
    }
    if any { Evidence::NotRenderable } else { Evidence::Unknown }
}

/// Whether a call's declared result is a type TypeScript cannot assign to `ReactNode`, `ComponentType` or
/// `ReactElement` under any instantiation, read from the type's own syntax with nothing resolved. This closed list is
/// the only evidence an assertion gives against a call ([`Evidence::NotRenderable`]): every other type, however it is
/// spelled, may be a node, an element or a component. One exhaustive match with no catch-all arm, so a new type node
/// kind in an OXC release does not compile until it is placed. Syntax cannot see a project that augments React's
/// `DO_NOT_USE_OR_YOU_WILL_BE_FIRED_EXPERIMENTAL_REACT_NODES` with one of these types.
fn never_renders(ty: &TSType<'_>) -> bool {
    match ty {
        // A non-primitive with no members: no member of `ReactNode` (`ReactElement | string | number | bigint |
        // Iterable<ReactNode> | ReactPortal | boolean | null | undefined`), not callable, and not an element.
        TSType::TSObjectKeyword(_) => true,
        // No member of `ReactNode`, not callable, not an object.
        TSType::TSSymbolKeyword(_) => true,
        // Assignable only to `void`, `any` and `unknown`, not even to `undefined`.
        TSType::TSVoidKeyword(_) => true,
        // Syntax only: `(object)` is `object`.
        TSType::TSParenthesizedType(inner) => never_renders(&inner.type_annotation),
        // A member of `ReactNode` since @types/react 18.3.
        TSType::TSBigIntKeyword(_) => false,
        // Members of `ReactNode`, which a component may return.
        TSType::TSStringKeyword(_)
        | TSType::TSNumberKeyword(_)
        | TSType::TSBooleanKeyword(_)
        | TSType::TSNullKeyword(_)
        | TSType::TSUndefinedKeyword(_)
        | TSType::TSLiteralType(_)
        | TSType::TSTemplateLiteralType(_) => false,
        // A type predicate's result is a `boolean`.
        TSType::TSTypePredicate(_) => false,
        // `any` and `never` are assignable to every type, and `unknown` only says nothing is known.
        TSType::TSAnyKeyword(_) | TSType::TSUnknownKeyword(_) | TSType::TSNeverKeyword(_) => false,
        // `Iterable<ReactNode>` takes an array or tuple of nodes, and an `any[]` is one.
        TSType::TSArrayType(_) | TSType::TSTupleType(_) | TSType::TSNamedTupleMember(_) => false,
        // `ReactElement` is an object type (`{ type; props; key }`), and a callable one may be a component.
        TSType::TSTypeLiteral(_) => false,
        // A component type.
        TSType::TSFunctionType(_) | TSType::TSConstructorType(_) => false,
        // Not read member by member: a member outside this list may render, and an intersection may narrow `object` to
        // an element (`object & ReactElement`).
        TSType::TSUnionType(_) | TSType::TSIntersectionType(_) => false,
        // A type only resolution can see, which may be any of the above.
        TSType::TSTypeReference(_)
        | TSType::TSTypeQuery(_)
        | TSType::TSImportType(_)
        | TSType::TSIndexedAccessType(_)
        | TSType::TSConditionalType(_)
        | TSType::TSMappedType(_)
        | TSType::TSInferType(_)
        | TSType::TSThisType(_)
        | TSType::TSTypeOperatorType(_)
        | TSType::TSIntrinsicKeyword(_) => false,
        // JSDoc types, which TypeScript reads as `any` or as the type they wrap.
        TSType::JSDocNullableType(_) | TSType::JSDocNonNullableType(_) | TSType::JSDocUnknownType(_) => false,
    }
}

/// Whether `typeof name` is the type of `expr`: the same identifier or the same chain of static members, read through
/// the namespaces' wrappers (the outermost assertion states the type whatever the namespace was asserted to).
fn query_names(name: &TSTypeQueryExprName<'_>, expr: &Expression<'_>) -> bool {
    match name {
        TSTypeQueryExprName::IdentifierReference(id) => matches!(expr, Expression::Identifier(e) if e.name == id.name),
        TSTypeQueryExprName::QualifiedName(qualified) => type_name_names(&qualified.left, &qualified.right, expr),
        TSTypeQueryExprName::ThisExpression(_) => matches!(expr, Expression::ThisExpression(_)),
        TSTypeQueryExprName::TSImportType(_) => false,
    }
}

fn type_name_names(left: &TSTypeName<'_>, right: &IdentifierName<'_>, expr: &Expression<'_>) -> bool {
    let Expression::StaticMemberExpression(member) = expr else { return false };
    if member.property.name != right.name {
        return false;
    }
    let object = peel_expr(&member.object);
    match left {
        TSTypeName::IdentifierReference(id) => matches!(object, Expression::Identifier(e) if e.name == id.name),
        TSTypeName::QualifiedName(qualified) => type_name_names(&qualified.left, &qualified.right, object),
        TSTypeName::ThisExpression(_) => matches!(object, Expression::ThisExpression(_)),
    }
}

/// Whether a dotted chain (`createButton`, `React.forwardRef`) names `expr`, read through the namespaces' wrappers.
fn chain_names(chain: &str, expr: &Expression<'_>) -> bool {
    match (chain.rsplit_once('.'), expr) {
        (None, Expression::Identifier(id)) => id.name == chain,
        (Some((left, right)), Expression::StaticMemberExpression(member)) => {
            member.property.name == right && chain_names(left, peel_expr(&member.object))
        }
        (None | Some(_), _) => false,
    }
}

/// A module-level `type Factory = typeof createButton`: the dotted chain it queries, and the scopes in which a
/// declaration of the same name shadows it.
pub(crate) struct TypeofAlias {
    pub(super) chain: CompactString,
    /// The shadowing scopes, sorted and with nested ones merged into the scope around them: disjoint, as scopes nest
    /// or are apart and never partly overlap.
    shadowed_in: Vec<oxc_span::Span>,
}

impl TypeofAlias {
    fn new(chain: &str, mut shadowed_in: Vec<oxc_span::Span>) -> Self {
        shadowed_in.sort_unstable_by_key(|scope| (scope.start, std::cmp::Reverse(scope.end)));
        let mut merged: Vec<oxc_span::Span> = Vec::with_capacity(shadowed_in.len());
        for scope in shadowed_in {
            match merged.last_mut() {
                Some(last) if scope.start < last.end => last.end = last.end.max(scope.end),
                Some(_) | None => merged.push(scope),
            }
        }
        Self { chain: CompactString::from(chain), shadowed_in: merged }
    }

    /// Whether a reference to the alias at `at` means it: no scope around `at` declares the name again. A binary
    /// search for the last scope that starts at or before `at`.
    pub(super) fn means_at(&self, at: oxc_span::Span) -> bool {
        let (mut low, mut high) = (0, self.shadowed_in.len());
        while low < high {
            #[cfg(test)]
            steps::count_probed();
            let middle = low + (high - low) / 2;
            if self.shadowed_in[middle].start <= at.start {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        !low.checked_sub(1).is_some_and(|index| at.end <= self.shadowed_in[index].end)
    }
}

/// Every declaration in type space of one of `names`, from one walk of the module: the one list of what declares a
/// type name. A declaration outside every scope is the module's own and is counted; one inside a scope shadows the
/// module's declaration there. Scopes: a function's or arrow's type parameters cover the whole function, its body's
/// declarations only the body (so they do not reach a parameter default); a class, a block (a catch clause's
/// included), a `switch`, a static block, a namespace and a `declare module`/`declare global` body. A type holds no
/// expression, so nothing declared in one (an alias's or interface's type parameters, `infer`, a mapped key) encloses
/// a cast.
struct TypeDeclarations<'n> {
    /// The candidate of each name the module declares one `typeof` alias of, so a declaration visits only the alias
    /// it can shadow.
    slots: FxHashMap<&'n str, usize>,
    scopes: Vec<oxc_span::Span>,
    /// Per candidate name, how many declarations the module itself makes.
    module_level: Vec<usize>,
    /// Per candidate name, the scopes that declare it again.
    shadowed_in: Vec<Vec<oxc_span::Span>>,
}

impl TypeDeclarations<'_> {
    /// Records a declaration of `name` in `scope`, or in the innermost open scope when `None`.
    fn declare(&mut self, name: &str, scope: Option<oxc_span::Span>) {
        let Some(&index) = self.slots.get(name) else {
            #[cfg(test)]
            steps::count_examined(1);
            return;
        };
        #[cfg(test)]
        steps::count_examined(2);
        match scope.or_else(|| self.scopes.last().copied()) {
            None => self.module_level[index] += 1,
            Some(scope) => self.shadowed_in[index].push(scope),
        }
    }

    fn type_parameters(&mut self, declared: Option<&TSTypeParameterDeclaration<'_>>, scope: oxc_span::Span) {
        for parameter in declared.into_iter().flat_map(|declared| &declared.params) {
            self.declare(&parameter.name.name, Some(scope));
        }
    }

    fn within(&mut self, scope: oxc_span::Span, walk: impl FnOnce(&mut Self)) {
        self.scopes.push(scope);
        walk(self);
        self.scopes.pop();
    }
}

impl<'a> oxc_ast_visit::Visit<'a> for TypeDeclarations<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        // Every import binds its name in type space too (a value import may be a class or a namespace).
        for specifier in it.specifiers.iter().flatten() {
            self.declare(&specifier.local().name, None);
        }
    }

    fn visit_declaration(&mut self, it: &Declaration<'a>) {
        let declared = match it {
            Declaration::ClassDeclaration(class) => class.id.as_ref().map(|id| &id.name),
            Declaration::TSTypeAliasDeclaration(alias) => Some(&alias.id.name),
            Declaration::TSInterfaceDeclaration(interface) => Some(&interface.id.name),
            Declaration::TSEnumDeclaration(declaration) => Some(&declaration.id.name),
            Declaration::TSNamespaceDeclaration(namespace) => Some(&namespace.id.name),
            // `import F = require('f')`, `import type F = require('f')` and `import F = N.M`.
            Declaration::TSImportEqualsDeclaration(import) => Some(&import.id.name),
            // Values, and modules named by a string or the global scope: no type name in this module's scope.
            Declaration::VariableDeclaration(_)
            | Declaration::FunctionDeclaration(_)
            | Declaration::TSExternalModuleDeclaration(_)
            | Declaration::TSGlobalDeclaration(_) => None,
        };
        if let Some(declared) = declared {
            self.declare(declared, None);
        }
        oxc_ast_visit::walk::walk_declaration(self, it);
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        let declared = match &it.declaration {
            ExportDefaultDeclarationKind::ClassDeclaration(class) => class.id.as_ref().map(|id| &id.name),
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(interface) => Some(&interface.id.name),
            // A function or an expression is a value.
            ExportDefaultDeclarationKind::FunctionDeclaration(_) => None,
            match_expression!(ExportDefaultDeclarationKind) => None,
        };
        if let Some(declared) = declared {
            self.declare(declared, None);
        }
        oxc_ast_visit::walk::walk_export_default_declaration(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        self.type_parameters(it.type_parameters.as_deref(), it.span);
        oxc_ast_visit::walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.type_parameters(it.type_parameters.as_deref(), it.span);
        oxc_ast_visit::walk::walk_arrow_function_expression(self, it);
    }

    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_function_body(this, it));
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        // A class declaration's name is declared where the declaration is (`visit_declaration`); a class expression's
        // only inside the class.
        if let (Some(id), false) = (&it.id, it.is_declaration()) {
            self.declare(&id.name, Some(it.span));
        }
        self.type_parameters(it.type_parameters.as_deref(), it.span);
        self.within(it.span, |this| oxc_ast_visit::walk::walk_class(this, it));
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_block_statement(this, it));
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_switch_statement(this, it));
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_static_block(this, it));
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_ts_namespace_declaration(this, it));
    }

    fn visit_ts_external_module_declaration(&mut self, it: &TSExternalModuleDeclaration<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_ts_external_module_declaration(this, it));
    }

    fn visit_ts_global_declaration(&mut self, it: &TSGlobalDeclaration<'a>) {
        self.within(it.span, |this| oxc_ast_visit::walk::walk_ts_global_declaration(this, it));
    }

    fn visit_ts_type(&mut self, _it: &TSType<'a>) {}
}

/// The module-level aliases of a value's type, `type Factory = typeof createButton`, by alias name. Only a name the
/// module declares once in type space is kept: an import or an interface of the same name may be what a use means. A
/// declaration of the name in an inner scope shadows the alias only inside that scope ([`TypeofAlias::means_at`]).
/// The module is walked once ([`TypeDeclarations`]), and only when it has such an alias.
pub(super) fn typeof_aliases(program: &Program<'_>) -> FxHashMap<CompactString, TypeofAlias> {
    /// `A.B.C` as one dotted string, written into one buffer left to right, so a long chain costs its length.
    fn chain(name: &TSTypeName<'_>, last: Option<&str>) -> Option<String> {
        let mut segments: Vec<&str> = last.into_iter().collect();
        let mut current = name;
        loop {
            match current {
                TSTypeName::IdentifierReference(id) => {
                    segments.push(id.name.as_str());
                    break;
                }
                TSTypeName::QualifiedName(qualified) => {
                    segments.push(qualified.right.name.as_str());
                    current = &qualified.left;
                }
                TSTypeName::ThisExpression(_) => return None,
            }
        }
        let mut out = String::with_capacity(segments.iter().map(|segment| segment.len() + 1).sum());
        for (index, segment) in segments.iter().rev().enumerate() {
            if index > 0 {
                out.push('.');
            }
            out.push_str(segment);
            #[cfg(test)]
            steps::count_written(segment.len() as u64 + 1);
        }
        Some(out)
    }
    let mut queries: Vec<(&str, &TSTypeQuery<'_>)> = Vec::new();
    for statement in &program.body {
        let declaration = match statement {
            Statement::ExportDeclaration(export) => Some(&export.declaration),
            _ => statement.as_declaration(),
        };
        let Some(Declaration::TSTypeAliasDeclaration(declaration)) = declaration else { continue };
        let mut ty = &declaration.type_annotation;
        while let TSType::TSParenthesizedType(inner) = ty {
            ty = &inner.type_annotation;
        }
        let TSType::TSTypeQuery(query) = ty else { continue };
        if declaration.type_parameters.is_some() || query.type_arguments.is_some() {
            continue;
        }
        queries.push((declaration.id.name.as_str(), query));
    }
    // A name with two `typeof` aliases is ambiguous whatever else the module declares, so it is dropped here, counted
    // and nothing more: no queried name is built for it and the walk keeps no slot for it.
    let mut aliases_of: FxHashMap<&str, usize> = FxHashMap::default();
    for (name, _) in &queries {
        *aliases_of.entry(*name).or_default() += 1;
    }
    let mut candidates: Vec<(&str, String)> = Vec::new();
    for (name, query) in queries {
        if aliases_of[name] != 1 {
            continue;
        }
        let queried = match &query.expr_name {
            TSTypeQueryExprName::IdentifierReference(id) => Some(id.name.as_str().to_owned()),
            TSTypeQueryExprName::QualifiedName(qualified) => {
                chain(&qualified.left, Some(qualified.right.name.as_str()))
            }
            TSTypeQueryExprName::ThisExpression(_) | TSTypeQueryExprName::TSImportType(_) => None,
        };
        if let Some(queried) = queried {
            candidates.push((name, queried));
        }
    }
    if candidates.is_empty() {
        return FxHashMap::default();
    }
    #[cfg(test)]
    steps::count_walk();
    let slots: FxHashMap<&str, usize> =
        candidates.iter().enumerate().map(|(index, (name, _))| (*name, index)).collect();
    let mut declarations = TypeDeclarations {
        slots,
        scopes: Vec::new(),
        module_level: vec![0; candidates.len()],
        shadowed_in: vec![Vec::new(); candidates.len()],
    };
    oxc_ast_visit::Visit::visit_program(&mut declarations, program);
    let TypeDeclarations { module_level, shadowed_in, .. } = declarations;
    candidates
        .iter()
        .zip(module_level)
        .zip(shadowed_in)
        .filter(|((_, module_level), _)| *module_level == 1)
        .map(|(((name, queried), _), shadowed_in)| (CompactString::from(*name), TypeofAlias::new(queried, shadowed_in)))
        .collect()
}

/// Counts the wrapper layers `peel` reads, so a test can show that a chain of assertions is read in one pass.
#[cfg(test)]
pub(super) mod steps {
    use std::cell::Cell;

    thread_local!(static STEPS: Cell<u64> = const { Cell::new(0) });

    pub(in crate::extractor) fn count() {
        STEPS.with(|steps| steps.set(steps.get() + 1));
    }

    pub(in crate::extractor) fn take() -> u64 {
        STEPS.with(|steps| steps.replace(0))
    }

    thread_local!(static WALKS: Cell<u64> = const { Cell::new(0) });

    /// Counts the walks `typeof_aliases` makes of a whole module, so a test can show a module without an alias makes
    /// none.
    pub(in crate::extractor) fn count_walk() {
        WALKS.with(|walks| walks.set(walks.get() + 1));
    }

    pub(in crate::extractor) fn take_walks() -> u64 {
        WALKS.with(|walks| walks.replace(0))
    }

    thread_local!(static EXAMINED: Cell<u64> = const { Cell::new(0) });

    /// Counts the name lookups and candidate slots a declaration examines in `typeof_aliases`'s walk, so a test can
    /// show the work grows with the declarations, not with declarations times aliases.
    pub(in crate::extractor) fn count_examined(n: u64) {
        EXAMINED.with(|examined| examined.set(examined.get() + n));
    }

    pub(in crate::extractor) fn take_examined() -> u64 {
        EXAMINED.with(|examined| examined.replace(0))
    }

    thread_local!(static WRITTEN: Cell<u64> = const { Cell::new(0) });

    /// Counts the bytes `typeof_aliases` writes to build a queried chain's name.
    pub(in crate::extractor) fn count_written(n: u64) {
        WRITTEN.with(|written| written.set(written.get() + n));
    }

    pub(in crate::extractor) fn take_written() -> u64 {
        WRITTEN.with(|written| written.replace(0))
    }

    thread_local!(static PROBED: Cell<u64> = const { Cell::new(0) });

    /// Counts the shadowing scopes `TypeofAlias::means_at` probes.
    pub(in crate::extractor) fn count_probed() {
        PROBED.with(|probed| probed.set(probed.get() + 1));
    }

    pub(in crate::extractor) fn take_probed() -> u64 {
        PROBED.with(|probed| probed.replace(0))
    }
}

/// A value read only for its name (the namespace a member is read from, a link of a heritage chain, a hook's callee),
/// beneath every wrapper. No evidence is read: what calling or constructing `ns.member` gives is the member's evidence,
/// which [`peel`] of the member reads from the namespace's assertion.
pub(super) fn beneath<'a, 'b>(expr: &'b Expression<'a>) -> &'b Expression<'a> {
    layers(expr).0
}

/// What a call invokes, when its name still describes the call: `(forwardRef as F)(…)` and `(forwardRef<R, P>)(…)`
/// call `forwardRef`; `(Object as (v: object) => object)(…)` calls nothing by a name. See [`Peeled::named`].
pub(super) fn callee_of<'a, 'b>(call: &'b CallExpression<'a>) -> Option<&'b Expression<'a>> {
    called(call).named()
}

/// A call's callee, peeled for its call signatures.
pub(super) fn called<'a, 'b>(call: &'b CallExpression<'a>) -> Peeled<'b, 'a> {
    peel(&call.callee, Signatures::Call)
}

/// The type arguments a call is made with: its own, or those of the instantiation it calls (`(forwardRef<R, P>)(…)`).
pub(super) fn call_type_arguments<'a, 'b>(
    call: &'b CallExpression<'a>,
) -> Option<&'b TSTypeParameterInstantiation<'a>> {
    call.type_arguments.as_deref().or_else(|| layers(&call.callee).1)
}

/// What a class extends and the type arguments it extends it with: `extends React.Component<P>` and
/// `extends (React.Component<P>)` both give `React.Component` and `P`. The class constructs its base, so an assertion
/// on it is read for its construct signatures.
pub(super) fn super_class<'a, 'b>(class: &'b Class<'a>) -> Option<Peeled<'b, 'a>> {
    let heritage = class.heritage.as_ref()?;
    let peeled = peel(&heritage.expression, Signatures::Construct);
    Some(Peeled { type_arguments: heritage.type_arguments.as_deref().or(peeled.type_arguments), ..peeled })
}

/// What a call wrapping an aliased component is, as read from the call before the component is known.
#[derive(Clone, Copy)]
pub(crate) struct AliasCall {
    /// The call follows the component-wrapper conventions (`memo`, `withX(...)`, `forwardRef`, ...).
    wrapper: bool,
    forward_ref: bool,
}

/// What `const Button = Inner` or `memo(Inner)` re-binds, read before `Inner` needs to be known.
pub(crate) struct AliasShape {
    inner: String,
    call: Option<AliasCall>,
}

/// An alias of a component that was not collected yet when the alias was read.
pub(crate) struct PendingAlias {
    name: String,
    shape: AliasShape,
    span: oxc_span::Span,
}

enum AliasOutcome {
    Done,
    /// The aliased component is known but the call around it is not a component wrapper; the reason is the skip's message.
    Declined(String),
    NoBase,
}

/// What the component rules read from a binding: its initializer, span and type annotation. Built from a
/// `const X = ...` declarator or from an `export default ...` expression, so both go through the same rules.
#[derive(Clone, Copy)]
pub(super) struct Binding<'b, 'a> {
    pub(super) init: Option<&'b Expression<'a>>,
    pub(super) span: oxc_span::Span,
    pub(super) annotation: Option<&'b TSTypeAnnotation<'a>>,
}

impl<'b, 'a> Binding<'b, 'a> {
    pub(super) fn of(decl: &'b VariableDeclarator<'a>) -> Self {
        Self { init: decl.init.as_ref(), span: decl.span, annotation: decl.type_annotation.as_deref() }
    }
}

/// Whether `bare` is a React export that types a function component: the shared family plus the forwardRef
/// component type. Used by `extract_props_from_type_annotation` (which additionally requires the props argument
/// itself to resolve) and `type_annotation_is_fc_family` below (which only needs to know the annotation *looked
/// like* one of these) so the two checks can't drift apart.
fn is_fc_family_type(bare: &str) -> bool {
    crate::react_types::FC_FAMILY_TYPE_NAMES.contains(&bare) || bare == "ForwardRefComponent"
}

/// PascalCase a file's stem for naming an anonymous default-exported
/// component that has no identifier of its own — e.g. `button-group.tsx` →
/// `ButtonGroup`, `useButton.tsx` → `UseButton`. Splits on any non-alphanumeric
/// separator (`-`, `_`, `.`, space) and capitalizes each segment's first
/// character. Returns `None` when the path has no usable stem at all (e.g.
/// nothing but separators).
pub(super) fn pascal_case_from_file_stem(file_path: &Utf8Path) -> Option<String> {
    let stem = file_path.file_stem()?;
    let mut pascal = String::new();
    for segment in stem.split(|c: char| !c.is_alphanumeric()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            pascal.extend(first.to_uppercase());
            pascal.push_str(chars.as_str());
        }
    }
    (!pascal.is_empty()).then_some(pascal)
}

impl<'src> SourceDataCollector<'src> {
    /// An asserted React component alias can replace the inner component's props before the alias is exported.
    pub(super) fn try_record_cast_component_alias<'a>(&mut self, decl: &VariableDeclarator<'a>, name: &str) {
        let Some(Expression::TSAsExpression(assertion)) = decl.init.as_ref() else { return };
        let Expression::Identifier(inner) = assertion.expression.without_parentheses() else { return };
        let has_base = self.data.component_mappings.iter().any(|mapping| mapping.is_binding_named(inner.name.as_str()));
        if !has_base {
            return;
        }
        let TSType::TSTypeReference(reference) = &assertion.type_annotation else { return };
        let type_name = self.extract_type_ref_name(reference);
        let Some(bare) = self.react_type_export(&type_name) else { return };
        if bare != "ForwardRefExoticComponent" && !is_fc_family_type(&bare) {
            return;
        }
        let Some(props) = reference.type_arguments.as_ref().and_then(|args| args.params.first()) else { return };
        let alias_name = format!("__{name}CastProps{}", decl.span.start);
        let alias_key = self.scoped_key(&alias_name);
        let target = self.ts_type_to_collected(props);
        self.data
            .type_aliases
            .insert(alias_key, CollectedTypeAlias::Passthrough { target, file_path: self.file_path.clone() });
        self.cast_component_aliases.insert(name.into(), (inner.name.as_str().to_owned(), alias_name.into()));
    }

    // ─── Component detection helpers ──────────────────────────────────────────

    /// Try to extract a PascalCase name from a VariableDeclarator's binding.
    pub(super) fn extract_pascal_name<'a>(&self, decl: &VariableDeclarator<'a>) -> Option<String> {
        match &decl.id {
            BindingPattern::BindingIdentifier(id) => {
                let name = id.name.as_str();
                if is_pascal_case(name) { Some(name.to_owned()) } else { None }
            }
            _ => None,
        }
    }

    /// The ordered rules for a binding whose initializer may be a component; the first that matches wins.
    pub(super) fn detect_component<'a>(&mut self, binding: &Binding<'_, 'a>, name: &str) -> Option<ComponentMapping> {
        self.try_fc_annotation(binding, name)
            .or_else(|| self.try_forward_ref(binding, name))
            .or_else(|| self.try_hoc_wrapped(binding, name))
            .or_else(|| self.try_class_expression_wrapped(binding, name))
            .or_else(|| self.try_typed_function_expression(binding, name))
    }

    /// Whether the callee's declaration describes the call, so `resolver::callee` may read it: the evidence is the
    /// identity, or a reference to a module-level `type F = typeof <the callee>` that the module declares once and no
    /// scope around the cast declares again ([`typeof_aliases`]). Any other assertion may hide what the declaration
    /// says (`(wrap as <T>(r: FC<T>) => FC<T>)<P>(…)` drops the props `wrap`'s result adds).
    pub(super) fn describes_call(&self, peeled: &Peeled<'_, '_>) -> bool {
        match peeled.evidence {
            Evidence::Identity => true,
            Evidence::Reference(reference) => {
                let alias = match &reference.type_name {
                    TSTypeName::IdentifierReference(alias) => self.typeof_aliases.get(alias.name.as_str()),
                    TSTypeName::QualifiedName(_) | TSTypeName::ThisExpression(_) => None,
                };
                reference.type_arguments.is_none()
                    && alias
                        .is_some_and(|alias| alias.means_at(reference.span) && chain_names(&alias.chain, peeled.expr))
            }
            Evidence::NotRenderable | Evidence::Unknown => false,
        }
    }

    /// Try to detect: `const Button: FC<ButtonProps> = ({ variant = 'primary' }) => ...`
    pub(super) fn try_fc_annotation<'a>(&mut self, decl: &Binding<'_, 'a>, name: &str) -> Option<ComponentMapping> {
        let type_ann = decl.annotation?;
        let init_params = match decl.init.map(peel_expr) {
            Some(Expression::ArrowFunctionExpression(afe)) => Some(&*afe.params),
            Some(Expression::FunctionExpression(fe)) => Some(&*fe.params),
            _ => None,
        };
        self.extract_props_from_type_annotation(
            &type_ann.type_annotation,
            name,
            decl.span.start,
            decl.span.end,
            init_params,
        )
    }

    pub(super) fn extract_props_from_type_annotation<'a>(
        &mut self,
        ty: &TSType<'a>,
        name: &str,
        span_start: u32,
        span_end: u32,
        init_params: Option<&FormalParameters<'a>>,
    ) -> Option<ComponentMapping> {
        match ty {
            TSType::TSTypeReference(tr) => {
                let type_name = self.extract_type_ref_name(tr);
                if !self.react_type_export(&type_name).is_some_and(|bare| is_fc_family_type(&bare)) {
                    return None;
                }
                let (props_name, type_args) = self.extract_props_arg(&tr.type_arguments)?;
                let (description, tags) = self.find_jsdoc_with_tags(span_start);
                Some(
                    ComponentMapping::new(
                        name.to_owned(),
                        props_name,
                        self.file_path.clone(),
                        span_start,
                        span_end,
                        DetectionRule::FcAnnotation,
                    )
                    .with_props_type_args(type_args)
                    .with_doc(description, tags)
                    .with_param_defaults(init_params.map(|p| self.extract_param_defaults(p)).unwrap_or_default()),
                )
            }
            TSType::TSParenthesizedType(p) => {
                self.extract_props_from_type_annotation(&p.type_annotation, name, span_start, span_end, init_params)
            }
            // `FC<DrawerProps> & { Panel: typeof Panel }`: the function-component member carries the props, the
            // other members are static properties attached to it.
            TSType::TSIntersectionType(members) => members.types.iter().find_map(|member| {
                self.extract_props_from_type_annotation(member, name, span_start, span_end, init_params)
            }),
            _ => None,
        }
    }

    /// Try to detect: `const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(...)`
    pub(super) fn try_forward_ref<'a>(&mut self, decl: &Binding<'_, 'a>, name: &str) -> Option<ComponentMapping> {
        let init = decl.init.as_ref()?;
        let call = match peel_expr(init) {
            Expression::CallExpression(ce) => ce,
            _ => return None,
        };
        if !self.is_forward_ref_call(call) {
            return None;
        }

        // forwardRef<RefType, PropsType>(fn) — PropsType is the second type parameter
        let type_params = call_type_arguments(call)?;
        if type_params.params.len() < 2 {
            return None;
        }
        let props_type = &type_params.params[1];
        let (props_name, type_args) = self.extract_type_name_from_type(props_type)?;

        // The render function is forwardRef's call argument — its first parameter is
        // props (the second is `ref`, which never carries destructured defaults we care about).
        let render_params = match call.arguments.first() {
            Some(Argument::FunctionExpression(fe)) => Some(&fe.params),
            Some(Argument::ArrowFunctionExpression(afe)) => Some(&afe.params),
            _ => None,
        };
        let param_defaults = render_params.map(|p| self.extract_param_defaults(p)).unwrap_or_default();
        let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);

        Some(
            ComponentMapping::new(
                name.to_owned(),
                props_name,
                self.file_path.clone(),
                decl.span.start,
                decl.span.end,
                DetectionRule::ForwardRef,
            )
            .with_props_type_args(type_args)
            .with_doc(description, tags)
            .with_param_defaults(param_defaults),
        )
    }

    /// Try to detect: `const Button = ({ variant }: ButtonProps) => ...` and the `function` expression form. The
    /// function must show it renders (see `evidence`): a typed first parameter alone also describes helpers such as
    /// `FormatName = (u: User) => string`. A first parameter that is not a named type is not a props object.
    pub(super) fn try_typed_function_expression<'a>(
        &mut self,
        decl: &Binding<'_, 'a>,
        name: &str,
    ) -> Option<ComponentMapping> {
        let (params, shape) = match peel_expr(decl.init.as_ref()?) {
            Expression::ArrowFunctionExpression(afe) => (&*afe.params, FunctionShape::of_arrow(afe)),
            Expression::FunctionExpression(fe) => (&*fe.params, FunctionShape::of_function(fe)),
            _ => return None,
        };
        let type_ann = params.items.first()?.type_annotation.as_ref()?;
        if let Err(reason) = renders(&shape, &*self) {
            self.decline_reason = Some(format!("'{name}' takes a typed props parameter but {reason}"));
            return None;
        }
        let (props_name, type_args) = self.extract_type_name_from_type(&type_ann.type_annotation)?;
        let param_defaults = self.extract_param_defaults(params);
        let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);
        Some(
            ComponentMapping::new(
                name.to_owned(),
                props_name,
                self.file_path.clone(),
                decl.span.start,
                decl.span.end,
                DetectionRule::TypedFunctionExpression,
            )
            .with_props_type_args(type_args)
            .with_doc(description, tags)
            .with_param_defaults(param_defaults),
        )
    }

    /// Try to detect: `const Button = anyHOC(function Button(props: ButtonProps) {...})`
    pub(super) fn try_hoc_wrapped<'a>(&mut self, decl: &Binding<'_, 'a>, name: &str) -> Option<ComponentMapping> {
        let init = decl.init.as_ref()?;
        let call = match peel_expr(init) {
            Expression::CallExpression(ce) => ce,
            _ => return None,
        };

        // `(hoc as (c: unknown) => object)(…)`: the assertion says what the call returns, and it is no component.
        callee_of(call)?;
        // First arg should be a function with a typed props param — or a forwardRef() call.
        let first_arg = call.arguments.first()?;

        // `memo(forwardRef(…))`: the props come from forwardRef's type arguments, or from its render function.
        if let Argument::CallExpression(inner) = first_arg {
            if !self.is_forward_ref_call(inner) {
                return None;
            }
            // `forwardRef<Ref, Props>(…)` names the props in its second type argument; otherwise the render function's
            // first parameter does. A call with one type argument and no function (`forwardRef<Ref>()`) names neither.
            let props_type = call_type_arguments(inner).and_then(|type_params| type_params.params.get(1));
            let Some(props_type) = props_type else {
                let render = inner.arguments.first()?;
                return self.mapping_from_typed_function(decl, name, call, render, DetectionRule::ForwardRef);
            };
            let (props_name, type_args) = self.extract_type_name_from_type(props_type)?;
            let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);
            return Some(
                ComponentMapping::new(
                    name.to_owned(),
                    props_name,
                    self.file_path.clone(),
                    decl.span.start,
                    decl.span.end,
                    DetectionRule::ForwardRef,
                )
                .with_props_type_args(type_args)
                .with_doc(description, tags),
            );
        }
        let rule = if self.is_forward_ref_call(call) { DetectionRule::ForwardRef } else { DetectionRule::HocWrapper };
        self.mapping_from_typed_function(decl, name, call, first_arg, rule)
    }

    /// The component a wrapper call builds from a render function whose first parameter names the props type:
    /// `memo(function Button(props: ButtonProps) {…})`, and the same inside `memo(forwardRef(…))`. `call` is the outermost
    /// wrapper call, whose type arguments may add props (`wrapper.rs`).
    fn mapping_from_typed_function<'a>(
        &mut self,
        decl: &Binding<'_, 'a>,
        name: &str,
        call: &CallExpression<'a>,
        function: &Argument<'a>,
        rule: DetectionRule,
    ) -> Option<ComponentMapping> {
        let (fn_name, params, shape) = match function {
            Argument::FunctionExpression(fe) => {
                (fe.id.as_ref().map(|id| id.name.as_str()), &fe.params, FunctionShape::of_function(fe))
            }
            Argument::ArrowFunctionExpression(afe) => (None, &afe.params, FunctionShape::of_arrow(afe)),
            _ => return None,
        };

        // Rules out anonymous utility functions, which aren't components.
        if let Some(fn_name_str) = fn_name
            && !is_pascal_case(fn_name_str)
        {
            return None;
        }

        let first_param = params.items.first()?;
        let type_ann = first_param.type_annotation.as_ref()?;
        // `debounce((q: Row) => …)` and `createSelector((s: State) => …)` have the same shape as `memo(…)`.
        if let Err(reason) = renders(&shape, &*self) {
            self.decline_reason = Some(format!("'{name}' wraps a typed function that {reason}"));
            return None;
        }
        let (props_name, type_args) = self.extract_type_name_from_type(&type_ann.type_annotation)?;

        let param_defaults = self.extract_param_defaults(params);
        let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);

        let mapping = ComponentMapping::new(
            name.to_owned(),
            props_name,
            self.file_path.clone(),
            decl.span.start,
            decl.span.end,
            rule,
        )
        .with_props_type_args(type_args)
        .with_doc(description, tags)
        .with_param_defaults(param_defaults);
        // `factory<Payload>((props: P) => …)`: a generic wrapper's result type can add props, as in `wrapper.rs`.
        Some(match self.wrapper_callee(call) {
            Some(callee) => mapping.with_wrapper_callee(callee),
            None => mapping,
        })
    }

    /// Try to detect declared `ForwardRefExoticComponent<Props>` and `MemoExoticComponent<(props: Props) => JSX>` values.
    ///
    /// Common in .d.ts files — no initializer, just a type annotation.
    pub(super) fn try_exotic_component_decl<'a>(
        &mut self,
        decl: &VariableDeclarator<'a>,
        name: &str,
    ) -> Option<ComponentMapping> {
        let type_ann = decl.type_annotation.as_ref()?;
        let ct = self.ts_type_to_collected(&type_ann.type_annotation);

        let is_react_type = |name: &str, expected: &str| {
            name == expected
                || name.rsplit_once('.').is_some_and(|(prefix, suffix)| {
                    suffix == expected
                        && (prefix == "React"
                            || self.data.imports.iter().any(|binding| {
                                binding.local_name == prefix
                                    && binding.exported_name == "*"
                                    && super::is_react_module(&binding.specifier)
                            }))
                })
        };
        let exotic_args = |ty: &crate::types::CollectedType| match ty {
            crate::types::CollectedType::Named { name, args } if is_react_type(name, "ForwardRefExoticComponent") => {
                Some((args.clone(), false))
            }
            crate::types::CollectedType::Named { name, args } if is_react_type(name, "MemoExoticComponent") => {
                Some((args.clone(), true))
            }
            _ => None,
        };
        let (args, memoized) = match &ct {
            crate::types::CollectedType::Intersection(members) => members.iter().find_map(exotic_args),
            other => exotic_args(other),
        }?;

        let first_arg = args.first()?;

        // RefAttributes is metadata on the value, not a prop base. Keep every other member: declaration generators
        // often write several props objects into one ForwardRefExoticComponent argument.
        let props_type = match first_arg {
            crate::types::CollectedType::Function { params, .. } if memoized => params.first()?.clone(),
            _ if memoized => return None,
            crate::types::CollectedType::Intersection(members) => {
                let mut props: Vec<_> = members
                    .iter()
                    .filter(|member| {
                        !matches!(member, crate::types::CollectedType::Named { name, .. }
                            if is_react_type(name, "RefAttributes"))
                    })
                    .cloned()
                    .collect();
                match props.len() {
                    0 => return None,
                    1 => props.remove(0),
                    _ => crate::types::CollectedType::Intersection(props),
                }
            }
            other => other.clone(),
        };

        let (props_name, props_args) = match props_type {
            crate::types::CollectedType::Named { name, args } => (name, args),
            props => {
                let name = crate::types::synthesized_alias_name(self.data.type_aliases.len());
                let key = self.scoped_key(&name);
                self.data.type_aliases.insert(
                    key,
                    crate::types::CollectedTypeAlias::Passthrough { target: props, file_path: self.file_path.clone() },
                );
                (name.into(), vec![])
            }
        };

        Some(
            ComponentMapping::new(
                name.to_owned(),
                props_name,
                self.file_path.clone(),
                decl.span.start,
                decl.span.end,
                if memoized { DetectionRule::MemoExoticDecl } else { DetectionRule::ForwardRefExoticDecl },
            )
            .with_props_type_args(props_args)
            .with_doc(self.find_jsdoc(decl.span.start), BTreeMap::new())
            .with_param_defaults(rustc_hash::FxHashMap::default()),
        )
    }

    /// A dependency can publish a component as `declare const Tab: (props: TabProps) => ReactElement | null`.
    pub(super) fn try_declared_callable_component<'a>(
        &mut self,
        decl: &VariableDeclarator<'a>,
        name: &str,
    ) -> Option<ComponentMapping> {
        let annotation = &decl.type_annotation.as_ref()?.type_annotation;
        let (params, return_type, type_parameters) = if let TSType::TSFunctionType(function) = annotation {
            (function.params.as_ref(), function.return_type.as_ref(), function.type_parameters.as_deref())
        } else if let TSType::TSTypeLiteral(literal) = annotation {
            let mut calls = literal.members.iter().filter_map(|member| match member {
                TSSignature::TSCallSignatureDeclaration(call) => Some(call.as_ref()),
                TSSignature::TSIndexSignature(_)
                | TSSignature::TSPropertySignature(_)
                | TSSignature::TSConstructSignatureDeclaration(_)
                | TSSignature::TSMethodSignature(_) => None,
            });
            let call = calls.next()?;
            if calls.next().is_some() {
                return None;
            }
            (call.params.as_ref(), call.return_type.as_deref()?, call.type_parameters.as_deref())
        } else {
            return None;
        };
        let [parameter] = params.items.as_slice() else { return None };
        let return_type = self.ts_type_to_collected(&return_type.type_annotation);
        let is_element = |name: &str| {
            matches!(name, "React.ReactElement" | "React.JSX.Element" | "JSX.Element")
                || name.strip_suffix(".JSX.Element").is_some_and(|namespace| {
                    self.data.imports.iter().any(|binding| {
                        binding.local_name == namespace
                            && binding.exported_name == "*"
                            && matches!(binding.specifier.as_str(), "react/jsx-runtime" | "react/jsx-dev-runtime")
                    })
                })
                || self.data.imports.iter().any(|binding| {
                    binding.local_name == name
                        && binding.exported_name == "ReactElement"
                        && super::is_react_module(&binding.specifier)
                })
        };
        let returns_element = |ty: &crate::types::CollectedType| match ty {
            crate::types::CollectedType::Named { name, .. } => is_element(name),
            crate::types::CollectedType::Union(members) => members.iter().any(|member| {
                if let crate::types::CollectedType::Named { name, .. } = member { is_element(name) } else { false }
            }),
            _ => false,
        };
        if !returns_element(&return_type) {
            return None;
        }
        let (props_name, mut props_args) =
            self.extract_type_name_from_type(&parameter.type_annotation.as_ref()?.type_annotation)?;
        let type_params: Vec<crate::types::TypeName> = type_parameters
            .map(|declaration| declaration.params.iter().map(|param| param.name.name.as_str().into()).collect())
            .unwrap_or_default();
        if crate::types::is_synthesized_alias(&props_name) && !type_params.is_empty() {
            self.data.type_alias_params.insert(self.scoped_key(&props_name), type_params.clone());
            props_args = type_params
                .iter()
                .map(|name| crate::types::CollectedType::Named { name: name.clone(), args: Vec::new() })
                .collect();
        }
        let mut mapping = ComponentMapping::new(
            name.to_owned(),
            props_name,
            self.file_path.clone(),
            decl.span.start,
            decl.span.end,
            DetectionRule::FunctionDeclaration,
        )
        .with_props_type_args(props_args)
        .with_doc(self.find_jsdoc(decl.span.start), BTreeMap::new());
        mapping.type_params = type_params;
        Some(mapping)
    }

    /// Try to detect: `export default function(props: Props) { ... }` — an
    /// anonymous default-exported function component, common in Next.js page
    /// files. There's no identifier to name it after, so the PascalCased
    /// file stem is used instead. Mirrors Pattern 4's
    /// (`visit_function`'s named-function handling) exact contract: a
    /// missing type annotation is a flagged skip, but zero declared params
    /// at all is silent — same as the named-function sibling.
    pub(super) fn try_anonymous_default_export_component<'a>(&mut self, func: &Function<'a>) {
        let Some(name) = pascal_case_from_file_stem(&self.file_path) else { return };

        let Some(first_param) = func.params.items.first() else { return };
        let Some(type_ann) = &first_param.type_annotation else {
            self.record_skip(
                DiagnosticCode::SkippedCandidate,
                format!("'{name}' is an anonymous default-exported function component with an untyped first param"),
                first_param.span,
            );
            return;
        };
        let Some((props_name, type_args)) = self.extract_type_name_from_type(&type_ann.type_annotation) else {
            self.record_skip(
                DiagnosticCode::SkippedCandidate,
                format!(
                    "'{name}' is an anonymous default-exported function component whose first param's type \
                     annotation isn't a recognizable props type reference"
                ),
                type_ann.span,
            );
            return;
        };

        let (description, tags) = self.find_jsdoc_with_tags(func.span.start);
        let param_defaults = self.extract_param_defaults(&func.params);
        self.anonymous_default = Some(func.span.start);
        self.data.component_mappings.push(
            ComponentMapping::new(
                name,
                props_name,
                self.file_path.clone(),
                func.span.start,
                func.span.end,
                DetectionRule::AnonymousDefaultExport,
            )
            .without_binding()
            .with_props_type_args(type_args)
            .with_doc(description, tags)
            .with_param_defaults(param_defaults),
        );
    }

    /// True when `class`'s `extends` clause is `Component`/`PureComponent`
    /// (bare or `React.`-qualified). Returns the matched bare name for use in
    /// diagnostic messages.
    pub(super) fn super_class_is_component_family<'a>(&self, class: &Class<'a>) -> Option<&'static str> {
        let super_name = self.expression_to_ident_name(super_class(class)?.named()?);
        match self.react_type_export(&super_name)?.as_str() {
            "Component" => Some("Component"),
            "PureComponent" => Some("PureComponent"),
            _ => None,
        }
    }

    /// Try to detect: `class Button extends React.Component<Props> { ... }`
    /// (or bare `Component`/`PureComponent`). Shared by both a named
    /// `ClassDeclaration` (called directly from `visit_class`) and an
    /// anonymous `ClassExpression` assigned to a variable (via
    /// `try_class_expression_wrapped` below). Silent on every failure path,
    /// mirroring `try_fc_annotation`/`try_forward_ref`/`try_hoc_wrapped`'s own
    /// contract — callers decide whether and how to surface a diagnostic.
    pub(super) fn try_class_component<'a>(&mut self, class: &Class<'a>, name: &str) -> Option<ComponentMapping> {
        self.super_class_is_component_family(class)?;
        let type_args = super_class(class)?.type_arguments?;
        let props_type = type_args.params.first()?;
        let (props_name, props_type_args) = self.extract_type_name_from_type(props_type)?;
        let (description, tags) = self.find_jsdoc_with_tags(class.span.start);
        Some(
            ComponentMapping::new(
                name.to_owned(),
                props_name,
                self.file_path.clone(),
                class.span.start,
                class.span.end,
                DetectionRule::ClassComponent,
            )
            .with_props_type_args(props_type_args)
            .with_doc(description, tags)
            .with_param_defaults(rustc_hash::FxHashMap::default()),
        )
    }

    /// Try to detect: `const Button = class extends React.Component<Props> { ... }`
    /// — an anonymous class expression assigned to a PascalCase binding. Only
    /// fires when the class expression has no name of its own; a self-named
    /// one (`class Button extends ...`) is instead picked up directly by
    /// `visit_class` under its own identifier, avoiding a duplicate mapping
    /// under both names.
    pub(super) fn try_class_expression_wrapped<'a>(
        &mut self,
        decl: &Binding<'_, 'a>,
        name: &str,
    ) -> Option<ComponentMapping> {
        let init = decl.init.as_ref()?;
        let Expression::ClassExpression(class) = peel_expr(init) else { return None };
        if class.id.is_some() {
            return None;
        }
        self.try_class_component(class, name)
    }

    /// True when `decl`'s type annotation (after peeling parens) is a bare
    /// reference to one of the FC-family type names, regardless of whether
    /// its props type argument is itself extractable.
    ///
    /// Used by the no-initializer Pattern 5 diagnostic guard in `visit.rs`:
    /// `try_fc_annotation` already runs unconditionally for every PascalCase
    /// declarator (init or no init) and would have produced a mapping had the
    /// props argument resolved, so reaching this check with no mapping means
    /// the props argument specifically was the unrecognized part — an exotic
    /// `TSType` shape `extract_type_name_from_type` doesn't match — not that
    /// the declaration isn't FC-shaped at all.
    pub(super) fn type_annotation_is_fc_family<'a>(&self, decl: &VariableDeclarator<'a>) -> bool {
        fn peel_parens<'t, 'a>(ty: &'t TSType<'a>) -> &'t TSType<'a> {
            match ty {
                TSType::TSParenthesizedType(p) => peel_parens(&p.type_annotation),
                other => other,
            }
        }
        let Some(type_ann) = decl.type_annotation.as_ref() else { return false };
        let TSType::TSTypeReference(tr) = peel_parens(&type_ann.type_annotation) else { return false };
        let type_name = self.extract_type_ref_name(tr);
        self.react_type_export(&type_name).is_some_and(|bare| is_fc_family_type(&bare))
    }

    /// Rename an already-collected component mapping to its real public export name
    /// when the mapping's own identifier is immediately wrapped by an *unrecognized*
    /// call and reassigned to a new binding — e.g. Headless UI's real
    /// `export let ListboxButton = forwardRefWithAs(ButtonFn) as X`. `forwardRefWithAs`
    /// is a library-defined wrapper, not `React.forwardRef` itself, so `try_forward_ref`
    /// never matches it; meanwhile `ButtonFn` — a standalone top-level function
    /// declaration — was already independently collected as its own component by the
    /// `visit_function` Pattern 4 check, under the wrong (inner, implementation-only)
    /// name. Without this, the real export name never appears at all.
    ///
    /// Also handles the same rename with no wrapper call at all — a bare passthrough
    /// alias, `const Button = InternalCompoundedButton;` (optionally `as X`), antd's
    /// real `Button` export shape.
    ///
    /// Clones the matched mapping under the new name rather than renaming it in
    /// place: a second, different alias to the same base
    /// (`const A = Base; const Bcopy = Base;`) is a genuinely distinct exported
    /// binding, not the same rename twice, and renaming in place would make the
    /// base unfindable for that second alias — silently dropping it. The base's
    /// own (implementation-only) name is filtered out of the final output in
    /// `finish()` via `aliased_away`.
    pub(super) fn try_rename_identifier_wrapped_component<'a>(&mut self, decl: &Binding<'_, 'a>, name: &str) -> bool {
        let Some(shape) = self.alias_shape(decl) else { return false };
        match self.alias_outcome(name, &shape) {
            AliasOutcome::Done => true,
            AliasOutcome::Declined(reason) => {
                self.decline_reason = Some(reason);
                false
            }
            AliasOutcome::NoBase => {
                self.alias_candidate = Some(shape);
                false
            }
        }
    }

    /// The component `decl` re-binds, `const Button = Inner` or `memo(Inner)`, and what the call is, read before `Inner`
    /// needs to be known. `None` when the initializer is not that shape, or the call names the props it builds with.
    fn alias_shape<'a>(&mut self, decl: &Binding<'_, 'a>) -> Option<AliasShape> {
        let init = decl.init.as_ref()?;
        let (inner, call) = match peel_expr(init) {
            Expression::CallExpression(call) => match aliased_component(call) {
                Some(Expression::Identifier(id)) => (id.name.as_str().to_owned(), Some(&**call)),
                _ => return None,
            },
            Expression::Identifier(id) => (id.name.as_str().to_owned(), None),
            _ => return None,
        };
        let Some(call) = call else { return Some(AliasShape { inner, call: None }) };
        // The component it receives is not the one it builds when a type argument names the props.
        if self.call_names_props_type(call) {
            return None;
        }
        // `getDisplayName(Card)` has the same shape as `memo(Card)` but returns a string; only a call that follows the
        // component-wrapper conventions produces a component.
        // `Object.assign(Impl, { Item })` returns `Impl` itself with the statics attached.
        let wrapper = is_component_alias_call(&*self, call);
        // `forwardRef(Inner)` makes a component that takes a `ref` even when `Inner` is a plain typed function.
        let call = Some(AliasCall { wrapper, forward_ref: self.is_forward_ref_call(call) });
        Some(AliasShape { inner, call })
    }

    /// Bind `name` to a clone of the component `shape.inner` once that is collected.
    fn alias_outcome(&mut self, name: &str, shape: &AliasShape) -> AliasOutcome {
        let (inner, call) = (shape.inner.as_str(), shape.call);
        let Some(base) = self.data.component_mappings.iter().find(|m| m.is_binding_named(inner)) else {
            return AliasOutcome::NoBase;
        };
        if call.is_some_and(|call| !call.wrapper) {
            return AliasOutcome::Declined(format!(
                "'{name}' passes component '{inner}' to a call that does not look like a component wrapper"
            ));
        }
        let mut alias = base.clone();
        if call.is_some_and(|call| call.forward_ref) {
            alias.rule = DetectionRule::ForwardRef;
        }
        alias.component_name = name.to_owned();
        self.data.component_mappings.push(alias);
        self.aliased_away.insert(inner.into());
        AliasOutcome::Done
    }

    /// Hold back `const Button = Inner` whose `Inner` is not collected yet: it may be declared further down the file.
    /// Only the alias shape `try_rename_identifier_wrapped_component` just read, and only after every other rule
    /// declined it.
    pub(super) fn try_defer_identifier_alias(&mut self, name: &str, span: oxc_span::Span) -> bool {
        let Some(shape) = self.alias_candidate.take() else { return false };
        self.pending_aliases.push(PendingAlias { name: name.to_owned(), shape, span });
        true
    }

    /// Resolve the held aliases now that every declaration in the file is known; an alias of an alias takes more than
    /// one pass. What is still unresolved is reported as the skip it would have been.
    pub(super) fn promote_identifier_aliases(&mut self) {
        let mut pending = std::mem::take(&mut self.pending_aliases);
        loop {
            let before = pending.len();
            let mut waiting = Vec::new();
            for alias in pending {
                if self.data.component_mappings.iter().any(|m| m.is_binding_named(&alias.name)) {
                    continue;
                }
                match self.alias_outcome(&alias.name, &alias.shape) {
                    AliasOutcome::Done => {}
                    AliasOutcome::Declined(reason) => {
                        self.record_skip(DiagnosticCode::SkippedCandidate, reason, alias.span);
                    }
                    AliasOutcome::NoBase => waiting.push(alias),
                }
            }
            pending = waiting;
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        let reported: Vec<PendingAlias> = pending
            .into_iter()
            .filter(|alias| {
                !super::is_constant_name(&alias.name)
                    && !self.pending_factory_calls.iter().any(|factory| factory.name == alias.name)
            })
            .collect();
        for alias in reported {
            self.record_skip(
                DiagnosticCode::SkippedCandidate,
                format!(
                    "'{}' is a PascalCase binding but matched no known component pattern \
                     (FC annotation, forwardRef, HOC wrapper, or identifier alias)",
                    alias.name
                ),
                alias.span,
            );
        }
    }

    /// Record `const X = Imported.Member` and `const X = Imported` as a component whose props are the imported
    /// component's (`ComponentProps<typeof Imported.Member>`), when `Imported` is an import binding and the component is
    /// PascalCase.
    pub(super) fn try_record_imported_member_alias<'a>(&mut self, decl: &Binding<'_, 'a>, name: &str) -> bool {
        let Some(init) = decl.init.as_ref() else { return false };
        let target = match peel_expr(init) {
            Expression::StaticMemberExpression(member) => {
                let Expression::Identifier(object) = beneath(&member.object) else { return false };
                let property = member.property.name.as_str();
                if !self.imported_names.contains(object.name.as_str()) || !is_pascal_case(property) {
                    return false;
                }
                format!("{}.{property}", object.name)
            }
            // A constant is not a component whatever it is bound to; `const ORIGIN = Thing` says nothing about `Thing`.
            Expression::Identifier(id)
                if self.imported_names.contains(id.name.as_str())
                    && is_pascal_case(id.name.as_str())
                    && !super::is_constant_name(name) =>
            {
                id.name.to_string()
            }
            _ => return false,
        };
        self.push_imported_alias(name, target, decl.span);
        true
    }

    /// Record `const { Option } = Imported` and `const { Option: Opt } = Imported` as components named by the binding,
    /// one per PascalCase member, when `Imported` is an import binding.
    pub(super) fn try_record_destructured_imports<'a>(&mut self, declarator: &VariableDeclarator<'a>) {
        let BindingPattern::ObjectPattern(pattern) = &declarator.id else { return };
        let Some(Expression::Identifier(object)) = declarator.init.as_ref().map(peel_expr) else { return };
        if !self.imported_names.contains(object.name.as_str()) {
            return;
        }
        for property in &pattern.properties {
            let PropertyKey::StaticIdentifier(key) = &property.key else { continue };
            let BindingPattern::BindingIdentifier(local) = &property.value else { continue };
            let (key, local) = (key.name.as_str(), local.name.as_str());
            if !is_pascal_case(key) || !is_pascal_case(local) || super::is_constant_name(local) {
                continue;
            }
            self.push_imported_alias(local, format!("{}.{key}", object.name), declarator.span);
        }
    }

    fn push_imported_alias(&mut self, name: &str, target: String, span: oxc_span::Span) {
        let (description, tags) = self.find_jsdoc_with_tags(span.start);
        self.data.component_mappings.push(
            ComponentMapping::new(
                name.to_owned(),
                "ComponentProps".into(),
                self.file_path.clone(),
                span.start,
                span.end,
                DetectionRule::ImportedMemberAlias,
            )
            .with_props_type_args(vec![crate::types::CollectedType::TypeOf(target.into())])
            .with_doc(description, tags),
        );
    }

    /// Whether `call` makes a component that takes a `ref` whatever its props type declares: React's `forwardRef` however
    /// it is imported or spelled, or another module's own `forwardRef` (React Aria's).
    fn is_forward_ref_call(&self, call: &CallExpression<'_>) -> bool {
        if self.react_api(call) == Some(ReactApi::ForwardRef) {
            return true;
        }
        let name = match callee_of(call) {
            Some(Expression::Identifier(id)) => id.name.as_str(),
            Some(Expression::StaticMemberExpression(member)) => member.property.name.as_str(),
            _ => return false,
        };
        name == "forwardRef" && self.react_api(call).is_none()
    }

    /// Extract the callee name of a call expression (simple ident or member expr).
    pub(super) fn extract_callee_name<'a>(&self, call: &CallExpression<'a>) -> Option<String> {
        match callee_of(call)? {
            Expression::Identifier(id) => Some(id.name.as_str().to_owned()),
            Expression::StaticMemberExpression(me) => {
                Some(format!("{}.{}", self.expression_to_ident_name(&me.object), me.property.name.as_str()))
            }
            _ => None,
        }
    }
}

/// Whether a call wrapping a component returns a component: React's `memo` and `forwardRef` as the file binds them,
/// or the ecosystem's conventions.
pub(super) fn is_component_wrapper(scope: &dyn Scope, call: &CallExpression<'_>) -> bool {
    scope.is_react_wrapper(call) || is_component_wrapper_call(call)
}

/// Whether a call returns the component it is given, so that what it is bound or exported as is that component: a
/// component wrapper, or `Object.assign(Impl, { Item })`, which returns `Impl` with the statics attached. The one place
/// that says which calls make an alias of a component; [`aliased_component`] says which argument it is.
pub(super) fn is_component_alias_call(scope: &dyn Scope, call: &CallExpression<'_>) -> bool {
    is_component_wrapper(scope, call) || is_object_assign_call(call)
}

/// The argument of a component alias call that is the component: the first.
pub(super) fn aliased_component<'a, 'b>(call: &'b CallExpression<'a>) -> Option<&'b Expression<'a>> {
    call.arguments.first()?.as_expression().map(peel_expr)
}

/// Whether the call is `Object.assign(...)`.
pub(super) fn is_object_assign_call(call: &CallExpression<'_>) -> bool {
    matches!(callee_of(call), Some(Expression::StaticMemberExpression(callee))
        if callee.property.name == "assign"
            && matches!(beneath(&callee.object), Expression::Identifier(object) if object.name == "Object"))
}

/// Whether a call wrapping a component returns a component, by the ecosystem's conventions:
/// `memo`, anything named like `forwardRef`, `withX(...)` higher-order components, `createXComponent(...)` factories,
/// `observer`, and curried wrappers such as `connect(...)(Component)` or `styled(...)(Component)`.
fn is_component_wrapper_call(call: &CallExpression<'_>) -> bool {
    // `(forwardRef as forwardRefType)(X)` is React Aria's spelling of `forwardRef(X)`.
    let callee = match callee_of(call) {
        Some(Expression::Identifier(id)) => id.name.as_str(),
        Some(Expression::StaticMemberExpression(m)) => m.property.name.as_str(),
        Some(Expression::CallExpression(_)) => return true,
        _ => return false,
    };
    let lower = callee.to_ascii_lowercase();
    callee == "memo"
        || callee == "observer"
        || lower.contains("forwardref")
        || callee.contains("Component")
        || (callee.len() > 4 && callee.starts_with("with") && callee[4..].starts_with(|c: char| c.is_ascii_uppercase()))
}
