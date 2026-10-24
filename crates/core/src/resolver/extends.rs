//! Extends ref resolution.

use camino::Utf8Path;

use crate::types::*;

use crate::pipeline::HtmlAttributeMode;

use super::chain::{resolve_interface_chain, resolve_props_chain};
use super::import::{resolve_import_specifier, resolve_to_canonical};
use super::react::resolve_react_types_file;
use super::{ResolutionContext, ResolvedChain};

/// Resolve a single `ExtendsRef` and return `(chain, Option<InheritedLayer>)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_extends_ref(
    extends_ref: &ExtendsRef,
    iface_file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> (ResolvedChain, Option<InheritedLayer>) {
    match extends_ref {
        // The extractor names builtins by spelling before it has seen the whole file; a project's own declaration, or
        // one imported from another package, resolves as that declaration when it can be found.
        ExtendsRef::Builtin { name, type_args, .. }
            if !super::may_be_react_builtin(name, iface_file, ctx)
                && super::declaration_resolves(name, iface_file, ctx) =>
        {
            (resolve_props_chain(name.as_str(), type_args, iface_file, mapping, ctx, state, depth), None)
        }
        ExtendsRef::Builtin { name, element, type_args } => {
            // Bound elsewhere but not found (a package not installed): React's meaning is a reported guess.
            if !super::may_be_react_builtin(name, iface_file, ctx) {
                state.diagnostics.push(super::builtin_fallback_diagnostic(name, iface_file, ctx));
            }
            // HTML element attrs. In Curated/None mode the actual props aren't
            // resolved here (they'd live in @types/react) — we just record an
            // InheritedLayer, and resolve_component synthesizes a curated subset
            // separately. In Full mode, real_html_attrs_chain looks up the actual
            // interface (already merged into GlobalSourceData under @types/react's
            // real file path by the pipeline) and resolves it for real, same as any
            // other interface — falling back to the InheritedLayer-only behavior if
            // it's not found (e.g. @types/react wasn't actually merged).
            if let Some(element_name) = element {
                let file_name = resolve_react_types_file(iface_file, ctx);
                let real_chain = if ctx.html_attributes == HtmlAttributeMode::Full {
                    real_html_attrs_chain(name.as_str(), type_args, iface_file, &file_name, mapping, ctx, state, depth)
                } else {
                    None
                };
                let total_props = real_chain.as_ref().map_or(0, |c| c.props.len() as u32);
                let layer = InheritedLayer {
                    type_name: name.to_string(),
                    file_name,
                    omitted: vec![],
                    html_element: Some(element_name.clone()),
                    total_props,
                };
                (real_chain.unwrap_or_else(ResolvedChain::empty), Some(layer))
            } else {
                // Non-HTML-element builtins. ComponentPropsWithoutRef/ComponentProps:
                // expand directly to HtmlAttributes based on the first type arg.
                let bare = name.as_str().strip_prefix("React.").unwrap_or(name.as_str());
                if matches!(bare, "ComponentPropsWithoutRef" | "ComponentPropsWithRef" | "ComponentProps") {
                    match type_args.first() {
                        // `ComponentProps<"button">`: an intrinsic element's attributes.
                        Some(CollectedType::StringLiteral(tag)) => {
                            let layer = InheritedLayer {
                                type_name: name.to_string(),
                                file_name: resolve_react_types_file(iface_file, ctx),
                                omitted: vec![],
                                html_element: Some(tag.to_lowercase().to_string()),
                                total_props: 0,
                            };
                            return (ResolvedChain::empty(), Some(layer));
                        }
                        // `ComponentProps<typeof Button>`: that component's own props.
                        Some(CollectedType::TypeOf(component)) => {
                            return (component_props_of(component, iface_file, mapping, ctx, state, depth), None);
                        }
                        _ => {}
                    }
                }
                // Other non-HTML builtins (PropsWithChildren, ElementRef, etc.)
                // — resolve through the chain so resolve_known can handle them.
                let chain = resolve_props_chain(name.as_str(), type_args, iface_file, mapping, ctx, state, depth);
                (chain, None)
            }
        }

        ExtendsRef::SameFile { name, type_args } => {
            let chain = resolve_props_chain(name.as_str(), type_args, iface_file, mapping, ctx, state, depth);
            (chain, None)
        }

        ExtendsRef::Imported { local_name, type_args, source_specifier } => {
            // `type_args` were written in `iface_file`, so the lookup keeps that file as its consuming file;
            // `resolve_props_chain` follows the import, including barrel re-exports, from there. Only a name with no
            // import binding at all falls back to resolving the recorded specifier directly.
            let bound = resolve_to_canonical(local_name.as_str(), iface_file, ctx, &mut state.diagnostics).is_some();
            let consuming_file = if bound {
                iface_file.to_owned()
            } else {
                source_specifier
                    .as_deref()
                    .and_then(|spec| resolve_import_specifier(spec, iface_file, ctx, &mut state.diagnostics))
                    .unwrap_or_else(|| iface_file.to_owned())
            };
            let chain =
                resolve_props_chain(local_name.as_str(), type_args, &consuming_file, mapping, ctx, state, depth);
            (chain, None)
        }
    }
}

/// The props of the component bound to `name` as seen from `file` (`typeof Button`), found through the component
/// mappings, with a warning when no detected component has that name.
pub(super) fn component_props_of(
    name: &str,
    file: &Utf8Path,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> ResolvedChain {
    let (canonical_file, canonical_name) = resolve_to_canonical(name, file, ctx, &mut state.diagnostics)
        .unwrap_or_else(|| (file.to_owned(), name.to_owned()));
    let target = ctx
        .global
        .component_mappings
        .iter()
        .find(|m| m.file_path == canonical_file && m.component_name == canonical_name)
        // A dependency's component, from the loaded layer's value index; never emitted itself (ADR 0010).
        .or_else(|| ctx.dependencies.as_ref()?.component(&canonical_file, &canonical_name));
    match target {
        Some(target) => resolve_props_chain(
            target.props_type_name.as_str(),
            &target.props_type_args,
            &target.file_path,
            mapping,
            ctx,
            state,
            depth + 1,
        ),
        // `typeof Primitive.div`: a member of a value whose declared type is a mapped type over its keys.
        None if let Some((props, props_file)) = name
            .rsplit_once('.')
            .and_then(|(base, member)| super::callee::value_member_props(base, member, file, ctx)) =>
        {
            super::alias::resolve_base_as_chain(&props, &props_file, mapping, ctx, state, depth + 1)
        }
        None => {
            state.diagnostics.push(
                Diagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::UnresolvableImport,
                    format!("Cannot find the component '{name}' whose props are referenced from '{file}'"),
                )
                .with_file(file.to_string())
                .with_help("ComponentProps<typeof X> needs X to be a component detected in the project or its installed types."),
            );
            ResolvedChain::empty_with_compose(format!("ComponentProps<typeof {name}>"))
        }
    }
}

/// Full mode only: look up `name` (e.g. "ButtonHTMLAttributes") directly in
/// `GlobalSourceData.interfaces` under `react_file` — the exact path the pipeline
/// merges @types/react's parsed interfaces under when Full mode is on — and
/// resolve it for real if found. Bypasses the normal same-file/import-resolution
/// path entirely: the consuming file very often has no literal `import { X } from
/// 'react'` naming this type at all (`React.ButtonHTMLAttributes<...>` via a
/// namespace import, or no import at all in a `.d.ts`), so there's nothing for
/// that path to resolve — we already know exactly which file these types live in.
/// Returns `None` (not a diagnostic) when @types/react wasn't actually merged;
/// callers fall back to the existing curated/metadata-only behavior. `type_args`, written in `args_file`, bind the
/// interface's element parameter (`DOMAttributes<T>` reached from `SVGProps<SVGSVGElement>`).
#[allow(clippy::too_many_arguments)]
fn real_html_attrs_chain(
    name: &str,
    type_args: &[CollectedType],
    args_file: &Utf8Path,
    react_file: &str,
    mapping: &ComponentMapping,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<ResolvedChain> {
    // @types/react declares all of these inside `declare namespace React { ... }`,
    // so once merged they're stored under the namespace-qualified key regardless
    // of how the consuming file referenced them (`ButtonHTMLAttributes` via a named
    // import, or `React.ButtonHTMLAttributes` via a namespace import) — try the
    // qualified form first since that's the real, common case, falling back to the
    // bare form for resilience.
    let bare_name = name.strip_prefix("React.").unwrap_or(name);
    let react_file_path = Utf8Path::new(react_file);
    let qualified_name = format!("React.{bare_name}");
    let iface = ctx
        .named_types
        .lookup_interface_exact(react_file_path, &qualified_name)
        .or_else(|| ctx.named_types.lookup_interface_exact(react_file_path, bare_name))?;
    Some(resolve_interface_chain(iface, type_args, args_file, mapping, ctx, state, depth + 1))
}
