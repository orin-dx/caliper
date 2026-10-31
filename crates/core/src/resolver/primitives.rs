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

    // A shortcut for React's own types when their declarations aren't loaded (precedence).
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
        if const_array_values(name, consuming_file, ctx).is_some() {
            let mut members = Vec::new();
            collect_const_array_members(name, consuming_file, ctx, state, &mut members, &mut Vec::new());
            return PropType::LiteralUnion { members, has_default: false };
        }
    }

    // Try to resolve the object type and look for the key.
    let obj_resolved = resolve_collected_type(obj, consuming_file, ctx, state, depth + 1);
    if let PropType::Object(fields) = &obj_resolved {
        if let Some(field) = fields.iter().find(|f| f.is_member() && f.name == key_str) {
            return field.prop_type.clone();
        }
    }

    // TypeScript resolves the object type of an indexed access by the one path every reference to it takes, then reads
    // the property from the type that gave (`getIndexedAccessType`). The members of a named type are its props chain,
    // the one an `extends` of the same type would read: `ComponentPropsWithoutRef<typeof Tooltip>` is `Tooltip`'s props
    // there, so it is here.
    if let (CollectedType::Named { name, args }, false) = (obj, key_str.is_empty()) {
        if let Some(found) = read_from_props_chain(name, args, key_str, consuming_file, ctx, state) {
            return found;
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

/// The type of the property `key` of the named type `name<args>`, read from the props its chain resolves to, or `None`
/// when the chain has no such prop (the caller then reports the access as opaque).
///
/// The chain is resolved as a declaration of its own, from depth 0 (the nesting depth of the access says nothing about
/// the type it reads), in a branch of the caller's state: a type that reads itself meets the cycle sets of the walk it
/// is part of and stops there, as TypeScript's circular `getIndexedAccessType` does. A chain resolves every member of
/// the type, and only one was asked for, so the diagnostics about the others are not the access's; they are kept only
/// when the member read is itself degraded, which is what they would explain.
fn read_from_props_chain(
    name: &str,
    args: &[CollectedType],
    key: &str,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
) -> Option<PropType> {
    // The read is a question about the type alone, asked in a state of its own: the caller's cycle sets and type
    // parameters in scope are not part of it, so its answer is the same whichever walk asks, and the run keeps it
    // (`React.ComponentProps<typeof Button>["variant"]` is read by every variant of every component that forwards it,
    // and each read resolves all of `Button`'s props). What bounds a type that reads itself is the nesting of reads on
    // this call stack, since the state that would cut it is not shared.
    let _nesting = ReadNesting::enter()?;
    let cache_key: PropsReadKey = {
        use std::hash::Hasher;
        let mut hasher = rustc_hash::FxHasher::default();
        args.iter().for_each(|arg| arg.structural_hash(&mut hasher));
        (Box::from(consuming_file.as_str()), format!("{name}<{:x}>", hasher.finish()).into(), Box::from(key))
    };
    if let Some(read) = ctx.props_reads.get(&cache_key) {
        super::deps::replay(&read.deps);
        return Some(read.found.clone());
    }
    let mapping = ComponentMapping::of_no_component(consuming_file);
    let mut asked = ResolveState::default();
    let (found, deps) = super::deps::capturing(|| {
        let chain = super::chain::resolve_props_chain(name, args, consuming_file, &mapping, ctx, &mut asked, 0);
        chain.props.iter().find(|prop| prop.name == key).map(|prop| prop.prop_type.clone())
    });
    // A chain resolves every member of the type, and only one was asked for: the diagnostics about the others are not
    // the access's. They are kept when the member itself is degraded, which is what they would explain.
    match &found {
        Some(found) if !found.holds_opaque() => {
            ctx.props_reads.insert(cache_key, PropsRead { found: found.clone(), deps });
        }
        Some(_) => {
            state.diagnostics.append(&mut asked.diagnostics);
            state.shared_diagnostics.append(&mut asked.shared_diagnostics);
            state.named_unevaluated |= asked.named_unevaluated;
        }
        None => {}
    }
    found
}

thread_local! {
    /// How many reads from a props chain this thread's call stack is inside of.
    static READS_IN_PROGRESS: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// The most reads from a props chain that may be inside one another. A type that reads itself
/// (`function Self(props: { b: ComponentProps<typeof Self>["a"] })`) is circular, which TypeScript reports as an error
/// and types as `any`; here the read at this depth finds nothing and the access is reported as opaque.
const MAX_NESTED_PROPS_READS: u8 = 4;

/// One level of `READS_IN_PROGRESS`, left when it is dropped.
struct ReadNesting;

impl ReadNesting {
    fn enter() -> Option<Self> {
        READS_IN_PROGRESS.with(|depth| {
            (depth.get() < MAX_NESTED_PROPS_READS).then(|| {
                depth.set(depth.get() + 1);
                ReadNesting
            })
        })
    }
}

impl Drop for ReadNesting {
    fn drop(&mut self) {
        READS_IN_PROGRESS.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// `(file, name and structural hash of the arguments, key)` of a cached read from a props chain.
pub(crate) type PropsReadKey = (Box<str>, Box<str>, Box<str>);

/// A property read from a props chain that came out whole, and the files it read.
#[derive(Clone)]
pub(crate) struct PropsRead {
    found: PropType,
    deps: super::deps::Dependencies,
}

impl PropsRead {
    pub(super) fn deps(&self) -> &super::deps::Dependencies {
        &self.deps
    }
}

/// The members of the array-typed const `name`, as the symbol in scope in `consuming_file` (TypeScript reads `typeof X`
/// from the file that wrote it, project or dependency: Radix's `Align` is `(typeof ALIGN_OPTIONS)[number]` over
/// `declare const ALIGN_OPTIONS: readonly [...]` in the same `.d.ts`), else the array of that name the project declares
/// (an import of it, which this does not follow by file).
fn const_array_values<'a>(
    name: &str,
    consuming_file: &Utf8Path,
    ctx: &'a ResolutionContext,
) -> Option<&'a [EnumValue]> {
    ctx.named_types.lookup_const_array(consuming_file, name).or_else(|| {
        let key = ctx.const_array_bare_index.get(name)?;
        ctx.global.const_arrays.get(key.as_str()).map(Vec::as_slice)
    })
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
    let Some(values) = const_array_values(name, consuming_file, ctx) else {
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
        } else if const_array_values(spread, consuming_file, ctx).is_some() {
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
