//! The unions a file writes for its props, member by member in written order (ADR 0026, S7a2).
//!
//! The checker returns a union's members sorted by kind, then value, and the order the author wrote is the order a
//! Storybook control lists the options in. This reads it from the syntax alone: the union a property signature writes,
//! directly or through a type alias or enum the same file declares at module level. A name the file does not declare
//! (an import, a global, a type reached through a namespace) is kept as the name: what it stands for is in another
//! file, which oxc cannot find without resolving modules.

use compact_str::CompactString;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use rustc_hash::{FxHashMap, FxHashSet};

use super::js_text;
use crate::types::output::{BigIntValue, PropType, TupleElement};

/// The property signatures of one file whose written type is a union of at least two members.
#[derive(Debug, Default)]
pub(crate) struct WrittenUnions {
    props: Vec<WrittenProp>,
}

#[derive(Debug)]
struct WrittenProp {
    /// The interface whose body declares it; `None` for a member of a type literal.
    interface: Option<CompactString>,
    name: CompactString,
    members: Vec<WrittenMember>,
}

/// One member of a written union, as far as the file states it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WrittenMember {
    /// A member the syntax states in full: a literal, a keyword, an array or tuple of those.
    Type(PropType),
    /// A reference to a type the file does not declare, as written (`React.ReactNode`, `Size.Small` for a member of an
    /// enum the file declares).
    Name(CompactString),
    /// A member of which only the kind is written: a function type, an object literal, an intersection.
    Kind(MemberKind),
}

/// The kind of a union member whose type is not written out in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Function,
    Object,
    Callable,
    Array,
    Tuple,
    Readonly,
    Intersection,
    Template,
}

impl WrittenUnions {
    /// The union written for prop `name` by the interface `interface` (`Some`) or by a type literal (`None`): the one
    /// every such declaration writes alike, or `None` when there is none or two write different unions.
    pub(crate) fn find(&self, interface: Option<&str>, name: &str) -> Option<&[WrittenMember]> {
        let mut found = self
            .props
            .iter()
            .filter(|prop| prop.name == name && prop.interface.as_deref() == interface)
            .map(|prop| prop.members.as_slice());
        let first = found.next()?;
        found.all(|other| other == first).then_some(first)
    }
}

/// The written unions of `source`, the text of `path`. Empty for a file too deeply nested to parse safely.
pub(crate) fn written_unions(path: &camino::Utf8Path, source: &str) -> WrittenUnions {
    if super::max_bracket_nesting_depth(source) > super::MAX_SOURCE_NESTING_DEPTH {
        return WrittenUnions::default();
    }
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let options = ParseOptions { preserve_parens: false, ..ParseOptions::default() };
    let parsed = Parser::new(&allocator, source, source_type).with_options(options).parse();
    let mut collector = Collector {
        declared: Declared::of(&parsed.program),
        owner: None,
        member_depth: 0,
        type_params: Vec::new(),
        props: Vec::new(),
    };
    collector.visit_program(&parsed.program);
    WrittenUnions { props: collector.props }
}

/// The module-level declarations a union member can name: a name declared twice is in `ambiguous` and expands to
/// nothing.
#[derive(Default)]
struct Declared<'a, 's> {
    aliases: FxHashMap<&'s str, &'s TSTypeAliasDeclaration<'a>>,
    interfaces: FxHashSet<&'s str>,
    enums: FxHashMap<&'s str, Vec<CompactString>>,
    ambiguous: Vec<&'s str>,
}

impl<'a, 's> Declared<'a, 's> {
    fn of(program: &'s Program<'a>) -> Self {
        let mut declared = Declared::default();
        for statement in &program.body {
            let declaration = match statement {
                Statement::ExportDeclaration(export) => Some(&export.declaration),
                _ => statement.as_declaration(),
            };
            match declaration {
                Some(Declaration::TSTypeAliasDeclaration(alias)) => {
                    let name = alias.id.name.as_str();
                    if declared.aliases.insert(name, &**alias).is_some() {
                        declared.ambiguous.push(name);
                    }
                }
                Some(Declaration::TSInterfaceDeclaration(interface)) => {
                    declared.interfaces.insert(interface.id.name.as_str());
                }
                Some(Declaration::TSEnumDeclaration(declaration)) => {
                    let members = declaration
                        .body
                        .members
                        .iter()
                        .filter_map(|member| match &member.id {
                            TSEnumMemberName::Identifier(id) => Some(CompactString::from(id.name.as_str())),
                            TSEnumMemberName::String(s) | TSEnumMemberName::ComputedString(s) => {
                                Some(CompactString::from(js_text::lossy(s.value).as_ref()))
                            }
                            TSEnumMemberName::ComputedTemplateString(_) => None,
                        })
                        .collect();
                    let name = declaration.id.name.as_str();
                    if declared.enums.insert(name, members).is_some() {
                        declared.ambiguous.push(name);
                    }
                }
                _ => {}
            }
        }
        declared
    }
}

struct Collector<'a, 's> {
    declared: Declared<'a, 's>,
    /// The interface or type literal whose body is being read: `Some(Some(name))` for an interface.
    owner: Option<Option<CompactString>>,
    /// How many property signatures enclose the current node: a member of a type literal nested in another member's
    /// type is not a prop.
    member_depth: usize,
    /// The type parameters in scope, which shadow a module-level declaration of the same name.
    type_params: Vec<CompactString>,
    props: Vec<WrittenProp>,
}

impl<'a> Visit<'a> for Collector<'a, '_> {
    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        let owner = self.owner.replace(Some(CompactString::from(it.id.name.as_str())));
        walk::walk_ts_interface_declaration(self, it);
        self.owner = owner;
        self.type_params.truncate(scope);
    }

    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        walk::walk_ts_type_alias_declaration(self, it);
        self.type_params.truncate(scope);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        walk::walk_function(self, it, flags);
        self.type_params.truncate(scope);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        walk::walk_arrow_function_expression(self, it);
        self.type_params.truncate(scope);
    }

    fn visit_ts_type_literal(&mut self, it: &TSTypeLiteral<'a>) {
        let owner = self.owner.replace(None);
        walk::walk_ts_type_literal(self, it);
        self.owner = owner;
    }

    fn visit_ts_property_signature(&mut self, it: &TSPropertySignature<'a>) {
        if self.member_depth == 0
            && let Some(owner) = &self.owner
            && let Some(name) = it.key.static_name()
            && let Some(annotation) = &it.type_annotation
        {
            let mut members = Vec::new();
            self.push_members(&annotation.type_annotation, &mut Vec::new(), &mut members);
            if members.len() > 1 {
                self.props.push(WrittenProp {
                    interface: owner.clone(),
                    name: CompactString::from(js_text::property_name(&name)),
                    members,
                });
            }
        }
        self.member_depth += 1;
        walk::walk_ts_property_signature(self, it);
        self.member_depth -= 1;
    }
}

impl<'a> Collector<'a, '_> {
    fn enter_type_params(&mut self, params: Option<&TSTypeParameterDeclaration<'a>>) -> usize {
        let scope = self.type_params.len();
        for param in params.into_iter().flat_map(|params| &params.params) {
            self.type_params.push(CompactString::from(param.name.name.as_str()));
        }
        scope
    }

    /// A module-level declaration named `name` that no type parameter in scope shadows and no other declaration shares.
    fn declares(&self, name: &str) -> bool {
        !self.type_params.iter().any(|param| param == name) && !self.declared.ambiguous.contains(&name)
    }

    /// Appends the members `ty` writes, flattening a union and a same-file non-generic alias or enum it names. `seen`
    /// holds the aliases being expanded, so a recursive one (`type Json = string | Json[]`) stops.
    fn push_members<'s>(&'s self, ty: &'s TSType<'a>, seen: &mut Vec<&'s str>, out: &mut Vec<WrittenMember>) {
        match ty {
            TSType::TSUnionType(union) => {
                for member in &union.types {
                    self.push_members(member, seen, out);
                }
            }
            TSType::TSParenthesizedType(inner) => self.push_members(&inner.type_annotation, seen, out),
            TSType::TSTypeReference(reference) => {
                if let TSTypeName::IdentifierReference(id) = &reference.type_name
                    && reference.type_arguments.is_none()
                    && self.declares(id.name.as_str())
                {
                    let name = id.name.as_str();
                    if let Some(alias) = self.declared.aliases.get(name)
                        && alias.type_parameters.is_none()
                    {
                        if !seen.contains(&name) {
                            seen.push(name);
                            self.push_members(&alias.type_annotation, seen, out);
                            seen.pop();
                        }
                        return;
                    }
                    if let Some(members) = self.declared.enums.get(name) {
                        out.extend(members.iter().map(|member| WrittenMember::Name(format!("{name}.{member}").into())));
                        return;
                    }
                }
                out.extend(self.member(ty));
            }
            _ => out.extend(self.member(ty)),
        }
    }

    /// One member as written; `None` for a type the syntax does not state (`typeof x`, `keyof T`, an indexed access, a
    /// conditional or mapped type).
    fn member(&self, ty: &TSType<'a>) -> Option<WrittenMember> {
        let written = |prop_type| Some(WrittenMember::Type(prop_type));
        let kind = |kind| Some(WrittenMember::Kind(kind));
        match ty {
            TSType::TSStringKeyword(_) => written(PropType::String),
            TSType::TSNumberKeyword(_) => written(PropType::Number),
            TSType::TSBooleanKeyword(_) => written(PropType::Boolean),
            TSType::TSNullKeyword(_) => written(PropType::Null),
            TSType::TSUndefinedKeyword(_) => written(PropType::Undefined),
            TSType::TSAnyKeyword(_) => written(PropType::Any),
            TSType::TSNeverKeyword(_) => written(PropType::Never),
            TSType::TSUnknownKeyword(_) => written(PropType::Unknown),
            TSType::TSVoidKeyword(_) => written(PropType::Void),
            TSType::TSBigIntKeyword(_) => written(PropType::BigInt),
            TSType::TSLiteralType(literal) => literal_type(&literal.literal).map(WrittenMember::Type),
            TSType::TSParenthesizedType(inner) => self.member(&inner.type_annotation),
            TSType::TSArrayType(array) => match self.member(&array.element_type) {
                Some(WrittenMember::Type(element)) => written(PropType::Array(Box::new(element))),
                _ => kind(MemberKind::Array),
            },
            TSType::TSTupleType(tuple) => {
                let elements: Option<Vec<TupleElement>> = tuple
                    .element_types
                    .iter()
                    .map(|element| match self.member(element.as_ts_type()?) {
                        Some(WrittenMember::Type(element)) => Some(TupleElement::required(element)),
                        _ => None,
                    })
                    .collect();
                match elements {
                    Some(elements) => written(PropType::Tuple(elements)),
                    None => kind(MemberKind::Tuple),
                }
            }
            TSType::TSTypeOperatorType(operator) => match operator.operator {
                TSTypeOperatorOperator::Readonly => match self.member(&operator.type_annotation) {
                    Some(WrittenMember::Type(inner)) => written(PropType::Readonly(Box::new(inner))),
                    _ => kind(MemberKind::Readonly),
                },
                TSTypeOperatorOperator::Keyof | TSTypeOperatorOperator::Unique => None,
            },
            TSType::TSFunctionType(_) | TSType::TSConstructorType(_) => kind(MemberKind::Function),
            TSType::TSTypeLiteral(_) | TSType::TSIntersectionType(_) | TSType::TSTemplateLiteralType(_) => {
                body_kind(ty).map(WrittenMember::Kind)
            }
            TSType::TSTypeReference(reference) => self.reference(reference),
            TSType::TSUnionType(_)
            | TSType::TSObjectKeyword(_)
            | TSType::TSSymbolKeyword(_)
            | TSType::TSIntrinsicKeyword(_)
            | TSType::TSThisType(_)
            | TSType::TSTypeQuery(_)
            | TSType::TSIndexedAccessType(_)
            | TSType::TSConditionalType(_)
            | TSType::TSMappedType(_)
            | TSType::TSInferType(_)
            | TSType::TSImportType(_)
            | TSType::TSTypePredicate(_)
            | TSType::TSNamedTupleMember(_)
            | TSType::JSDocNullableType(_)
            | TSType::JSDocNonNullableType(_)
            | TSType::JSDocUnknownType(_) => None,
        }
    }

    /// A reference that is not a same-file alias of a union: the kind of a same-file interface or generic alias, an
    /// array type by its standard name, else the name as written.
    fn reference(&self, reference: &TSTypeReference<'a>) -> Option<WrittenMember> {
        let name = type_name(&reference.type_name);
        if let TSTypeName::IdentifierReference(id) = &reference.type_name
            && self.declares(id.name.as_str())
        {
            let name = id.name.as_str();
            if self.declared.interfaces.contains(name) {
                return Some(WrittenMember::Kind(MemberKind::Object));
            }
            if let Some(alias) = self.declared.aliases.get(name) {
                // A generic alias's members depend on its arguments, and a non-generic one named here is not flattened:
                // only the kind of its body is certain.
                return body_kind(&alias.type_annotation).map(WrittenMember::Kind);
            }
        }
        match name.as_str() {
            "Array" => Some(WrittenMember::Kind(MemberKind::Array)),
            "ReadonlyArray" => Some(WrittenMember::Kind(MemberKind::Readonly)),
            _ => Some(WrittenMember::Name(name)),
        }
    }
}

/// The kind of a type as written at its top level, without following a name.
fn body_kind(ty: &TSType<'_>) -> Option<MemberKind> {
    match ty {
        TSType::TSArrayType(_) => Some(MemberKind::Array),
        TSType::TSTupleType(_) => Some(MemberKind::Tuple),
        TSType::TSFunctionType(_) | TSType::TSConstructorType(_) => Some(MemberKind::Function),
        TSType::TSTypeLiteral(literal) => Some(
            if literal.members.iter().any(|member| {
                matches!(
                    member,
                    TSSignature::TSCallSignatureDeclaration(_) | TSSignature::TSConstructSignatureDeclaration(_)
                )
            }) {
                MemberKind::Callable
            } else {
                MemberKind::Object
            },
        ),
        TSType::TSIntersectionType(_) => Some(MemberKind::Intersection),
        TSType::TSTemplateLiteralType(_) => Some(MemberKind::Template),
        TSType::TSTypeOperatorType(operator) if operator.operator == TSTypeOperatorOperator::Readonly => {
            Some(MemberKind::Readonly)
        }
        TSType::TSParenthesizedType(inner) => body_kind(&inner.type_annotation),
        _ => None,
    }
}

fn type_name(name: &TSTypeName<'_>) -> CompactString {
    match name {
        TSTypeName::IdentifierReference(id) => CompactString::from(id.name.as_str()),
        TSTypeName::QualifiedName(qualified) => {
            let mut text = type_name(&qualified.left);
            text.push('.');
            text.push_str(qualified.right.name.as_str());
            text
        }
        TSTypeName::ThisExpression(_) => CompactString::from("this"),
    }
}

fn literal_type(literal: &TSLiteral<'_>) -> Option<PropType> {
    match literal {
        TSLiteral::StringLiteral(s) => Some(PropType::StringLiteral(js_text::lossy(s.value).into_owned())),
        TSLiteral::NumericLiteral(n) => Some(PropType::NumberLiteral(n.value)),
        TSLiteral::BooleanLiteral(b) => Some(PropType::BoolLiteral(b.value)),
        TSLiteral::BigIntLiteral(b) => BigIntValue::parse(b.value.as_str()).map(PropType::BigIntLiteral),
        TSLiteral::UnaryExpression(unary) => match (&unary.operator, &unary.argument) {
            (UnaryOperator::UnaryNegation, Expression::NumericLiteral(n)) => Some(PropType::NumberLiteral(-n.value)),
            (UnaryOperator::UnaryNegation, Expression::BigIntLiteral(b)) => {
                BigIntValue::parse(&format!("-{}", b.value.as_str())).map(PropType::BigIntLiteral)
            }
            _ => None,
        },
        TSLiteral::TemplateLiteral(template) if template.expressions.is_empty() => template
            .quasis
            .first()
            .and_then(|quasi| quasi.value.cooked.as_ref())
            .map(|cooked| PropType::StringLiteral(js_text::lossy(*cooked).into_owned())),
        TSLiteral::TemplateLiteral(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(source: &str) -> WrittenUnions {
        written_unions(camino::Utf8Path::new("/p/a.tsx"), source)
    }

    fn lit(text: &str) -> WrittenMember {
        WrittenMember::Type(PropType::StringLiteral(text.into()))
    }

    #[test]
    fn an_interface_member_writes_its_union_in_order() {
        let file = written("interface Props { size?: 'sm' | 'md' | 'lg'; on: boolean }");
        assert_eq!(file.find(Some("Props"), "size"), Some(&[lit("sm"), lit("md"), lit("lg")][..]));
        assert_eq!(file.find(Some("Props"), "on"), None);
        assert_eq!(file.find(None, "size"), None);
    }

    #[test]
    fn a_same_file_alias_and_enum_are_flattened_where_they_are_named() {
        let file = written(
            "type Size = 'b' | 'a'\nenum Tone { Warm = 'w', Cold = 'c' }\n\
             interface Props { p: 'z' | Size | null; t?: Tone | -1 }",
        );
        assert_eq!(
            file.find(Some("Props"), "p"),
            Some(&[lit("z"), lit("b"), lit("a"), WrittenMember::Type(PropType::Null)][..])
        );
        assert_eq!(
            file.find(Some("Props"), "t"),
            Some(
                &[
                    WrittenMember::Name("Tone.Warm".into()),
                    WrittenMember::Name("Tone.Cold".into()),
                    WrittenMember::Type(PropType::NumberLiteral(-1.0)),
                ][..]
            )
        );
    }

    #[test]
    fn an_imported_alias_stays_a_name_and_a_type_parameter_shadows_a_module_alias() {
        let file = written(
            "import type { Size } from './size'\ntype T = 'x' | 'y'\n\
             interface Props<T> { p: Size | 'a'; q: T | 'b' }",
        );
        assert_eq!(file.find(Some("Props"), "p"), Some(&[WrittenMember::Name("Size".into()), lit("a")][..]));
        assert_eq!(file.find(Some("Props"), "q"), Some(&[WrittenMember::Name("T".into()), lit("b")][..]));
    }

    #[test]
    fn a_type_literal_member_has_no_interface_and_a_nested_one_is_not_a_prop() {
        let file = written(
            "export default function Icon(props: Base & { size?: 'L' | 'S' }) {}\n\
             interface Props { style: { size: 'x' | 'y' } }",
        );
        assert_eq!(file.find(None, "size"), Some(&[lit("L"), lit("S")][..]));
    }

    #[test]
    fn two_declarations_that_write_different_unions_give_none() {
        let file = written("type A = { p: 'a' | 'b' }\ntype B = { p: 'b' | 'a' }");
        assert_eq!(file.find(None, "p"), None);
        let file = written("type A = { p: 'a' | 'b' }\ntype B = { p: 'a' | 'b' }");
        assert_eq!(file.find(None, "p"), Some(&[lit("a"), lit("b")][..]));
    }

    #[test]
    fn members_written_only_by_kind_keep_their_kind() {
        let file = written(
            "interface Item { a: string }\ntype Wrap<T> = T[]\n\
             interface Props { p: (() => void) | { a: 1 } | [number] | Wrap<number> | Item | string[] | `x${string}` | ReadonlyArray<Item> }",
        );
        assert_eq!(
            file.find(Some("Props"), "p"),
            Some(
                &[
                    WrittenMember::Kind(MemberKind::Function),
                    WrittenMember::Kind(MemberKind::Object),
                    WrittenMember::Type(PropType::Tuple(vec![TupleElement::required(PropType::Number)])),
                    WrittenMember::Kind(MemberKind::Array),
                    WrittenMember::Kind(MemberKind::Object),
                    WrittenMember::Type(PropType::Array(Box::new(PropType::String))),
                    WrittenMember::Kind(MemberKind::Template),
                    WrittenMember::Kind(MemberKind::Readonly),
                ][..]
            )
        );
    }

    #[test]
    fn a_recursive_alias_stops_where_it_names_itself() {
        let file = written("type Json = string | null | Json[]\ninterface Props { p?: Json }");
        assert_eq!(
            file.find(Some("Props"), "p"),
            Some(
                &[
                    WrittenMember::Type(PropType::String),
                    WrittenMember::Type(PropType::Null),
                    WrittenMember::Kind(MemberKind::Array),
                ][..]
            )
        );
    }
}
