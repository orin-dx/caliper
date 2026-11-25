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
/// support unions on @types/react"), check it with `satisfies`, or instantiate a generic component (`memo(List<Row>)`,
/// `(List<Row>)!`). The value is the same function or component beneath an instantiation, so every wrapper is read
/// through; only a callee or an `extends` target needs the instantiation's type arguments ([`peel`]).
pub(super) fn peel_expr<'a, 'b>(mut expr: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        expr = match wrapper(expr) {
            Some(Wrapper::Transparent(inner) | Wrapper::Asserted(inner, _)) => inner,
            Some(Wrapper::Instantiation(instantiation)) => &instantiation.expression,
            None => return expr,
        };
    }
}

/// What the outermost assertion on a callee, or on the namespace it is read from, says about calling it. Only the
/// outermost one counts: `(e as A) as B` has type `B` whatever `A` is. An assertion declines a candidate only on
/// affirmative evidence ([`Evidence::NotRenderable`]); a type the extractor cannot read leaves the call the candidate
/// its name makes it ([`Evidence::Unknown`]), as before assertions were read at all.
#[derive(Clone, Copy)]
pub(super) enum Evidence<'b, 'a> {
    /// No assertion, or one that keeps the callee's own type: `typeof` the same expression, `any` and `unknown`. The
    /// callee's declaration describes the call.
    Identity,
    /// A type reference (`(forwardRef as forwardRefType)`). The file may alias it to `typeof` the callee
    /// (`SourceDataCollector::describes_call`); otherwise it reads as [`Evidence::Unknown`].
    Reference(&'b TSTypeReference<'a>),
    /// Every call signature of the asserted type returns what no component is, and at least one what React renders as a
    /// node ([`Returns::Node`]): `(Greeting as (p: P) => string)(…)` gives no component, but a component may return
    /// its result.
    NoComponent,
    /// Every call signature of the asserted type returns what neither a component nor a node is ([`Returns::Neither`]):
    /// `(Object as (v: object) => object)(…)` neither is a component nor renders, whatever its name.
    NotRenderable,
    /// Anything else: `typeof` another expression, a signature whose result may render or is not read, a type with no
    /// call signature. The name still says what the call is; the callee's declaration no longer describes it.
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
    /// `expr`, when what it is called still says what a call expected to give a component is (`forwardRef`, `memo`,
    /// `Object.assign`, a HOC): always, unless the assertion gives the call a result no component has.
    pub(super) fn named(&self) -> Option<&'b Expression<'a>> {
        match self.evidence {
            Evidence::NoComponent | Evidence::NotRenderable => None,
            Evidence::Identity | Evidence::Reference(_) | Evidence::Unknown => Some(self.expr),
        }
    }

    /// `expr`, when what it is called still says what a call whose result is rendered is (a component invoked
    /// directly, an element factory, `map`): always, unless the assertion gives the call a result React cannot render.
    /// A string, number or `null` result renders, so it is no evidence against such a call.
    pub(super) fn renderable(&self) -> Option<&'b Expression<'a>> {
        match self.evidence {
            Evidence::NotRenderable => None,
            Evidence::Identity | Evidence::Reference(_) | Evidence::NoComponent | Evidence::Unknown => Some(self.expr),
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

/// See [`Peeled`]. For what a callee or an `extends` target is: OXC 0.159 keeps `(f<T>)(x)` and `extends (B<T>)` as an
/// instantiation node beneath the call or heritage clause, where 0.158 moved its type arguments onto them.
pub(super) fn peel<'a, 'b>(expr: &'b Expression<'a>) -> Peeled<'b, 'a> {
    let (expr, type_arguments, outermost) = layers(expr);
    let evidence = match (outermost, expr) {
        (Some(ty), _) => asserted(ty, expr),
        // `(React as { forwardRef: typeof React.forwardRef }).forwardRef`: the namespace's assertion gives the member
        // the type it declares for it.
        (None, Expression::StaticMemberExpression(member)) => match receiver(&member.object) {
            Typed::Own => Evidence::Identity,
            Typed::Asserted(ty) => member_evidence(ty, member.property.name.as_str(), expr),
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

fn receiver<'a, 'b>(expr: &'b Expression<'a>) -> Typed<'b, 'a> {
    let (expr, _, outermost) = layers(expr);
    let declared = match (outermost, expr) {
        (Some(ty), _) => Some(ty),
        (None, Expression::StaticMemberExpression(object)) => match receiver(&object.object) {
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
        Some(ty) if matches!(asserted(ty, expr), Evidence::Identity) => Typed::Own,
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
                // A setter's parameter is what is assigned, not what is read.
                TSMethodSignatureKind::Set => MemberDeclaration::Typed(None),
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

/// What calling `expr`, a member read from a namespace asserted to `ty`, is.
fn member_evidence<'a, 'b>(ty: &'b TSType<'a>, name: &str, expr: &Expression<'_>) -> Evidence<'b, 'a> {
    match member(ty, name) {
        Member::Property(ty) => asserted(ty, expr),
        // The member has every type declared for it: no component only when each declaration says so.
        Member::Declared(literal) => combine(declarations(literal, name).map(|declaration| match declaration {
            MemberDeclaration::Typed(Some(ty)) => asserted(ty, expr),
            MemberDeclaration::Typed(None) => Evidence::Unknown,
            MemberDeclaration::Method(result) => results(std::iter::once(result)),
        })),
        Member::Unread => Evidence::Unknown,
    }
}

/// What asserting `ty` on `beneath` says about calling it. One exhaustive match: a new type kind in an OXC release
/// has to be placed here before it compiles.
fn asserted<'a, 'b>(ty: &'b TSType<'a>, beneath: &Expression<'_>) -> Evidence<'b, 'a> {
    match ty {
        TSType::TSAnyKeyword(_) | TSType::TSUnknownKeyword(_) => Evidence::Identity,
        TSType::TSParenthesizedType(inner) => asserted(&inner.type_annotation, beneath),
        TSType::TSTypeQuery(query) if query.type_arguments.is_none() && query_names(&query.expr_name, beneath) => {
            Evidence::Identity
        }
        TSType::TSTypeReference(reference) => Evidence::Reference(reference),
        TSType::TSFunctionType(function) => results(std::iter::once(Some(&function.return_type.type_annotation))),
        TSType::TSTypeLiteral(literal) => results(literal.members.iter().filter_map(|signature| match signature {
            TSSignature::TSCallSignatureDeclaration(call) => {
                Some(call.return_type.as_ref().map(|annotation| &annotation.type_annotation))
            }
            TSSignature::TSPropertySignature(_)
            | TSSignature::TSMethodSignature(_)
            | TSSignature::TSIndexSignature(_)
            | TSSignature::TSConstructSignatureDeclaration(_) => None,
        })),
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
        | TSType::TSConstructorType(_)
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
    combine(signatures.map(|result| match result.map(result_of) {
        Some(Returns::Neither) => Evidence::NotRenderable,
        Some(Returns::Node) => Evidence::NoComponent,
        Some(Returns::Empty | Returns::Maybe) | None => Evidence::Unknown,
    }))
}

/// The evidence of several signatures or declarations of one callee, each read alone: [`Evidence::NotRenderable`] when
/// every one is, [`Evidence::NoComponent`] when every one is that or `NotRenderable`, and [`Evidence::Unknown`]
/// otherwise or when there are none. One that may give a component keeps the whole callee a candidate.
fn combine<'a, 'b>(all: impl Iterator<Item = Evidence<'b, 'a>>) -> Evidence<'b, 'a> {
    let mut combined = None;
    for evidence in all {
        combined = Some(match (combined, evidence) {
            (None | Some(Evidence::NotRenderable), Evidence::NotRenderable) => Evidence::NotRenderable,
            (None | Some(Evidence::NotRenderable | Evidence::NoComponent), Evidence::NoComponent)
            | (Some(Evidence::NoComponent), Evidence::NotRenderable) => Evidence::NoComponent,
            (_, Evidence::Identity | Evidence::Reference(_) | Evidence::Unknown)
            | (
                Some(Evidence::Identity | Evidence::Reference(_) | Evidence::Unknown),
                Evidence::NoComponent | Evidence::NotRenderable,
            ) => return Evidence::Unknown,
        });
    }
    combined.unwrap_or(Evidence::Unknown)
}

/// What a call's declared result says about whether it is a component and whether React renders it.
#[derive(Clone, Copy)]
enum Returns {
    /// What React renders as a node and no component is: a string, number, bigint or boolean keyword, `void`, a
    /// literal or template literal, a type predicate, an array of those, or a union with one of them and nothing that
    /// may be a component.
    Node,
    /// Neither a component nor a node: the `object`, `symbol` and `never` keywords, an object type with no call or
    /// construct signature, an array of those or of `unknown`/`any` (TypeScript does not take `unknown` for a node).
    Neither,
    /// `null` or `undefined`: what a component may return, and no evidence alone.
    Empty,
    /// Anything that may be a component or an element, or that the extractor does not read (a reference, `unknown`).
    Maybe,
}

fn result_of(ty: &TSType<'_>) -> Returns {
    match ty {
        TSType::TSBigIntKeyword(_)
        | TSType::TSBooleanKeyword(_)
        | TSType::TSNumberKeyword(_)
        | TSType::TSStringKeyword(_)
        | TSType::TSVoidKeyword(_)
        | TSType::TSLiteralType(_)
        | TSType::TSTemplateLiteralType(_)
        | TSType::TSTypePredicate(_) => Returns::Node,
        TSType::TSNeverKeyword(_) | TSType::TSObjectKeyword(_) | TSType::TSSymbolKeyword(_) => Returns::Neither,
        TSType::TSNullKeyword(_) | TSType::TSUndefinedKeyword(_) => Returns::Empty,
        TSType::TSParenthesizedType(inner) => result_of(&inner.type_annotation),
        TSType::TSTypeLiteral(literal) => {
            let callable = literal.members.iter().any(|signature| match signature {
                TSSignature::TSCallSignatureDeclaration(_) | TSSignature::TSConstructSignatureDeclaration(_) => true,
                TSSignature::TSPropertySignature(_)
                | TSSignature::TSMethodSignature(_)
                | TSSignature::TSIndexSignature(_) => false,
            });
            if callable { Returns::Maybe } else { Returns::Neither }
        }
        // `ReactNode[]` and `string[]` render; `unknown[]` does not.
        TSType::TSArrayType(array) => match (&array.element_type, result_of(&array.element_type)) {
            (TSType::TSAnyKeyword(_) | TSType::TSUnknownKeyword(_), _) => Returns::Neither,
            (_, Returns::Maybe) => Returns::Maybe,
            (_, Returns::Node | Returns::Empty) => Returns::Node,
            (_, Returns::Neither) => Returns::Neither,
        },
        // The value is one of the members: it may render when one does, and is a candidate when one may be.
        TSType::TSUnionType(union) => {
            let mut result = Returns::Empty;
            for member in &union.types {
                result = match (result, result_of(member)) {
                    (Returns::Maybe, _) | (_, Returns::Maybe) => return Returns::Maybe,
                    (Returns::Node, _) | (_, Returns::Node) => Returns::Node,
                    (Returns::Neither, _) | (_, Returns::Neither) => Returns::Neither,
                    (Returns::Empty, Returns::Empty) => Returns::Empty,
                };
            }
            result
        }
        TSType::TSAnyKeyword(_)
        | TSType::TSUnknownKeyword(_)
        | TSType::TSIntrinsicKeyword(_)
        | TSType::TSThisType(_)
        | TSType::TSConditionalType(_)
        | TSType::TSConstructorType(_)
        | TSType::TSFunctionType(_)
        | TSType::TSImportType(_)
        | TSType::TSIndexedAccessType(_)
        | TSType::TSInferType(_)
        | TSType::TSIntersectionType(_)
        | TSType::TSMappedType(_)
        | TSType::TSNamedTupleMember(_)
        | TSType::TSTupleType(_)
        | TSType::TSTypeOperatorType(_)
        | TSType::TSTypeQuery(_)
        | TSType::TSTypeReference(_)
        | TSType::JSDocNullableType(_)
        | TSType::JSDocNonNullableType(_)
        | TSType::JSDocUnknownType(_) => Returns::Maybe,
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

/// Counts every name the file declares in type space, at any depth: type aliases, interfaces, classes, enums,
/// namespaces, type parameters (`infer` and mapped keys among them) and every import, which may bring a type.
#[derive(Default)]
struct TypeNames(FxHashMap<CompactString, u32>);

impl TypeNames {
    fn declare(&mut self, name: &str) {
        *self.0.entry(CompactString::from(name)).or_default() += 1;
    }
}

impl<'a> oxc_ast_visit::Visit<'a> for TypeNames {
    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        self.declare(&it.id.name);
        oxc_ast_visit::walk::walk_ts_type_alias_declaration(self, it);
    }

    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        self.declare(&it.id.name);
        oxc_ast_visit::walk::walk_ts_interface_declaration(self, it);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if let Some(id) = &it.id {
            self.declare(&id.name);
        }
        oxc_ast_visit::walk::walk_class(self, it);
    }

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        self.declare(&it.id.name);
        oxc_ast_visit::walk::walk_ts_enum_declaration(self, it);
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        self.declare(&it.id.name);
        oxc_ast_visit::walk::walk_ts_namespace_declaration(self, it);
    }

    fn visit_ts_type_parameter(&mut self, it: &TSTypeParameter<'a>) {
        self.declare(&it.name.name);
        oxc_ast_visit::walk::walk_ts_type_parameter(self, it);
    }

    fn visit_ts_mapped_type(&mut self, it: &TSMappedType<'a>) {
        self.declare(&it.key.name);
        oxc_ast_visit::walk::walk_ts_mapped_type(self, it);
    }

    fn visit_import_declaration_specifier(&mut self, it: &ImportDeclarationSpecifier<'a>) {
        self.declare(&it.local().name);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        self.declare(&it.id.name);
    }
}

/// The module-level aliases of a value's type, `type Factory = typeof createButton`, as alias name to the dotted chain
/// it queries. Only a name the file declares once in type space is kept: an import, an interface merged with it, a
/// type parameter or a namespace's own type of that name may be what a use means, and which one is not read here.
pub(super) fn typeof_aliases(program: &Program<'_>) -> FxHashMap<CompactString, CompactString> {
    fn chain(name: &TSTypeName<'_>) -> Option<String> {
        match name {
            TSTypeName::IdentifierReference(id) => Some(id.name.as_str().to_owned()),
            TSTypeName::QualifiedName(qualified) => {
                Some(format!("{}.{}", chain(&qualified.left)?, qualified.right.name))
            }
            TSTypeName::ThisExpression(_) => None,
        }
    }
    let mut declared = TypeNames::default();
    oxc_ast_visit::Visit::visit_program(&mut declared, program);
    let mut aliases = FxHashMap::default();
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
        let queried = match ty {
            TSType::TSTypeQuery(query) if declaration.type_parameters.is_none() && query.type_arguments.is_none() => {
                match &query.expr_name {
                    TSTypeQueryExprName::IdentifierReference(id) => Some(id.name.as_str().to_owned()),
                    TSTypeQueryExprName::QualifiedName(qualified) => {
                        chain(&qualified.left).map(|left| format!("{left}.{}", qualified.right.name))
                    }
                    TSTypeQueryExprName::ThisExpression(_) | TSTypeQueryExprName::TSImportType(_) => None,
                }
            }
            _ => None,
        };
        if let Some(queried) = queried
            && declared.0.get(declaration.id.name.as_str()) == Some(&1)
        {
            aliases.insert(CompactString::from(declaration.id.name.as_str()), CompactString::from(queried));
        }
    }
    aliases
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
}

/// A value read for its name (a callee, the namespace a member is read from), when that name still describes it: see
/// [`Peeled::named`]. `(forwardRef as typeof forwardRef)(…)` and `(React as any).memo` keep it.
pub(super) fn named<'a, 'b>(expr: &'b Expression<'a>) -> Option<&'b Expression<'a>> {
    peel(expr).named()
}

/// What a call invokes, when its name still describes the call: `(forwardRef as F)(…)` and `(forwardRef<R, P>)(…)`
/// call `forwardRef`; `(Object as (v: object) => object)(…)` calls nothing by a name. See [`Peeled::named`].
pub(super) fn callee_of<'a, 'b>(call: &'b CallExpression<'a>) -> Option<&'b Expression<'a>> {
    peel(&call.callee).named()
}

/// What a call whose result is rendered invokes, when its name still describes the call: `return (Greeting as (p: P)
/// => string)(props)` calls `Greeting`, as a component may return a string; `(Object as (v: object) => object)(…)`
/// calls nothing by a name. See [`Peeled::renderable`].
pub(super) fn rendered_callee<'a, 'b>(call: &'b CallExpression<'a>) -> Option<&'b Expression<'a>> {
    peel(&call.callee).renderable()
}

/// The type arguments a call is made with: its own, or those of the instantiation it calls (`(forwardRef<R, P>)(…)`).
pub(super) fn call_type_arguments<'a, 'b>(
    call: &'b CallExpression<'a>,
) -> Option<&'b TSTypeParameterInstantiation<'a>> {
    call.type_arguments.as_deref().or_else(|| peel(&call.callee).type_arguments)
}

/// What a class extends and the type arguments it extends it with: `extends React.Component<P>` and
/// `extends (React.Component<P>)` both give `React.Component` and `P`.
pub(super) fn super_class<'a, 'b>(class: &'b Class<'a>) -> Option<Peeled<'b, 'a>> {
    let heritage = class.heritage.as_ref()?;
    let peeled = peel(&heritage.expression);
    Some(Peeled { type_arguments: heritage.type_arguments.as_deref().or(peeled.type_arguments), ..peeled })
}

/// What a call wrapping an aliased component is, as read from the call before the component is known.
#[derive(Clone, Copy)]
pub(crate) struct AliasCall {
    /// The call follows the component-wrapper conventions (`memo`, `withX(...)`, `forwardRef`, ...).
    wrapper: bool,
    forward_ref: bool,
}

/// An alias of a component that was not collected yet when the alias was read.
pub(crate) struct PendingAlias {
    name: String,
    inner: String,
    call: Option<AliasCall>,
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
    /// identity, or a reference to a module-level `type F = typeof <the callee>` whose name means nothing else in the
    /// file ([`typeof_aliases`]). Any other assertion may hide what the declaration says (`(wrap as <T>(r: FC<T>) =>
    /// FC<T>)<P>(…)` drops the props `wrap`'s result adds).
    pub(super) fn describes_call(&self, peeled: &Peeled<'_, '_>) -> bool {
        match peeled.evidence {
            Evidence::Identity => true,
            Evidence::Reference(reference) => {
                let chain = match &reference.type_name {
                    TSTypeName::IdentifierReference(alias) => self.typeof_aliases.get(alias.name.as_str()),
                    TSTypeName::QualifiedName(_) | TSTypeName::ThisExpression(_) => None,
                };
                reference.type_arguments.is_none() && chain.is_some_and(|chain| chain_names(chain, peeled.expr))
            }
            Evidence::NoComponent | Evidence::NotRenderable | Evidence::Unknown => false,
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
        let Some((inner, call)) = self.alias_shape(decl) else { return false };
        match self.alias_outcome(name, &inner, call) {
            AliasOutcome::Done => true,
            AliasOutcome::Declined(reason) => {
                self.decline_reason = Some(reason);
                false
            }
            AliasOutcome::NoBase => {
                self.alias_candidate = Some((inner, call));
                false
            }
        }
    }

    /// The component `decl` re-binds, `const Button = Inner` or `memo(Inner)`, and what the call is, read before `Inner`
    /// needs to be known. `None` when the initializer is not that shape, or the call names the props it builds with.
    fn alias_shape<'a>(&mut self, decl: &Binding<'_, 'a>) -> Option<(String, Option<AliasCall>)> {
        let init = decl.init.as_ref()?;
        let (inner, call) = match peel_expr(init) {
            Expression::CallExpression(call) => match aliased_component(call) {
                Some(Expression::Identifier(id)) => (id.name.as_str().to_owned(), Some(&**call)),
                _ => return None,
            },
            Expression::Identifier(id) => (id.name.as_str().to_owned(), None),
            _ => return None,
        };
        let Some(call) = call else { return Some((inner, None)) };
        // The component it receives is not the one it builds when a type argument names the props.
        if self.call_names_props_type(call) {
            return None;
        }
        // `getDisplayName(Card)` has the same shape as `memo(Card)` but returns a string; only a call that follows the
        // component-wrapper conventions produces a component.
        // `Object.assign(Impl, { Item })` returns `Impl` itself with the statics attached.
        let wrapper = is_component_alias_call(&*self, call);
        // `forwardRef(Inner)` makes a component that takes a `ref` even when `Inner` is a plain typed function.
        Some((inner, Some(AliasCall { wrapper, forward_ref: self.is_forward_ref_call(call) })))
    }

    /// Bind `name` to a clone of the component `inner` once that is collected.
    fn alias_outcome(&mut self, name: &str, inner: &str, call: Option<AliasCall>) -> AliasOutcome {
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
        let Some((inner, call)) = self.alias_candidate.take() else { return false };
        self.pending_aliases.push(PendingAlias { name: name.to_owned(), inner, call, span });
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
                match self.alias_outcome(&alias.name, &alias.inner, alias.call) {
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
                let Some(Expression::Identifier(object)) = named(&member.object) else { return false };
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
            && matches!(named(&callee.object), Some(Expression::Identifier(object)) if object.name == "Object"))
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
