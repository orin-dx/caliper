//! The props of a component a generic call builds, read from what the callee returns.
//!
//! `const Button = factory<ButtonFactory>(render)` is whatever `factory` returns. With the callee's type parameters
//! bound to the call's type arguments, the props parameter of the React component type in its result is the
//! component's props, as TypeScript (and react-docgen-typescript) reads them: Mantine's `factory` returns
//! `MantineComponent<Payload>`, whose props are `Payload['props']` plus `component` and `renderRoot`. The extractor's
//! choice among the type arguments (`extractor/wrapper.rs`) is the fallback for a callee with no known result. Nothing
//! here names a library; the React component types are React's own.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};

use crate::types::*;

use super::alias::resolve_base_as_chain;
use super::import::{lookup_interface, lookup_type_alias, resolve_to_canonical};
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
    let (props, props_file, type_params) = component_props_type(&result, &file, ctx, 0)?;
    let mut attempt = state.branch();
    attempt.add_type_params(type_params.iter());
    let mut chain = resolve_base_as_chain(&props, &props_file, mapping, ctx, &mut attempt, 0);
    if chain.composes.iter().any(|name| {
        function.type_params.iter().any(|param| name == param.as_str() || name.starts_with(&format!("{param}[")))
    }) {
        let fallback = super::chain::resolve_props_chain(
            mapping.props_type_name.as_str(),
            &mapping.props_type_args,
            &mapping.file_path,
            mapping,
            ctx,
            &mut attempt,
            0,
        );
        // An explicit `any` from the selected payload absorbs the conditional's tentative type in an intersection.
        for prop in &fallback.props {
            if matches!(prop.prop_type, PropType::Any) {
                if let Some(existing) = chain.props.iter_mut().find(|existing| existing.name == prop.name) {
                    *existing = Arc::clone(prop);
                }
            }
        }
        chain.merge_parent(fallback);
    }
    if chain.props.is_empty() && chain.inheritance.is_empty() {
        return None;
    }
    state.absorb(attempt);
    Some(chain)
}

/// The props of a member of a value declared with a mapped type: `Primitive.div` for `declare const Primitive:
/// Primitives`, `type Primitives = { [E in (typeof NODES)[number]]: PrimitiveForwardRefComponent<E> }`. The member's
/// type is the mapped type's value with the key variable bound to the member name, and its props are those of the
/// React component type that describes. `base` is the value as `file` writes it.
pub(super) fn value_member_props(
    base: &str,
    member: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<(CollectedType, Utf8PathBuf)> {
    let mut ignored = Vec::new();
    let (base_file, base_name) =
        resolve_to_canonical(base, file, ctx, &mut ignored).unwrap_or_else(|| (file.to_owned(), base.to_owned()));
    let (_, declared) = lookup_type_alias(ctx, base_file.as_str(), &format!("typeof {base_name}"))?;
    let name = match declared {
        CollectedTypeAlias::Passthrough { target: CollectedType::Named { name, .. }, .. } => name,
        CollectedTypeAlias::Passthrough { target: CollectedType::Object(fields), .. } => {
            return fields
                .iter()
                .find(|field| field.name == member)
                .and_then(|field| component_props_type(&field.collected_type, &base_file, ctx, 0))
                .map(|(props, file, _)| (props, file));
        }
        CollectedTypeAlias::Passthrough { target: CollectedType::Intersection(members), .. } => {
            return members.iter().find_map(|part| match part {
                CollectedType::Object(fields) => fields
                    .iter()
                    .find(|field| field.name == member)
                    .and_then(|field| component_props_type(&field.collected_type, &base_file, ctx, 0))
                    .map(|(props, file, _)| (props, file)),
                _ => None,
            });
        }
        _ => return None,
    };
    let (mapped_file, mapped_name) = resolve_to_canonical(name, &base_file, ctx, &mut ignored)
        .unwrap_or_else(|| (base_file.clone(), name.to_string()));
    let (_, mapped) = lookup_type_alias(ctx, mapped_file.as_str(), &mapped_name)?;
    let CollectedTypeAlias::Passthrough { target: CollectedType::Mapped { param, key_type, value_type, .. }, .. } =
        mapped
    else {
        return None;
    };
    let keys = super::evaluate::mapped_keys(key_type, &mapped_file, ctx, 0)?;
    if !keys.iter().any(|key| key.name == member) {
        return None;
    }
    let mut bindings = super::substitute::Substitution::default();
    bindings.insert(param.as_str(), CollectedType::StringLiteral(member.into()));
    let value = substitute_type(value_type, &bindings);
    component_props_type(&value, &mapped_file, ctx, 0).map(|(props, file, _)| (props, file))
}

/// The props parameter of the React component type `ty` describes, its file, and any callable type parameters in scope.
fn component_props_type(
    ty: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<(CollectedType, Utf8PathBuf, Vec<TypeName>)> {
    if depth > MAX_DEPTH {
        return None;
    }
    match ty {
        CollectedType::AtFile { file, inner } => component_props_type(inner, file, ctx, depth + 1),
        CollectedType::TypeOf(name) => {
            let mut ignored = Vec::new();
            let (canonical_file, canonical_name) = resolve_to_canonical(name, file, ctx, &mut ignored)
                .unwrap_or_else(|| (file.to_owned(), name.to_string()));
            let mapping = ctx
                .global
                .component_mappings
                .iter()
                .find(|mapping| mapping.file_path == canonical_file && mapping.component_name == canonical_name)
                .or_else(|| ctx.dependencies.as_ref()?.component(&canonical_file, &canonical_name))?;
            Some((
                CollectedType::Named {
                    name: mapping.props_type_name.clone(),
                    args: super::extends::instantiated_props_args(mapping),
                },
                mapping.file_path.clone(),
                mapping.type_params.clone(),
            ))
        }
        CollectedType::Intersection(members) => {
            members.iter().find_map(|member| component_props_type(member, file, ctx, depth + 1))
        }
        CollectedType::Function { type_params, params, .. } => {
            Some((params.first()?.clone(), file.to_owned(), type_params.clone()))
        }
        CollectedType::Named { name, args } => {
            if COMPONENT_TYPES.contains(&name.strip_prefix("React.").unwrap_or(name)) {
                return Some((args.first()?.clone(), file.to_owned(), Vec::new()));
            }
            let mut ignored = Vec::new();
            let (alias_file, alias_name) = resolve_to_canonical(name, file, ctx, &mut ignored)
                .unwrap_or_else(|| (file.to_owned(), name.to_string()));
            // `interface PrimitiveForwardRefComponent<E> extends React.ForwardRefExoticComponent<Props<E>> {}`.
            if let Some(iface) = lookup_interface(ctx, alias_file.as_str(), &alias_name) {
                let bindings = ctx
                    .named_types
                    .lookup_interface_type_params_for(iface)
                    .map(|params| build_substitution(params, args, file, &mut ignored));
                return iface.extends.iter().find_map(|extends| {
                    let (ExtendsRef::SameFile { name: base, type_args }
                    | ExtendsRef::Imported { local_name: base, type_args, .. }
                    | ExtendsRef::Builtin { name: base, type_args, .. }) = extends;
                    if !COMPONENT_TYPES.contains(&base.strip_prefix("React.").unwrap_or(base)) {
                        return None;
                    }
                    let props = type_args.first()?;
                    let props = bindings.as_ref().map_or_else(|| props.clone(), |b| substitute_type(props, b));
                    Some((props, alias_file.clone(), Vec::new()))
                });
            }
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
