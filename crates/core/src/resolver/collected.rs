//! Central dispatch: CollectedType → PropType.

use camino::Utf8Path;

use crate::types::*;

use super::func::{resolve_function_type, resolve_typeof};
use super::named::resolve_named;
use super::primitives::{resolve_indexed_access, resolve_intersection, resolve_union};
use super::template::resolve_template_literal;
use super::{ResolutionContext, MAX_DEPTH};

/// Central dispatch: convert a `CollectedType` to a `PropType`.
/// Never re-parses strings — everything is already structured.
#[allow(clippy::too_many_arguments)]
pub fn resolve_collected_type(
    ct: &CollectedType,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    if depth > MAX_DEPTH {
        let diag = super::max_depth_diagnostic(&format!("type '{}'", ct.to_raw_string()), consuming_file);
        return OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::DepthExceeded, diag);
    }

    match ct {
        // ── Primitives ────────────────────────────────────────────────────────
        CollectedType::String => PropType::String,
        CollectedType::Number => PropType::Number,
        CollectedType::Boolean => PropType::Boolean,
        CollectedType::Null => PropType::Null,
        CollectedType::Undefined => PropType::Undefined,
        CollectedType::Any => PropType::Any,
        CollectedType::Never => PropType::Never,
        CollectedType::Unknown => PropType::Unknown,
        CollectedType::Void => PropType::Void,
        // BigInt/Symbol — no dedicated PropType; surface as Named.
        CollectedType::BigInt => PropType::Named { name: "bigint".into(), args: vec![] },
        CollectedType::Symbol => PropType::Named { name: "symbol".into(), args: vec![] },

        // ── Literals ─────────────────────────────────────────────────────────
        CollectedType::StringLiteral(s) => PropType::StringLiteral(s.to_string()),
        CollectedType::NumberLiteral(n) => PropType::NumberLiteral(*n),
        CollectedType::BoolLiteral(b) => PropType::BoolLiteral(*b),

        // ── Composites ───────────────────────────────────────────────────────
        CollectedType::Union(members) => resolve_union(members, consuming_file, ctx, state, depth),
        CollectedType::Intersection(members) => resolve_intersection(members, consuming_file, ctx, state, depth),
        CollectedType::Array(inner) => {
            PropType::Array(Box::new(resolve_collected_type(inner, consuming_file, ctx, state, depth + 1)))
        }
        CollectedType::Tuple(members) => PropType::Tuple(
            members.iter().map(|m| resolve_collected_type(m, consuming_file, ctx, state, depth + 1)).collect(),
        ),
        CollectedType::Object(fields) => PropType::Object(
            fields
                .iter()
                .map(|f| ObjectField {
                    name: f.name.clone(),
                    prop_type: resolve_collected_type(&f.collected_type, consuming_file, ctx, state, depth + 1),
                    required: f.required,
                    description: f.description.clone(),
                })
                .collect(),
        ),

        // ── Named type reference ──────────────────────────────────────────────
        CollectedType::Named { name, args } => resolve_named(name, args, consuming_file, ctx, state, depth),

        // ── typeof X ─────────────────────────────────────────────────────────
        CollectedType::TypeOf(name) => resolve_typeof(name, consuming_file, ctx, &mut state.diagnostics),

        // ── keyof X — only meaningful as Omit's key argument (handled structurally
        // there, see resolver/alias.rs); standalone usage needs a general
        // type-to-key-names resolver we don't have, so degrade gracefully.
        CollectedType::KeyOf(_) => {
            let diag = opaque_diagnostic("a standalone 'keyof'", ct, consuming_file);
            OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::MappedType, diag)
        }

        // ── Generic-alias substitution marker — switch file context to where
        // `inner` was actually written (see the `CollectedType::AtFile` doc
        // comment) and continue resolving from there.
        CollectedType::AtFile { file, inner } => resolve_collected_type(inner, file, ctx, state, depth),

        // ── Indexed access ───────────────────────────────────────────────────
        CollectedType::IndexedAccess { obj, key } => {
            resolve_indexed_access(obj, key, consuming_file, ctx, state, depth)
        }

        // ── Template literal ─────────────────────────────────────────────────
        CollectedType::TemplateLiteral(parts) => resolve_template_literal(parts, consuming_file, ctx, state, depth),

        // ── Function type ─────────────────────────────────────────────────────
        CollectedType::Function { params, param_names, return_type } => {
            resolve_function_type(params, param_names, return_type, consuming_file, ctx, state, depth)
        }

        // ── Opaque (needs type checker) ───────────────────────────────────────
        CollectedType::Conditional { .. } => {
            let diag = opaque_diagnostic("a conditional type", ct, consuming_file);
            OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::ConditionalType, diag)
        }
        CollectedType::Mapped { .. } => {
            let diag = opaque_diagnostic("a mapped type", ct, consuming_file);
            OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::MappedType, diag)
        }

        // ── Raw fallback ─────────────────────────────────────────────────────
        CollectedType::Raw(s) => {
            let trimmed = s.trim();
            // `typeof X` — ExtendsRef.type_args serialized form; resolve as Named.
            if let Some(name) = trimmed.strip_prefix("typeof ") {
                return PropType::Named { name: name.trim().into(), args: vec![] };
            }
            // Double-quoted string literal: `"button"` → StringLiteral.
            if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
                return PropType::StringLiteral(trimmed[1..trimmed.len() - 1].to_owned());
            }
            // Single-quoted string literal: `'button'` → StringLiteral.
            if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
                return PropType::StringLiteral(trimmed[1..trimmed.len() - 1].to_owned());
            }
            // TypeScript primitive keywords — tuple elements come through Raw because
            // TSTupleElement falls through to source-text extraction for non-optional,
            // non-rest elements. Recognise keywords here so they emit the proper PropType.
            match trimmed {
                "string" => return PropType::String,
                "number" => return PropType::Number,
                "boolean" => return PropType::Boolean,
                "null" => return PropType::Null,
                "undefined" => return PropType::Undefined,
                "never" => return PropType::Never,
                "void" => return PropType::Void,
                "any" => return PropType::Any,
                "unknown" => return PropType::Unknown,
                _ => {}
            }
            // Simple identifier → Named type reference.
            if !trimmed.is_empty()
                && !trimmed.contains(' ')
                && !trimmed.contains('|')
                && !trimmed.contains('&')
                && !trimmed.contains('<')
            {
                PropType::Named { name: trimmed.into(), args: vec![] }
            } else if let Some(reference) = parse_generic_reference(trimmed) {
                resolve_collected_type(&reference, consuming_file, ctx, state, depth + 1)
            } else {
                let diag = opaque_diagnostic("an unparsable raw type expression", ct, consuming_file);
                OpaqueDetail::give_up(state, s.clone(), OpaqueReason::UnsupportedExpression, diag)
            }
        }
    }
}

/// Build the diagnostic explaining why a `CollectedType` degrades to
/// `PropType::Opaque` — expanding it needs the TypeScript type checker (or,
/// for the Raw fallback, needs the extractor to understand a syntax shape it
/// doesn't yet). Pure builder — the caller passes the result to
/// `OpaqueDetail::give_up`, which pushes it.
fn opaque_diagnostic(what: &str, ct: &CollectedType, file: &Utf8Path) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!(
            "'{}' is {} and can't be statically resolved — it will appear as opaque",
            ct.to_raw_string(),
            what
        ),
        file: Some(file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
    }
}

/// Split `Name<A, B<C>>` into a `Named` reference with `Raw` arguments, or `None` when `text` is not exactly one
/// generic reference. Type arguments reach the resolver as display strings (roadmap item 2.1 replaces that); this keeps
/// a nested generic from degrading to an opaque type in the meantime.
pub(super) fn parse_generic_reference(text: &str) -> Option<CollectedType> {
    let text = text.trim();
    let open = text.find('<')?;
    let name = text[..open].trim();
    let inner = text[open + 1..].strip_suffix('>')?;
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.')) {
        return None;
    }
    let mut args = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None, 0usize);
    for (i, c) in inner.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '<' | '(' | '[' | '{') => depth += 1,
            (None, '>' | ')' | ']' | '}') => depth -= 1,
            (None, ',') if depth == 0 => {
                args.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        if depth < 0 {
            return None;
        }
    }
    if depth != 0 || quote.is_some() {
        return None;
    }
    args.push(inner[start..].trim());
    if args.iter().any(|a| a.is_empty()) {
        return None;
    }
    Some(CollectedType::Named {
        name: name.into(),
        args: args.into_iter().map(|a| CollectedType::Raw(a.to_owned())).collect(),
    })
}
