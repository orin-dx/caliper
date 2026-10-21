//! Function type and typeof resolution.

use camino::Utf8Path;
use compact_str::CompactString;

use crate::types::*;

use super::ResolutionContext;

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_function_type(
    params: &[CollectedType],
    param_names: &[Option<CompactString>],
    return_type: &CollectedType,
    consuming_file: &Utf8Path,
    // Unused in the body — kept so this function's signature matches its
    // dispatch siblings in resolve_collected_type's match arms (e.g.
    // resolve_indexed_access, resolve_template_literal), which all take the
    // same (ctx, state, depth) tail.
    _ctx: &ResolutionContext,
    state: &mut ResolveState,
    _depth: u8,
) -> PropType {
    // Check if the return type is a React node → render prop pattern.
    let returns_react_node = matches!(
        return_type,
        CollectedType::Named { name, .. }
            if matches!(
                name.as_str(),
                "ReactNode"
                    | "ReactElement"
                    | "JSX.Element"
                    | "Element"
                    | "ReactPortal"
                    | "ReactFragment"
            )
    );

    let first_param_name = || param_names.first().and_then(|n| n.as_ref()).map(|s| s.to_string());

    if returns_react_node && params.len() == 1 {
        let event_type = params[0].to_raw_string();
        return PropType::EventHandler { event_type, param_name: first_param_name() };
    }

    // Generic event handler: (e: SomeEvent) => void
    if params.len() == 1 {
        let event_type = params[0].to_raw_string();
        return PropType::EventHandler { event_type, param_name: first_param_name() };
    }

    // Zero-arg callback: () => void
    if params.is_empty() {
        return PropType::EventHandler { event_type: "void".into(), param_name: None };
    }

    // Multi-param function — describe as opaque. Deliberately does NOT resolve
    // return_type: resolve_collected_type mutates ResolveState (pushes
    // diagnostics, extends in_scope_type_params) rather than being a pure
    // query, and the function is emitted as opaque regardless of what the
    // return type resolves to — resolving it here only risked side effects
    // (e.g. a spurious "Cannot resolve type" warning) for a value nothing
    // ever reads.
    let param_strs: Vec<String> = params.iter().map(|p| p.to_raw_string()).collect();
    let raw = format!("({}) => {}", param_strs.join(", "), return_type.to_raw_string());

    let diagnostic = Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!(
            "'{}' is a multi-parameter function type and can't be statically resolved — it will appear as opaque",
            raw
        ),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
        affects: Vec::new(),
    };
    OpaqueDetail::give_up(state, raw, OpaqueReason::MultiParamFunction, diagnostic)
}

pub(super) fn resolve_typeof(
    name: &CompactString,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> PropType {
    // `typeof X` — look for X in global.enums (for cva() results), via the
    // precomputed bare-name index (O(1) instead of a linear scan over every
    // enum/cva/tv/recipe entry in the project with a per-candidate allocation).
    super::deps::record_bare_name(name);
    let found_enum = ctx.enum_bare_index.get(name.as_str()).and_then(|key| ctx.global.enums.get(key.as_str()));

    if found_enum.is_some() {
        // Has cva-like enum entries — the VariantProps<typeof X> pattern handles this.
        // At the type level, surface as Named.
        return PropType::Named { name: name.clone(), args: vec![] };
    }

    diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("typeof '{}' in '{}' — could not statically evaluate", name, consuming_file),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
        affects: Vec::new(),
    });

    PropType::Named { name: name.clone(), args: vec![] }
}
