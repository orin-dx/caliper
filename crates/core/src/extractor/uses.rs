//! How each name is used and exported in its file. A function that only looks like a component is not reported as one:
//! a helper that is only ever called (`IndexedAccess(type)`), or a render callback that takes a ref and is never
//! rendered (`{render: TableDropIndicatorWrapper}`). Every component records how its module exports it.

use compact_str::CompactString;
use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};
use rustc_hash::FxHashMap;

use crate::types::{ComponentExport, DiagnosticCode};

use super::SourceDataCollector;

/// The references to one name in a file.
#[derive(Default, Clone, Copy)]
pub(crate) struct NameUses {
    references: u32,
    /// References that are the callee of a call: `Name(...)`.
    calls: u32,
    /// References that render it: a JSX tag, or the first argument of `createElement` or a component wrapper.
    rendered: u32,
}

#[cfg(test)]
impl NameUses {
    pub(super) fn rendered(&self) -> u32 {
        self.rendered
    }
}

/// Which component an export or attachment is of. A named component is the module's binding of that name. An inline
/// sub-component (`Parent.Sub = (props: P) => …`) and an anonymous default export are no binding, and `A.Item = …`
/// beside `B.Item = …`, or a default export named after its file beside a binding of that name, give two mappings one
/// name, so only where the function starts tells them apart, and from a binding of the same name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Subject {
    /// The module's binding of this name.
    Local(CompactString),
    /// The function that starts at this offset: the `span_start` of its mapping.
    Inline(u32),
}

impl Subject {
    pub(super) fn local(name: &str) -> Self {
        Self::Local(CompactString::from(name))
    }

    /// The component a mapping is: one with no binding by where it starts, any other by its name.
    pub(super) fn of(mapping: &crate::types::ComponentMapping) -> Self {
        if mapping.bound { Self::local(mapping.component_name.as_str()) } else { Self::Inline(mapping.span_start) }
    }
}

/// How the module exports each of its components.
pub(crate) type ComponentExports = FxHashMap<Subject, Vec<ComponentExport>>;

/// A component attached to another: `Parent.Sub = Sub`, `Object.assign(Parent, { Sub })`.
pub(crate) struct Attachment {
    /// The attached component.
    sub: Subject,
    /// The local name of the component it is attached to.
    parent: CompactString,
    /// The property it is attached under; `None` when it is the parent itself under another name
    /// (`const Parent = Object.assign(Root, { Sub })` makes `Parent` the same object as `Root`).
    property: Option<CompactString>,
}

/// Whether an `Object.assign` call has the minimal shape a call origin/main never read is recognised in: exactly a target
/// and one object literal (through parentheses, `as const` or `satisfies`), every member a plain key written once.
fn minimal_object_assign(call: &CallExpression<'_>) -> bool {
    let [_, source] = call.arguments.as_slice() else { return false };
    let Some(Expression::ObjectExpression(object)) = source.as_expression().map(super::peeled::read_value) else {
        return false;
    };
    let mut keys = rustc_hash::FxHashSet::default();
    object.properties.iter().all(|property| match property {
        ObjectPropertyKind::ObjectProperty(property) if !property.computed => {
            property.key.static_name().is_some_and(|key| keys.insert(super::js_text::property_name(&key)))
        }
        _ => false,
    })
}

/// What an `Object.assign` call says its first argument is, the component it returns. `Rejected` is final: no other
/// reader may take the argument for the component. Only `NotApplicable` (no `Object.assign`, or a first argument that is
/// no name, such as a nested call) lets a caller read the argument its own way.
pub(super) enum Role {
    Accepted(super::names::Named),
    Rejected,
    NotApplicable,
}

/// Where a member attachment only a peel exposes would be recorded ([`SourceDataCollector::attachment_allowed`]).
pub(super) struct AttachmentSite<'s> {
    /// The start of the value at the top of the statement or declarator: an assignment, or an `Object.assign` call.
    pub(super) at: u32,
    /// Whether the assignment is a simple `=`; an `Object.assign` call always is.
    pub(super) simple: bool,
    /// The parent's name, which export propagation finds by its bare spelling.
    pub(super) parent: &'s str,
    /// For an `Object.assign` call, whether it has the minimal shape; `None` for an assignment.
    pub(super) assign_minimal: Option<bool>,
}

/// The starts of the values at the top of the module's own statements and declarators (an expression statement's
/// expression, a declarator's or default export's value, each as written and beneath its wrappers): an attachment
/// only a peel exposes is recorded only at one of them (`attachment_allowed`).
pub(super) fn top_level_values(program: &Program<'_>) -> rustc_hash::FxHashSet<u32> {
    use oxc_span::GetSpan;
    let mut starts = rustc_hash::FxHashSet::default();
    let mut add = |expr: &Expression<'_>| {
        starts.insert(expr.span().start);
        starts.insert(super::peeled::read_value(expr).span().start);
    };
    for statement in &program.body {
        let declaration = match statement {
            Statement::ExpressionStatement(statement) => {
                add(&statement.expression);
                continue;
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let Some(expr) = export.declaration.as_expression() {
                    add(expr);
                }
                continue;
            }
            Statement::ExportDeclaration(export) => &export.declaration,
            statement => match statement.as_declaration() {
                Some(declaration) => declaration,
                None => continue,
            },
        };
        if let Declaration::VariableDeclaration(variables) = declaration {
            for init in variables.declarations.iter().filter_map(|declarator| declarator.init.as_ref()) {
                add(init);
            }
        }
    }
    starts
}

/// The roles of an `Object.assign` call's arguments ([`SourceDataCollector::assign_roles`]).
pub(super) struct AssignRoles<'b, 'a> {
    /// The first argument's name, the object the call returns.
    pub(super) target: Option<super::names::Named>,
    /// The names the call's result is an alias of.
    pub(super) aliases: Vec<super::names::Named>,
    /// The object literals whose members are attached, each with its own read: `Plain` only when origin/main read the
    /// call and the literal (`component::main_peel`); one only a peel exposes (`<const>{ Item }`) makes its members
    /// `Peeled`.
    pub(super) objects: Vec<(&'b ObjectExpression<'a>, super::component::Recognition)>,
    /// Whether the call has the minimal shape, the only one a child only a peel exposes is recorded in.
    pub(super) minimal: bool,
}

impl<'src> SourceDataCollector<'src> {
    fn uses_of(&mut self, name: &str) -> &mut NameUses {
        self.name_uses.entry(CompactString::from(name)).or_default()
    }

    pub(super) fn record_reference(&mut self, reference: &IdentifierReference<'_>) {
        self.uses_of(reference.name.as_str()).references += 1;
    }

    /// A local name the module exports, and the name it exports it under (`export function Name` is `Name` under
    /// `Name`, `export { Name as Other }` under `Other`, `export default Name` under `default`).
    pub(super) fn record_exported_local(&mut self, local: &super::names::Named, exported: &str) {
        let names = self.exported_locals.entry(CompactString::from(local.as_str())).or_default();
        if !names.iter().any(|name| name == exported) {
            names.push(CompactString::from(exported));
        }
    }

    /// The local component a default export's call wraps: `React.forwardRef(Inner) as T`, `memo(forwardRef(Card))` and
    /// `Object.assign(Root, { Item })` give `Inner`, `Card` and `Root`. `None` for a call that is no component alias, or
    /// whose argument is no name.
    pub(super) fn component_wrapped_by(&self, expression: &Expression<'_>) -> Option<super::names::Named> {
        let Expression::CallExpression(call) = super::peeled::read_value(expression) else { return None };
        if !super::component::is_component_alias_call(self, call) {
            return None;
        }
        // An `Object.assign` is its target only as `assign_roles` reads it, and a target it rejects stays rejected.
        match self.assign_target(call) {
            Role::Accepted(target) => return Some(target),
            Role::Rejected => return None,
            Role::NotApplicable => {}
        }
        let first = call.arguments.first()?.as_expression()?;
        // The argument's name as `value_name` vouches for it; a call written through a wrapper main did not read makes it
        // a name only a peel shows.
        match super::peeled::value_name(first, &self.bindings) {
            Some(inner) if super::component::peeled_beyond_main(expression) => {
                self.bindings.named(inner.as_str(), super::component::Recognition::Peeled)
            }
            Some(inner) => Some(inner),
            None => match super::peeled::read_value(first) {
                Expression::CallExpression(_) => self.component_wrapped_by(first),
                _ => None,
            },
        }
    }

    /// An anonymous default export is named after its file. When another component of the file has that name, it is
    /// `Name (default)`, so the two have names and output keys of their own whatever the order they are written in.
    pub(super) fn name_the_anonymous_default(&mut self) {
        let Some(start) = self.anonymous_default else { return };
        let mappings = &mut self.data.component_mappings;
        let Some(at) = mappings.iter().position(|m| !m.bound && m.span_start == start) else { return };
        let name = mappings[at].component_name.clone();
        if mappings.iter().enumerate().any(|(index, m)| index != at && m.component_name == name) {
            mappings[at].component_name = format!("{name} (default)");
        }
    }

    /// `Parent.property = Sub` or `Object.assign(Parent, { property: Sub })`.
    pub(super) fn record_attached(&mut self, sub: Subject, parent: &super::names::Named, property: &str) {
        self.attached_to.push(Attachment {
            sub,
            parent: CompactString::from(parent.as_str()),
            property: Some(CompactString::from(property)),
        });
    }

    /// Whether this head reads an `Object.assign` call's members onto `parent`, which export propagation finds by its
    /// bare name: a `Plain` parent as origin/main did; a `Peeled` one (a `UniqueName`) only at module level, only when
    /// `Object` is the global one and the file writes no member that may be `Object.assign`.
    pub(super) fn trusts_object_assign(&self, parent: &super::names::Named) -> bool {
        match parent {
            super::names::Named::Plain(_) => true,
            super::names::Named::Peeled(_) => self.function_depth == 0 && self.object_assign_is_builtin(),
        }
    }

    /// Whether `Object.assign` is the built-in: `Object` is the global, and the file writes no member that may be
    /// `assign` (`Bindings::writes_to`). Consulted for every part of an `Object.assign` call only a peel exposes: the
    /// callee, the target, a source and a member.
    pub(super) fn object_assign_is_builtin(&self) -> bool {
        self.bindings
            .global(super::well_known::OBJECT.name)
            .is_some_and(|object| !self.bindings.writes_to(&object, "assign"))
    }

    /// The name an argument or member value of an `Object.assign` call is, classified by its own read and never by the
    /// call's (`peeled::value_name`): `Plain` only when origin/main read the call and that value as the identifier it
    /// is. A value only a peel exposes, or any value of a call main never read, only when the file declares the name
    /// exactly once, as a second declaration may be the one export propagation finds. A plain call never upgrades a
    /// peeled child.
    ///
    /// `minimal` is whether the whole call has the minimal shape (`minimal_object_assign`): a peeled child of any other
    /// call is not recorded, as a later source may overwrite it.
    pub(super) fn trusted_member(
        &self,
        recognition: super::component::Recognition,
        value: &Expression<'_>,
    ) -> Option<super::names::Named> {
        let named = super::peeled::value_name(value, &self.bindings)?;
        match recognition {
            super::component::Recognition::Plain => Some(named),
            super::component::Recognition::Peeled => {
                self.bindings.named(named.as_str(), super::component::Recognition::Peeled)
            }
        }
    }

    /// The one gate for a member attachment only a peel exposes (its member, its target or its call): recorded only when
    /// every condition holds. A plain attachment never comes here and keeps origin/main's reading.
    pub(super) fn attachment_allowed(&self, site: &AttachmentSite<'_>) -> bool {
        // (a) A simple `=`: a compound operator stores something else.
        let simple = site.simple;
        // (b) A statement or declarator at the top level of the module, not in a function, class or block.
        let module_scope = self.top_level_values.contains(&site.at);
        // (c) A parent that resolves to its one declaration in the file, which is not an import.
        let parent = self.bindings.unique(site.parent).is_some() && self.bindings.imported(site.parent).is_none();
        // (d) For an `Object.assign` call, the built-in.
        let builtin = site.assign_minimal.is_none() || self.object_assign_is_builtin();
        // (e) For an `Object.assign` call, the minimal shape: no later source may overwrite the member.
        let minimal = site.assign_minimal.unwrap_or(true);
        simple && module_scope && parent && builtin && minimal
    }

    /// What an `Object.assign` call's arguments are, read in one place for every consumer: the inline-member scan
    /// (`inline`), the named-member read, the alias rule and the default export. `None` when the call is no
    /// `Object.assign` or its parent (`parent`, else the target) is not trusted (`trusts_object_assign`).
    pub(super) fn assign_roles<'b, 'a>(
        &self,
        call: &'b CallExpression<'a>,
        inline: bool,
        parent: Option<&super::names::Named>,
    ) -> Option<AssignRoles<'b, 'a>> {
        if !super::component::is_object_assign_call(call, self) {
            return None;
        }
        let recognition = super::component::Recognition::of_object_assign(call, inline);
        let read = |argument: &'b Argument<'a>| argument.as_expression().map(super::peeled::read_value);
        let peeled = recognition == super::component::Recognition::Peeled;
        // A call main never read is recognised only in the minimal shape: a target and one object literal, with no
        // spread, computed or repeated key, so no later source can overwrite a member.
        let minimal = minimal_object_assign(call);
        if peeled && !minimal {
            return None;
        }
        // The target by its own read: one only a peel exposes (`<typeof Root>Root`) counts, in a plain call too, only as
        // a name declared once at module level, as for a call main never read.
        let target = call
            .arguments
            .first()
            .and_then(Argument::as_expression)
            .and_then(|first| self.trusted_member(recognition, first))
            .filter(|target| {
                matches!(target, super::names::Named::Plain(_)) || (minimal && self.trusts_object_assign(target))
            });
        if peeled {
            let parent = match parent {
                Some(parent) => self.bindings.named(parent.as_str(), recognition)?,
                None => target.clone()?,
            };
            if !self.trusts_object_assign(&parent) {
                return None;
            }
        }
        let mut aliases = Vec::new();
        let mut objects = Vec::new();
        for (position, argument) in call.arguments.iter().enumerate() {
            // The inline-member scan reads the second argument only.
            if inline && position != 1 {
                continue;
            }
            match read(argument) {
                // The call returns its target: for a call main never read, only the first argument is what the result
                // is, and only when the file declares it once; a later one is a source whose properties are copied.
                // Read plainly, every identifier argument, as origin/main read it.
                Some(Expression::Identifier(name)) => {
                    let alias = match recognition {
                        super::component::Recognition::Plain => {
                            Some(super::names::Named::Plain(name.name.as_str().into()))
                        }
                        super::component::Recognition::Peeled if position == 0 => target.clone(),
                        super::component::Recognition::Peeled => None,
                    };
                    aliases.extend(alias);
                }
                Some(Expression::ObjectExpression(object)) => {
                    let main_read = argument.as_expression().is_some_and(|value| {
                        matches!(super::component::main_peel(value), Expression::ObjectExpression(_))
                    });
                    let own = if main_read { recognition } else { super::component::Recognition::Peeled };
                    // Its members count only through `trusted_member`, which records a peeled one only in a call of
                    // the minimal shape.
                    objects.push((&**object, own));
                }
                // A target only a peel exposes is what the call returns, as `target` vouches for it.
                _ if position == 0 => aliases.extend(target.clone().filter(|_| !inline)),
                _ => {}
            }
        }
        Some(AssignRoles { target, aliases, objects, minimal })
    }

    /// The component an `Object.assign` call returns, as `assign_roles` reads its target ([`Role`]).
    pub(super) fn assign_target(&self, call: &CallExpression<'_>) -> Role {
        if !super::component::is_object_assign_call(call, self) {
            return Role::NotApplicable;
        }
        let accepted = self
            .assign_roles(call, false, None)
            .and_then(|roles| roles.target.clone().filter(|target| roles.aliases.contains(target)));
        let names =
            call.arguments.first().and_then(Argument::as_expression).is_some_and(super::peeled::names_identifier);
        match accepted {
            Some(target) => Role::Accepted(target),
            None if names => Role::Rejected,
            None => Role::NotApplicable,
        }
    }

    /// `const Parent = Object.assign(Root, { Sub })`, or the default export of one with `Root` as `parent`: `Parent` is
    /// the root under another name, with `Sub` attached.
    pub(super) fn record_object_assign_declarator(
        &mut self,
        parent: &super::names::Named,
        init: Option<&Expression<'_>>,
    ) {
        let Some(init) = init else { return };
        let Expression::CallExpression(call) = super::peeled::read_value(init) else { return };
        let Some(roles) = self.assign_roles(call, false, Some(parent)) else { return };
        let site = AttachmentSite {
            at: oxc_span::GetSpan::span(init).start,
            simple: true,
            parent: parent.as_str(),
            assign_minimal: Some(roles.minimal),
        };
        let allowed = self.attachment_allowed(&site);
        let peeled = |named: &super::names::Named| matches!(named, super::names::Named::Peeled(_));
        for alias in &roles.aliases {
            if peeled(alias) && !allowed {
                continue;
            }
            self.attached_to.push(Attachment {
                sub: Subject::local(alias.as_str()),
                parent: CompactString::from(parent.as_str()),
                property: None,
            });
        }
        for &(object, recognition) in &roles.objects {
            for property in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else { continue };
                let Some(key) = property.key.static_name() else { continue };
                if let Some(sub) = self.trusted_member(recognition, &property.value)
                    && (!peeled(&sub) || allowed)
                {
                    self.record_attached(Subject::local(sub.as_str()), parent, &super::js_text::property_name(&key));
                }
            }
        }
    }

    /// `Parent.Sub = Sub` at statement level.
    pub(super) fn record_member_assignment(&mut self, statement: &ExpressionStatement<'_>) {
        let Expression::AssignmentExpression(assignment) = &statement.expression else { return };
        let AssignmentTarget::StaticMemberExpression(target) = &assignment.left else { return };
        let Expression::Identifier(parent) = &target.object else { return };
        // The assigned value, read only through a wrapper origin/main did not read (`<number>Item`), only as the one
        // declaration its name can mean (`value_name`).
        let Some(sub) = super::peeled::value_name(&assignment.right, &self.bindings) else { return };
        // A value only a peel exposes attaches only to a parent that resolves too: export propagation finds the parent by
        // its bare name, and a parameter of that name is a second declaration.
        let parent = match sub {
            super::names::Named::Plain(_) => super::names::Named::plain(parent.name.as_str()),
            super::names::Named::Peeled(_) => {
                let site = AttachmentSite {
                    at: statement.expression.span().start,
                    simple: assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign,
                    parent: parent.name.as_str(),
                    assign_minimal: None,
                };
                // The gate resolves the parent (`attachment_allowed` (c)); it is found by its bare name from here on.
                if !self.attachment_allowed(&site) {
                    return;
                }
                super::names::Named::plain(parent.name.as_str())
            }
        };
        self.record_attached(Subject::local(sub.as_str()), &parent, target.property.name.as_str());
    }

    /// How the module exports each of its exported components: the exported locals under their export names, and any
    /// component attached to an exported one through the parent's exports, transitively. A component keeps the paths of
    /// the first round that reaches it, so attachments that form a cycle end. A parent is always a binding; a component
    /// is a [`Subject`], so an inline sub-component gets the paths of its own attachments and no other's.
    pub(super) fn component_exports(&self) -> ComponentExports {
        let mut exports: ComponentExports = self
            .exported_locals
            .iter()
            .map(|(local, names)| {
                (Subject::local(local), names.iter().map(|name| ComponentExport::named(name.as_str())).collect())
            })
            .collect();
        if let Some(start) = self.anonymous_default {
            exports.insert(Subject::Inline(start), vec![ComponentExport::named("default")]);
        }
        loop {
            let mut reached = ComponentExports::default();
            for Attachment { sub, parent, property } in &self.attached_to {
                if exports.contains_key(sub) {
                    continue;
                }
                let Some(through) = exports.get(&Subject::Local(parent.clone())) else { continue };
                let paths = reached.entry(sub.clone()).or_default();
                for export in through {
                    let path = match property {
                        Some(property) => export.member(property.as_str()),
                        None => export.clone(),
                    };
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
            if reached.is_empty() {
                return exports;
            }
            exports.extend(reached);
        }
    }

    pub(super) fn record_call(&mut self, call: &CallExpression<'_>) {
        // A call is a use of the value called, whatever an assertion says it returns: `(Helper as H)(props)` calls
        // `Helper`.
        let peeled = super::component::called(call);
        if let Some(Expression::Identifier(callee)) = peeled.resolve_called(&*self) {
            self.uses_of(callee.name.as_str()).calls += 1;
        }
        // An element factory asserted to give a string (`(createElement as (t: any) => string)(Foo)`) builds no
        // element of its argument.
        let renders_argument = super::component::is_component_wrapper(&*self, call)
            || peeled.resolve(&*self).is_some_and(|callee| {
                super::evidence::is_element_factory_callee(callee, super::well_known::beyond_main(&call.callee), &*self)
            });
        if renders_argument && let Some(Argument::Identifier(component)) = call.arguments.first() {
            self.uses_of(component.name.as_str()).rendered += 1;
        }
    }

    pub(super) fn record_jsx_element(&mut self, element: &JSXOpeningElement<'_>) {
        if let JSXElementName::IdentifierReference(component) = &element.name {
            self.uses_of(component.name.as_str()).rendered += 1;
        }
    }

    /// Drop the component candidates that are not exported and are used only as functions: every reference is a call,
    /// or they take a second parameter (a ref) and are never rendered. Each is reported as a skipped candidate.
    pub(super) fn drop_functions_used_only_as_functions(&mut self, exported: &ComponentExports) {
        let mut dropped: Vec<(String, String, Span)> = Vec::new();
        for mapping in &self.data.component_mappings {
            let name = mapping.component_name.as_str();
            // An inline sub-component is no binding, so no use of a name is a use of it.
            let subject = Subject::of(mapping);
            if matches!(subject, Subject::Inline(_)) || exported.contains_key(&subject) {
                continue;
            }
            let uses = self.name_uses.get(name).copied().unwrap_or_default();
            let reason = if uses.calls > 0 && uses.references == uses.calls {
                "is only ever called as a function in this file, never rendered or passed on, so it is a helper"
            } else if self.takes_a_second_parameter.contains(name) && uses.rendered == 0 {
                "takes a second parameter (a ref) and is never rendered or passed to a component wrapper in this \
                 file, so it is a render callback"
            } else {
                continue;
            };
            dropped.push((name.to_owned(), reason.to_owned(), Span::new(mapping.span_start, mapping.span_end)));
        }
        for (name, reason, span) in dropped {
            self.data.component_mappings.retain(|m| m.component_name != name || m.span_start != span.start);
            self.record_skip(DiagnosticCode::SkippedCandidate, format!("'{name}' {reason}, not a component"), span);
        }
    }
}
