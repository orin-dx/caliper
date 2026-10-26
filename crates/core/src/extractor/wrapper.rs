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

use crate::types::{
    CollectedType, CollectedTypeAlias, ComponentMapping, DetectionRule, DiagnosticCode, FunctionResult, TypeName,
    WrapperCallee,
};

use super::evidence::{renders, FunctionShape, ReactBinding};
use super::SourceDataCollector;

/// One type argument of a candidate wrapper call.
pub(crate) struct WrapperTypeArg {
    /// The reference name and its stringified type arguments, when the argument is a plain type reference.
    reference: Option<(TypeName, Vec<CollectedType>)>,
}

/// A wrapper call waiting for the rest of the file to be seen.
pub(crate) struct PendingWrapper {
    name: String,
    /// The only evidence was a tag name, so the props must come from a factory payload.
    needs_payload: bool,
    type_args: Vec<WrapperTypeArg>,
    callee: WrapperCallee,
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
fn component_argument_evidence(
    callee: &str,
    call: &CallExpression<'_>,
    react: ReactBinding<'_>,
) -> Result<ArgumentEvidence, &'static str> {
    if callee.rsplit('.').next() == Some("createContext") {
        return Err("creates a context");
    }
    let Some(first) = call.arguments.first().and_then(Argument::as_expression).map(super::component::peel_expr) else {
        return Err("has no argument that could be a component");
    };
    let element_type_arg = || call.type_arguments.as_ref().is_some_and(|a| a.params.iter().any(is_element_type_arg));
    let sufficient = |r: Result<(), &'static str>| r.map(|()| ArgumentEvidence::Sufficient);
    match first {
        Expression::ArrowFunctionExpression(f) => sufficient(
            renders(&FunctionShape::of_arrow(f), react).map_err(|_why| "is given a function that never renders"),
        ),
        Expression::FunctionExpression(f) => sufficient(
            renders(&FunctionShape::of_function(f), react).map_err(|_why| "is given a function that never renders"),
        ),
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
    pub(super) fn promote_function_aliases(&mut self) {
        let aliases = std::mem::take(&mut self.pending_function_aliases);
        for _ in 0..aliases.len() {
            let mut changed = false;
            for (name, target) in &aliases {
                let key = self.scoped_key(name);
                if self.data.function_results.contains_key(&key) {
                    continue;
                }
                if let Some(result) = self.data.function_results.get(&self.scoped_key(target)).cloned() {
                    self.data.function_results.insert(key, result);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Record `const Name = callee<...>(first, ...)` as a candidate. Returns whether it was recorded.
    pub(super) fn try_record_wrapper_call<'a>(&mut self, decl: &super::component::Binding<'_, 'a>, name: &str) -> bool {
        let Some(init) = decl.init else { return false };
        let Expression::CallExpression(call) = super::component::peel_expr(init) else { return false };
        let Some(type_arguments) = call.type_arguments.as_ref() else { return false };
        let Some(callee) = self.extract_callee_name(call) else { return false };
        let needs_payload = match component_argument_evidence(&callee, call, &*self) {
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
        let Some(callee) = self.wrapper_callee(call) else { return false };
        let (description, tags) = self.find_jsdoc_with_tags(decl.span.start);
        self.pending_wrappers.push(PendingWrapper {
            name: name.to_owned(),
            needs_payload,
            type_args,
            callee,
            description,
            tags,
            span: decl.span,
        });
        true
    }

    /// Whether a generic call names the props type of the component it builds in its type arguments, as in
    /// `withContext<HTMLDivElement, OverlayProps>(BaseOverlay, …)`. The call's own props then replace those of the
    /// component it receives.
    pub(super) fn call_names_props_type<'a>(&mut self, call: &CallExpression<'a>) -> bool {
        let Some(type_arguments) = call.type_arguments.as_ref() else { return false };
        let type_args: Vec<WrapperTypeArg> = type_arguments
            .params
            .iter()
            .map(|ty| WrapperTypeArg { reference: self.extract_type_name_from_type(ty) })
            .collect();
        matches!(self.select_wrapper_props(&type_args), PropsSelection::Found { from_payload: false, .. })
    }

    /// Turn the recorded candidates into mappings, now that every declaration in the file is known.
    pub(super) fn promote_wrapper_calls(&mut self) {
        for pending in std::mem::take(&mut self.pending_wrappers) {
            if self.data.component_mappings.iter().any(|m| m.component_name == pending.name) {
                continue;
            }
            let selected = match self.select_wrapper_props(&pending.type_args) {
                PropsSelection::Found { name, args, from_payload } if from_payload || !pending.needs_payload => {
                    Some((name, args))
                }
                PropsSelection::UnusablePayload { alias } => {
                    self.record_skip(
                        DiagnosticCode::SkippedCandidate,
                        format!(
                            "'{}' is built from the factory payload '{alias}', whose `props` member is not a named type",
                            pending.name
                        ),
                        pending.span,
                    );
                    continue;
                }
                PropsSelection::Found { .. } | PropsSelection::None => None,
            };
            let Some((props_name, props_args)) = selected else {
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
                .with_doc(pending.description, pending.tags)
                .with_wrapper_callee(pending.callee),
            );
        }
    }

    /// The callee of a generic call that builds a component, and its type arguments, for the props its result type
    /// adds (`resolver::callee`). `None` for a call without type arguments.
    pub(super) fn wrapper_callee<'a>(&mut self, call: &CallExpression<'a>) -> Option<WrapperCallee> {
        let type_arguments = call.type_arguments.as_ref()?;
        let name = self.extract_callee_name(call)?;
        Some(WrapperCallee {
            name: name.as_str().into(),
            type_args: type_arguments.params.iter().map(|ty| self.ts_type_to_collected(ty)).collect(),
        })
    }

    /// Record a generic function's annotated, asserted, or single returned local factory result. A wrapper call to
    /// it then gets the props that result adds (`resolver::callee`).
    pub(super) fn record_function_result<'a>(&mut self, func: &Function<'a>) {
        let (Some(id), Some(type_parameters)) = (&func.id, func.type_parameters.as_deref()) else { return };
        let (result, returned_call) = match &func.return_type {
            Some(annotation) => (self.ts_type_to_collected(&annotation.type_annotation), None),
            None => match asserted_return_type(func) {
                Some(ty) => (self.ts_type_to_collected(ty), None),
                None => (CollectedType::Unknown, returned_local_call(self, func)),
            },
        };
        if matches!(result, CollectedType::Unknown) && returned_call.is_none() {
            return;
        }
        let type_params = super::type_param_names(Some(type_parameters));
        let key = self.scoped_key(id.name.as_str());
        self.data.function_results.insert(key, FunctionResult { type_params, result, returned_call });
    }

    /// Pick the props type from a wrapper call's type arguments, last first (the props usually follow the element).
    fn select_wrapper_props(&self, type_args: &[WrapperTypeArg]) -> PropsSelection {
        for arg in type_args.iter().rev() {
            let Some((name, args)) = &arg.reference else { continue };
            if is_element_type(name) {
                continue;
            }
            // A factory payload is never itself the props type, even though it is declared in this file.
            match self.payload_props(name) {
                Some(Some((props, props_args))) => {
                    return PropsSelection::Found { name: props, args: props_args, from_payload: true };
                }
                Some(None) => return PropsSelection::UnusablePayload { alias: name.clone() },
                None => {}
            }
            let declared_here = self.data.interfaces.contains_key(&self.scoped_key(name))
                || self.data.type_aliases.contains_key(&self.scoped_key(name));
            // A name from another file cannot be inspected here; only the props naming convention vouches for it.
            if declared_here || name.ends_with("Props") {
                return PropsSelection::Found { name: name.clone(), args: args.clone(), from_payload: false };
            }
        }
        PropsSelection::None
    }

    /// For an alias that is a factory payload (`type ButtonFactory = Anything<{ props: ButtonProps; ... }>`), the
    /// props type and its arguments (`ButtonProps`, or `OverflowListProps` with `[any]`), or `None` when the `props`
    /// member is not a named type. `None` outright when the alias is not a payload.
    fn payload_props(&self, alias_name: &str) -> Option<Option<(TypeName, Vec<CollectedType>)>> {
        let CollectedTypeAlias::Passthrough { target: CollectedType::Named { args, .. }, .. } =
            self.data.type_aliases.get(&self.scoped_key(alias_name))?
        else {
            return None;
        };
        let props_member = args.iter().find_map(|arg| match arg {
            CollectedType::Object(fields) => fields.iter().find(|field| field.name == PAYLOAD_PROPS_MEMBER),
            _ => None,
        })?;
        Some(match &props_member.collected_type {
            CollectedType::Named { name, args } => Some((name.clone(), args.clone())),
            _ => None,
        })
    }
}

/// A single top-level `return Local` where `Local` is a const initialized by a generic call.
fn returned_local_call<'a>(collector: &mut SourceDataCollector<'_>, func: &Function<'a>) -> Option<WrapperCallee> {
    let body = func.body.as_ref()?;
    let mut returns = body.statements.iter().filter_map(|statement| match statement {
        Statement::ReturnStatement(ret) => ret.argument.as_ref(),
        _ => None,
    });
    let Expression::Identifier(returned) = returns.next()?.without_parentheses() else { return None };
    if returns.next().is_some() {
        return None;
    }
    for statement in &body.statements {
        let Statement::VariableDeclaration(declaration) = statement else { continue };
        if !declaration.kind.is_const() {
            continue;
        }
        for declarator in &declaration.declarations {
            let BindingPattern::BindingIdentifier(name) = &declarator.id else { continue };
            if name.name != returned.name {
                continue;
            }
            let Some(Expression::CallExpression(call)) = declarator.init.as_ref().map(super::component::peel_expr)
            else {
                return None;
            };
            return collector.wrapper_callee(call);
        }
    }
    None
}

/// Where a wrapper call's props come from.
enum PropsSelection {
    Found {
        name: TypeName,
        args: Vec<CollectedType>,
        from_payload: bool,
    },
    /// A factory payload whose `props` member names no type, so neither it nor the payload can be the props type.
    UnusablePayload {
        alias: TypeName,
    },
    None,
}

/// The type the function's only top-level `return` asserts with `as` (the outermost one of `x as unknown as T`).
fn asserted_return_type<'f, 'a>(func: &'f Function<'a>) -> Option<&'f TSType<'a>> {
    let mut returns = func.body.as_ref()?.statements.iter().filter_map(|statement| match statement {
        Statement::ReturnStatement(ret) => Some(ret.argument.as_ref()),
        _ => None,
    });
    let only = returns.next()??;
    if returns.next().is_some() {
        return None;
    }
    match only.without_parentheses() {
        Expression::TSAsExpression(assertion) => Some(&assertion.type_annotation),
        Expression::TSTypeAssertion(assertion) => Some(&assertion.type_annotation),
        _ => None,
    }
}
