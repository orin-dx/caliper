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

use crate::types::{CollectedType, CollectedTypeAlias, ComponentMapping, DetectionRule, DiagnosticCode, TypeName};

use super::evidence::{renders, FunctionShape};
use super::SourceDataCollector;

/// One type argument of a candidate wrapper call.
pub(crate) struct WrapperTypeArg {
    /// The reference name and its stringified type arguments, when the argument is a plain type reference.
    reference: Option<(TypeName, Vec<String>)>,
}

/// A wrapper call waiting for the rest of the file to be seen.
pub(crate) struct PendingWrapper {
    name: String,
    /// The only evidence was a tag name, so the props must come from a factory payload.
    needs_payload: bool,
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

fn is_element_type_arg(ty: &TSType<'_>) -> bool {
    match ty {
        TSType::TSTypeReference(r) => match &r.type_name {
            TSTypeName::IdentifierReference(id) => is_element_type(id.name.as_str()),
            TSTypeName::QualifiedName(q) => is_element_type(q.right.name.as_str()),
            _ => false,
        },
        _ => false,
    }
}

fn is_component_reference(name: &str) -> bool {
    // `_Box` is Mantine's spelling of an implementation component.
    name.trim_start_matches(['_', '$']).starts_with(|c: char| c.is_ascii_uppercase())
}

/// How strongly the first argument says the call builds a component.
enum ArgumentEvidence {
    Sufficient,
    /// A tag name with no element type argument: a component only if a type argument is a factory payload.
    TagNeedsPayload,
}

/// Evidence that the call builds a component from its first argument: a function that renders (Mantine's
/// `factory<F>((props) => <Box />)`), a tag name next to an element type argument (`withContext<HTMLDivElement, P>("div")`)
/// or a factory payload (`tableElement<TableTrFactory>("tr")`), a reference to a component
/// (`withRootProvider<P>(Ark.Root)`), or a wrapped component (`polymorphic<P>(forwardRef(...))`). A tag string alone
/// also describes `makeLabel<P>("button")`, and a lowercase value describes a context default or a store, so neither
/// counts.
fn component_argument_evidence(callee: &str, call: &CallExpression<'_>) -> Result<ArgumentEvidence, &'static str> {
    if callee.rsplit('.').next() == Some("createContext") {
        return Err("creates a context");
    }
    let Some(first) = call.arguments.first().and_then(Argument::as_expression) else {
        return Err("has no argument that could be a component");
    };
    let element_type_arg = || call.type_arguments.as_ref().is_some_and(|a| a.params.iter().any(is_element_type_arg));
    let sufficient = |r: Result<(), &'static str>| r.map(|()| ArgumentEvidence::Sufficient);
    match first {
        Expression::ArrowFunctionExpression(f) => {
            sufficient(renders(&FunctionShape::of_arrow(f)).map_err(|_why| "is given a function that never renders"))
        }
        Expression::FunctionExpression(f) => {
            sufficient(renders(&FunctionShape::of_function(f)).map_err(|_why| "is given a function that never renders"))
        }
        Expression::StringLiteral(_) if element_type_arg() => Ok(ArgumentEvidence::Sufficient),
        Expression::StringLiteral(_) => Ok(ArgumentEvidence::TagNeedsPayload),
        Expression::Identifier(id) if is_component_reference(id.name.as_str()) => Ok(ArgumentEvidence::Sufficient),
        Expression::StaticMemberExpression(m) if is_component_reference(m.property.name.as_str()) => {
            Ok(ArgumentEvidence::Sufficient)
        }
        Expression::CallExpression(inner) if super::component::is_component_wrapper_call(inner) => {
            Ok(ArgumentEvidence::Sufficient)
        }
        _ => Err("is given an argument that is not a render function, a tag or a component reference"),
    }
}

impl<'src> SourceDataCollector<'src> {
    /// Record `const Name = callee<...>(first, ...)` as a candidate. Returns whether it was recorded.
    pub(super) fn try_record_wrapper_call<'a>(&mut self, decl: &VariableDeclarator<'a>, name: &str) -> bool {
        let Some(init) = decl.init.as_ref() else { return false };
        let Expression::CallExpression(call) = super::component::peel_expr(init) else { return false };
        let Some(type_arguments) = call.type_arguments.as_ref() else { return false };
        let Some(callee) = self.extract_callee_name(call) else { return false };
        let needs_payload = match component_argument_evidence(&callee, call) {
            Ok(ArgumentEvidence::Sufficient) => false,
            Ok(ArgumentEvidence::TagNeedsPayload) => true,
            Err(reason) => {
                self.decline_reason = Some(format!("'{name}' is a generic call to '{callee}' that {reason}"));
                return false;
            }
        };
        let type_args: Vec<WrapperTypeArg> = type_arguments
            .params
            .iter()
            .map(|ty| WrapperTypeArg { reference: self.extract_type_name_from_type(ty) })
            .collect();
        let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);
        self.pending_wrappers.push(PendingWrapper {
            name: name.to_owned(),
            needs_payload,
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
            let selected = self
                .select_wrapper_props(&pending.type_args)
                .filter(|(_, _, from_payload)| *from_payload || !pending.needs_payload);
            let Some((props_name, props_args, _)) = selected else {
                self.record_skip(
                    DiagnosticCode::SkippedCandidate,
                    format!(
                        "'{}' looks like a component built by a wrapper call, but none of its type arguments names {}",
                        pending.name,
                        if pending.needs_payload {
                            "a factory payload, which a tag-name argument needs"
                        } else {
                            "a props type declared in this file or ending in Props"
                        }
                    ),
                    pending.span,
                );
                continue;
            };
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
    /// The third field says whether the props came from a factory payload.
    fn select_wrapper_props(&self, type_args: &[WrapperTypeArg]) -> Option<(TypeName, Vec<String>, bool)> {
        for arg in type_args.iter().rev() {
            let Some((name, args)) = &arg.reference else { continue };
            if is_element_type(name) {
                continue;
            }
            if let Some(props) = self.payload_props(name) {
                return Some((props, Vec::new(), true));
            }
            let declared_here = self.data.interfaces.contains_key(&self.scoped_key(name))
                || self.data.type_aliases.contains_key(&self.scoped_key(name));
            // A name from another file cannot be inspected here; only the props naming convention vouches for it.
            if declared_here || name.ends_with("Props") {
                return Some((name.clone(), args.clone(), false));
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
