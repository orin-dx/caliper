//! How each name is used in its file, so a function that only looks like a component is not reported as one: a helper
//! that is only ever called (`IndexedAccess(type)`), or a render callback that takes a ref and is never rendered
//! (`{render: TableDropIndicatorWrapper}`).

use compact_str::CompactString;
use oxc_ast::ast::*;
use oxc_span::Span;

use crate::types::{DiagnosticCode, LexedExport};

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

impl<'src> SourceDataCollector<'src> {
    fn uses_of(&mut self, name: &str) -> &mut NameUses {
        self.name_uses.entry(CompactString::from(name)).or_default()
    }

    pub(super) fn record_reference(&mut self, reference: &IdentifierReference<'_>) {
        self.uses_of(reference.name.as_str()).references += 1;
    }

    /// An exported declaration (`export default function Name`) is used from outside the file, like a reference.
    pub(super) fn record_exported_declaration(&mut self, name: &str) {
        self.uses_of(name).references += 1;
    }

    pub(super) fn record_call(&mut self, call: &CallExpression<'_>) {
        if let Expression::Identifier(callee) = super::component::peel_expr(&call.callee) {
            self.uses_of(callee.name.as_str()).calls += 1;
        }
        let renders_argument = super::component::is_component_wrapper_call(call)
            || matches!(
                super::component::peel_expr(&call.callee),
                Expression::Identifier(id) if id.name == "createElement"
            )
            || matches!(
                super::component::peel_expr(&call.callee),
                Expression::StaticMemberExpression(m) if m.property.name == "createElement"
            );
        if renders_argument {
            if let Some(Argument::Identifier(component)) = call.arguments.first() {
                self.uses_of(component.name.as_str()).rendered += 1;
            }
        }
    }

    pub(super) fn record_jsx_element(&mut self, element: &JSXOpeningElement<'_>) {
        if let JSXElementName::IdentifierReference(component) = &element.name {
            self.uses_of(component.name.as_str()).rendered += 1;
        }
    }

    /// Drop the component candidates that are not exported and are used only as functions: every reference is a call,
    /// or they take a second parameter (a ref) and are never rendered. Each is reported as a skipped candidate.
    pub(super) fn drop_functions_used_only_as_functions(&mut self) {
        let exported: rustc_hash::FxHashSet<&str> = self
            .data
            .exports
            .iter()
            .filter_map(|e| match e {
                LexedExport::LocalDeclaration { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        let mut dropped: Vec<(String, String, Span)> = Vec::new();
        for mapping in &self.data.component_mappings {
            let name = mapping.component_name.as_str();
            if exported.contains(name) {
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
            self.data.component_mappings.retain(|m| m.component_name != name);
            self.record_skip(DiagnosticCode::SkippedCandidate, format!("'{name}' {reason}, not a component"), span);
        }
    }
}
