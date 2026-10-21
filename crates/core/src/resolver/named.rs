//! Named type resolution.

use camino::Utf8Path;
use compact_str::CompactString;

use crate::known::{push_known_opaque_diagnostic, KnownPatternResult};
use crate::react_types;
use crate::types::*;

use super::alias::resolve_type_alias_type;
use super::collected::resolve_collected_type;
use super::import::{lookup_ambient_global, AmbientGlobalLookup};
use super::react::react_type_to_prop_type;
use super::{ResolutionContext, MAX_DEPTH};

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
    let visit_key: CompactString = format!("{}:{}<{:?}>", consuming_file, name, args).into();

    if !state.named_in_progress.insert(visit_key.clone()) {
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
    state.named_in_progress.remove(&visit_key);
    result
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
    if depth > MAX_DEPTH {
        let diag = super::max_depth_diagnostic(&format!("named type '{}'", name), consuming_file);
        return OpaqueDetail::give_up(state, name.to_string(), OpaqueReason::DepthExceeded, diag);
    }

    // ── 1. React builtin check ────────────────────────────────────────────────
    let lookup_name = name.strip_prefix("React.").unwrap_or(name.as_str());
    if react_types::is_react_builtin(lookup_name, &ctx.extra_builtins) {
        return react_type_to_prop_type(name.as_str(), args, consuming_file, ctx, state, depth);
    }

    // Resolve type arguments eagerly — needed for both source lookups and known patterns.
    let resolved_args: Vec<PropType> =
        args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect();

    // ── 2-5. Try the project's own source before a known-pattern shortcut ────
    // See `resolver::precedence` — the shared, single-source-of-truth order.
    let (canonical_file, canonical_name, matched) =
        super::precedence::resolve_source_defined_or_known(name.as_str(), &resolved_args, consuming_file, ctx, state);

    match matched {
        Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias }) => {
            // A generic alias's own declared type parameters (`type Foo<TData> = ...`)
            // are expected, unexpandable placeholders wherever referenced in its body —
            // not unresolvable types. Register them so step 7 below doesn't warn.
            // Substituting the call site's arguments here is correct but costs a third of the run on Mantine without a
            // resolution cache, so it waits for that cache.
            if let Some(params) = ctx.named_types.lookup_type_alias_params(&canonical_file, &matched_key) {
                state.in_scope_type_params.extend(params.iter().cloned());
            }
            return resolve_type_alias_type(&alias, ctx, state, depth);
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

    // ── 6. Silent no-op for well-known unresolvable types ────────────────────
    // TypeScript built-in utility types, DOM element types, and React HTML
    // attribute types all appear as prop types in real-world .d.ts files but
    // cannot be expanded without a type-checker. Return Named silently.
    let bare = name.strip_prefix("React.").unwrap_or(name.as_str());
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
    if state.in_scope_type_params.contains(bare) {
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
        help: Some("Check that the package is installed and its types are resolvable.".into()),
        code: DiagnosticCode::UnresolvableImport,
        affects: Vec::new(),
    });
    PropType::Named { name: name.clone(), args: resolved_args }
}
