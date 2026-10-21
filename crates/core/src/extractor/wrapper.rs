//! Components declared through a wrapper call whose props travel as a type argument:
//! `const Button = polymorphicFactory<ButtonFactory>(...)`, `const Item = withContext<HTMLDivElement, ItemProps>(...)`.
//!
//! Nothing here names a library. A candidate is a PascalCase binding initialized with a generic call whose first
//! argument could plausibly be a component (a function, a tag name, another component). The props type is chosen from
//! the type arguments by shape, and only against declarations visible in the same file, so the rule needs no
//! cross-file state. Candidates are resolved when the file finishes, because `type ButtonFactory = ...` is often
//! declared after the call that uses it.

use std::collections::BTreeMap;

use oxc_ast::ast::*;
use oxc_span::Span;

use crate::types::{CollectedType, CollectedTypeAlias, ComponentMapping, DetectionRule, TypeName};

use super::SourceDataCollector;

/// One type argument of a candidate wrapper call.
pub(crate) struct WrapperTypeArg {
    /// The reference name and its stringified type arguments, when the argument is a plain type reference.
    reference: Option<(TypeName, Vec<String>)>,
}

/// A wrapper call waiting for the rest of the file to be seen.
pub(crate) struct PendingWrapper {
    name: String,
    type_args: Vec<WrapperTypeArg>,
    description: String,
    tags: BTreeMap<String, String>,
    span: Span,
}

/// Member that carries the props type inside a factory payload: `Factory<{ props: ButtonProps; ... }>`.
const PAYLOAD_PROPS_MEMBER: &str = "props";

/// Names that are DOM elements or refs rather than props, so they are never the props argument.
fn is_element_type(name: &str) -> bool {
    let bare = name.rsplit('.').next().unwrap_or(name);
    bare == "Element" || ((bare.starts_with("HTML") || bare.starts_with("SVG")) && bare.ends_with("Element"))
}

/// Whether a first argument could be a component: a function, a tag name, or a reference to something else callable.
/// Literals, `null` and object or array values rule out context defaults, option bags and constants.
fn plausible_component_argument(arg: Option<&Argument<'_>>) -> bool {
    let Some(expr) = arg.and_then(Argument::as_expression) else { return false };
    matches!(
        expr,
        Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::StringLiteral(_)
            | Expression::Identifier(_)
            | Expression::CallExpression(_)
            | Expression::StaticMemberExpression(_)
    )
}

impl<'src> SourceDataCollector<'src> {
    /// Record `const Name = callee<...>(first, ...)` as a candidate. Returns whether it was recorded.
    pub(super) fn try_record_wrapper_call<'a>(&mut self, decl: &VariableDeclarator<'a>, name: &str) -> bool {
        let Some(init) = decl.init.as_ref() else { return false };
        let Expression::CallExpression(call) = super::component::peel_expr(init) else { return false };
        let Some(type_arguments) = call.type_arguments.as_ref() else { return false };
        if self.extract_callee_name(call).is_none() || !plausible_component_argument(call.arguments.first()) {
            return false;
        }
        let type_args: Vec<WrapperTypeArg> = type_arguments
            .params
            .iter()
            .map(|ty| WrapperTypeArg { reference: self.extract_type_name_from_type(ty) })
            .collect();
        let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);
        self.pending_wrappers.push(PendingWrapper {
            name: name.to_owned(),
            type_args,
            description,
            tags,
            span: decl.span,
        });
        true
    }

    /// Turn the recorded candidates into mappings, now that every declaration in the file is known.
    pub(super) fn promote_wrapper_calls(&mut self) {
        for pending in std::mem::take(&mut self.pending_wrappers) {
            if self.data.component_mappings.iter().any(|m| m.component_name == pending.name) {
                continue;
            }
            let Some((props_name, props_args)) = self.select_wrapper_props(&pending.type_args) else { continue };
            self.data.component_mappings.push(
                ComponentMapping::new(
                    pending.name,
                    props_name,
                    self.file_path.clone(),
                    pending.span.start,
                    pending.span.end,
                    DetectionRule::WrapperTypeArgument,
                )
                .with_props_type_args(props_args)
                .with_doc(pending.description, pending.tags),
            );
        }
    }

    /// Pick the props type from a wrapper call's type arguments, last first (the props usually follow the element).
    fn select_wrapper_props(&self, type_args: &[WrapperTypeArg]) -> Option<(TypeName, Vec<String>)> {
        for arg in type_args.iter().rev() {
            let Some((name, args)) = &arg.reference else { continue };
            if is_element_type(name) {
                continue;
            }
            if let Some(props) = self.payload_props(name) {
                return Some((props, Vec::new()));
            }
            let declared_here = self.data.interfaces.contains_key(&self.scoped_key(name))
                || self.data.type_aliases.contains_key(&self.scoped_key(name));
            // A name from another file cannot be inspected here; only the props naming convention vouches for it.
            if declared_here || name.ends_with("Props") {
                return Some((name.clone(), args.clone()));
            }
        }
        None
    }

    /// `type ButtonFactory = Anything<{ props: ButtonProps; ... }>` yields `ButtonProps`.
    fn payload_props(&self, alias_name: &str) -> Option<TypeName> {
        let CollectedTypeAlias::Passthrough { target: CollectedType::Named { args, .. }, .. } =
            self.data.type_aliases.get(&self.scoped_key(alias_name))?
        else {
            return None;
        };
        args.iter().find_map(|arg| match arg {
            CollectedType::Object(fields) => fields.iter().find_map(|field| match &field.collected_type {
                CollectedType::Named { name, args } if field.name == PAYLOAD_PROPS_MEMBER && args.is_empty() => {
                    Some(name.clone())
                }
                _ => None,
            }),
            _ => None,
        })
    }
}
