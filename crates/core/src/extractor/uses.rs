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

/// Which component an export or attachment is of. A named component is the module's binding of that name. An inline
/// sub-component (`Parent.Sub = (props: P) => …`) is no binding, and `A.Item = …` beside `B.Item = …` gives two
/// mappings one name, so only where its function starts tells them apart, and from a binding of the same name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Subject {
    /// The module's binding of this name.
    Local(CompactString),
    /// The inline function that starts at this offset: the `span_start` of its mapping.
    Inline(u32),
}

impl Subject {
    pub(super) fn local(name: &str) -> Self {
        Self::Local(CompactString::from(name))
    }

    /// The component a mapping is: an inline sub-component by where it starts, any other by its name.
    pub(super) fn of(mapping: &crate::types::ComponentMapping) -> Self {
        match mapping.rule {
            crate::types::DetectionRule::AttachedSubComponent => Self::Inline(mapping.span_start),
            _ => Self::local(mapping.component_name.as_str()),
        }
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

/// Whether the call is `Object.assign(...)`.
pub(super) fn is_object_assign_call(call: &CallExpression<'_>) -> bool {
    matches!(&call.callee, Expression::StaticMemberExpression(callee)
        if callee.property.name == "assign"
            && matches!(&callee.object, Expression::Identifier(object) if object.name == "Object"))
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
    pub(super) fn record_exported_local(&mut self, local: &str, exported: &str) {
        let names = self.exported_locals.entry(CompactString::from(local)).or_default();
        if !names.iter().any(|name| name == exported) {
            names.push(CompactString::from(exported));
        }
    }

    /// `Parent.property = Sub` or `Object.assign(Parent, { property: Sub })`.
    pub(super) fn record_attached(&mut self, sub: Subject, parent: &str, property: &str) {
        self.attached_to.push(Attachment {
            sub,
            parent: CompactString::from(parent),
            property: Some(CompactString::from(property)),
        });
    }

    /// `const Parent = Object.assign(Root, { Sub })`: `Parent` is the root under another name, with `Sub` attached.
    pub(super) fn record_object_assign_declarator(&mut self, parent: &str, init: Option<&Expression<'_>>) {
        let Some(Expression::CallExpression(call)) = init.map(super::component::peel_expr) else { return };
        if !is_object_assign_call(call) {
            return;
        }
        for argument in &call.arguments {
            match argument.as_expression().map(super::component::peel_expr) {
                Some(Expression::Identifier(root)) => self.attached_to.push(Attachment {
                    sub: Subject::local(root.name.as_str()),
                    parent: CompactString::from(parent),
                    property: None,
                }),
                Some(Expression::ObjectExpression(object)) => {
                    for property in &object.properties {
                        let ObjectPropertyKind::ObjectProperty(property) = property else { continue };
                        let Some(key) = property.key.static_name() else { continue };
                        if let Expression::Identifier(sub) = super::component::peel_expr(&property.value) {
                            self.record_attached(
                                Subject::local(sub.name.as_str()),
                                parent,
                                &super::js_text::property_name(&key),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// `Parent.Sub = Sub` at statement level.
    pub(super) fn record_member_assignment(&mut self, statement: &ExpressionStatement<'_>) {
        let Expression::AssignmentExpression(assignment) = &statement.expression else { return };
        let AssignmentTarget::StaticMemberExpression(target) = &assignment.left else { return };
        let (Expression::Identifier(parent), Expression::Identifier(sub)) =
            (&target.object, super::component::peel_expr(&assignment.right))
        else {
            return;
        };
        self.record_attached(Subject::local(sub.name.as_str()), parent.name.as_str(), target.property.name.as_str());
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
        if let Expression::Identifier(callee) = super::component::peel_expr(&call.callee) {
            self.uses_of(callee.name.as_str()).calls += 1;
        }
        let renders_argument = super::evidence::Scope::is_react_wrapper(&*self, call)
            || super::component::is_component_wrapper_call(call)
            || super::evidence::is_element_factory_callee(super::component::peel_expr(&call.callee), &*self);
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
