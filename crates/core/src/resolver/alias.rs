//! Type alias chain and type-level resolution.

use std::collections::BTreeMap;

use camino::Utf8Path;
use rustc_hash::FxHashSet;

use crate::types::*;

use super::chain::resolve_props_chain;
use super::collected::resolve_collected_type;
use super::{ResolutionContext, ResolvedChain};

/// Resolve a `CollectedTypeAlias` as a props chain.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_type_alias_chain(
    alias: &CollectedTypeAlias,
    _consuming_file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    match alias {
        // Delegate to the generic CollectedType→chain resolver, which already
        // handles Named (interfaces/type aliases), Object (inline literals),
        // Intersection, and Union targets. Previously this arm only matched
        // `Named` and silently dropped everything else (e.g. an inline
        // `{ x: string }` object literal used directly as props), which lost
        // the whole component's props with no diagnostic.
        CollectedTypeAlias::Passthrough { target, file_path } => {
            resolve_base_as_chain(target, file_path, mapping, ctx, state, depth)
        }

        CollectedTypeAlias::Omit { base, keys, file_path } => {
            let mut chain = resolve_base_as_chain(base, file_path, mapping, ctx, state, depth);
            let Some(omitted) = resolve_key_set_or_warn("Omit", keys, file_path, mapping, ctx, state, depth) else {
                return chain;
            };
            let omitted_set: FxHashSet<&str> = omitted.iter().map(String::as_str).collect();
            chain.props.retain(|p| !omitted_set.contains(p.name.as_str()));
            // Record omitted keys in the relevant inheritance layer.
            for layer in &mut chain.inheritance {
                for key in &omitted {
                    if !layer.omitted.contains(key) {
                        layer.omitted.push(key.clone());
                    }
                }
            }
            chain
        }

        CollectedTypeAlias::Pick { base, keys, file_path } => {
            let mut chain = resolve_base_as_chain(base, file_path, mapping, ctx, state, depth);
            let Some(picked) = resolve_key_set_or_warn("Pick", keys, file_path, mapping, ctx, state, depth) else {
                chain.props.clear();
                return chain;
            };
            let picked_set: FxHashSet<&str> = picked.iter().map(String::as_str).collect();
            chain.props.retain(|p| picked_set.contains(p.name.as_str()));
            chain
        }

        CollectedTypeAlias::Partial { base, file_path } => {
            let mut chain = resolve_base_as_chain(base, file_path, mapping, ctx, state, depth);
            // Make all props optional.
            for prop in &mut chain.props {
                prop.required = false;
            }
            chain
        }

        CollectedTypeAlias::Required { base, file_path } => {
            let mut chain = resolve_base_as_chain(base, file_path, mapping, ctx, state, depth);
            // Make all props required.
            for prop in &mut chain.props {
                prop.required = true;
            }
            chain
        }

        CollectedTypeAlias::Union { members, file_path } => {
            // Discriminated union — try to find the discriminant and merge all members.
            resolve_union_alias(members, file_path, mapping, ctx, state, depth)
        }

        CollectedTypeAlias::Intersection { members, file_path } => {
            // Merge all members' props.
            let mut chain = ResolvedChain::empty();
            for member in members {
                let member_chain = resolve_base_as_chain(member, file_path, mapping, ctx, state, depth);
                chain.merge_parent(member_chain);
            }
            chain
        }

        CollectedTypeAlias::LiteralUnion { members, file_path } => {
            // Pure string union used directly as a component's props base — malformed
            // usage, mirroring the diagnostic `resolve_base_as_chain`'s non-object-like
            // fallback pushes for the same "this isn't shaped like props" situation.
            let display = members.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(" | ");
            let diag = Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "'{}' is a literal union and can't be used as a component's props base in '{}' — \
                     expected an interface, intersection, union, or inline object type",
                    display, file_path
                ),
                file: Some(file_path.to_string()),
                line: None,
                column: None,
                help: Some("Check that this type resolves to an object-like shape.".into()),
                code: DiagnosticCode::OpaqueType,
            };
            ResolvedChain::give_up(display, Some(diag), state)
        }
    }
}

/// Resolve a `CollectedType` as a props chain (for base types in aliases).
pub(super) fn resolve_base_as_chain(
    base: &CollectedType,
    file_path: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    match base {
        CollectedType::Named { name, args } => {
            // Omit/Pick/Partial/Readonly applied to a *structured* reference — e.g. a
            // member of `Omit<T, keyof U> & U` after `T`/`U` were substituted with real
            // types (see resolver/substitute.rs). Handled directly here, rather than via
            // the string-based `resolve_props_chain` path below, so nested generics and
            // `keyof` operands survive structurally instead of being flattened to a
            // single opaque display string first.
            if let Some(alias) = super::substitute::synthesize_utility_alias(name.as_str(), args, file_path) {
                return resolve_type_alias_chain(&alias, file_path, mapping, ctx, state, depth);
            }

            // A user-defined generic alias referenced with structured (possibly
            // cross-file, possibly nested-generic) arguments — e.g.
            // `SelectRootBaseProps<T>` used as `Assign<T, U>`'s own `U` argument.
            // Substituted and resolved directly here so the nested generic survives,
            // rather than falling through to the string-based path below (which would
            // flatten it to a single opaque display string before substitution ever ran).
            if let Some(alias) = super::substitute::generic_alias_with_structured_args(
                name.as_str(),
                args,
                file_path,
                ctx,
                &mut state.diagnostics,
            ) {
                return resolve_type_alias_chain(&alias, file_path, mapping, ctx, state, depth + 1);
            }

            resolve_props_chain(name.as_str(), args, file_path, mapping, ctx, state, depth + 1)
        }
        // Generic-alias substitution marker (see resolver/substitute.rs and the
        // `CollectedType::AtFile` doc comment): `inner` was written in `file`, not
        // whatever file this call chain has been resolving relative to — e.g. a
        // type argument substituted from the caller's scope into a callee alias
        // declared in a different file. Switch file context and continue.
        CollectedType::AtFile { file, inner } => resolve_base_as_chain(inner, file, mapping, ctx, state, depth),
        CollectedType::Intersection(members) => {
            let mut chain = ResolvedChain::empty();
            for member in members {
                let sub = resolve_base_as_chain(member, file_path, mapping, ctx, state, depth);
                chain.merge_parent(sub);
            }
            chain
        }
        CollectedType::Object(fields) => {
            // Inline object type in an intersection: `ComponentPropsWithoutRef<'button'> & { asChild?: boolean }`
            // Expand the object fields directly as own props.
            let mut chain = ResolvedChain::empty();
            for field in fields {
                let prop_type = resolve_collected_type(&field.collected_type, file_path, ctx, state, depth);
                chain.props.push(ParsedProp::new(
                    field.name.clone(),
                    prop_type,
                    field.required,
                    None,
                    field.description.clone(),
                    Default::default(),
                    None,
                    vec![],
                ));
            }
            chain
        }
        CollectedType::Union(members) => {
            // Delegate to the same discriminant-detection merge used for a
            // directly-aliased union (`type X = A | B`). A union doesn't stop being
            // discriminated just because it's wrapped in an intersection
            // (`type X = Base & (A | B)`, e.g. react-day-picker's real
            // `DayPickerProps`) or used as `Omit<A | B, K>`'s base — falling back to
            // a naive per-member merge here silently lost the discriminant and
            // collapsed each prop's type to whichever branch happened to be seen
            // last instead of a proper union across all branches.
            resolve_union_alias(members, file_path, mapping, ctx, state, depth)
        }
        // None of these can ever provide a props chain (a props type must be
        // object-like — an interface, intersection, union, or inline object).
        // Named explicitly, not via a wildcard, so a future CollectedType
        // variant forces a compile error here instead of silently falling
        // through to an empty, undiagnosed chain the way IndexedAccess and
        // Function once did.
        CollectedType::String
        | CollectedType::Number
        | CollectedType::Boolean
        | CollectedType::Null
        | CollectedType::Undefined
        | CollectedType::Any
        | CollectedType::Never
        | CollectedType::Unknown
        | CollectedType::Void
        | CollectedType::BigInt
        | CollectedType::Symbol
        | CollectedType::StringLiteral(_)
        | CollectedType::NumberLiteral(_)
        | CollectedType::BoolLiteral(_)
        | CollectedType::Array(_)
        | CollectedType::Tuple(_)
        | CollectedType::TypeOf(_)
        | CollectedType::IndexedAccess { .. }
        | CollectedType::TemplateLiteral(_)
        | CollectedType::Function { .. }
        | CollectedType::Conditional { .. }
        | CollectedType::Mapped { .. }
        | CollectedType::KeyOf(_)
        | CollectedType::Raw(_) => {
            state.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "'{}' cannot be used as a component's props base in '{}' — expected an interface, \
                     intersection, union, or inline object type",
                    base.to_raw_string(),
                    file_path
                ),
                file: Some(file_path.to_string()),
                line: None,
                column: None,
                help: Some("Check that this member of the props type resolves to an object-like shape.".into()),
                code: DiagnosticCode::OpaqueType,
            });
            // `composes` (react-docgen's own mechanism for "props come from this type,
            // named instead of flattened") records the raw expression we just diagnosed,
            // instead of vanishing with only a diagnostic to show for it.
            ResolvedChain::empty_with_compose(base.to_raw_string())
        }
    }
}

/// Resolve a `CollectedTypeAlias::Union` — detect discriminated union if possible.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_union_alias(
    members: &[CollectedType],
    file_path: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    use super::chain::find_discriminant_prop;

    // Collect all member chains for named types (only Named can be discriminated).
    let named_members: Vec<(&str, Vec<ParsedProp>)> = members
        .iter()
        .filter_map(|m| {
            if let CollectedType::Named { name, .. } = m {
                let mut branch_state = state.branch();
                let chain =
                    resolve_props_chain(name.as_str(), &[], file_path, mapping, ctx, &mut branch_state, depth + 1);
                state.absorb(branch_state);
                Some((name.as_str(), chain.props))
            } else {
                None
            }
        })
        .collect();

    if named_members.len() < 2 {
        // Not a discriminated union — just merge all.
        let mut chain = ResolvedChain::empty();
        for member in members {
            let sub = resolve_base_as_chain(member, file_path, mapping, ctx, state, depth);
            chain.merge_parent(sub);
        }
        return chain;
    }

    // Try to find a discriminant prop (a prop whose type is a distinct string literal across all members).
    let discriminant = find_discriminant_prop(&named_members);

    // Merge all props from all members. A prop's merged type is the union of every
    // distinct type it has across the members that declare it (deduped — two members
    // contributing the identical type must not produce a redundant single-member
    // union); a prop missing from at least one member becomes optional overall, even
    // if it was required in every member where it does appear, since the union type
    // doesn't guarantee its presence.
    let mut merged_props: BTreeMap<String, ParsedProp> = BTreeMap::new();
    let mut prop_types: BTreeMap<String, Vec<PropType>> = BTreeMap::new();
    let mut seen_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut required_in_all: BTreeMap<String, bool> = BTreeMap::new();
    let mut total_variants: usize = 0;

    for (_, member_props) in &named_members {
        total_variants += 1;
        record_variant_props(member_props, &mut merged_props, &mut prop_types, &mut seen_count, &mut required_in_all);
    }

    // Also merge non-Named members (inline objects, intersections, nested unions, etc.)
    // that were excluded from the discriminant analysis above.
    for member in members {
        if !matches!(member, CollectedType::Named { .. }) {
            let sub_chain = resolve_base_as_chain(member, file_path, mapping, ctx, state, depth);
            total_variants += 1;
            record_variant_props(
                &sub_chain.props,
                &mut merged_props,
                &mut prop_types,
                &mut seen_count,
                &mut required_in_all,
            );
        }
    }

    for (name, mut types) in prop_types {
        let prop_type = match types.len() {
            0 | 1 => match types.pop() {
                Some(t) => t,
                None => continue,
            },
            _ => PropType::Union(types),
        };

        if let Some(entry) = merged_props.get_mut(&name) {
            entry.prop_type = prop_type;
            let present_everywhere = seen_count.get(&name).copied().unwrap_or(0) == total_variants;
            let required_everywhere = required_in_all.get(&name).copied().unwrap_or(false);
            entry.required = present_everywhere && required_everywhere;
        }
    }

    // If we found a discriminant, merge its type as a union of all its literal values.
    if let Some(ref disc_name) = discriminant {
        let disc_literals: Vec<PropType> = named_members
            .iter()
            .filter_map(|(_, props)| props.iter().find(|p| &p.name == disc_name).map(|p| p.prop_type.clone()))
            .collect();

        if let Some(disc_prop) = merged_props.get_mut(disc_name) {
            disc_prop.prop_type = PropType::Union(disc_literals);
        }
    }

    // Emit discriminated union diagnostic if applicable.
    if discriminant.is_some() {
        state.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Info,
            message: format!(
                "Discriminated union detected with discriminant prop '{}'",
                discriminant.as_deref().unwrap_or("")
            ),
            file: Some(file_path.to_string()),
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::DiscriminatedUnion,
        });
    }

    ResolvedChain {
        props: merged_props.into_values().collect(),
        discriminant_prop: discriminant,
        ..ResolvedChain::empty()
    }
}

/// Record one union member's props into the shared merge accumulators: the
/// first-seen `ParsedProp` (for name/description/parent/etc.), the set of
/// distinct types contributed for each prop name, how many members declared
/// it, and whether every member that declared it marked it required.
fn record_variant_props(
    props: &[ParsedProp],
    merged_props: &mut BTreeMap<String, ParsedProp>,
    prop_types: &mut BTreeMap<String, Vec<PropType>>,
    seen_count: &mut BTreeMap<String, usize>,
    required_in_all: &mut BTreeMap<String, bool>,
) {
    for prop in props {
        merged_props.entry(prop.name.clone()).or_insert_with(|| prop.clone());

        let types = prop_types.entry(prop.name.clone()).or_default();
        if !types.contains(&prop.prop_type) {
            types.push(prop.prop_type.clone());
        }

        *seen_count.entry(prop.name.clone()).or_insert(0) += 1;

        required_in_all
            .entry(prop.name.clone())
            .and_modify(|required| *required = *required && prop.required)
            .or_insert(prop.required);
    }
}

/// Resolve a `CollectedTypeAlias` to a `PropType` (at the type level, not chain level).
///
/// Members reference relative to `alias.file_path()` — the alias's OWN declaring
/// file — never the caller's `consuming_file`. A `type Combined = A | B` alias
/// imported cross-file still has `A`/`B` as same-file siblings of `Combined`
/// wherever it was actually declared, not siblings of whatever file imported it.
pub(super) fn resolve_type_alias_type(
    alias: &CollectedTypeAlias,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    let file_path = alias.file_path();
    match alias {
        CollectedTypeAlias::LiteralUnion { members, .. } => {
            PropType::LiteralUnion { members: members.clone(), has_default: false }
        }
        CollectedTypeAlias::Passthrough { target, .. } => {
            resolve_collected_type(target, file_path, ctx, state, depth + 1)
        }
        CollectedTypeAlias::Union { members, .. } => {
            let resolved: Vec<PropType> =
                members.iter().map(|m| resolve_collected_type(m, file_path, ctx, state, depth + 1)).collect();
            PropType::Union(resolved)
        }
        CollectedTypeAlias::Intersection { members, .. } => {
            let resolved: Vec<PropType> =
                members.iter().map(|m| resolve_collected_type(m, file_path, ctx, state, depth + 1)).collect();
            PropType::Intersection(resolved)
        }
        CollectedTypeAlias::Partial { base, .. } | CollectedTypeAlias::Required { base, .. } => {
            resolve_collected_type(base, file_path, ctx, state, depth + 1)
        }
        CollectedTypeAlias::Omit { base, .. } | CollectedTypeAlias::Pick { base, .. } => {
            // At the type level, just forward to the base type.
            resolve_collected_type(base, file_path, ctx, state, depth + 1)
        }
    }
}

/// The key set a `Pick`/`Omit` key expression denotes, or a warning and `None` when it can't be known statically.
#[allow(clippy::too_many_arguments)]
fn resolve_key_set_or_warn(
    utility: &str,
    keys: &CollectedType,
    file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<Vec<String>> {
    let resolved = resolve_key_set(keys, file, mapping, ctx, state, depth);
    if resolved.is_none() {
        let what = if utility == "Omit" { "nothing is omitted" } else { "no props are picked" };
        state.diagnostics.push(
            Diagnostic::new(
                DiagnosticSeverity::Warning,
                DiagnosticCode::OpaqueType,
                format!("Cannot determine the keys of {utility}<…, {}> in '{file}', so {what}", keys.to_raw_string()),
            )
            .with_file(file.to_string()),
        );
    }
    resolved
}

/// Literals, unions, `keyof X`, a key alias (`type Keys = "a" | "b"`), `Exclude`/`Extract` of key sets, and an
/// argument substituted from another file. `None` for anything else, which only the checker could evaluate.
fn resolve_key_set(
    keys: &CollectedType,
    file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<Vec<String>> {
    if depth > super::MAX_DEPTH {
        return None;
    }
    match keys {
        CollectedType::StringLiteral(s) => Some(vec![s.to_string()]),
        CollectedType::NumberLiteral(n) => Some(vec![n.to_string()]),
        CollectedType::Never => Some(vec![]),
        CollectedType::Union(members) => {
            let mut all = Vec::new();
            for member in members {
                all.extend(resolve_key_set(member, file, mapping, ctx, state, depth + 1)?);
            }
            Some(all)
        }
        CollectedType::AtFile { file, inner } => resolve_key_set(inner, file, mapping, ctx, state, depth + 1),
        CollectedType::KeyOf(target) => {
            // A fresh cycle set: reading a type's keys is a separate question from the props walk that called it, which
            // has usually just visited the same type (`Omit<Base, keyof Base>` resolves `Base` first). Sharing the set
            // made `keyof Base` look circular and come back empty. Depth still bounds the recursion.
            let mut branch_state = state.detached();
            let chain = resolve_base_as_chain(target, file, mapping, ctx, &mut branch_state, depth + 1);
            state.absorb(branch_state);
            Some(chain.props.into_iter().map(|p| p.name).collect())
        }
        CollectedType::Named { name, args } if (name == "Exclude" || name == "Extract") && args.len() == 2 => {
            let from = resolve_key_set(&args[0], file, mapping, ctx, state, depth + 1)?;
            let by: FxHashSet<String> =
                resolve_key_set(&args[1], file, mapping, ctx, state, depth + 1)?.into_iter().collect();
            let keep_members = name == "Extract";
            Some(from.into_iter().filter(|k| by.contains(k) == keep_members).collect())
        }
        // An unfilled type parameter (`Omit<T, "style" | PropsToOmit>` with no argument for `PropsToOmit`) contributes
        // no keys; its default is unknown to the extractor and is almost always `never`.
        CollectedType::Named { name, args } if args.is_empty() && state.in_scope_type_params.contains(name) => {
            Some(vec![])
        }
        CollectedType::Named { name, args } => {
            let (canonical_file, _, matched) =
                super::precedence::resolve_source_defined_or_known(name, &[], file, ctx, state);
            let Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias }) = matched else {
                return None;
            };
            let alias = super::substitute::apply_generic_args(
                alias,
                (&canonical_file, &matched_key),
                args,
                file,
                ctx,
                &mut state.diagnostics,
            );
            match &alias {
                CollectedTypeAlias::LiteralUnion { members, .. } => Some(members.clone()),
                CollectedTypeAlias::Union { members, file_path } => {
                    let union = CollectedType::Union(members.clone());
                    resolve_key_set(&union, file_path, mapping, ctx, state, depth + 1)
                }
                CollectedTypeAlias::Passthrough { target, file_path } => {
                    resolve_key_set(target, file_path, mapping, ctx, state, depth + 1)
                }
                _ => None,
            }
        }
        _ => None,
    }
}
