//! The props of a component a generic call builds, read from what the callee returns.
//!
//! `const Button = factory<ButtonFactory>(render)` is whatever `factory` returns. With the callee's type parameters
//! bound to the call's type arguments, the props parameter of the React component type in its result is the
//! component's props, as TypeScript (and react-docgen-typescript) reads them: Mantine's `factory` returns
//! `MantineComponent<Payload>`, whose props are `Payload['props']` plus `component` and `renderRoot`. The extractor's
//! choice among the type arguments (`extractor/wrapper.rs`) is the fallback for a callee with no known result. Nothing
//! here names a library; the React component types are React's own.

use camino::{Utf8Path, Utf8PathBuf};

use crate::types::*;

use super::alias::resolve_base_as_chain;
use super::import::{lookup_type_alias, resolve_to_canonical};
use super::substitute::{build_substitution, substitute_alias, substitute_type};
use super::ResolutionContext;

/// React types whose first type argument is a component's props.
const COMPONENT_TYPES: &[&str] = &[
    "NamedExoticComponent",
    "ForwardRefExoticComponent",
    "ExoticComponent",
    "FC",
    "FunctionComponent",
    "VFC",
    "VoidFunctionComponent",
    "ComponentType",
    "JSXElementConstructor",
];

/// Aliases followed from the result type to the component type; the bound only guards against a cycle.
const MAX_DEPTH: u8 = 8;

/// The props chain of the component a generic call builds, from its callee's result type. `None` when the callee is
/// not a project function with a known result, the result has no React component type, or its props evaluate to
/// nothing (a conditional over an unbound generic, as in Mantine's `polymorphicFactory`): the extractor's selection
/// is then the closer answer.
pub(super) fn result_chain(
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
) -> Option<super::ResolvedChain> {
    let callee = mapping.wrapper_callee.as_ref()?;
    let mut ignored = Vec::new();
    let (file, name) = resolve_to_canonical(&callee.name, &mapping.file_path, ctx, &mut ignored)
        .unwrap_or_else(|| (mapping.file_path.clone(), callee.name.to_string()));
    super::deps::record(&file);
    let function = ctx.global.function_results.get(&format!("{file}:{name}"))?;
    let bindings = build_substitution(&function.type_params, &callee.type_args, &mapping.file_path, &mut ignored);
    let result = substitute_type(&function.result, &bindings);
    let (props, props_file) = component_props_type(&result, &file, ctx, 0)?;
    let mut attempt = state.branch();
    let chain = resolve_base_as_chain(&props, &props_file, mapping, ctx, &mut attempt, 0);
    if chain.props.is_empty() && chain.inheritance.is_empty() {
        return None;
    }
    state.absorb(attempt);
    Some(chain)
}

/// The props parameter of the React component type `ty` describes, and the file its names resolve in.
fn component_props_type(
    ty: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<(CollectedType, Utf8PathBuf)> {
    if depth > MAX_DEPTH {
        return None;
    }
    match ty {
        CollectedType::AtFile { file, inner } => component_props_type(inner, file, ctx, depth + 1),
        CollectedType::Intersection(members) => {
            members.iter().find_map(|member| component_props_type(member, file, ctx, depth + 1))
        }
        CollectedType::Function { params, .. } => Some((params.first()?.clone(), file.to_owned())),
        CollectedType::Named { name, args } => {
            if COMPONENT_TYPES.contains(&name.strip_prefix("React.").unwrap_or(name)) {
                return Some((args.first()?.clone(), file.to_owned()));
            }
            let mut ignored = Vec::new();
            let (alias_file, alias_name) = resolve_to_canonical(name, file, ctx, &mut ignored)
                .unwrap_or_else(|| (file.to_owned(), name.to_string()));
            let (matched, alias) = lookup_type_alias(ctx, alias_file.as_str(), &alias_name)?;
            let alias = match ctx.named_types.lookup_type_alias_params(&alias_file, &matched) {
                Some(params) => substitute_alias(alias, &build_substitution(params, args, file, &mut ignored)),
                None => alias.clone(),
            };
            let body = match alias {
                CollectedTypeAlias::Passthrough { target, .. } => target,
                CollectedTypeAlias::Intersection { members, .. } => CollectedType::Intersection(members),
                _ => return None,
            };
            component_props_type(&body, &alias_file, ctx, depth + 1)
        }
        _ => None,
    }
}
