//! Props a wrapper-built component gets from what its callee returns.
//!
//! `const Button = factory<ButtonFactory>(render)`: the extractor picks the props type from the type arguments
//! (`extractor/wrapper.rs`), but the component is whatever `factory` returns, and that can declare more props than the
//! payload carries. Mantine's `factory` returns `MantineComponent<Payload>`, a `NamedExoticComponent` whose props add
//! `component` and `renderRoot`. With the callee's type parameters bound to the call's type arguments, the props
//! parameter of the React component type in its result is what TypeScript (and react-docgen-typescript) reads, so its
//! inline object members join the component's props. Nothing here names a library; the React component types are
//! React's own.

use std::sync::Arc;

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

/// The props the callee's result type declares as inline object members (`{ component?: any }`), for a component a
/// wrapper call builds. Empty when the callee is not a project function with a known result, or the result has no
/// component type. A member the walk cannot reach (an indexed access such as `Payload['props']`, already the selected
/// props type) is left to the selection, so this never warns.
pub(super) fn result_props(
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
) -> Vec<Arc<ParsedProp>> {
    let Some(callee) = &mapping.wrapper_callee else { return Vec::new() };
    let mut ignored = Vec::new();
    let (file, name) = resolve_to_canonical(&callee.name, &mapping.file_path, ctx, &mut ignored)
        .unwrap_or_else(|| (mapping.file_path.clone(), callee.name.to_string()));
    super::deps::record(&file);
    let Some(function) = ctx.global.function_results.get(&format!("{file}:{name}")) else { return Vec::new() };
    let bindings = build_substitution(&function.type_params, &callee.type_args, &mapping.file_path, &mut ignored);
    let result = substitute_type(&function.result, &bindings);
    let Some((props, props_file)) = component_props_type(&result, &file, ctx, 0) else { return Vec::new() };

    let mut members = Vec::new();
    flatten_intersection(&props, &props_file, &mut members);
    members
        .into_iter()
        .filter(|(member, _)| matches!(member, CollectedType::Object(_)))
        .flat_map(|(member, member_file)| resolve_base_as_chain(&member, &member_file, mapping, ctx, state, 1).props)
        .collect()
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

/// The members of an intersection, each with the file its names resolve in.
fn flatten_intersection(ty: &CollectedType, file: &Utf8Path, out: &mut Vec<(CollectedType, Utf8PathBuf)>) {
    match ty {
        CollectedType::AtFile { file, inner } => flatten_intersection(inner, file, out),
        CollectedType::Intersection(members) => {
            for member in members {
                flatten_intersection(member, file, out);
            }
        }
        other => out.push((other.clone(), file.to_owned())),
    }
}
