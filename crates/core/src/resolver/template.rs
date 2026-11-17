//! Template literal type expansion.

use camino::Utf8Path;

use crate::types::*;

use super::ResolutionContext;
use super::collected::resolve_collected_type;
use super::import::resolve_to_canonical;

pub(super) fn resolve_template_literal(
    parts: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // Try to expand: `compact-${Size}` where Size = "xs"|"sm"|...
    // Each part must be either a string literal or a type that resolves to a LiteralUnion.
    let expanded = try_expand_template_literal(parts, consuming_file, ctx, state, depth);

    if let Some(values) = expanded {
        if let [only] = values.as_slice() {
            return PropType::StringLiteral(only.clone());
        }
        return PropType::LiteralUnion { members: values, has_default: false };
    }

    // A pattern such as `${number}px`: kept as one. It is the exact type, but it lists no values. Its holes stay as
    // written, names and all, as a function signature's types do: resolving them spelled `HTMLInputAutoCompleteAttribute`
    // out as a cross product of every token union.
    let raw = CollectedType::TemplateLiteral(parts.to_vec()).to_raw_string();
    state.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("Template literal type '{}' could not be statically expanded", raw),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some("Declare the prop as an explicit string-literal union to document its values.".into()),
        code: DiagnosticCode::TemplateLiteralOpaque,
        affects: Vec::new(),
    });
    template_of(parts, super::func::as_written)
}

/// A template literal's parts as a pattern: its text pieces, and each hole's type read by `read`.
pub(super) fn template_of(parts: &[CollectedType], mut read: impl FnMut(&CollectedType) -> PropType) -> PropType {
    PropType::TemplateLiteral(
        parts
            .iter()
            .map(|part| match part {
                CollectedType::StringLiteral(text) => TemplatePart::Text(text.to_string()),
                hole => TemplatePart::Type(read(hole)),
            })
            .collect(),
    )
}

/// TypeScript's intrinsic string mappings, `Capitalize<'click'>` is `'Click'`, applied to one value.
fn string_mapping(name: &str, value: &str) -> Option<String> {
    let mut chars = value.chars();
    match name {
        "Uppercase" => Some(value.to_uppercase()),
        "Lowercase" => Some(value.to_lowercase()),
        "Capitalize" => Some(chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()),
        "Uncapitalize" => {
            Some(chars.next().map(|first| first.to_lowercase().chain(chars).collect()).unwrap_or_default())
        }
        _ => None,
    }
}

/// Cap on the Cartesian product `try_expand_template_literal` accumulates
/// across a template literal's parts. Each part can itself be a union with
/// many members, so the product is exponential in part count — an attacker-
/// or generator-controlled type with enough parts/members would otherwise
/// allocate unboundedly before ever returning. Same shape of guard as
/// `extractor/mod.rs`'s `MAX_SOURCE_NESTING_DEPTH`, applied to a different
/// input-derived size.
const MAX_TEMPLATE_LITERAL_EXPANSIONS: usize = 4096;

/// Try to fully expand a template literal into a list of concrete string values.
/// Returns `None` if any part cannot be resolved to string literals.
pub(super) fn try_expand_template_literal(
    parts: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<Vec<String>> {
    // Collect per-part string alternatives.
    let mut per_part: Vec<Vec<String>> = Vec::new();

    for part in parts {
        match part {
            CollectedType::StringLiteral(s) => {
                per_part.push(vec![s.to_string()]);
            }
            CollectedType::NumberLiteral(n) => per_part.push(vec![n.to_string()]),
            CollectedType::BoolLiteral(b) => per_part.push(vec![b.to_string()]),
            // `${Capitalize<Ev>}` over a literal union `Ev`: each value mapped.
            CollectedType::Named { name, args }
                if let [arg] = args.as_slice()
                    && matches!(name.as_str(), "Uppercase" | "Lowercase" | "Capitalize" | "Uncapitalize")
                    && super::may_be_ts_lib_type(name, consuming_file, ctx) =>
            {
                let values =
                    try_expand_template_literal(std::slice::from_ref(arg), consuming_file, ctx, state, depth + 1)?;
                per_part.push(values.iter().map(|value| string_mapping(name, value)).collect::<Option<_>>()?);
            }
            CollectedType::Named { name, .. } => {
                // Look up in global type aliases for a LiteralUnion.
                // `None`: this template literal has no finite string expansion.
                per_part.push(resolve_named_to_string_literals(name.as_str(), consuming_file, ctx, state, depth + 1)?);
            }
            _ => {
                let pt = resolve_collected_type(part, consuming_file, ctx, state, depth + 1);
                // A literal's text, as `${1}` and `${true}` spell it.
                let text = |m: &PropType| match m {
                    PropType::StringLiteral(s) => Some(s.clone()),
                    PropType::NumberLiteral(n) => Some(n.to_string()),
                    PropType::BoolLiteral(b) => Some(b.to_string()),
                    _ => None,
                };
                match &pt {
                    PropType::LiteralUnion { members, .. } => per_part.push(members.clone()),
                    PropType::Union(members) => per_part.push(members.iter().map(text).collect::<Option<_>>()?),
                    other => per_part.push(vec![text(other)?]),
                }
            }
        }
    }

    if per_part.is_empty() {
        return Some(vec![String::new()]);
    }

    // Cartesian product across all parts.
    let mut result = vec![String::new()];
    for alternatives in per_part {
        if result.len().saturating_mul(alternatives.len()) > MAX_TEMPLATE_LITERAL_EXPANSIONS {
            return None; // Falls through to the Opaque+diagnostic degrade path in resolve_template_literal.
        }
        let mut next = Vec::with_capacity(result.len() * alternatives.len());
        for prefix in &result {
            for alt in &alternatives {
                next.push(format!("{}{}", prefix, alt));
            }
        }
        result = next;
    }

    Some(result)
}

pub(super) fn resolve_named_to_string_literals(
    name: &str,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> Option<Vec<String>> {
    let (canonical_file, canonical_name) = resolve_to_canonical(name, consuming_file, ctx, &mut state.diagnostics)
        .unwrap_or_else(|| (consuming_file.to_owned(), name.to_owned()));

    let scoped_key = format!("{}:{}", canonical_file, canonical_name);

    super::deps::record(&canonical_file);
    if let Some(alias) = ctx.global.type_aliases.get(&scoped_key) {
        match alias {
            CollectedTypeAlias::LiteralUnion { members, .. } => {
                return Some(members.clone());
            }
            CollectedTypeAlias::Union { members, .. } => {
                let strs: Option<Vec<String>> = members
                    .iter()
                    .map(|m| if let CollectedType::StringLiteral(s) = m { Some(s.to_string()) } else { None })
                    .collect();
                return strs;
            }
            _ => {}
        }
    }

    // Try resolving via resolve_collected_type and extracting literals.
    let ct = CollectedType::Named { name: name.into(), args: vec![] };
    let pt = resolve_collected_type(&ct, consuming_file, ctx, state, depth);
    match pt {
        PropType::StringLiteral(s) => Some(vec![s]),
        PropType::LiteralUnion { members, .. } => Some(members),
        PropType::Union(members) => {
            members.into_iter().map(|m| if let PropType::StringLiteral(s) = m { Some(s) } else { None }).collect()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use camino::Utf8PathBuf;

    use crate::types::options::PipelineOptions;

    fn empty_ctx() -> ResolutionContext {
        let global = Arc::new(GlobalSourceData::default());
        let options = PipelineOptions::default();
        ResolutionContext::new(global, &options)
    }

    /// 5 parts x 10-member unions = 10^5 = 100,000 combinations — without a
    /// cap this builds a 100k-entry `Vec<String>` (and grows exponentially
    /// with more parts/members, the unbounded-allocation shape from
    /// P1-1/P1-2). `MAX_TEMPLATE_LITERAL_EXPANSIONS` (4096) must stop the
    /// accumulation loop well before that.
    fn oversized_parts() -> Vec<CollectedType> {
        let member = |part: usize, i: usize| CollectedType::StringLiteral(format!("p{part}v{i}").into());
        (0..5).map(|part| CollectedType::Union((0..10).map(|i| member(part, i)).collect())).collect()
    }

    #[test]
    fn test_try_expand_template_literal_returns_none_past_cap() {
        let ctx = empty_ctx();
        let mut state = ResolveState::default();
        let file = Utf8PathBuf::from("/test/button.tsx");

        let result = try_expand_template_literal(&oversized_parts(), &file, &ctx, &mut state, 0);
        assert!(result.is_none(), "expected None once the Cartesian product exceeds MAX_TEMPLATE_LITERAL_EXPANSIONS");
    }

    #[test]
    fn test_template_literal_expansion_caps_and_keeps_the_pattern() {
        let ctx = empty_ctx();
        let mut state = ResolveState::default();
        let file = Utf8PathBuf::from("/test/button.tsx");

        let result = resolve_template_literal(&oversized_parts(), &file, &ctx, &mut state, 0);

        // Past the cap the template stays a pattern of its five holes, not 100,000 strings.
        let PropType::TemplateLiteral(parts) = &result else {
            panic!("expected capped expansion to stay a template literal, got {result:?}")
        };
        assert_eq!(parts.len(), 5);
        assert!(result.degraded_by().is_empty(), "the pattern is the exact type");
        // Exact contract: AC-6 specifies Info severity, DiagnosticCode::TemplateLiteralOpaque,
        // AND a message containing "could not be statically expanded" — checking the
        // code alone wouldn't catch a wrong severity or a reworded/missing message.
        let diag = state.diagnostics.iter().find(|d| d.code == DiagnosticCode::TemplateLiteralOpaque);
        assert!(
            diag.is_some(),
            "expected a TemplateLiteralOpaque diagnostic to be recorded, got {:?}",
            state.diagnostics
        );
        assert_eq!(diag.unwrap().severity, DiagnosticSeverity::Info);
        assert!(
            diag.unwrap().message.contains("could not be statically expanded"),
            "expected the message to contain 'could not be statically expanded', got {:?}",
            diag.unwrap().message
        );
    }
}
