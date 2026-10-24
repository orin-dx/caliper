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

/// `Exclude<T, U>`, `Extract<T, U>` and `NonNullable<T>` as TypeScript evaluates them: each member of `T` kept or
/// dropped by whether it is assignable to `U`, or to `null | undefined`. `None` when assignability needs more than
/// literals and primitives (an object member, a type parameter), so the caller prints the type as written.
pub(super) fn evaluate_filter(name: &str, args: &[PropType]) -> Option<PropType> {
    let (members, targets, keep_assignable) = match (name, args) {
        ("NonNullable", [t]) => (members_of(t), vec![PropType::Null, PropType::Undefined], false),
        ("Exclude", [t, u]) => (members_of(t), members_of(u), false),
        ("Extract", [t, u]) => (members_of(t), members_of(u), true),
        _ => return None,
    };
    let mut kept = Vec::new();
    for member in members {
        // `boolean` is `false | true`, which a boolean literal target splits.
        let split =
            matches!(member, PropType::Boolean) && targets.iter().any(|t| matches!(t, PropType::BoolLiteral(_)));
        let parts = if split { vec![PropType::BoolLiteral(false), PropType::BoolLiteral(true)] } else { vec![member] };
        for part in parts {
            if assignable(&part, &targets)? == keep_assignable {
                kept.push(part);
            }
        }
    }
    Some(if kept.is_empty() { PropType::Never } else { union_of(kept) })
}

fn members_of(ty: &PropType) -> Vec<PropType> {
    match ty {
        PropType::Union(members) => members.clone(),
        PropType::LiteralUnion { members, .. } => members.iter().cloned().map(PropType::StringLiteral).collect(),
        other => vec![other.clone()],
    }
}

/// Whether `member` is assignable to one of `targets`; `None` unless both are literals or primitives.
fn assignable(member: &PropType, targets: &[PropType]) -> Option<bool> {
    let simple = |t: &PropType| {
        matches!(
            t,
            PropType::String
                | PropType::Number
                | PropType::Boolean
                | PropType::Null
                | PropType::Undefined
                | PropType::StringLiteral(_)
                | PropType::NumberLiteral(_)
                | PropType::BoolLiteral(_)
        )
    };
    // Removing `null`/`undefined` is decidable for any member whose type is known: nothing else is assignable to them.
    // A name (an unexpanded alias, a type parameter) may stand for either, so it isn't.
    let nullish_only = targets.iter().all(|t| matches!(t, PropType::Null | PropType::Undefined))
        && !matches!(member, PropType::Named { .. } | PropType::Opaque(_) | PropType::Any | PropType::Unknown);
    if !(nullish_only || simple(member) && targets.iter().all(simple)) {
        return None;
    }
    Some(targets.iter().any(|target| {
        member == target
            || matches!(
                (member, target),
                (PropType::StringLiteral(_), PropType::String)
                    | (PropType::NumberLiteral(_), PropType::Number)
                    | (PropType::BoolLiteral(_), PropType::Boolean)
            )
    }))
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
        if let Some(values) =
            ctx.const_array_bare_index.get(name.as_str()).and_then(|k| ctx.global.const_arrays.get(k.as_str()))
        {
            let members: Vec<String> = values.iter().map(EnumValue::to_display_string).collect();
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
