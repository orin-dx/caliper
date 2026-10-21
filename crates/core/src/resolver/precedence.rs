//! Shared "try the project's own source before falling back to known
//! library-pattern shortcuts" resolution order.
//!
//! `named.rs` documented this order as intentional (source-defined types
//! like `ThemingProps`/`StylesApiProps` must never be silently replaced by
//! an opaque known-pattern shortcut). `chain.rs`'s `extends`-clause path
//! independently reimplemented the same sequence and got it backwards —
//! checking known patterns first. This is now the *only* place either call
//! site may implement that order, so they can't drift apart again.

use camino::{Utf8Path, Utf8PathBuf};

use crate::known::{resolve_known, KnownPatternResult};
use crate::types::*;

use super::import::{lookup_interface, lookup_type_alias, resolve_to_canonical};
use super::ResolutionContext;

/// What `resolve_source_defined_or_known` found, in the order it checked.
pub(super) enum SourceOrKnownMatch<'g> {
    /// A type alias declared in the project's own source (or an already-merged
    /// ambient/library `.d.ts`). `matched_key` is the exact key it was found
    /// under (bare or `React.`-qualified) — callers need it to also look up
    /// `type_alias_params` under the same key.
    TypeAlias { matched_key: String, alias: CollectedTypeAlias },
    /// An interface declared in the project's own source. `named.rs` only
    /// needs to know a match occurred (it returns a bare `Named` either way);
    /// `chain.rs`'s wiring reads the interface itself to expand its props.
    Interface(&'g CollectedInterface),
    /// No source declaration found — a recognized library pattern instead.
    Known(KnownPatternResult),
}

/// Resolve `name` to canonical `(file, name)`, then try — in this fixed
/// order — a type alias, an interface, and only then a known-pattern
/// shortcut. Returns the canonical `(file, name)` pair (callers need it
/// regardless of outcome, e.g. for an "unresolvable" diagnostic when nothing
/// matched) alongside whichever of the three matched, if any.
pub(super) fn resolve_source_defined_or_known<'g>(
    name: &str,
    resolved_args: &[PropType],
    consuming_file: &Utf8Path,
    ctx: &'g ResolutionContext,
    state: &mut ResolveState,
) -> (Utf8PathBuf, String, Option<SourceOrKnownMatch<'g>>) {
    super::deps::record(consuming_file);
    let (canonical_file, canonical_name) = resolve_to_canonical(name, consuming_file, ctx, &mut state.diagnostics)
        .unwrap_or_else(|| (consuming_file.to_owned(), name.to_owned()));

    if let Some((matched_key, alias)) = lookup_type_alias(ctx, canonical_file.as_str(), &canonical_name) {
        return (
            canonical_file,
            canonical_name,
            Some(SourceOrKnownMatch::TypeAlias { matched_key: matched_key.to_string(), alias: alias.clone() }),
        );
    }

    if let Some(iface) = lookup_interface(ctx, canonical_file.as_str(), &canonical_name) {
        return (canonical_file, canonical_name, Some(SourceOrKnownMatch::Interface(iface)));
    }

    if let Some(result) = resolve_known(name, resolved_args, &ctx.global, &ctx.enum_bare_index) {
        return (canonical_file, canonical_name, Some(SourceOrKnownMatch::Known(result)));
    }

    (canonical_file, canonical_name, None)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use camino::Utf8PathBuf;
    use std::collections::BTreeMap;

    use super::*;
    use crate::pipeline::PipelineOptions;

    #[test]
    fn source_defined_interface_wins_over_a_known_pattern_shortcut() {
        // A project that declares its own `interface SxProps` must resolve to
        // that interface, never to the hardcoded MUI SxProps opaque shortcut.
        let file_path = Utf8PathBuf::from("/test/theme.ts");
        let scoped_key = format!("{}:SxProps", file_path);

        let mut global = GlobalSourceData::default();
        global.interfaces.insert(
            scoped_key.clone(),
            CollectedInterface {
                scoped_key: scoped_key.clone(),
                name: "SxProps".into(),
                file_path: file_path.clone(),
                props: vec![],
                extends: vec![],
                description: String::new(),
                tags: BTreeMap::new(),
            },
        );

        let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
        let mut state = ResolveState::default();

        let (_canonical_file, _canonical_name, matched) =
            resolve_source_defined_or_known("SxProps", &[], &file_path, &ctx, &mut state);

        assert!(
            matches!(matched, Some(SourceOrKnownMatch::Interface(_))),
            "expected the project's own SxProps interface to win, got {:?}",
            match &matched {
                Some(SourceOrKnownMatch::Interface(_)) => "Interface",
                Some(SourceOrKnownMatch::TypeAlias { .. }) => "TypeAlias",
                Some(SourceOrKnownMatch::Known(_)) => "Known",
                None => "None",
            }
        );
    }
}
