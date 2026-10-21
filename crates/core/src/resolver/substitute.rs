//! Structural substitution for user-defined generic type aliases.
//!
//! `type Assign<T, U> = Omit<T, keyof U> & U` declares type parameters `T`/`U`
//! that are opaque names until a call site supplies concrete arguments, e.g.
//! `Assign<HTMLProps<'div'>, SelectRootBaseProps<T>>`. Previously the resolver
//! looked up `Assign` by name and resolved its body directly — `T`/`U` were
//! never replaced with the caller's arguments, so `Omit<T, keyof U>` resolved
//! against literal type names `"T"`/`"U"` (unresolvable) instead of the real
//! types, and the whole component's props came back empty.
//!
//! This module walks the alias body's `CollectedType`/`CollectedTypeAlias` tree
//! and replaces every bare `Named` reference to a declared parameter with the
//! caller's argument, before normal resolution proceeds. Purely structural —
//! no type inference, no conditional-type evaluation.

use camino::Utf8Path;
use rustc_hash::FxHashMap;

use crate::types::{
    CollectedObjectField, CollectedType, CollectedTypeAlias, Diagnostic, DiagnosticCode, DiagnosticSeverity,
};

use super::import::resolve_to_canonical;
use super::ResolutionContext;

/// Map from a generic alias's declared parameter name to the caller-supplied
/// argument, each pre-wrapped in `CollectedType::AtFile` pinned to the file the
/// argument was actually written in (see that variant's doc comment) — the
/// callee alias being substituted into may live in a different file entirely.
pub(super) type Substitution<'a> = FxHashMap<&'a str, CollectedType>;

/// Build a `Substitution` from declared parameter names and the caller's
/// arguments, tagging each argument with `origin_file` — the file the *caller*
/// wrote them in, which is where any further name lookups on them must happen.
///
/// If `params` declares more type parameters than `args` supplies (a call site
/// under-applying a generic alias, e.g. `Foo<string>` for `type Foo<T, U> = ...`),
/// the trailing unfilled parameters are silently dropped from the substitution —
/// pushes a `Warning` diagnostic naming them so this doesn't degrade silently.
pub(super) fn build_substitution<'a>(
    params: &'a [compact_str::CompactString],
    args: &[CollectedType],
    origin_file: &Utf8Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Substitution<'a> {
    if params.len() > args.len() {
        let unfilled: Vec<&str> = params[args.len()..].iter().map(|p| p.as_str()).collect();
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "Generic alias in '{}' declares {} type parameter(s) but only {} were supplied — '{}' left unsubstituted",
                origin_file,
                params.len(),
                args.len(),
                unfilled.join("', '")
            ),
            file: Some(origin_file.to_string()),
            line: None,
            column: None,
            help: Some("Check the call site supplies a type argument for every declared type parameter.".into()),
            code: DiagnosticCode::GenericArgumentMismatch,
        });
    }
    params
        .iter()
        .map(|p| p.as_str())
        .zip(args.iter().map(|a| CollectedType::AtFile { file: origin_file.to_owned(), inner: Box::new(a.clone()) }))
        .collect()
}

/// If `scoped_key` names a generic alias (type parameters were recorded during
/// extraction — see `extractor/visit.rs`) and the call site supplied concrete
/// `type_args`, substitute the parameters into the alias body. Non-generic
/// aliases (the common case — no `type_alias_params` entry) pass through
/// unchanged, so this is a strict no-op for all existing behavior.
///
/// `type_args` arrive as display strings here (the chain-level resolution path
/// — see `resolve_props_chain`'s `type_args: &[String]`), so they're recovered
/// into `CollectedType` on a best-effort basis via `raw_arg_to_collected_type`.
/// When a call site's own argument is itself structured (e.g. a `Named` type's
/// `args: Vec<CollectedType>`), prefer `generic_alias_with_structured_args`
/// instead — it skips this string round-trip entirely.
pub(super) fn apply_generic_args(
    alias: CollectedTypeAlias,
    declared_at: (&Utf8Path, &str),
    type_args: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> CollectedTypeAlias {
    let (declared_file, declared_name) = declared_at;
    let Some(params) = ctx.named_types.lookup_type_alias_params(declared_file, declared_name) else {
        return alias;
    };
    if params.is_empty() || type_args.is_empty() {
        return alias;
    }
    let subst = build_substitution(params, type_args, consuming_file, diagnostics);
    substitute_alias(&alias, &subst)
}

/// Same substitution as `apply_generic_args`, but for a `Named` type reference
/// whose arguments are already structured `CollectedType`s — e.g.
/// `SelectRootBaseProps<T>` used as an argument to another generic alias
/// (`Assign<HTMLProps<'div'>, SelectRootBaseProps<T>>`). Resolves `name` to its
/// canonical declaration first (it may live in a different file), so this
/// covers the cross-file case too. Returns `None` when `name` isn't a generic
/// alias (no recorded type parameters) or has no arguments — the caller then
/// falls back to its ordinary string-based resolution path unchanged.
pub(super) fn generic_alias_with_structured_args(
    name: &str,
    args: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<CollectedTypeAlias> {
    if args.is_empty() {
        return None;
    }
    let (canonical_file, canonical_name) = resolve_to_canonical(name, consuming_file, ctx, diagnostics)
        .unwrap_or_else(|| (consuming_file.to_owned(), name.to_owned()));

    let params = ctx.named_types.lookup_type_alias_params(&canonical_file, &canonical_name)?;
    if params.is_empty() {
        return None;
    }
    let alias = ctx.named_types.lookup_type_alias_exact(&canonical_file, &canonical_name)?;
    // `args` were written wherever `name` (the reference being resolved) appears,
    // i.e. `consuming_file` — not `canonical_file` (where the generic alias itself
    // is declared).
    let subst = build_substitution(params, args, consuming_file, diagnostics);
    Some(substitute_alias(alias, &subst))
}

/// Recursively replace parameter references inside a `CollectedType`.
pub(super) fn substitute_type(ct: &CollectedType, subst: &Substitution) -> CollectedType {
    match ct {
        CollectedType::Named { name, args } if args.is_empty() => match subst.get(name.as_str()) {
            Some(replacement) => replacement.clone(),
            None => ct.clone(),
        },
        CollectedType::Named { name, args } => {
            CollectedType::Named { name: name.clone(), args: args.iter().map(|a| substitute_type(a, subst)).collect() }
        }
        CollectedType::Union(members) => {
            CollectedType::Union(members.iter().map(|m| substitute_type(m, subst)).collect())
        }
        CollectedType::Intersection(members) => {
            CollectedType::Intersection(members.iter().map(|m| substitute_type(m, subst)).collect())
        }
        CollectedType::Array(inner) => CollectedType::Array(Box::new(substitute_type(inner, subst))),
        CollectedType::Tuple(members) => {
            CollectedType::Tuple(members.iter().map(|m| substitute_type(m, subst)).collect())
        }
        CollectedType::Object(fields) => CollectedType::Object(
            fields
                .iter()
                .map(|f| CollectedObjectField {
                    name: f.name.clone(),
                    collected_type: substitute_type(&f.collected_type, subst),
                    required: f.required,
                    description: f.description.clone(),
                })
                .collect(),
        ),
        CollectedType::KeyOf(inner) => CollectedType::KeyOf(Box::new(substitute_type(inner, subst))),
        // Already resolved to a caller-supplied argument by an earlier substitution
        // pass — `file` is authoritative for `inner` and must not be disturbed, but
        // `inner` itself may still contain further params if it came from an outer
        // generic scope, so keep recursing.
        CollectedType::AtFile { file, inner } => {
            CollectedType::AtFile { file: file.clone(), inner: Box::new(substitute_type(inner, subst)) }
        }
        CollectedType::IndexedAccess { obj, key } => CollectedType::IndexedAccess {
            obj: Box::new(substitute_type(obj, subst)),
            key: Box::new(substitute_type(key, subst)),
        },
        CollectedType::TemplateLiteral(parts) => {
            CollectedType::TemplateLiteral(parts.iter().map(|p| substitute_type(p, subst)).collect())
        }
        CollectedType::Function { params, param_names, return_type } => CollectedType::Function {
            params: params.iter().map(|p| substitute_type(p, subst)).collect(),
            param_names: param_names.clone(),
            return_type: Box::new(substitute_type(return_type, subst)),
        },
        CollectedType::Conditional { check, extends_type, true_type, false_type } => CollectedType::Conditional {
            check: Box::new(substitute_type(check, subst)),
            extends_type: Box::new(substitute_type(extends_type, subst)),
            true_type: Box::new(substitute_type(true_type, subst)),
            false_type: Box::new(substitute_type(false_type, subst)),
        },
        CollectedType::Mapped { key_type, value_type } => CollectedType::Mapped {
            key_type: Box::new(substitute_type(key_type, subst)),
            value_type: Box::new(substitute_type(value_type, subst)),
        },
        // Leaves that can't structurally reference a type parameter.
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
        | CollectedType::TypeOf(_)
        | CollectedType::Raw(_) => ct.clone(),
    }
}

/// Recursively replace parameter references inside a `CollectedTypeAlias` body.
fn substitute_alias(alias: &CollectedTypeAlias, subst: &Substitution) -> CollectedTypeAlias {
    match alias {
        CollectedTypeAlias::Omit { base, keys, file_path } => CollectedTypeAlias::Omit {
            base: substitute_type(base, subst),
            keys: substitute_type(keys, subst),
            file_path: file_path.clone(),
        },
        CollectedTypeAlias::Pick { base, keys, file_path } => CollectedTypeAlias::Pick {
            base: substitute_type(base, subst),
            keys: substitute_type(keys, subst),
            file_path: file_path.clone(),
        },
        CollectedTypeAlias::Partial { base, file_path } => {
            CollectedTypeAlias::Partial { base: substitute_type(base, subst), file_path: file_path.clone() }
        }
        CollectedTypeAlias::Required { base, file_path } => {
            CollectedTypeAlias::Required { base: substitute_type(base, subst), file_path: file_path.clone() }
        }
        CollectedTypeAlias::Union { members, file_path } => CollectedTypeAlias::Union {
            members: members.iter().map(|m| substitute_type(m, subst)).collect(),
            file_path: file_path.clone(),
        },
        CollectedTypeAlias::Intersection { members, file_path } => CollectedTypeAlias::Intersection {
            members: members.iter().map(|m| substitute_type(m, subst)).collect(),
            file_path: file_path.clone(),
        },
        CollectedTypeAlias::LiteralUnion { members, file_path } => {
            CollectedTypeAlias::LiteralUnion { members: members.clone(), file_path: file_path.clone() }
        }
        CollectedTypeAlias::Passthrough { target, file_path } => {
            CollectedTypeAlias::Passthrough { target: substitute_type(target, subst), file_path: file_path.clone() }
        }
    }
}

/// Synthesize an inline `CollectedTypeAlias` for `Omit`/`Pick`/`Partial`/`Readonly`
/// when one of them appears as a `Named` reference with *structured* type
/// arguments — e.g. as a member of `Omit<T, keyof U> & U` after `T`/`U` have been
/// substituted with real types. This mirrors `chain.rs`'s step-0.5 handling of the
/// same utility types in `extends` position, but operates on already-structured
/// `CollectedType` args (so nested generics and `keyof` survive) instead of
/// re-parsing display strings.
pub(super) fn synthesize_utility_alias(
    name: &str,
    args: &[CollectedType],
    file_path: &Utf8Path,
) -> Option<CollectedTypeAlias> {
    match name {
        "Omit" if args.len() >= 2 => Some(CollectedTypeAlias::Omit {
            base: args[0].clone(),
            keys: args[1].clone(),
            file_path: file_path.to_owned(),
        }),
        "Pick" if args.len() >= 2 => Some(CollectedTypeAlias::Pick {
            base: args[0].clone(),
            keys: args[1].clone(),
            file_path: file_path.to_owned(),
        }),
        "Partial" if !args.is_empty() => {
            Some(CollectedTypeAlias::Partial { base: args[0].clone(), file_path: file_path.to_owned() })
        }
        "Required" if !args.is_empty() => {
            Some(CollectedTypeAlias::Required { base: args[0].clone(), file_path: file_path.to_owned() })
        }
        "Readonly" | "PropsWithChildren" | "PropsWithRef" if !args.is_empty() => {
            Some(CollectedTypeAlias::Passthrough { target: args[0].clone(), file_path: file_path.to_owned() })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DiagnosticSeverity;
    use compact_str::CompactString;

    #[test]
    fn build_substitution_diagnoses_unfilled_trailing_type_params() {
        // `type Foo<T, U> = { a: T; b: U }` called as `Foo<string>` — `U` never
        // supplied, so it's left as a bare `Named` reference with no diagnostic
        // today. This should now warn.
        let params: Vec<CompactString> = vec![CompactString::from("T"), CompactString::from("U")];
        let args = vec![CollectedType::String];
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        let subst = build_substitution(&params, &args, Utf8Path::new("src/foo.ts"), &mut diagnostics);

        // "contains no entry for any unfilled parameter" is the criterion's exact
        // claim — asserting only len()==1 would equally pass a map keyed {"U": ..}
        // with T silently dropped instead. Assert both directly.
        assert!(subst.contains_key("T"), "T should have been substituted");
        assert!(!subst.contains_key("U"), "U must not appear in the substitution map — it was never supplied");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(diagnostics[0].code, DiagnosticCode::GenericArgumentMismatch);
        assert!(
            diagnostics[0].message.contains("'U'"),
            "message should name the unfilled param by its quoted name: {}",
            diagnostics[0].message
        );
    }

    #[test]
    fn build_substitution_is_silent_when_all_params_are_supplied() {
        let params: Vec<CompactString> = vec![CompactString::from("T")];
        let args = vec![CollectedType::String];
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        let subst = build_substitution(&params, &args, Utf8Path::new("src/foo.ts"), &mut diagnostics);

        assert_eq!(subst.len(), 1);
        assert!(diagnostics.is_empty());
    }
}
