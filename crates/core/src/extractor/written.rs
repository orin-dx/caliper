//! The unions a file writes for its props, member by member in written order (ADR 0026, S7a2).
//!
//! The checker returns a union's members sorted by kind, then value, and the order the author wrote is the order a
//! Storybook control lists the options in. This reads it from the syntax alone: the union a property signature writes,
//! directly or through a type alias or enum the same file declares at module level. A name the file does not declare
//! (an import, a global, a type reached through a namespace) is kept as the name: what it stands for is in another
//! file, which oxc cannot find without resolving modules.
//!
//! A name is expanded only where it is the declaration in scope: a type parameter or a type an enclosing namespace,
//! function or block declares shadows the module-level one, and the name stays as written. An alias is expanded once
//! per file, in the module scope it is declared in, and a union past [`MAX_WRITTEN_MEMBERS`] is not read at all.

use std::cell::RefCell;
use std::rc::Rc;

use compact_str::CompactString;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{SourceType, Span};
use rustc_hash::{FxHashMap, FxHashSet};

use super::js_text;
use crate::types::output::{BigIntValue, PropType, TupleElement};

/// The most members a written union may state, counting those of every alias it names. Past it the union is not read,
/// so the work an alias graph can ask for stays bounded.
pub(crate) const MAX_WRITTEN_MEMBERS: usize = 1024;

/// The property signatures of one file whose written type is a union of at least two members.
#[derive(Debug, Default)]
pub(crate) struct WrittenUnions {
    props: Vec<WrittenProp>,
    /// How many property signatures write a union past [`MAX_WRITTEN_MEMBERS`] and were not read.
    pub(crate) declined: usize,
}

#[derive(Debug)]
struct WrittenProp {
    owner: Owner,
    name: CompactString,
    members: Vec<WrittenMember>,
}

/// What a property signature is a member of, as the syntax shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The body of the interface of this name, wherever it is declared.
    Interface(CompactString),
    /// A type literal that is the body of the module-level alias of this name, or a member of the intersection or union
    /// that body is.
    Alias(CompactString),
    /// A type literal written in an annotation outside every declaration's body (a component's parameter, a
    /// `forwardRef` type argument), starting at this byte offset.
    Annotation(u32),
    /// A type literal anywhere else: in a function body, a namespace, a class body or another type's arguments.
    Elsewhere,
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
    /// The union written for prop `name` by the declarations `held` accepts the owner of: the one every such
    /// declaration writes alike, or `None` when there is none or two write different unions.
    pub(crate) fn find(&self, name: &str, held: impl Fn(&Owner) -> bool) -> Option<&[WrittenMember]> {
        let mut found =
            self.props.iter().filter(|prop| prop.name == name && held(&prop.owner)).map(|prop| prop.members.as_slice());
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
        body_depth: 0,
        alias_literals: Vec::new(),
        shadows: Vec::new(),
        expansions: RefCell::default(),
        unions: WrittenUnions::default(),
    };
    collector.visit_program(&parsed.program);
    collector.unions
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

/// Where a name in a written union is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// At the property signature: the type parameters and the declarations of enclosing blocks shadow the module's.
    Prop,
    /// In a module-level alias's body, where only the module's declarations are in scope.
    Module,
}

/// Why a union is not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unread {
    /// It states more than [`MAX_WRITTEN_MEMBERS`] members.
    TooMany,
    /// An alias it names flattens into itself, which TypeScript rejects.
    Circular,
}

/// What a module-level alias flattens to, once per file.
#[derive(Debug, Clone)]
enum Expansion {
    InProgress,
    Done(Result<Rc<[WrittenMember]>, Unread>),
}

/// The members of one union as they are gathered: a member stated twice is one member (`U | U` is `U`), and an alias
/// already flattened into it adds nothing again.
#[derive(Default)]
struct Members<'s> {
    list: Vec<WrittenMember>,
    aliases: Vec<&'s str>,
}

impl Members<'_> {
    fn push(&mut self, member: WrittenMember) -> Result<(), Unread> {
        match &member {
            // Two members written by kind can be two different types.
            WrittenMember::Kind(_) => {}
            WrittenMember::Type(_) | WrittenMember::Name(_) => {
                if self.list.contains(&member) {
                    return Ok(());
                }
            }
        }
        if self.list.len() == MAX_WRITTEN_MEMBERS {
            return Err(Unread::TooMany);
        }
        self.list.push(member);
        Ok(())
    }
}

struct Collector<'a, 's> {
    declared: Declared<'a, 's>,
    /// What the property signatures being read are members of.
    owner: Option<Owner>,
    /// How many property signatures enclose the current node: a member of a type literal nested in another member's
    /// type is not a prop.
    member_depth: usize,
    /// How many declaration bodies enclose the current node (an interface's, an alias's, a function's, a class's, a
    /// namespace's, a block): a type literal outside all of them is written in an annotation.
    body_depth: usize,
    /// The type literals that make up the body of the module-level alias being read, with its name.
    alias_literals: Vec<(Span, CompactString)>,
    /// The names in scope that shadow a module-level declaration: type parameters, and the types enclosing namespaces,
    /// functions and blocks declare.
    shadows: Vec<CompactString>,
    expansions: RefCell<FxHashMap<&'s str, Expansion>>,
    unions: WrittenUnions,
}

impl<'a, 's> Visit<'a> for Collector<'a, 's> {
    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        let owner = self.owner.replace(Owner::Interface(CompactString::from(it.id.name.as_str())));
        self.body_depth += 1;
        walk::walk_ts_interface_declaration(self, it);
        self.body_depth -= 1;
        self.owner = owner;
        self.shadows.truncate(scope);
    }

    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        let literals = self.alias_literals.len();
        let name = it.id.name.as_str();
        let module_level = self.body_depth == 0
            && !self.declared.ambiguous.contains(&name)
            && self.declared.aliases.get(name).is_some_and(|alias| std::ptr::eq(*alias, it));
        if module_level {
            let mut spans = Vec::new();
            body_literals(&it.type_annotation, &mut spans);
            self.alias_literals.extend(spans.into_iter().map(|span| (span, CompactString::from(name))));
        }
        self.body_depth += 1;
        walk::walk_ts_type_alias_declaration(self, it);
        self.body_depth -= 1;
        self.alias_literals.truncate(literals);
        self.shadows.truncate(scope);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        walk::walk_function(self, it, flags);
        self.shadows.truncate(scope);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        walk::walk_arrow_function_expression(self, it);
        self.shadows.truncate(scope);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let scope = self.enter_type_params(it.type_parameters.as_deref());
        walk::walk_class(self, it);
        self.shadows.truncate(scope);
    }

    fn visit_class_body(&mut self, it: &ClassBody<'a>) {
        self.in_block([], |collector| walk::walk_class_body(collector, it));
    }

    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        self.in_block(&it.statements, |collector| walk::walk_function_body(collector, it));
    }

    fn visit_ts_module_block(&mut self, it: &TSModuleBlock<'a>) {
        self.in_block(&it.body, |collector| walk::walk_ts_module_block(collector, it));
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.in_block(&it.body, |collector| walk::walk_block_statement(collector, it));
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.in_block(&it.body, |collector| walk::walk_static_block(collector, it));
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        let cases = it.cases.iter().flat_map(|case| &case.consequent);
        self.in_block(cases, |collector| walk::walk_switch_statement(collector, it));
    }

    fn visit_ts_type_literal(&mut self, it: &TSTypeLiteral<'a>) {
        let holder = match self.alias_literals.iter().find(|(span, _)| *span == it.span) {
            Some((_, alias)) => Owner::Alias(alias.clone()),
            None if self.body_depth == 0 => Owner::Annotation(it.span.start),
            None => Owner::Elsewhere,
        };
        let owner = self.owner.replace(holder);
        walk::walk_ts_type_literal(self, it);
        self.owner = owner;
    }

    fn visit_ts_property_signature(&mut self, it: &TSPropertySignature<'a>) {
        if self.member_depth == 0
            && let Some(owner) = &self.owner
            && let Some(name) = it.key.static_name()
            && let Some(annotation) = &it.type_annotation
        {
            let mut members = Members::default();
            match self.push_members(&annotation.type_annotation, Scope::Prop, &mut members) {
                Ok(()) if members.list.len() > 1 => self.unions.props.push(WrittenProp {
                    owner: owner.clone(),
                    name: CompactString::from(js_text::property_name(&name)),
                    members: members.list,
                }),
                Ok(()) | Err(Unread::Circular) => {}
                Err(Unread::TooMany) => self.unions.declined += 1,
            }
        }
        self.member_depth += 1;
        walk::walk_ts_property_signature(self, it);
        self.member_depth -= 1;
    }
}

impl<'a, 's> Collector<'a, 's> {
    fn enter_type_params(&mut self, params: Option<&TSTypeParameterDeclaration<'a>>) -> usize {
        let scope = self.shadows.len();
        for param in params.into_iter().flat_map(|params| &params.params) {
            self.shadows.push(CompactString::from(param.name.name.as_str()));
        }
        scope
    }

    /// Walks a body whose `statements` may declare types that shadow the module's.
    fn in_block<'b>(&mut self, statements: impl IntoIterator<Item = &'b Statement<'a>>, walk: impl FnOnce(&mut Self))
    where
        'a: 'b,
    {
        let scope = self.shadows.len();
        self.shadows.extend(statements.into_iter().filter_map(declared_type));
        self.body_depth += 1;
        walk(self);
        self.body_depth -= 1;
        self.shadows.truncate(scope);
    }

    /// A module-level declaration named `name` that nothing in `scope` shadows and no other declaration shares.
    fn declares(&self, name: &str, scope: Scope) -> bool {
        let shadowed = match scope {
            Scope::Prop => self.shadows.iter().any(|shadow| shadow == name),
            Scope::Module => false,
        };
        !shadowed && !self.declared.ambiguous.contains(&name)
    }

    /// Adds the members `ty` writes to `out`, flattening a union and a same-file non-generic alias or enum it names.
    fn push_members(&self, ty: &TSType<'a>, scope: Scope, out: &mut Members<'s>) -> Result<(), Unread> {
        match ty {
            TSType::TSUnionType(union) => {
                for member in &union.types {
                    self.push_members(member, scope, out)?;
                }
                Ok(())
            }
            TSType::TSParenthesizedType(inner) => self.push_members(&inner.type_annotation, scope, out),
            TSType::TSTypeReference(reference) => {
                if let TSTypeName::IdentifierReference(id) = &reference.type_name
                    && reference.type_arguments.is_none()
                    && self.declares(id.name.as_str(), scope)
                {
                    let name = id.name.as_str();
                    if let Some((&name, &alias)) = self.declared.aliases.get_key_value(name)
                        && alias.type_parameters.is_none()
                    {
                        if !out.aliases.contains(&name) {
                            out.aliases.push(name);
                            for member in self.expand(name, alias)?.iter() {
                                out.push(member.clone())?;
                            }
                        }
                        return Ok(());
                    }
                    if let Some(members) = self.declared.enums.get(name) {
                        for member in members {
                            out.push(WrittenMember::Name(format!("{name}.{member}").into()))?;
                        }
                        return Ok(());
                    }
                }
                self.member(ty, scope).map_or(Ok(()), |member| out.push(member))
            }
            _ => self.member(ty, scope).map_or(Ok(()), |member| out.push(member)),
        }
    }

    /// The members the module-level alias `name` flattens to, read once per file in the module scope.
    fn expand(&self, name: &'s str, alias: &'s TSTypeAliasDeclaration<'a>) -> Result<Rc<[WrittenMember]>, Unread> {
        let known = self.expansions.borrow().get(name).cloned();
        match known {
            Some(Expansion::Done(done)) => return done,
            Some(Expansion::InProgress) => return Err(Unread::Circular),
            None => {}
        }
        self.expansions.borrow_mut().insert(name, Expansion::InProgress);
        let mut members = Members::default();
        let done =
            self.push_members(&alias.type_annotation, Scope::Module, &mut members).map(|()| Rc::from(members.list));
        self.expansions.borrow_mut().insert(name, Expansion::Done(done.clone()));
        done
    }

    /// One member as written; `None` for a type the syntax does not state (`typeof x`, `keyof T`, an indexed access, a
    /// conditional or mapped type).
    fn member(&self, ty: &TSType<'a>, scope: Scope) -> Option<WrittenMember> {
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
            TSType::TSParenthesizedType(inner) => self.member(&inner.type_annotation, scope),
            TSType::TSArrayType(array) => match self.member(&array.element_type, scope) {
                Some(WrittenMember::Type(element)) => written(PropType::Array(Box::new(element))),
                _ => kind(MemberKind::Array),
            },
            TSType::TSTupleType(tuple) => {
                let elements: Option<Vec<TupleElement>> = tuple
                    .element_types
                    .iter()
                    .map(|element| match self.member(element.as_ts_type()?, scope) {
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
                TSTypeOperatorOperator::Readonly => match self.member(&operator.type_annotation, scope) {
                    Some(WrittenMember::Type(inner)) => written(PropType::Readonly(Box::new(inner))),
                    _ => kind(MemberKind::Readonly),
                },
                TSTypeOperatorOperator::Keyof | TSTypeOperatorOperator::Unique => None,
            },
            TSType::TSFunctionType(_) | TSType::TSConstructorType(_) => kind(MemberKind::Function),
            TSType::TSTypeLiteral(_) | TSType::TSIntersectionType(_) | TSType::TSTemplateLiteralType(_) => {
                body_kind(ty).map(WrittenMember::Kind)
            }
            TSType::TSTypeReference(reference) => self.reference(reference, scope),
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
    fn reference(&self, reference: &TSTypeReference<'a>, scope: Scope) -> Option<WrittenMember> {
        let name = type_name(&reference.type_name);
        if let TSTypeName::IdentifierReference(id) = &reference.type_name
            && self.declares(id.name.as_str(), scope)
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

/// The spans of the type literals that make up an alias's body: the body itself, or the members of the intersection or
/// union it is.
fn body_literals(ty: &TSType<'_>, out: &mut Vec<Span>) {
    match ty {
        TSType::TSTypeLiteral(literal) => out.push(literal.span),
        TSType::TSParenthesizedType(inner) => body_literals(&inner.type_annotation, out),
        TSType::TSIntersectionType(intersection) => {
            for member in &intersection.types {
                body_literals(member, out);
            }
        }
        TSType::TSUnionType(union) => {
            for member in &union.types {
                body_literals(member, out);
            }
        }
        _ => {}
    }
}

/// The name of the type `statement` declares in its block, when it declares one.
fn declared_type(statement: &Statement<'_>) -> Option<CompactString> {
    let declaration = match statement {
        Statement::ExportDeclaration(export) => &export.declaration,
        _ => statement.as_declaration()?,
    };
    let name = match declaration {
        Declaration::TSTypeAliasDeclaration(alias) => &alias.id.name,
        Declaration::TSInterfaceDeclaration(interface) => &interface.id.name,
        Declaration::TSEnumDeclaration(declaration) => &declaration.id.name,
        Declaration::ClassDeclaration(class) => &class.id.as_ref()?.name,
        Declaration::TSImportEqualsDeclaration(import) => &import.id.name,
        Declaration::VariableDeclaration(_)
        | Declaration::FunctionDeclaration(_)
        | Declaration::TSExternalModuleDeclaration(_)
        | Declaration::TSNamespaceDeclaration(_)
        | Declaration::TSGlobalDeclaration(_) => return None,
    };
    Some(CompactString::from(name.as_str()))
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

    fn in_interface<'f>(file: &'f WrittenUnions, interface: &str, name: &str) -> Option<&'f [WrittenMember]> {
        file.find(name, |owner| *owner == Owner::Interface(interface.into()))
    }

    fn in_literal<'f>(file: &'f WrittenUnions, name: &str) -> Option<&'f [WrittenMember]> {
        file.find(name, |owner| !matches!(owner, Owner::Interface(_)))
    }

    fn lit(text: &str) -> WrittenMember {
        WrittenMember::Type(PropType::StringLiteral(text.into()))
    }

    #[test]
    fn an_interface_member_writes_its_union_in_order() {
        let file = written("interface Props { size?: 'sm' | 'md' | 'lg'; on: boolean }");
        assert_eq!(in_interface(&file, "Props", "size"), Some(&[lit("sm"), lit("md"), lit("lg")][..]));
        assert_eq!(in_interface(&file, "Props", "on"), None);
        assert_eq!(in_literal(&file, "size"), None);
    }

    #[test]
    fn a_same_file_alias_and_enum_are_flattened_where_they_are_named() {
        let file = written(
            "type Size = 'b' | 'a'\nenum Tone { Warm = 'w', Cold = 'c' }\n\
             interface Props { p: 'z' | Size | null; t?: Tone | -1 }",
        );
        assert_eq!(
            in_interface(&file, "Props", "p"),
            Some(&[lit("z"), lit("b"), lit("a"), WrittenMember::Type(PropType::Null)][..])
        );
        assert_eq!(
            in_interface(&file, "Props", "t"),
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
        assert_eq!(in_interface(&file, "Props", "p"), Some(&[WrittenMember::Name("Size".into()), lit("a")][..]));
        assert_eq!(in_interface(&file, "Props", "q"), Some(&[WrittenMember::Name("T".into()), lit("b")][..]));
    }

    #[test]
    fn a_type_literal_member_has_no_interface_and_a_nested_one_is_not_a_prop() {
        let file = written(
            "export default function Icon(props: Base & { size?: 'L' | 'S' }) {}\n\
             interface Props { style: { size: 'x' | 'y' } }",
        );
        assert_eq!(in_literal(&file, "size"), Some(&[lit("L"), lit("S")][..]));
    }

    #[test]
    fn two_declarations_that_write_different_unions_give_none() {
        let file = written("type A = { p: 'a' | 'b' }\ntype B = { p: 'b' | 'a' }");
        assert_eq!(in_literal(&file, "p"), None);
        let file = written("type A = { p: 'a' | 'b' }\ntype B = { p: 'a' | 'b' }");
        assert_eq!(in_literal(&file, "p"), Some(&[lit("a"), lit("b")][..]));
    }

    #[test]
    fn members_written_only_by_kind_keep_their_kind() {
        let file = written(
            "interface Item { a: string }\ntype Wrap<T> = T[]\n\
             interface Props { p: (() => void) | { a: 1 } | [number] | Wrap<number> | Item | string[] | `x${string}` | ReadonlyArray<Item> }",
        );
        assert_eq!(
            in_interface(&file, "Props", "p"),
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
    fn an_alias_named_many_times_is_expanded_once() {
        // Each alias names the one before twice: expanded per reference, `U30` has 2^31 members.
        let mut source = String::from("type U0 = 'a' | 'b'\n");
        for level in 1..=30 {
            source.push_str(&format!("type U{level} = U{} | U{}\n", level - 1, level - 1));
        }
        source.push_str("interface Props { p: U30 | 'c' }");
        let file = written(&source);
        assert_eq!(in_interface(&file, "Props", "p"), Some(&[lit("a"), lit("b"), lit("c")][..]));
    }

    #[test]
    fn a_union_past_the_member_bound_is_not_written() {
        let members: Vec<String> = (0..=MAX_WRITTEN_MEMBERS).map(|index| format!("'m{index}'")).collect();
        let file =
            written(&format!("type Big = {}\ninterface Props {{ p: Big | 'z'; q: 'x' | 'y' }}", members.join(" | ")));
        assert_eq!(in_interface(&file, "Props", "p"), None);
        assert_eq!(in_interface(&file, "Props", "q"), Some(&[lit("x"), lit("y")][..]));
        assert_eq!(file.declined, 1);
    }

    #[test]
    fn a_declaration_in_an_enclosing_namespace_or_function_shadows_a_module_alias() {
        let file = written(
            "type Size = 'a' | 'b'\n\
             namespace Ns { type Size = 'b' | 'a'; export interface Props { size: Size | 'c' } }\n\
             function make() { interface Size { x: 1 } interface Local { size: Size | 'c' } }",
        );
        assert_eq!(in_interface(&file, "Props", "size"), Some(&[WrittenMember::Name("Size".into()), lit("c")][..]));
        assert_eq!(in_interface(&file, "Local", "size"), Some(&[WrittenMember::Name("Size".into()), lit("c")][..]));
    }

    #[test]
    fn a_module_alias_expands_in_the_module_scope_where_a_local_name_shadows_one_it_names() {
        let file = written(
            "type Inner = 'a' | 'b'\ntype Outer = Inner | 'c'\n\
             namespace Ns { type Inner = 'x'; export interface Props { p: Outer } }",
        );
        assert_eq!(in_interface(&file, "Props", "p"), Some(&[lit("a"), lit("b"), lit("c")][..]));
    }

    #[test]
    fn a_type_literal_is_held_by_the_module_alias_whose_body_it_is_or_by_the_annotation_it_is_written_in() {
        let file = written(
            "type Props = Base & { p: 'a' | 'b' }\n\
             type Other = { q: 'a' | 'b' }\n\
             type Wrapped = Box<{ r: 'a' | 'b' }>\n\
             namespace Ns { export type Props = { s: 'a' | 'b' } }\n\
             export function C(props: { t: 'a' | 'b' }) { type Local = { u: 'a' | 'b' }; return null }",
        );
        let holder =
            |name: &str| file.props.iter().find(|prop| prop.name == name).map(|prop| prop.owner.clone()).unwrap();
        assert_eq!(holder("p"), Owner::Alias("Props".into()));
        assert_eq!(holder("q"), Owner::Alias("Other".into()));
        assert_eq!(holder("r"), Owner::Elsewhere);
        assert_eq!(holder("s"), Owner::Elsewhere);
        assert!(matches!(holder("t"), Owner::Annotation(_)));
        assert_eq!(holder("u"), Owner::Elsewhere);
    }

    #[test]
    fn a_recursive_alias_stops_where_it_names_itself() {
        let file = written("type Json = string | null | Json[]\ninterface Props { p?: Json }");
        assert_eq!(
            in_interface(&file, "Props", "p"),
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
