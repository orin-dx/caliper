//! Import and canonical resolution.

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use rustc_hash::FxHashSet;

use crate::import_map::ReExportStep;
use crate::types::*;

use super::ResolutionContext;

/// Re-export chains in real code are shallow (one or two barrel hops); this
/// only guards against a pathological or cyclic `export * from` graph.
const MAX_REEXPORT_DEPTH: u8 = 8;

/// Counts real invocations of `follow_reexports`, so tests can assert the
/// visited-set is actually collapsing shared-descendant re-exploration
/// (linear in graph size) instead of just checking wall-clock time — the
/// per-call work here is cheap enough, and oxc_resolver's own path cache
/// absorbing enough of the redundant filesystem cost, that wall-clock alone
/// doesn't reliably distinguish "linear" from "branching_factor^depth". Only calls for files under the scoped directory
/// count, so resolution in tests running in parallel does not inflate the number.
#[cfg(test)]
static FOLLOW_REEXPORTS_CALLS: std::sync::Mutex<Option<(Utf8PathBuf, usize)>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn reset_follow_reexports_call_count(scope: &Utf8Path) {
    *FOLLOW_REEXPORTS_CALLS.lock().unwrap_or_else(|p| p.into_inner()) = Some((scope.to_owned(), 0));
}

#[cfg(test)]
pub(crate) fn follow_reexports_call_count() -> usize {
    FOLLOW_REEXPORTS_CALLS.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map_or(0, |(_, n)| *n)
}

/// Resolve `name` to its canonical `(file_path, name)` pair.
/// Returns `None` if `name` is a local declaration (not imported).
pub(super) fn resolve_to_canonical(
    name: &str,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<(Utf8PathBuf, String)> {
    if let Some(import_ref) = ctx.import_map.find_import(consuming_file, name) {
        let resolved_file = resolve_import_specifier(&import_ref.specifier, consuming_file, ctx, diagnostics)?;
        let canonical_name = import_ref.exported_name.to_string();
        let mut visited = FxHashSet::default();
        return Some(follow_reexports(resolved_file, canonical_name, ctx, diagnostics, &mut visited, 0));
    }

    // Namespace-import member access: `import * as React from "react"` then a
    // reference to `React.DependencyList`. `find_import` only tracks the
    // namespace binding itself ("React", with `exported_name == "*"`) — a
    // literal dotted name like "React.DependencyList" is never itself a
    // binding, so the lookup above always misses it. Split off the namespace
    // prefix and route the member name through the namespace's own import
    // specifier instead.
    let (namespace, member) = name.split_once('.')?;
    let namespace_ref = ctx.import_map.find_import(consuming_file, namespace)?;
    if namespace_ref.exported_name != "*" {
        return None;
    }
    let resolved_file = resolve_import_specifier(&namespace_ref.specifier, consuming_file, ctx, diagnostics)?;
    let mut visited = FxHashSet::default();
    Some(follow_reexports(resolved_file, member.to_string(), ctx, diagnostics, &mut visited, 0))
}

/// A directly-imported name may land on a barrel file (`export { X } from
/// './x'` or `export * from './x'`) that doesn't declare `name` itself —
/// ubiquitous in real component libraries' `index.ts` files. Follow the
/// re-export chain until we reach a file that actually declares `name`, or
/// run out of chain to follow — in which case the original (barrel_file,
/// name) is returned unchanged, so the caller's existing "cannot resolve"
/// diagnostic still fires exactly as it did before this existed.
fn follow_reexports(
    file: Utf8PathBuf,
    name: String,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
    visited: &mut FxHashSet<(Utf8PathBuf, CompactString)>,
    depth: u8,
) -> (Utf8PathBuf, String) {
    #[cfg(test)]
    if let Some((scope, count)) = FOLLOW_REEXPORTS_CALLS.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        if file.starts_with(&*scope) {
            *count += 1;
        }
    }

    super::deps::record(&file);
    if depth >= MAX_REEXPORT_DEPTH || ctx.named_types.is_declared_at(&file, &name) {
        return (file, name);
    }
    // Real barrels commonly share descendants (several sub-barrels wildcarding
    // the same underlying `types.ts`, say) — without this, re-exploring an
    // already-visited (file, name) pair from a different parent multiplies out
    // to branching_factor^depth calls instead of one call per graph node. This
    // also subsumes cycle detection: a cycle revisits a pair by definition.
    if !visited.insert((file.clone(), CompactString::from(name.as_str()))) {
        return (file, name);
    }
    match ctx.import_map.resolve_reexport_chain(&file, &name, &ctx.global) {
        Some(ReExportStep::Named { source_specifier, source_name }) => {
            let Some(next_file) = resolve_import_specifier(&source_specifier, &file, ctx, diagnostics) else {
                return (file, name);
            };
            follow_reexports(next_file, source_name, ctx, diagnostics, visited, depth + 1)
        }
        Some(ReExportStep::Wildcards(specifiers)) => {
            for specifier in &specifiers {
                let Some(candidate_file) = resolve_import_specifier(specifier, &file, ctx, diagnostics) else {
                    continue;
                };
                let resolved = follow_reexports(candidate_file, name.clone(), ctx, diagnostics, visited, depth + 1);
                if ctx.named_types.is_declared_at(&resolved.0, &resolved.1) {
                    return resolved;
                }
            }
            (file, name)
        }
        None => (file, name),
    }
}

/// Look up an interface by canonical `(file, name)`, falling back to a
/// `React.`-namespace-qualified key. `@types/react` declares everything
/// inside `declare namespace React { ... }`, so real declarations are keyed
/// "React.HTMLAttributes" even though nothing imports them that way — a plain
/// `import type { HTMLAttributes } from "react"` binds the bare name.
pub(super) fn lookup_interface<'g>(
    ctx: &'g ResolutionContext,
    canonical_file: &str,
    canonical_name: &str,
) -> Option<&'g CollectedInterface> {
    super::deps::record(Utf8Path::new(canonical_file));
    ctx.named_types.lookup_interface(Utf8Path::new(canonical_file), canonical_name)
}

/// Same as `lookup_interface`, for type aliases. Returns the name that
/// actually matched (bare or `React.`-qualified) alongside the value, since
/// callers also need it to look up `type_alias_params` under the same
/// `(file, name)`.
pub(super) fn lookup_type_alias<'g>(
    ctx: &'g ResolutionContext,
    canonical_file: &str,
    canonical_name: &str,
) -> Option<(CompactString, &'g CollectedTypeAlias)> {
    super::deps::record(Utf8Path::new(canonical_file));
    ctx.named_types.lookup_type_alias(Utf8Path::new(canonical_file), canonical_name)
}

/// Same as `lookup_interface`, but also falls back to TypeScript's own ambient
/// lib files (`ctx.ambient_global_files`) when the name isn't found via
/// import/same-file resolution — e.g. `HTMLDivElement`, declared ambiently in
/// `lib.dom.d.ts`, is never imported, so `resolve_to_canonical` never resolves
/// it to that file and `canonical_file` stays the consuming file instead.
pub(super) fn lookup_interface_including_ambient<'g>(
    ctx: &'g ResolutionContext,
    canonical_file: &str,
    canonical_name: &str,
) -> Option<&'g CollectedInterface> {
    super::deps::record(Utf8Path::new(canonical_file));
    ctx.named_types.lookup_interface_including_ambient(
        Utf8Path::new(canonical_file),
        canonical_name,
        &ctx.ambient_global_files,
    )
}

/// Look up a name directly on TypeScript's own ambient lib files
/// (`ctx.ambient_global_files` — `lib.es5.d.ts`/`lib.dom.d.ts`), by bare name
/// only (these are ambient globals, never namespace-qualified). Deliberately
/// separate from — and only ever consulted AFTER — every other resolution
/// path (imports, same-file declarations, the well-known-TS-utility-type
/// silent list): `Record`/`Partial`/etc. are themselves declared here too
/// (`Record<K, T> = { [P in K]: T }`, a mapped type this tool can't expand),
/// so checking this first would intercept those already-handled names and
/// degrade them to Opaque instead of leaving them to the existing shortcuts.
pub(super) fn lookup_ambient_global<'g>(ctx: &'g ResolutionContext, name: &str) -> Option<AmbientGlobalLookup<'g>> {
    match ctx.named_types.lookup_ambient(&ctx.ambient_global_files, name)? {
        crate::named_type_index::AmbientMatch::Interface => Some(AmbientGlobalLookup::Interface),
        crate::named_type_index::AmbientMatch::TypeAlias(alias) => Some(AmbientGlobalLookup::TypeAlias(alias)),
    }
}

pub(super) enum AmbientGlobalLookup<'g> {
    Interface,
    TypeAlias(&'g CollectedTypeAlias),
}

/// Use `oxc_resolver` to turn an import specifier into an absolute file path.
pub(super) fn resolve_import_specifier(
    specifier: &str,
    from_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Utf8PathBuf> {
    super::deps::record_module_resolution();
    let from_dir = from_file.parent()?;

    // Bare package specifiers (not relative/absolute) need real TypeScript
    // declaration resolution: prefer a real .d.ts over whatever JS entry point
    // the package's own "main"/"exports" happens to point to, and fall back to
    // the separate `@types/<package>` package when the package ships no types
    // of its own at all — e.g. "react": its package.json has no "types" field
    // and no "types" export condition, so a plain `resolve()` below lands on
    // `index.js`, and a named type-only import like `HTMLAttributes` can never
    // be found there.
    if !specifier.starts_with('.') && !specifier.starts_with('/') {
        if let Some(path) = super::react::resolve_package_types_file(&ctx.oxc_resolver, from_dir, specifier) {
            return Utf8PathBuf::from_path_buf(std::path::PathBuf::from(path)).ok();
        }
    }

    match ctx.oxc_resolver.resolve(from_dir.as_std_path(), specifier) {
        Ok(resolved) => {
            let resolved = Utf8PathBuf::from_path_buf(resolved.path().to_owned()).ok()?;
            super::deps::record(&resolved);
            Some(resolved)
        }
        Err(e) => {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("Cannot resolve '{}' from '{}'", specifier, from_file),
                file: Some(from_file.to_string()),
                line: None,
                column: None,
                help: Some(format!("Resolution error: {}", e)),
                code: DiagnosticCode::UnresolvableImport,
                affects: Vec::new(),
            });
            None
        }
    }
}
