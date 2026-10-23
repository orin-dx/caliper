//! Props chain resolution — the recursive core of prop resolution.

use std::sync::Arc;

use camino::Utf8Path;
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::known::{push_known_opaque_diagnostic, KnownPatternResult};
use crate::react_types;
use crate::types::*;

use super::alias::resolve_type_alias_chain;
use super::collected::resolve_collected_type;
use super::extends::resolve_extends_ref;
use super::react::resolve_react_types_file;
use super::{ResolutionContext, ResolvedChain, MAX_DEPTH};

/// Resolve a named type to a chain of props.
/// This is the recursive core — handles interfaces, aliases, known patterns, etc.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_props_chain(
    type_name: &str,
    type_args: &[CollectedType],
    consuming_file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    if depth > MAX_DEPTH {
        let diag = super::max_depth_diagnostic(&format!("type '{}'", type_name), consuming_file);
        return ResolvedChain::give_up(type_name.to_owned(), Some(diag), state);
    }

    // Cycle detection — scoped by file so same type name in different files is OK.
    // Must include type_args: distinct instantiations of the same generic alias
    // (e.g. `Partial<RowHeights>` vs `Partial<ColumnWidths>` in the same interface's
    // extends list) are not the same visit and must not collide on the bare name alone. The arguments are keyed by
    // structure: their display form abbreviates object types to `{ ... }`, so distinct instantiations collided.
    let visit_key: CompactString = format!("{}:{}<{:?}>", consuming_file, type_name, type_args).into();
    if !state.visit(visit_key) {
        return ResolvedChain::give_up(
            type_name.to_owned(),
            Some(Diagnostic {
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Circular type reference detected resolving '{}' in '{}' — stopping here to avoid infinite recursion",
                    type_name, consuming_file
                ),
                file: Some(consuming_file.to_string()),
                line: None,
                column: None,
                help: Some("This type (directly or indirectly) extends or references itself.".into()),
                code: DiagnosticCode::CircularReference,
                affects: Vec::new(),
            }),
            state,
        );
    }

    // Strip "React." namespace prefix before all builtin/utility checks.
    let type_name_bare = type_name.strip_prefix("React.").unwrap_or(type_name);

    // ── Step 0.5: Inline utility type in extends position ─────────────────────
    // `extends Pick<T, K>` (and Omit, Partial, Required, Readonly) resolves through the same alias logic as
    // `type X = Pick<T, K>`, on the structured arguments, instead of the step-1 silent no-op.
    // A project's own `Partial` or `Required`, or one imported from a package, is that declaration (step 2), not
    // TypeScript's global.
    let ts_lib = super::may_be_ts_lib_type(type_name, consuming_file, ctx);
    if super::may_be_utility(type_name, consuming_file, ctx) {
        if let Some(alias) = super::substitute::synthesize_utility_alias(type_name_bare, type_args, consuming_file) {
            return resolve_type_alias_chain(&alias, consuming_file, mapping, ctx, state, depth);
        }
    }

    // ── Step 0.6: React's `ComponentProps<typeof Button>` is `Button`'s props ─
    // `typeof` names a value, the component, so this finds the component instead of a type called `Button`, exactly as
    // the same reference does in an `extends` clause.
    if let Some(CollectedType::TypeOf(component)) = type_args.first() {
        let component_props =
            matches!(type_name_bare, "ComponentProps" | "ComponentPropsWithoutRef" | "ComponentPropsWithRef");
        if component_props && super::may_be_react_builtin(type_name, consuming_file, ctx) {
            return super::extends::component_props_of(component, consuming_file, mapping, ctx, state, depth);
        }
    }

    // ── Step 1: TypeScript built-in utility types — silent no-op ─────────────
    // Not prop providers; suppress false "unresolvable" warnings.
    if ts_lib && super::is_ts_utility_type(type_name_bare) {
        return ResolvedChain::empty();
    }

    // ── Step 2: Try the project's own source before a known-pattern shortcut ─
    // See `resolver::precedence` — the shared, single-source-of-truth order
    // `named.rs` already used correctly; this path used to reimplement the
    // sequence independently and check known patterns first, silently
    // replacing project-defined types (e.g. a project's own `interface
    // SxProps`) with the hardcoded library shortcut. Fixed: P0-1.
    let resolved_args: Vec<PropType> =
        type_args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect();

    let (canonical_file, canonical_name, matched) =
        super::precedence::resolve_source_defined_or_known(type_name_bare, &resolved_args, consuming_file, ctx, state);

    match matched {
        Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias }) => {
            // As for interfaces and for aliases in a prop position: parameters left unfilled by the call site are
            // placeholders in the body, not unresolvable types.
            if let Some(params) = ctx.named_types.lookup_type_alias_params(&canonical_file, &matched_key) {
                state.add_type_params(params.iter().skip(type_args.len()));
            }
            let alias = super::substitute::apply_generic_args(
                alias,
                (&canonical_file, &matched_key),
                type_args,
                consuming_file,
                ctx,
                &mut state.diagnostics,
            );
            return resolve_type_alias_chain(&alias, consuming_file, mapping, ctx, state, depth);
        }
        Some(super::precedence::SourceOrKnownMatch::Interface(iface)) => {
            return resolve_interface_chain(iface, type_args, consuming_file, mapping, ctx, state, depth);
        }
        Some(super::precedence::SourceOrKnownMatch::Known(result)) => {
            return match result {
                KnownPatternResult::Props(props) => {
                    ResolvedChain { props: props.into_iter().map(Arc::new).collect(), ..ResolvedChain::empty() }
                }
                KnownPatternResult::Type(PropType::HtmlAttributes { element, omitted }) => {
                    let layer = InheritedLayer {
                        type_name: type_name.to_owned(),
                        file_name: resolve_react_types_file(consuming_file, ctx),
                        omitted,
                        html_element: Some(element),
                        total_props: 0,
                    };
                    ResolvedChain { inheritance: vec![layer], ..ResolvedChain::empty() }
                }
                KnownPatternResult::Type(pt) => {
                    if let PropType::Opaque(detail) = &pt {
                        push_known_opaque_diagnostic(
                            &mut state.diagnostics,
                            detail.reason(),
                            type_name_bare,
                            consuming_file,
                        );
                    }
                    ResolvedChain::empty_with_compose(pt.raw_string())
                }
                KnownPatternResult::Alias { name } => {
                    resolve_props_chain(&name, &[], consuming_file, mapping, ctx, state, depth + 1)
                }
            };
        }
        None => {}
    }

    // ── Step 2.5: React builtin check (after source and known patterns) ──────
    // Terminal React types (ReactNode, Ref, FC, etc.) that survived both are not
    // prop providers — add to composes and stop.
    // A builtin name bound elsewhere reaches here only when that binding didn't resolve (a re-export of React's type, a
    // package not installed); React's meaning is then a reported guess.
    if react_types::is_react_builtin(type_name_bare, &ctx.extra_builtins) {
        if !ctx.extra_builtins.contains(type_name_bare) && !super::may_be_react_builtin(type_name, consuming_file, ctx)
        {
            state.diagnostics.push(super::builtin_fallback_diagnostic(type_name, consuming_file, ctx));
        }
        return ResolvedChain::empty_with_compose(type_name.to_owned());
    }

    // HTML attribute interfaces (`InputHTMLAttributes`, ...) are terminal in curated mode, like the builtins above: the
    // element's notable attributes come from the inheritance layer, not from expanding the interface.
    if react_types::html_element_for(type_name_bare).is_some() {
        return ResolvedChain::empty_with_compose(type_name.to_owned());
    }

    // ── Step 6: Unresolvable ──────────────────────────────────────────────────
    // Import resolution may have redirected `type_name` to a different name/file
    // (re-exports, barrel files) — surface that resolved location when it differs,
    // since "Cannot resolve X in file A" is confusing if X actually lives in file B.
    let location_note = super::unresolved_location_note(type_name, consuming_file, &canonical_file, &canonical_name);
    state.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!("Cannot resolve type '{}' in '{}'{}", type_name, consuming_file, location_note),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some(
            "Type may be in an unresolvable cross-package location. Check that the package is installed.".into(),
        ),
        code: DiagnosticCode::UnresolvableImport,
        affects: Vec::new(),
    });
    ResolvedChain::empty_with_compose(type_name.to_owned())
}

/// Resolve an interface to a chain of props. `type_args` are the call site's arguments for the interface's declared type
/// parameters (`Box<string>` for `interface Box<T>`); they replace the parameters in its members and in its own extends
/// clause.
///
/// A non-generic interface's chain is resolved once per run and shared by every component whose state would resolve it
/// the same way (`resolver::memo`).
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_interface_chain(
    iface: &CollectedInterface,
    type_args: &[CollectedType],
    consuming_file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    let generic = !type_args.is_empty()
        || ctx.named_types.lookup_interface_type_params_for(iface).is_some_and(|params| !params.is_empty());
    if generic {
        return resolve_interface_chain_uncached(iface, type_args, consuming_file, mapping, ctx, state, depth);
    }
    ctx.interface_chains.get_or_resolve(&iface.scoped_key, depth, state, mapping, |state| {
        resolve_interface_chain_uncached(iface, type_args, consuming_file, mapping, ctx, state, depth)
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_interface_chain_uncached(
    iface: &CollectedInterface,
    type_args: &[CollectedType],
    consuming_file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    let mut chain = ResolvedChain::empty();

    // A generic interface's own declared type parameters (`interface Foo<TData>`)
    // are expected, unexpandable placeholders wherever referenced in its body —
    // not unresolvable types. Register them so `resolve_named` doesn't warn.
    let declared_params = ctx.named_types.lookup_interface_type_params_for(iface);
    if let Some(params) = declared_params {
        state.add_type_params(params.iter());
    }
    // Supplied arguments replace the parameters; with none supplied the parameters stay placeholders.
    let substitution = match declared_params {
        Some(params) if !params.is_empty() && !type_args.is_empty() => {
            Some(super::substitute::build_substitution(params, type_args, consuming_file, &mut state.diagnostics))
        }
        _ => None,
    };

    // ── Resolve extends first — parent props come before own props ────────────
    // Each extends entry gets its own cloned `visited` set (same pattern as
    // alias.rs's Omit<T, keyof U> branch and its union-discriminant probe):
    // ordinary diamond inheritance (`interface C extends A, B` where both A and
    // B extend `Base`) would otherwise share one `visited` set across sibling
    // branches, so resolving A's branch marks `Base` visited and B's branch
    // then finds it "already visited" and reports a false "Circular type
    // reference detected" — Base is reached twice because it's genuinely
    // shared by two independent parents, not because of an actual cycle.
    // Cloning per-branch preserves real cycle detection *within* each branch
    // (a parent's own extends chain looping back on itself) while letting
    // sibling branches independently reach the same shared ancestor.
    for extends_ref in &iface.extends {
        let substituted;
        let extends_ref = match &substitution {
            Some(subst) => {
                substituted = extends_ref.map_type_args(|arg| super::substitute::substitute_type(arg, subst));
                &substituted
            }
            None => extends_ref,
        };
        let mut branch_state = state.branch();
        let (parent_chain, maybe_layer) =
            resolve_extends_ref(extends_ref, &iface.file_path, mapping, ctx, &mut branch_state, depth + 1);
        state.absorb(branch_state);
        if let Some(layer) = maybe_layer {
            chain.inheritance.push(layer);
        }
        chain.merge_parent(parent_chain);
    }

    // ── Resolve own props ────────────────────────────────────────────────────
    // A redeclared prop replaces the inherited one, as in TS. An undocumented redeclaration keeps the inherited
    // description, as TS's JSDoc lookup does (`resolve_component` keeps the first duplicate, so drop them here).
    let own_names: FxHashSet<&str> = iface.props.iter().map(|prop| prop.name.as_str()).collect();
    let (redeclared, inherited): (Vec<_>, Vec<_>) =
        std::mem::take(&mut chain.props).into_iter().partition(|prop| own_names.contains(prop.name.as_str()));
    chain.props = inherited;
    let inherited_descriptions: FxHashMap<&str, &str> =
        redeclared.iter().map(|prop| (prop.name.as_str(), prop.description.as_str())).collect();

    let parent = PropParent { name: iface.name.to_string(), file_name: iface.file_path.to_string() };

    for raw_prop in &iface.props {
        let prop_type = match &substitution {
            Some(subst) => {
                let collected = super::substitute::substitute_type(&raw_prop.collected_type, subst);
                resolve_collected_type(&collected, &iface.file_path, ctx, state, depth)
            }
            None => resolve_collected_type(&raw_prop.collected_type, &iface.file_path, ctx, state, depth),
        };

        // Default value: code default takes precedence over JSDoc @default.
        let code_default = state.param_default(mapping, &raw_prop.name);
        let jsdoc_default =
            raw_prop.tags.get("default").or_else(|| raw_prop.tags.get("defaultValue")).map(|s| s.trim());

        let default_value = match (code_default, jsdoc_default) {
            (Some(code), Some(jsdoc))
                if code.value.trim_matches('"').trim_matches('\'') != jsdoc.trim_matches('"').trim_matches('\'') =>
            {
                state.diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Info,
                    message: format!(
                        "JSDoc @default '{}' differs from code default '{}' for prop '{}' — using code value",
                        jsdoc, code.value, raw_prop.name
                    ),
                    file: Some(iface.file_path.to_string()),
                    line: None,
                    column: None,
                    help: Some("Update the JSDoc @default to match the code default.".into()),
                    code: DiagnosticCode::JsDocDefaultMismatch,
                    affects: Vec::new(),
                });
                Some(DefaultValue { value: code.value.clone(), computed: code.computed })
            }
            (Some(code), _) => Some(DefaultValue { value: code.value.clone(), computed: code.computed }),
            (None, Some(jsdoc)) => Some(DefaultValue { value: jsdoc.to_owned(), computed: false }),
            (None, None) => None,
        };

        let description = match inherited_descriptions.get(raw_prop.name.as_str()) {
            Some(inherited) if raw_prop.description.is_empty() => (*inherited).to_owned(),
            Some(_) | None => raw_prop.description.clone(),
        };

        chain.props.push(Arc::new(ParsedProp::new(
            raw_prop.name.clone(),
            prop_type,
            raw_prop.required,
            default_value,
            description,
            raw_prop.tags.clone(),
            Some(parent.clone()),
            vec![parent.clone()],
        )));
    }

    chain
}

/// Find a discriminant prop — a prop that has a distinct string literal type across all members.
pub(super) fn find_discriminant_prop(members: &[(&str, Vec<Arc<ParsedProp>>)]) -> Option<String> {
    if members.is_empty() {
        return None;
    }

    // Collect candidate prop names: appear in all members AND have PropType::StringLiteral in at least one.
    // Use the intersection of all member prop names as candidates.
    let mut candidate_names: Option<std::collections::BTreeSet<&str>> = None;
    for (_, member_props) in members {
        let names: std::collections::BTreeSet<&str> = member_props.iter().map(|p| p.name.as_str()).collect();
        candidate_names = Some(match candidate_names {
            None => names,
            Some(existing) => existing.intersection(&names).copied().collect(),
        });
    }
    let candidate_names = candidate_names.unwrap_or_default();

    'outer: for candidate in &candidate_names {
        // Check that this prop has a string literal type in at least one member.
        let has_literal = members.iter().any(|(_, props)| {
            props.iter().any(|p| p.name.as_str() == *candidate && matches!(p.prop_type, PropType::StringLiteral(_)))
        });
        if !has_literal {
            continue;
        }

        let mut literal_values: Vec<&str> = Vec::new();
        for (_, member_props) in members {
            let found = member_props.iter().find(|p| p.name.as_str() == *candidate);
            match found {
                Some(p) => {
                    if let PropType::StringLiteral(ref s) = p.prop_type {
                        if literal_values.contains(&s.as_str()) {
                            // Not distinct — try next prop.
                            continue 'outer;
                        }
                        literal_values.push(s.as_str());
                    } else {
                        continue 'outer; // Not a literal in this member.
                    }
                }
                None => continue 'outer, // Not present in all members.
            }
        }

        if literal_values.len() == members.len() {
            return Some((*candidate).to_owned());
        }
    }

    None
}
