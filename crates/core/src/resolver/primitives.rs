//! Union, intersection, and indexed access resolution.

use camino::Utf8Path;

use crate::types::*;

use super::collected::resolve_collected_type;
use super::import::{lookup_interface, lookup_interface_including_ambient, resolve_to_canonical};
use super::substitute::{build_substitution, substitute_type};
use super::{ResolutionContext, MAX_DEPTH};

pub(super) fn resolve_union(
    members: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // Filter out `undefined` from optional unions: `string | undefined` → `string`
    // (the `required: false` on the prop already captures optionality).
    let meaningful: Vec<&CollectedType> = members.iter().filter(|m| !matches!(m, CollectedType::Undefined)).collect();

    let to_resolve = if meaningful.is_empty() { members.iter().collect::<Vec<_>>() } else { meaningful };

    union_of(to_resolve.iter().map(|m| resolve_collected_type(m, consuming_file, ctx, state, depth + 1)))
}

/// The union of `members` as TypeScript forms it: a member that is itself a union contributes its members, a member
/// equal to an earlier one is dropped, and a single member stands alone. Two members can be equal here although their
/// source differs (`T[] | readonly T[]`, or two aliases that both resolve to `ReactNode | null`).
pub(super) fn union_of(members: impl IntoIterator<Item = PropType>) -> PropType {
    fn push(members: impl IntoIterator<Item = PropType>, flat: &mut Vec<PropType>) {
        for member in members {
            match member {
                PropType::Union(inner) => push(inner, flat),
                member if !flat.contains(&member) => flat.push(member),
                _ => {}
            }
        }
    }
    let mut flat = Vec::new();
    push(members, &mut flat);
    if flat.len() == 1 {
        flat.remove(0)
    } else {
        PropType::Union(flat)
    }
}

pub(super) fn resolve_intersection(
    members: &[CollectedType],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // `(string & {})` — a common trick for "string, but keep literal-type
    // autocomplete" — degrades to plain `string` once `{}` (`CollectedType::Object([])`,
    // an empty object type) is filtered out below and only one real member remains.
    let non_empty: Vec<&CollectedType> =
        members.iter().filter(|m| !matches!(m, CollectedType::Object(f) if f.is_empty())).collect();

    if non_empty.len() == 1 {
        return resolve_collected_type(non_empty[0], consuming_file, ctx, state, depth + 1);
    }

    let resolved: Vec<PropType> =
        members.iter().map(|m| resolve_collected_type(m, consuming_file, ctx, state, depth + 1)).collect();

    PropType::Intersection(resolved)
}

pub(super) fn resolve_indexed_access(
    obj: &CollectedType,
    key: &CollectedType,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    // A substituted argument (`T["_output"]` with `T = Parsed`) is looked up where it was written.
    match (obj, key) {
        (CollectedType::AtFile { file, inner }, _) => {
            return resolve_indexed_access(inner, key, file, ctx, state, depth);
        }
        (_, CollectedType::AtFile { inner, .. }) => {
            return resolve_indexed_access(obj, inner, consuming_file, ctx, state, depth);
        }
        _ => {}
    }
    let obj_name = match obj {
        CollectedType::Named { name, .. } => name.as_str(),
        _ => "",
    };
    let key_str = match key {
        CollectedType::StringLiteral(s) => s.as_str(),
        _ => "",
    };

    // Known table lookup — avoids needing the type checker for common cases.
    let known = match (obj_name, key_str) {
        (
            "CSSProperties" | "React.CSSProperties",
            "zIndex"
            | "opacity"
            | "order"
            | "flexGrow"
            | "flexShrink"
            | "flexBasis"
            | "lineHeight"
            | "fontWeight"
            | "columnCount"
            | "tabSize"
            | "animationIterationCount",
        ) => Some(PropType::Number),
        ("CSSProperties" | "React.CSSProperties", _) if !key_str.is_empty() => Some(PropType::String),
        (
            "HTMLAttributes" | "React.HTMLAttributes" | "DOMAttributes" | "React.DOMAttributes",
            "className" | "id" | "slot" | "title" | "lang" | "dir",
        ) => Some(PropType::String),
        ("HTMLAttributes" | "React.HTMLAttributes", "tabIndex") => Some(PropType::Number),
        ("HTMLAttributes" | "React.HTMLAttributes", "style") => Some(PropType::CssProperties),
        _ => None,
    };

    if let Some(pt) = known {
        return pt;
    }

    // `(typeof X)[number]` on a flat `const X = [...] as const` array literal
    // — antd's `type ButtonType = (typeof _ButtonTypes)[number]` pattern.
    // `[number]` means "the union of every element's type", which for a
    // `const`-asserted array of literals is exactly the array's own values.
    if let (CollectedType::TypeOf(name), CollectedType::Number) = (obj, key) {
        super::deps::record_bare_name(name);
        if let Some(values) =
            ctx.const_array_bare_index.get(name.as_str()).and_then(|k| ctx.global.const_arrays.get(k.as_str()))
        {
            let members: Vec<String> = values.iter().map(EnumValue::to_display_string).collect();
            return PropType::LiteralUnion { members, has_default: false };
        }
    }

    // A generic interface referenced with concrete args (e.g.
    // `RenderableProps<FieldRenderProps<FieldValue, T>>["children"]`) resolves
    // to a bare `PropType::Named` at the type level — interfaces are never
    // expanded there (only at the chain/component level) — so the
    // `PropType::Object` fallback below never matches an interface's own
    // fields. Look the field up directly on the interface's declaration
    // instead, substituting its declared type parameters with the caller's
    // concrete arguments before resolving.
    if let CollectedType::Named { name: obj_type_name, args: obj_args } = obj {
        let (canonical_file, canonical_name) =
            resolve_to_canonical(obj_type_name.as_str(), consuming_file, ctx, &mut state.diagnostics)
                .unwrap_or_else(|| (consuming_file.to_owned(), obj_type_name.to_string()));
        if let Some(iface) = lookup_interface_including_ambient(ctx, canonical_file.as_str(), &canonical_name) {
            if let Some(field) = iface.props.iter().find(|f| f.name == key_str) {
                let field_type = match ctx.named_types.lookup_interface_type_params_for(iface) {
                    Some(params) if !params.is_empty() && !obj_args.is_empty() => {
                        let subst = build_substitution(params, obj_args, consuming_file, &mut state.diagnostics);
                        substitute_type(&field.collected_type, &subst)
                    }
                    _ => field.collected_type.clone(),
                };
                let iface_file_path = iface.file_path.clone();
                return resolve_collected_type(&field_type, &iface_file_path, ctx, state, depth + 1);
            }
            // Not declared directly on this interface — DOM/ambient interfaces
            // commonly inherit shared attributes through `extends` (e.g.
            // `HTMLDivElement extends HTMLElement`, which declares `dir`/`lang`/
            // `title`), so walk the chain before giving up.
            if let Some((field, owner)) = find_field_in_ancestors(iface, key_str, ctx, depth) {
                let owner_file_path = owner.file_path.clone();
                return resolve_collected_type(&field.collected_type, &owner_file_path, ctx, state, depth + 1);
            }
        }
    }

    // Try to resolve the object type and look for the key.
    let obj_resolved = resolve_collected_type(obj, consuming_file, ctx, state, depth + 1);
    if let PropType::Object(fields) = &obj_resolved {
        if let Some(field) = fields.iter().find(|f| f.name == key_str) {
            return field.prop_type.clone();
        }
    }

    let expression = format!("{}[{}]", obj.to_raw_string(), key.to_raw_string());
    let diagnostic = Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("Indexed access type '{}' could not be statically resolved", expression),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some("Declare the prop's type directly, or index a type declared in the project.".into()),
        code: DiagnosticCode::IndexedAccessOpaque,
        affects: Vec::new(),
    };
    OpaqueDetail::give_up(state, expression.clone(), OpaqueReason::IndexedAccess { expression }, diagnostic)
}

/// Search an interface's `extends` chain (depth-first) for a field, returning
/// it alongside the interface that actually declares it. Only `SameFile`
/// ancestors are followed — TypeScript's own ambient lib files (the only
/// callers of this today) declare their whole DOM interface hierarchy in one
/// file, so cross-file ancestry never applies here.
fn find_field_in_ancestors<'g>(
    iface: &'g CollectedInterface,
    key_str: &str,
    ctx: &'g ResolutionContext,
    depth: u8,
) -> Option<(&'g RawProp, &'g CollectedInterface)> {
    if depth > MAX_DEPTH {
        return None;
    }
    for extends_ref in &iface.extends {
        let parent = match extends_ref {
            ExtendsRef::SameFile { name, .. } => lookup_interface(ctx, iface.file_path.as_str(), name.as_str()),
            ExtendsRef::Imported { .. } | ExtendsRef::Builtin { .. } => None,
        };
        let Some(parent) = parent else { continue };
        if let Some(field) = parent.props.iter().find(|f| f.name == key_str) {
            return Some((field, parent));
        }
        if let Some(found) = find_field_in_ancestors(parent, key_str, ctx, depth + 1) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::pipeline::PipelineOptions;

    #[test]
    fn indexed_access_on_an_unresolvable_object_gives_up_with_a_diagnostic() {
        let ctx = ResolutionContext::new(Arc::new(GlobalSourceData::default()), &PipelineOptions::default());
        let mut state = ResolveState::default();
        let obj = CollectedType::Named { name: "TotallyUnknownType".into(), args: vec![] };
        let key = CollectedType::StringLiteral("whatever".into());

        let result = resolve_indexed_access(&obj, &key, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);

        let PropType::Opaque(detail) = &result else { panic!("expected Opaque, got {:?}", result) };
        assert!(matches!(detail.reason(), OpaqueReason::IndexedAccess { .. }));
        assert!(
            state.diagnostics.iter().any(|d| d.code == DiagnosticCode::IndexedAccessOpaque),
            "expected an IndexedAccessOpaque diagnostic, got {:?}",
            state.diagnostics
        );
    }
}
