//! Named type resolution.

use camino::Utf8Path;
use compact_str::CompactString;

use crate::known::{push_known_opaque_diagnostic, resolve_known, KnownPatternResult};
use crate::react_types;
use crate::types::*;

use super::alias::resolve_type_alias_type;
use super::collected::resolve_collected_type;
use super::import::{lookup_ambient_global, AmbientGlobalLookup};
use super::precedence::Declared;
use super::react::react_type_to_prop_type;
use super::{ResolutionContext, MAX_NAMED_NESTING};

/// Cycle-detecting entry point — see `ResolveState::named_in_progress`'s doc
/// comment for why this can't reuse chain.rs's permanent `visited` set or
/// mimic its clone-per-branch pattern. Delegates the actual resolution to
/// `resolve_named_uncycled` so every one of that function's many early
/// returns doesn't need to remember to remove the in-progress key itself.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_named(
    name: &CompactString,
    args: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // Structural arguments: their display form abbreviates object types to `{ ... }`, which collided distinct
    // instantiations into a false cycle.
    let visit_key: CompactString = {
        use std::hash::Hasher;
        let mut hasher = rustc_hash::FxHasher::default();
        args.iter().for_each(|arg| arg.structural_hash(&mut hasher));
        format!("{}:{}<{:x}>", consuming_file, name, hasher.finish()).into()
    };

    // The same reference resolves the same way in any state that answers the reads it made (`resolver::memo`), and
    // Chakra's style props name a few thousand distinct ones a million times.
    ctx.named_results.get_or_resolve(&visit_key, depth, state, None, |state| {
        if !state.enter_named(&visit_key) {
            state.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Circular type reference detected resolving '{}' in '{}' — stopping here to avoid infinite recursion",
                    name, consuming_file
                ),
                file: Some(consuming_file.to_string()),
                line: None,
                column: None,
                help: Some("This type (directly or indirectly) references itself.".into()),
                code: DiagnosticCode::CircularReference,
                affects: Vec::new(),
            });
            return OpaqueDetail::new(name.to_string(), OpaqueReason::DepthExceeded);
        }

        let result = resolve_named_uncycled(name, args, consuming_file, ctx, state, depth);
        state.leave_named(&visit_key);
        result
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_named_uncycled(
    name: &CompactString,
    args: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // `depth` counts how deeply one type expression nests; an alias body starts again at 0, since the alias is a
    // declaration of its own (csstype's `Property.X` is four aliases deep under a prop already 15 levels down). What
    // bounds the hops between declarations, and the expansion of a generic that never repeats, is the names in progress.
    if super::past_max_depth(state, depth) || state.named_in_progress.len() > MAX_NAMED_NESTING {
        let diag = super::max_depth_diagnostic(&format!("named type '{}'", name), consuming_file);
        return OpaqueDetail::give_up(state, name.to_string(), OpaqueReason::DepthExceeded, diag);
    }

    // ── 1. React builtin check ────────────────────────────────────────────────
    let lookup_name = name.strip_prefix("React.").unwrap_or(name.as_str());
    // React's meaning only for React's binding: a project's own `ReactNode` or `Ref` resolves as declared below.
    if react_types::is_react_builtin(lookup_name, &ctx.extra_builtins)
        && (ctx.extra_builtins.contains(lookup_name) || super::may_be_react_builtin(name, consuming_file, ctx))
    {
        return react_type_to_prop_type(name.as_str(), args, consuming_file, ctx, state, depth);
    }

    // ── 2-5. Try the project's own source before a known-pattern shortcut ────
    // See `resolver::precedence` — the shared, single-source-of-truth order.
    let (canonical_file, canonical_name, declared) =
        super::precedence::find_declaration(name.as_str(), consuming_file, ctx, state);
    // The arguments of everything but an alias are resolved here; an alias's are resolved once, where its expansion
    // reads them, instead of here as well.
    let resolved_args: Vec<PropType> = if matches!(declared, Some(Declared::TypeAlias { .. })) {
        Vec::new()
    } else {
        args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect()
    };
    let matched = match declared {
        Some(Declared::TypeAlias { matched_key, alias }) => {
            Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias: *alias })
        }
        Some(Declared::Interface(iface)) => Some(super::precedence::SourceOrKnownMatch::Interface(iface)),
        None => resolve_known(name, &resolved_args, &ctx.global, &|n| ctx.enum_key_for(n, consuming_file))
            .map(super::precedence::SourceOrKnownMatch::Known),
    };

    match matched {
        Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias }) => {
            // The call site's arguments replace the alias's declared parameters (`type Foo<TData> = ...` used as
            // `Foo<string>`). Parameters left unfilled stay expected, unexpandable placeholders, not unresolvable
            // types, so they are registered for step 7 below.
            let params = ctx.named_types.lookup_type_alias_params(&canonical_file, &matched_key);
            let defaults = ctx.named_types.lookup_type_param_defaults(&canonical_file, &matched_key);
            let outer_named_unevaluated = std::mem::take(&mut state.named_unevaluated);
            let expanded = match params {
                Some(params) if !args.is_empty() || defaults.is_some() => {
                    state.add_type_params(params.iter());
                    expand_alias(&alias, params, defaults, args, consuming_file, &canonical_file, ctx, state, 0)
                }
                Some(params) => {
                    state.add_type_params(params.iter());
                    resolve_type_alias_type(&alias, ctx, state, depth)
                }
                None => resolve_type_alias_type(&alias, ctx, state, 0),
            };
            let unevaluated = state.named_unevaluated || expanded.holds_unevaluated_type();
            state.named_unevaluated = outer_named_unevaluated || unevaluated;
            // Printed as its name when it can't be evaluated, or when a generic instantiation expands to a long union, as
            // TypeScript prints it; its arguments are then shown as far as they evaluate.
            let by_name = unevaluated || (!args.is_empty() && exceeds_expanded_size(&expanded));
            if matches!(expanded, PropType::Object(_)) || !by_name {
                return expanded;
            }
            let args: Vec<CollectedType> =
                args.iter().map(|arg| displayed_arg(arg, consuming_file, ctx, state, depth)).collect();
            return PropType::Named { name: name.clone(), args: args.iter().map(super::func::as_written).collect() };
        }
        Some(super::precedence::SourceOrKnownMatch::Interface(_)) => {
            // At the prop-TYPE level (not chain level), an interface name is returned as Named.
            // Full prop expansion only happens at the component level via resolve_props_chain.
            return PropType::Named { name: name.clone(), args: resolved_args };
        }
        Some(super::precedence::SourceOrKnownMatch::Known(result)) => {
            return match result {
                KnownPatternResult::Type(pt) => {
                    if let PropType::Opaque(detail) = &pt {
                        push_known_opaque_diagnostic(
                            &mut state.diagnostics,
                            detail.reason(),
                            name.as_str(),
                            consuming_file,
                        );
                    }
                    pt
                }
                KnownPatternResult::Alias { name: alias_name } => {
                    // Follow the alias through resolve_named.
                    let alias_ct = CollectedType::Named { name: alias_name.as_str().into(), args: vec![] };
                    resolve_collected_type(&alias_ct, consuming_file, ctx, state, depth + 1)
                }
                KnownPatternResult::Props(_) => {
                    // Props result at type level — surface as Named.
                    PropType::Named { name: name.clone(), args: resolved_args }
                }
            };
        }
        None => {}
    }

    // A builtin name bound elsewhere whose binding didn't resolve (a re-export of React's type, a package not
    // installed): React's meaning is then a reported guess.
    if react_types::is_react_builtin(lookup_name, &ctx.extra_builtins) {
        state.diagnostics.push(super::builtin_fallback_diagnostic(name, consuming_file, ctx));
        return react_type_to_prop_type(name.as_str(), args, consuming_file, ctx, state, depth);
    }

    // ── 6. Silent no-op for well-known unresolvable types ────────────────────
    // TypeScript built-in utility types, DOM element types, and React HTML
    // attribute types all appear as prop types in real-world .d.ts files but
    // cannot be expanded without a type-checker. Return Named silently.
    let bare = name.strip_prefix("React.").unwrap_or(name.as_str());
    // A lib alias that evaluates (`Exclude`, `Extract`, `NonNullable`) is read from TypeScript's own definition.
    if !args.is_empty() {
        if let Some(evaluated) = evaluate_lib_alias(bare, args, consuming_file, ctx, state, depth) {
            return evaluated;
        }
    }
    if super::is_ts_utility_type(bare)
        || matches!(
            bare,
            // TypeScript primitives used as type names
            "object" | "Object" | "Function" | "Symbol" | "BigInt"
            // React HTML attribute types (not in is_react_builtin; appear as prop types)
            | "HTMLAttributes" | "InputHTMLAttributes" | "TextareaHTMLAttributes"
            | "SelectHTMLAttributes" | "ButtonHTMLAttributes" | "AnchorHTMLAttributes"
            | "FormHTMLAttributes" | "LabelHTMLAttributes" | "ImgHTMLAttributes"
            | "VideoHTMLAttributes" | "AudioHTMLAttributes" | "DOMAttributes"
            | "AriaAttributes" | "HTMLInputTypeAttribute" | "HTMLAttributeReferrerPolicy"
            | "HTMLAttributeAnchorTarget" | "HTMLInputAutoCompleteAttribute"
            // React SVG/generic HTML prop types
            | "SVGAttributes" | "SVGProps" | "HTMLProps"
            // React component utility types
            | "ComponentRef" | "JSXElementConstructor"
        )
        || bare.ends_with("HTMLAttributes")
    {
        return PropType::Named { name: name.clone(), args: resolved_args };
    }
    // DOM element ref types (HTMLDivElement, HTMLInputElement, etc.)
    if bare.starts_with("HTML") && bare.ends_with("Element") {
        return PropType::Named { name: name.clone(), args: resolved_args };
    }
    if bare.starts_with("SVG") && bare.ends_with("Element") {
        return PropType::Named { name: name.clone(), args: resolved_args };
    }

    // ── 6.5 Enclosing generic's own type parameter — expected, not unresolvable ─
    if state.has_type_param(bare) {
        return PropType::Named { name: name.clone(), args: resolved_args };
    }

    // ── 6.7 TypeScript's own ambient globals (Date, RegExp, Element, ...) ────
    // Deliberately last: these never go through an import, so nothing earlier
    // ever had a reason to look here — but checked only after every other
    // shortcut above (including the well-known-TS-utility-type list at step 6)
    // has had its chance, since some names (Record, Partial, ...) are
    // themselves declared in these same lib files as mapped types this tool
    // can't expand, and step 6 already handles those correctly.
    if let Some(found) = lookup_ambient_global(ctx, bare) {
        return match found {
            AmbientGlobalLookup::Interface => PropType::Named { name: name.clone(), args: resolved_args },
            AmbientGlobalLookup::TypeAlias(alias) => {
                let alias = alias.clone();
                resolve_type_alias_type(&alias, ctx, state, depth)
            }
        };
    }

    // ── 6.8 A type React declares that the builtin list doesn't name ──────────
    if super::declared_by_react(name, &canonical_name, consuming_file, ctx) {
        return react_type_to_prop_type(name.as_str(), args, consuming_file, ctx, state, depth);
    }

    // ── 7. Unresolvable — emit diagnostic, return Named ───────────────────────
    let location_note =
        super::unresolved_location_note(name.as_str(), consuming_file, &canonical_file, &canonical_name);
    state.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!(
            "Cannot resolve type '{}' in '{}'{} — it will appear as an unexpanded named reference",
            name, consuming_file, location_note
        ),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some(super::unresolved_type_help(
            name.as_str(),
            consuming_file,
            ctx,
            "Check that the package is installed and its types are resolvable.",
        )),
        code: DiagnosticCode::UnresolvableImport,
        affects: Vec::new(),
    });
    PropType::Named { name: name.clone(), args: resolved_args }
}

/// A referenced alias whose expansion is `unevaluated` — it holds a conditional or mapped type the resolver can't
/// evaluate, or an alias named for that reason — shows as its name with its arguments as written
/// (`StyleProp<MantineSpacing>`), the way TypeScript prints it, rather than as that type's source text. An object
/// expansion stays: its fields still read, and only the unevaluated field shows source.
/// A generic alias with `args` bound, its omitted parameters at their defaults. An alias that is a conditional over a bare
/// type parameter (`type Exclude<T, U> = T extends U ? never : T`) distributes over the members of the argument bound to
/// it, each evaluated alone and the results united, as TypeScript does.
#[allow(clippy::too_many_arguments)]
fn expand_alias(
    alias: &CollectedTypeAlias,
    params: &[CompactString],
    defaults: Option<&[Option<CollectedType>]>,
    args: &[CollectedType],
    args_file: &Utf8Path,
    declared_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    let subst = super::substitute::bind_args(params, defaults, args, args_file, declared_file, &mut state.diagnostics);
    if let Some(param) = super::evaluate::distributive_param(alias) {
        if let Some(bound) = subst.get(param) {
            let members = literal_members(super::evaluate::union_members(bound, args_file, ctx, 0), ctx, state, depth);
            if members.len() != 1 {
                let results = members.into_iter().map(|(member, member_file)| {
                    let mut one = subst.clone();
                    one.insert(param, CollectedType::AtFile { file: member_file, inner: Box::new(member) });
                    resolve_type_alias_type(&super::substitute::substitute_alias(alias, &one), ctx, state, depth)
                });
                return super::primitives::union_of(results.collect::<Vec<_>>());
            }
        }
    }
    resolve_type_alias_type(&super::substitute::substitute_alias(alias, &subst), ctx, state, depth)
}

/// `members` with each one that is not already a literal or primitive resolved (`(typeof SIZES)[number]`, a template
/// literal), replaced by the literals it comes to; one that comes to something else stays as written.
fn literal_members(
    members: Vec<super::evaluate::Located>,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Vec<super::evaluate::Located> {
    let mut out = Vec::with_capacity(members.len());
    for (member, file) in members {
        let plain = matches!(
            member,
            CollectedType::StringLiteral(_)
                | CollectedType::NumberLiteral(_)
                | CollectedType::BoolLiteral(_)
                | CollectedType::String
                | CollectedType::Number
                | CollectedType::Boolean
                | CollectedType::Null
                | CollectedType::Undefined
        );
        if plain {
            out.push((member, file));
            continue;
        }
        let resolved = resolve_collected_type(&member, &file, ctx, state, depth + 1);
        match literals_of(&resolved) {
            Some(literals) => out.extend(literals.into_iter().map(|literal| (literal, file.clone()))),
            None => out.push((member, file)),
        }
    }
    out
}

/// The literals and primitives `ty` is a union of, as collected types; `None` when any member is something else.
fn literals_of(ty: &PropType) -> Option<Vec<CollectedType>> {
    Some(match ty {
        PropType::StringLiteral(s) => vec![CollectedType::StringLiteral(s.as_str().into())],
        PropType::NumberLiteral(n) => vec![CollectedType::NumberLiteral(*n)],
        PropType::BoolLiteral(b) => vec![CollectedType::BoolLiteral(*b)],
        PropType::LiteralUnion { members, .. } => {
            members.iter().map(|m| CollectedType::StringLiteral(m.as_str().into())).collect()
        }
        PropType::Null => vec![CollectedType::Null],
        PropType::Undefined => vec![CollectedType::Undefined],
        PropType::String => vec![CollectedType::String],
        PropType::Number => vec![CollectedType::Number],
        PropType::Boolean => vec![CollectedType::Boolean],
        PropType::Never => vec![],
        PropType::Union(members) => {
            let mut all = Vec::new();
            for member in members {
                all.extend(literals_of(member)?);
            }
            all
        }
        _ => return None,
    })
}

/// `name` as TypeScript's lib declares it, when that is a conditional or an intersection and every part of it evaluates
/// with `args` (`Exclude`, `Extract`, `NonNullable`). A lib alias that needs inference (`ReturnType`) or a mapped type
/// (`Partial`) leaves its result unevaluated, and the caller keeps its shortcut.
fn evaluate_lib_alias(
    name: &str,
    args: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<PropType> {
    let (lib_file, alias) = ctx.named_types.lookup_ambient_alias(&ctx.ambient_global_files, name)?;
    let can_evaluate = matches!(
        alias,
        CollectedTypeAlias::Intersection { .. }
            | CollectedTypeAlias::Passthrough { target: CollectedType::Conditional { .. }, .. }
    );
    let params = ctx.named_types.lookup_type_alias_params(lib_file, name).filter(|_| can_evaluate)?;
    let defaults = ctx.named_types.lookup_type_param_defaults(lib_file, name);
    // Its diagnostics are the lib's, not the project's: kept only when the whole alias evaluates.
    let mut branch = state.detached();
    let expanded = expand_alias(alias, params, defaults, args, consuming_file, lib_file, ctx, &mut branch, depth);
    if branch.named_unevaluated || expanded.holds_unevaluated_type() {
        return None;
    }
    state.absorb_detached(branch);
    Some(expanded)
}

/// An alias argument as TypeScript prints it: a member read (`Variant["size"]`) shows as the member's type and a finite
/// template literal as its strings, union members included; anything else as written.
fn displayed_arg(
    arg: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> CollectedType {
    if super::past_max_depth(state, depth) {
        return arg.clone();
    }
    match arg {
        // What the member read reads may itself read a member (csstype's `Fallback<T>` gives `T[P] | …`).
        CollectedType::IndexedAccess { .. } => match super::evaluate::displayed_member(arg, file, ctx) {
            Some(found) => displayed_arg(&found.0, &found.1, ctx, state, depth + 1),
            None => arg.clone(),
        },
        CollectedType::Named { name, args } if !args.is_empty() => {
            let args: Vec<CollectedType> = args.iter().map(|a| displayed_arg(a, file, ctx, state, depth + 1)).collect();
            // A lib alias over what the arguments came to (`NonNullable<Property.Filter | undefined>`) is its value.
            let bare = name.strip_prefix("React.").unwrap_or(name.as_str());
            match evaluate_lib_alias(bare, &args, file, ctx, state, depth + 1).as_ref().and_then(literals_of) {
                Some(literals) if !literals.is_empty() => CollectedType::Union(literals),
                _ => CollectedType::Named { name: name.clone(), args },
            }
        }
        CollectedType::Array(inner) => {
            CollectedType::Array(Box::new(displayed_arg(inner, file, ctx, state, depth + 1)))
        }
        CollectedType::TemplateLiteral(parts) => {
            match super::template::try_expand_template_literal(parts, file, ctx, state, depth + 1) {
                Some(values) => CollectedType::Union(
                    values.into_iter().map(|value| CollectedType::StringLiteral(value.into())).collect(),
                ),
                None => arg.clone(),
            }
        }
        CollectedType::Union(members) => {
            CollectedType::Union(members.iter().map(|m| displayed_arg(m, file, ctx, state, depth + 1)).collect())
        }
        CollectedType::AtFile { file, inner } => CollectedType::AtFile {
            file: file.clone(),
            inner: Box::new(displayed_arg(inner, file, ctx, state, depth + 1)),
        },
        _ => arg.clone(),
    }
}

/// The most nodes a generic alias's expansion may have to be shown instead of its name (`ConditionalValue<…>` over
/// csstype's keyword unions has hundreds). Its arguments say as much, and a copy of the union for every style prop of
/// every component is most of a large design system's memory.
const MAX_EXPANDED_NODES: usize = 64;

/// Whether `ty` has more than `MAX_EXPANDED_NODES` nodes, counting no further once it does.
fn exceeds_expanded_size(ty: &PropType) -> bool {
    fn count(ty: &PropType, budget: &mut usize) -> bool {
        if *budget == 0 {
            return true;
        }
        *budget -= 1;
        match ty {
            PropType::Union(members) | PropType::Intersection(members) | PropType::Tuple(members) => {
                members.iter().any(|member| count(member, budget))
            }
            PropType::LiteralUnion { members, .. } => {
                *budget = budget.saturating_sub(members.len());
                *budget == 0
            }
            PropType::Array(inner) => count(inner, budget),
            PropType::Object(fields) => fields.iter().any(|field| count(&field.prop_type, budget)),
            PropType::Named { args, .. } => args.iter().any(|arg| count(arg, budget)),
            _ => false,
        }
    }
    count(ty, &mut MAX_EXPANDED_NODES.clone())
}
