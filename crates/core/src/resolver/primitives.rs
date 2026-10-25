//! Union, intersection, and indexed access resolution.

use camino::Utf8Path;

use crate::types::*;

use super::collected::resolve_collected_type;
use super::ResolutionContext;

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

/// The union of `members` as TypeScript forms it: a member that is itself a union, a literal union alias's included,
/// contributes its members, a member equal to an earlier one is dropped, and a single member stands alone. Two members
/// can be equal here although their source differs (`T[] | readonly T[]`, or two aliases that both resolve to
/// `ReactNode | null`).
pub(super) fn union_of(members: impl IntoIterator<Item = PropType>) -> PropType {
    /// The members collected so far; string literals, which make up the long keyword unions, are looked up in a set
    /// instead of compared with every earlier member.
    #[derive(Default)]
    struct Flat {
        members: Vec<PropType>,
        strings: rustc_hash::FxHashSet<String>,
    }
    impl Flat {
        fn push(&mut self, members: impl IntoIterator<Item = PropType>) {
            for member in members {
                match member {
                    PropType::Union(inner) => self.push(inner),
                    PropType::LiteralUnion { members, .. } => {
                        self.push(members.into_iter().map(PropType::StringLiteral))
                    }
                    PropType::StringLiteral(s) => {
                        if self.strings.insert(s.clone()) {
                            self.members.push(PropType::StringLiteral(s));
                        }
                    }
                    // `never` contributes nothing to a union.
                    PropType::Never => {}
                    member if !self.members.contains(&member) => self.members.push(member),
                    _ => {}
                }
            }
        }
    }
    let mut members: Vec<PropType> = members.into_iter().collect();
    if let [PropType::LiteralUnion { .. }] = members.as_slice() {
        return members.remove(0);
    }
    let mut flat = Flat::default();
    flat.push(members);
    // A union of nothing is `never`.
    match flat.members.len() {
        0 => PropType::Never,
        1 => flat.members.remove(0),
        _ => PropType::Union(flat.members),
    }
}

/// `ty` with `null` and `undefined` removed from it, as intersecting it with `{}` does.
fn without_nullish(ty: PropType) -> PropType {
    match ty {
        PropType::Null | PropType::Undefined => PropType::Never,
        PropType::Union(members) => {
            union_of(members.into_iter().filter(|m| !matches!(m, PropType::Null | PropType::Undefined)))
        }
        other => other,
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
        let resolved = resolve_collected_type(non_empty[0], consuming_file, ctx, state, depth + 1);
        // `T & {}` is `T` without `null` and `undefined` (lib's `NonNullable`), whatever else it is.
        return if non_empty.len() < members.len() { without_nullish(resolved) } else { resolved };
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

    // The member as its declaration reads, ancestors and type arguments included: React's `CSSProperties` reaches
    // csstype's `Property.ZIndex` when dependencies are loaded.
    if !key_str.is_empty() {
        if let Some(found) = super::evaluate::member_shared(obj, key_str, consuming_file, ctx, 0) {
            return resolve_collected_type(&found.0, &found.1, ctx, state, depth + 1);
        }
    }

    // A shortcut for React's own types when their declarations aren't loaded (ADR 0014).
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
        if ctx.const_array_bare_index.contains_key(name.as_str()) {
            let mut members = Vec::new();
            collect_const_array_members(name, consuming_file, ctx, state, &mut members, &mut Vec::new());
            return PropType::LiteralUnion { members, has_default: false };
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

/// The values of the `as const` array `name` in order, the elements of an array it spreads (`...PresetColors`) in
/// place. A spread of an array that is not in the project, or that is already being read (a cycle), leaves its elements
/// out and says so; the rest of the array stays.
fn collect_const_array_members(
    name: &str,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    members: &mut Vec<String>,
    reading: &mut Vec<String>,
) {
    let Some(values) = ctx.const_array_bare_index.get(name).and_then(|key| ctx.global.const_arrays.get(key.as_str()))
    else {
        return;
    };
    reading.push(name.to_owned());
    for value in values {
        let EnumValue::Spread { spread } = value else {
            let display = value.to_display_string();
            if !members.contains(&display) {
                members.push(display);
            }
            continue;
        };
        super::deps::record_bare_name(spread);
        if reading.iter().any(|reading| reading == spread) {
            state.diagnostics.push(const_array_spread_diagnostic(spread, "spreads itself", consuming_file));
        } else if ctx.const_array_bare_index.contains_key(spread.as_str()) {
            collect_const_array_members(spread, consuming_file, ctx, state, members, reading);
        } else {
            state.diagnostics.push(const_array_spread_diagnostic(
                spread,
                "is not an array declared in the project",
                consuming_file,
            ));
        }
    }
    reading.pop();
}

fn const_array_spread_diagnostic(spread: &str, why: &str, consuming_file: &Utf8Path) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!("'...{spread}' in an `as const` array {why}; its values are left out of the literal union"),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some("Declare the values in the array itself, or load the dependency that declares them.".into()),
        code: DiagnosticCode::OpaqueType,
        affects: Vec::new(),
    }
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
