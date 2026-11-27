//! How each name is used and exported in its file. A function that only looks like a component is not reported as one:
//! a helper that is only ever called (`IndexedAccess(type)`), or a render callback that takes a ref and is never
//! rendered (`{render: TableDropIndicatorWrapper}`). Every component records how its module exports it.

use compact_str::CompactString;
use oxc_ast::ast::*;
use oxc_span::Span;
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

/// What an `Object.assign` call says its first argument is, the component it returns. `Rejected` is final: no other
/// reader may take the argument for the component. Only `NotApplicable` (no `Object.assign`, or a first argument that is
/// no name, such as a nested call) lets a caller read the argument its own way.
pub(super) enum Role {
    Accepted(super::names::Named),
    Rejected,
    NotApplicable,
}

/// The roles of a plainly written `Object.assign` call's arguments ([`SourceDataCollector::assign_roles`]), each read as
/// origin/main read it: a member attachment that only a peel exposes is not recorded.
pub(super) struct AssignRoles<'b, 'a> {
    /// The first argument's name, the object the call returns.
    pub(super) target: Option<super::names::Named>,
    /// The names the call's result is an alias of.
    pub(super) aliases: Vec<super::names::Named>,
    /// The object literals whose members are attached.
    pub(super) objects: Vec<&'b ObjectExpression<'a>>,
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

    /// The name a value of an `Object.assign` call or a member assignment is, only as origin/main read it
    /// (`peeled::value_name` `Plain`): a name only a peel exposes is attached to nothing.
    pub(super) fn plain_member(&self, value: &Expression<'_>) -> Option<super::names::Named> {
        super::peeled::value_name(value, &self.bindings).filter(|named| matches!(named, super::names::Named::Plain(_)))
    }

    /// What an `Object.assign` call's arguments are, read in one place for every consumer: the inline-member scan
    /// (`inline`), the named-member read, the alias rule and the default export. `None` when the call is no plainly
    /// written `Object.assign` (`Recognition::of_object_assign`): a call only a peel exposes attaches nothing.
    pub(super) fn assign_roles<'b, 'a>(
        &self,
        call: &'b CallExpression<'a>,
        inline: bool,
    ) -> Option<AssignRoles<'b, 'a>> {
        if !super::component::is_object_assign_call(call, self)
            || super::component::Recognition::of_object_assign(call, inline) == super::component::Recognition::Peeled
        {
            return None;
        }
        let target =
            call.arguments.first().and_then(Argument::as_expression).and_then(|first| self.plain_member(first));
        let mut aliases = Vec::new();
        let mut objects = Vec::new();
        for (position, argument) in call.arguments.iter().enumerate() {
            // The inline-member scan reads the second argument only.
            if inline && position != 1 {
                continue;
            }
            let Some(value) = argument.as_expression() else { continue };
            // Every identifier argument is what the result is, and every object literal a source, as main read them.
            match super::component::main_peel(value) {
                Expression::Identifier(name) => aliases.push(super::names::Named::plain(name.name.as_str())),
                Expression::ObjectExpression(object) => objects.push(&**object),
                _ => {}
            }
        }
        Some(AssignRoles { target, aliases, objects })
    }

    /// The component an `Object.assign` call returns, as `assign_roles` reads its target ([`Role`]).
    pub(super) fn assign_target(&self, call: &CallExpression<'_>) -> Role {
        if !super::component::is_object_assign_call(call, self) {
            return Role::NotApplicable;
        }
        // A call `assign_roles` rejects, such as one whose callee only a peel exposes, is no built-in at all: nothing in
        // its arguments is what it returns.
        let Some(roles) = self.assign_roles(call, false) else { return Role::Rejected };
        let names =
            call.arguments.first().and_then(Argument::as_expression).is_some_and(super::peeled::names_identifier);
        match roles.target.clone().filter(|target| roles.aliases.contains(target)) {
            Some(target) => Role::Accepted(target),
            None if names => Role::Rejected,
            // A target that is no name, such as `memo(Impl)`, of an accepted call: its caller reads it.
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
        let Some(Expression::CallExpression(call)) = init.map(super::peeled::read_value) else { return };
        let Some(roles) = self.assign_roles(call, false) else { return };
        for alias in &roles.aliases {
            self.attached_to.push(Attachment {
                sub: Subject::local(alias.as_str()),
                parent: CompactString::from(parent.as_str()),
                property: None,
            });
        }
        for object in &roles.objects {
            for property in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else { continue };
                let Some(key) = property.key.static_name() else { continue };
                if let Some(sub) = self.plain_member(&property.value) {
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
        // The assigned value as origin/main read it: one only a peel exposes (`<number>Item`) is attached to nothing.
        let Some(sub) = self.plain_member(&assignment.right) else { return };
        let parent = super::names::Named::plain(parent.name.as_str());
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
