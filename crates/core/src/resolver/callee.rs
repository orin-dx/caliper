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

use super::ResolutionContext;
use super::alias::resolve_base_as_chain;
use super::import::{lookup_interface, lookup_type_alias, resolve_to_canonical};
use super::substitute::{build_substitution, substitute_alias, substitute_type};

/// React types whose first type argument is a component's props.
const COMPONENT_TYPES: &[&str] =
    &["NamedExoticComponent", "ForwardRefExoticComponent", "ExoticComponent", "JSXElementConstructor"];

/// Whether `name` (without any `React.` prefix) is a React type whose first type argument is a component's props.
fn is_component_type(name: &str) -> bool {
    COMPONENT_TYPES.contains(&name) || crate::react_types::FC_FAMILY_TYPE_NAMES.contains(&name)
}

/// Aliases followed from the result type to the component type; the bound only guards against a cycle.
const MAX_DEPTH: u8 = 8;

pub(super) fn component_props_of_typeof(
    name: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<(CollectedType, Utf8PathBuf)> {
    component_props_type(&CollectedType::TypeOf(name.into()), file, ctx, 0, &mut None)
        .map(|(props, file, _)| (props, file))
}

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
    let (props, props_file, type_params, factory_params, written_in) =
        result_props(callee, &mapping.file_path, ctx, 0)?;
    let mut attempt = state.branch();
    // The result names the declarations local to the factory from inside it.
    drop(attempt.enter_declared_in(&written_in));
    // A declaration local to the factory names its type parameters wherever nothing substitutes them, so they stay in
    // scope under every declaration read. The call's arguments and the result's own signature parameters cannot
    // collide with them: `result_props` marks the first as declarations and the second as placeholders.
    attempt.closure = TypeScope::new(factory_params.iter());
    let mut chain = resolve_base_as_chain(&props, &props_file, mapping, ctx, &mut attempt, 0);
    if chain
        .composes
        .iter()
        .any(|name| factory_params.iter().any(|param| name == param.as_str() || name.starts_with(&format!("{param}["))))
    {
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
            if matches!(prop.prop_type, PropType::Any)
                && let Some(existing) = chain.props.iter_mut().find(|existing| existing.name == prop.name)
            {
                *existing = Arc::clone(prop);
            }
        }
        chain.merge_parent(fallback);
    }
    if chain.props.is_empty() && chain.inheritance.is_empty() {
        return None;
    }
    erase_unbound_params(&mut chain, type_params.iter().chain(&factory_params));
    state.absorb(attempt);
    Some(chain)
}

/// A type parameter of the factory or of its result's call signature, beside other members of a union, stands for one
/// of them (`C | ElementType` with `C extends ElementType`), and its bare name tells a reader nothing, so the union
/// keeps the other members.
fn erase_unbound_params<'p>(chain: &mut super::ResolvedChain, params: impl Iterator<Item = &'p TypeName> + Clone) {
    let is_param = |member: &PropType| {
        matches!(member, PropType::Named { name, args }
            if args.is_empty() && params.clone().any(|p| p.as_str() == name.as_str()))
    };
    for prop in &mut chain.props {
        let PropType::Union(members) = &prop.prop_type else { continue };
        if !members.iter().any(is_param) {
            continue;
        }
        let kept: Vec<PropType> = members.iter().filter(|member| !is_param(member)).cloned().collect();
        if kept.is_empty() {
            continue;
        }
        let mut edited = (**prop).clone();
        edited.prop_type = super::primitives::union_of(kept);
        *prop = Arc::new(edited);
    }
}

/// The props type a callee's result gives a component, the file that writes it, the type parameters of the result's own
/// call signature and of the callee, and the scope the callee writes the props type in.
type ResultProps = (CollectedType, Utf8PathBuf, Vec<TypeName>, Vec<TypeName>, DeclarationScope);

fn result_props(
    callee: &WrapperCallee,
    caller_file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<ResultProps> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut ignored = Vec::new();
    let (file, name) = resolve_to_canonical(&callee.name, caller_file, ctx, &mut ignored)
        .unwrap_or_else(|| (caller_file.to_owned(), callee.name.to_string()));
    super::deps::record(&file);
    let function = ctx.global.function_results.get(&format!("{file}:{name}"))?;
    let mut also_declared: Vec<TypeName> = Vec::new();
    loop {
        // The arguments are written at the call, where the function's parameters and its result's signature parameters
        // are not in scope: an argument named like one is a declaration.
        let type_args: Vec<CollectedType> = callee
            .type_args
            .iter()
            .map(|arg| {
                arg.rename_named(&|name| {
                    (function.type_params.iter().chain(&also_declared).any(|param| param == name))
                        .then(|| mark_declaration(name))
                })
            })
            .collect();
        let mut bindings = build_substitution(&function.type_params, &type_args, caller_file, &mut ignored);
        // The parameters the call leaves unfilled stand for themselves in the result, whatever module types are named
        // alike.
        super::substitute::fill_placeholders(&mut bindings, &function.type_params);
        if let Some(inner) = &function.returned_call {
            let inner = WrapperCallee {
                name: inner.name.clone(),
                type_args: inner.type_args.iter().map(|arg| substitute_type(arg, &bindings)).collect(),
            };
            return result_props(&inner, &file, ctx, depth + 1);
        }
        let result = substitute_type(&function.result, &bindings);
        let mut props_scope = None;
        let (props, props_file, type_params) = component_props_type(&result, &file, ctx, 0, &mut props_scope)?;
        // The result's call signature binds type parameters of its own (`<C>(props: …)`), placeholders in the props.
        // An argument that names one is read again with it marked as a declaration, so the substitution leaves it be.
        let named_by_arguments: Vec<TypeName> =
            callee.type_args.iter().flat_map(CollectedType::referenced_names).collect();
        let clashing: Vec<TypeName> = type_params
            .iter()
            .filter(|param| named_by_arguments.contains(param) && !also_declared.contains(param))
            .cloned()
            .collect();
        if !clashing.is_empty() {
            also_declared.extend(clashing);
            continue;
        }
        let props = substitute_type(&props, &super::substitute::unfilled_substitution(&type_params));
        // The props are read where the declaration that writes them is, or in the factory when its own result does.
        let written_in = props_scope.unwrap_or_else(|| function.scope.clone());
        return Some((props, props_file, type_params, function.type_params.clone(), written_in));
    }
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
    value_member_props_at_depth(base, member, file, ctx, 0)
}

fn value_member_props_at_depth(
    base: &str,
    member: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<(CollectedType, Utf8PathBuf)> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut ignored = Vec::new();
    let (base_file, base_name) =
        resolve_to_canonical(base, file, ctx, &mut ignored).unwrap_or_else(|| (file.to_owned(), base.to_owned()));
    // The value may sit behind a chain of re-exports (`index.d.ts`'s `export default Mentions`, itself imported from
    // `Mentions.d.ts`): follow each module's local name for it until a declaration of the value is found.
    let (mut base_file, mut base_name, mut member) = (base_file, base_name, member.to_owned());
    for _ in 0..MAX_DEPTH {
        if lookup_type_alias(ctx, base_file.as_str(), &format!("typeof {base_name}")).is_some() {
            break;
        }
        if let Some(Some((declaring_file, qualified))) =
            ctx.dependencies.as_ref().and_then(|layer| layer.export_of(&base_file, &format!("{base_name}.{member}")))
        {
            let (declaring_name, declared_member) = qualified.split_once('.')?;
            (base_file, base_name, member) =
                (declaring_file.clone(), declaring_name.to_owned(), declared_member.to_owned());
            break;
        }
        let local = ctx.import_map.local_export(&base_file, &base_name)?;
        (base_file, base_name) = resolve_to_canonical(&local, &base_file, ctx, &mut ignored)?;
    }
    let (_, declared) = lookup_type_alias(ctx, base_file.as_str(), &format!("typeof {base_name}"))?;
    let name = match declared {
        CollectedTypeAlias::Passthrough { target: named @ CollectedType::Named { .. }, .. } => {
            if let Some((member_type, member_file)) = super::evaluate::member(named, &member, &base_file, ctx, 0)
                && let Some((props, file, _)) =
                    component_props_type(&member_type, &member_file, ctx, depth + 1, &mut None)
            {
                return Some((props, file));
            }
            let CollectedType::Named { name, .. } = named else { return None };
            name
        }
        CollectedTypeAlias::Passthrough { target: CollectedType::Object(fields), .. } => {
            return fields
                .iter()
                .find(|field| field.is_member() && field.name == member)
                .and_then(|field| component_props_type(&field.collected_type, &base_file, ctx, depth + 1, &mut None))
                .map(|(props, file, _)| (props, file));
        }
        CollectedTypeAlias::Passthrough { target: CollectedType::Intersection(members), .. } => {
            return members.iter().find_map(|part| match part {
                CollectedType::Object(fields) => fields
                    .iter()
                    .find(|field| field.is_member() && field.name == member)
                    .and_then(|field| {
                        component_props_type(&field.collected_type, &base_file, ctx, depth + 1, &mut None)
                    })
                    .map(|(props, file, _)| (props, file)),
                _ => None,
            });
        }
        // Not a namespace-like object whose member could be the component; a new variant must say whether it is.
        CollectedTypeAlias::Passthrough { .. }
        | CollectedTypeAlias::Omit { .. }
        | CollectedTypeAlias::Pick { .. }
        | CollectedTypeAlias::Partial { .. }
        | CollectedTypeAlias::Required { .. }
        | CollectedTypeAlias::Union { .. }
        | CollectedTypeAlias::Intersection { .. }
        | CollectedTypeAlias::LiteralUnion { .. } => return None,
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
    component_props_type(&value, &mapped_file, ctx, depth + 1, &mut None).map(|(props, file, _)| (props, file))
}

/// The props parameter of the React component type `ty` describes, its file, and any callable type parameters in scope.
/// `declared_in` is set to the scope of the declaration that writes the props when one does (an alias, an interface or
/// a component), and left alone where they are written in `ty` itself.
fn component_props_type(
    ty: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
    declared_in: &mut Option<DeclarationScope>,
) -> Option<(CollectedType, Utf8PathBuf, Vec<TypeName>)> {
    if depth > MAX_DEPTH {
        return None;
    }
    match ty {
        CollectedType::AtFile { file, inner } => component_props_type(inner, file, ctx, depth + 1, declared_in),
        CollectedType::TypeOf(name) => {
            let mut ignored = Vec::new();
            let (canonical_file, canonical_name) = resolve_to_canonical(name, file, ctx, &mut ignored)
                .unwrap_or_else(|| (file.to_owned(), name.to_string()));
            let mapping = ctx
                .component_mapping(&canonical_file, &canonical_name)
                .or_else(|| ctx.dependencies.as_ref()?.component(&canonical_file, &canonical_name));
            let Some(mapping) = mapping else {
                // A value declared with a React component type, `declare const Option: React.FC<OptionProps>`.
                if let Some((_, CollectedTypeAlias::Passthrough { target, .. })) =
                    lookup_type_alias(ctx, canonical_file.as_str(), &format!("typeof {canonical_name}"))
                    && let Some(found) = component_props_type(target, &canonical_file, ctx, depth + 1, declared_in)
                {
                    return Some(found);
                }
                let (base, member) = name.rsplit_once('.')?;
                let (props, props_file) = value_member_props_at_depth(base, member, file, ctx, depth + 1)?;
                return Some((props, props_file, Vec::new()));
            };
            *declared_in = Some(DeclarationScope::default());
            Some((
                CollectedType::Named {
                    name: mapping.props_type_name.clone(),
                    args: super::extends::instantiated_props_args(mapping),
                },
                mapping.file_path.clone(),
                mapping.type_params.clone(),
            ))
        }
        CollectedType::Intersection(members) => members.iter().find_map(|member| {
            let mut own = None;
            let found = component_props_type(member, file, ctx, depth + 1, &mut own)?;
            if own.is_some() {
                *declared_in = own;
            }
            Some(found)
        }),
        CollectedType::Function { type_params, params, .. } => {
            Some((params.first()?.clone(), file.to_owned(), type_params.clone()))
        }
        CollectedType::Named { name, args } => {
            if is_component_type(name.strip_prefix("React.").unwrap_or(name)) {
                return Some((args.first()?.clone(), file.to_owned(), Vec::new()));
            }
            // `MemoExoticComponent<typeof Impl>` wraps a component, whose props are the component's.
            if name.strip_prefix("React.").unwrap_or(name) == "MemoExoticComponent" {
                return component_props_type(args.first()?, file, ctx, depth + 1, declared_in);
            }
            let mut ignored = Vec::new();
            let (alias_file, alias_name) = resolve_to_canonical(name, file, ctx, &mut ignored)
                .unwrap_or_else(|| (file.to_owned(), name.to_string()));
            // `interface PrimitiveForwardRefComponent<E> extends React.ForwardRefExoticComponent<Props<E>> {}`.
            if let Some(iface) = lookup_interface(ctx, alias_file.as_str(), &alias_name) {
                *declared_in = Some(iface.scope.clone());
                let bindings = ctx
                    .named_types
                    .lookup_interface_type_params_for(iface)
                    .map(|params| build_substitution(params, args, file, &mut ignored));
                return iface.extends.iter().find_map(|extends| {
                    let (ExtendsRef::SameFile { name: base, type_args }
                    | ExtendsRef::Imported { local_name: base, type_args, .. }
                    | ExtendsRef::Builtin { name: base, type_args, .. }) = extends;
                    if !is_component_type(base.strip_prefix("React.").unwrap_or(base)) {
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
            let found = component_props_type(&body, &alias_file, ctx, depth + 1, declared_in)?;
            // The props are written in the alias's body, unless the alias hands them on from another declaration.
            if declared_in.is_none() {
                *declared_in =
                    Some(ctx.named_types.lookup_local_alias_scope(&alias_file, &matched).cloned().unwrap_or_default());
            }
            Some(found)
        }
        _ => None,
    }
}
