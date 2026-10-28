//! The type-level evaluation a props base needs before its props can be listed, when the operands are known: the branch
//! a conditional takes for a literal (`T extends keyof Recipes ? … : …` with `T = "button"`), a member read through
//! an indexed access (`Recipes["button"]["__type"]`, generic interface members included), and the keys a mapped type
//! ranges over (`{ [K in keyof Variant]?: … }`). Anything that needs a checker (an unknown operand, a generic left
//! unbound, an inferred type) evaluates to `None`, and the caller keeps its existing handling.
//!
//! Its `depth` counts only its own steps: it reads declarations and never re-enters the resolver, so callers start it
//! at 0 instead of passing the resolver's depth, which a deep chain (csstype's `Fallback<…>` under Chakra's style
//! props) would exhaust first.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;

use crate::types::*;

use super::import::{lookup_interface, lookup_interface_including_ambient, lookup_type_alias, resolve_to_canonical};
use super::substitute::{bind_args, build_substitution, substitute_alias, substitute_type};
use super::{ResolutionContext, MAX_DEPTH};

/// A type, and the file its names resolve in.
pub(super) type Located = (CollectedType, Utf8PathBuf);

/// One key a mapped type ranges over, with what its source member says (a homomorphic `keyof X` keeps X's
/// optionality and documentation).
pub(super) struct Key {
    pub(super) name: String,
    pub(super) required: bool,
    pub(super) description: String,
    /// The JSDoc tags of the source member (`@default "md"`).
    pub(super) tags: std::collections::BTreeMap<String, String>,
}

/// `ty` with a decidable conditional replaced by its branch, or an indexed access by the member it reads. `None` when
/// neither applies.
pub(super) fn reduce(ty: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Located> {
    if depth > MAX_DEPTH {
        return None;
    }
    match ty {
        CollectedType::AtFile { file, inner } => reduce(inner, file, ctx, depth + 1),
        CollectedType::Conditional { check, extends_type, true_type, false_type } => {
            if let Some(branch) = inferred_component_branch(check, extends_type, true_type, false_type, file, ctx) {
                return Some((branch, file.to_owned()));
            }
            let branch = if extends(check, extends_type, file, ctx, depth)? { true_type } else { false_type };
            Some((branch.as_ref().clone(), file.to_owned()))
        }
        // Same depth: `member` counts its own steps, and at 0 it answers from the run's cache.
        CollectedType::IndexedAccess { obj, key } => member(obj, &literal(key)?, file, ctx, depth),
        _ => None,
    }
}

fn inferred_component_branch(
    check: &CollectedType,
    extends_type: &CollectedType,
    true_type: &CollectedType,
    false_type: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<CollectedType> {
    let (CollectedType::TypeOf(component), check_file) = unwrap_at_file(check) else { return None };
    let check_file = check_file.unwrap_or(file);
    let (props, props_file) = super::callee::component_props_of_typeof(component, check_file, ctx)?;
    let (CollectedType::Named { name, args }, _) = unwrap_at_file(extends_type) else { return None };
    if !super::may_be_react_builtin(name, file, ctx) {
        return None;
    }
    let [CollectedType::Raw(infer)] = args.as_slice() else { return None };
    let param = infer.trim().strip_prefix("infer ")?.trim();
    match name.strip_prefix("React.").unwrap_or(name) {
        "Context" => Some(false_type.clone()),
        "ComponentType" if !param.is_empty() => {
            let mut subst = super::substitute::Substitution::default();
            subst.insert(param, CollectedType::AtFile { file: props_file, inner: Box::new(props) });
            Some(substitute_type(true_type, &subst))
        }
        _ => None,
    }
}

/// The keys of a mapped type's constraint: `keyof X`, a string literal or a union of them.
pub(super) fn mapped_keys(
    constraint: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<Vec<Key>> {
    if depth > MAX_DEPTH {
        return None;
    }
    match constraint {
        CollectedType::AtFile { file, inner } => mapped_keys(inner, file, ctx, depth + 1),
        // `Record<Breakpoint, V>`: an alias for a union of literals, here or in another file.
        CollectedType::Named { name, args } => {
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            mapped_keys(&body, &body_file, ctx, depth + 1)
        }
        CollectedType::KeyOf(target) => keys_of(target, file, ctx, depth + 1),
        // `(typeof NODES)[number]` over `declare const NODES: readonly ["a", "b"]`: the tuple's elements.
        CollectedType::IndexedAccess { obj, key } if matches!(key.as_ref(), CollectedType::Number) => {
            let (CollectedType::TypeOf(array), array_file) = unwrap_at_file(obj) else { return None };
            let array_file = array_file.unwrap_or(file);
            // `export const responsiveArray = ['xxxl', 'xxl'] as const`: its string values, when it holds nothing else.
            if let Some(values) = ctx.named_types.lookup_const_array(array_file, array) {
                return values
                    .iter()
                    .map(|value| match value {
                        EnumValue::String(name) => Some(Key {
                            name: name.clone(),
                            required: true,
                            description: String::new(),
                            tags: Default::default(),
                        }),
                        _ => None,
                    })
                    .collect();
            }
            let (_, alias) = lookup_type_alias(ctx, array_file.as_str(), &format!("typeof {array}"))?;
            let CollectedTypeAlias::Passthrough { target: CollectedType::Tuple(elements), .. } = alias else {
                return None;
            };
            // A tuple's elements come through as the source text of a literal (`"a"`).
            let names = elements.iter().map(|element| match element {
                CollectedType::StringLiteral(name) => Some(name.to_string()),
                CollectedType::Raw(text) => Some(text.trim().strip_prefix('"')?.strip_suffix('"')?.to_owned()),
                _ => None,
            });
            Some(
                names
                    .map(|name| {
                        Some(Key { name: name?, required: true, description: String::new(), tags: Default::default() })
                    })
                    .collect::<Option<_>>()?,
            )
        }
        other => Some(
            literal_set(other)?
                .into_iter()
                .map(|name| Key { name, required: true, description: String::new(), tags: Default::default() })
                .collect(),
        ),
    }
}

/// One type that assignability here can decide: a literal, a primitive, `null` or `undefined`. `boolean` is its two
/// literals.
#[derive(Clone, PartialEq)]
enum Atom {
    Str(String),
    Num(String),
    Bool(bool),
    String,
    Number,
    Null,
    Undefined,
}

impl Atom {
    fn assignable_to(&self, targets: &[Atom]) -> bool {
        targets.iter().any(|target| {
            self == target || matches!((self, target), (Atom::Str(_), Atom::String) | (Atom::Num(_), Atom::Number))
        })
    }
}

/// The atoms `ty` is a union of, following aliases; `None` when any member is something else (an object, a type
/// parameter, `any`). `never` is none.
fn atoms(ty: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Vec<Atom>> {
    if depth > MAX_DEPTH {
        return None;
    }
    Some(match ty {
        CollectedType::AtFile { file, inner } => return atoms(inner, file, ctx, depth + 1),
        CollectedType::StringLiteral(s) => vec![Atom::Str(s.to_string())],
        CollectedType::NumberLiteral(n) => vec![Atom::Num(n.to_string())],
        CollectedType::BoolLiteral(b) => vec![Atom::Bool(*b)],
        CollectedType::Boolean => vec![Atom::Bool(false), Atom::Bool(true)],
        CollectedType::String => vec![Atom::String],
        CollectedType::Number => vec![Atom::Number],
        CollectedType::Null => vec![Atom::Null],
        CollectedType::Undefined => vec![Atom::Undefined],
        CollectedType::Never => vec![],
        CollectedType::Union(members) => {
            let mut all = Vec::new();
            for member in members {
                all.extend(atoms(member, file, ctx, depth + 1)?);
            }
            all
        }
        CollectedType::Named { name, args } => {
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            return atoms(&body, &body_file, ctx, depth + 1);
        }
        _ => return None,
    })
}

/// Whether `check` is assignable to `extends_type`; `None` when either side isn't known. `check` is a type made of
/// atoms, or a member read whose type is: a member read is not a naked type parameter, so the conditional doesn't
/// distribute and the whole set must be assignable (`Factory["variant"] extends string`).
fn extends(
    check: &CollectedType,
    extends_type: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<bool> {
    // Everything extends `unknown` (and `any`), whatever the check is: `T extends unknown ? Omit<T, K> : never`, the
    // idiom for distributing over a union, takes its true branch.
    if matches!(unwrap_at_file(extends_type).0, CollectedType::Unknown | CollectedType::Any) {
        return Some(true);
    }
    let values = check_atoms(check, file, ctx, depth + 1)?;
    if values.is_empty() {
        return None;
    }
    match unwrap_at_file(extends_type) {
        (CollectedType::KeyOf(target), target_file) => {
            let keys = keys_of(target, target_file.unwrap_or(file), ctx, depth + 1)?;
            Some(values.iter().all(|value| matches!(value, Atom::Str(s) if keys.iter().any(|key| &key.name == s))))
        }
        _ => {
            let targets = atoms(extends_type, file, ctx, depth + 1)?;
            Some(values.iter().all(|value| value.assignable_to(&targets)))
        }
    }
}

/// What a conditional's check stands for: its atoms, or those of the member a member read reads.
fn check_atoms(check: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Vec<Atom>> {
    match check {
        CollectedType::AtFile { file, inner } => check_atoms(inner, file, ctx, depth + 1),
        CollectedType::IndexedAccess { obj, key } => match reduce(check, file, ctx, depth + 1) {
            Some((member, member_file)) => atoms(&member, &member_file, ctx, depth + 1),
            // `Payload["variant"]` on a payload object that declares no `variant` (Mantine's factories).
            None if lacks_member(obj, &literal(key)?, file, ctx, depth + 1) => Some(vec![Atom::Undefined]),
            None => None,
        },
        other => atoms(other, file, ctx, depth + 1),
    }
}

/// The bare type parameter an alias's conditional is over (`type Exclude<T, U> = T extends U ? never : T`): the
/// argument bound to it is distributed over, member by member, as TypeScript does for a naked type parameter.
pub(super) fn distributive_param(alias: &CollectedTypeAlias) -> Option<&str> {
    let CollectedTypeAlias::Passthrough { target: CollectedType::Conditional { check, .. }, .. } = alias else {
        return None;
    };
    match check.as_ref() {
        CollectedType::Named { name, args } if args.is_empty() => Some(name.as_str()),
        _ => None,
    }
}

/// The members `arg` is a union of, each with the file it is written in: through aliases, with `boolean` as `false |
/// true` and `never` as none. A type that is not a union is its own one member.
pub(super) fn union_members(arg: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Vec<Located> {
    if depth > MAX_DEPTH {
        return vec![(arg.clone(), file.to_owned())];
    }
    match arg {
        CollectedType::AtFile { file, inner } => union_members(inner, file, ctx, depth + 1),
        CollectedType::Union(members) => {
            members.iter().flat_map(|member| union_members(member, file, ctx, depth + 1)).collect()
        }
        CollectedType::Never => vec![],
        CollectedType::Boolean => vec![
            (CollectedType::BoolLiteral(false), file.to_owned()),
            (CollectedType::BoolLiteral(true), file.to_owned()),
        ],
        CollectedType::Named { name, args } => match alias_body(name, args, file, ctx) {
            Some((body, body_file)) if matches!(body, CollectedType::Union(_)) => {
                union_members(&body, &body_file, ctx, depth + 1)
            }
            _ => vec![(arg.clone(), file.to_owned())],
        },
        other => vec![(other.clone(), file.to_owned())],
    }
}

/// Expand the object union read by `Extract<Items[number], { type?: "submenu" }>` only when every step is known.
/// This is separate from `union_members`: a failed indexed read must not become a plausible union member.
fn object_union_members(
    arg: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<Vec<Located>> {
    if depth > MAX_DEPTH {
        return None;
    }
    match arg {
        CollectedType::AtFile { file, inner } => object_union_members(inner, file, ctx, depth + 1),
        CollectedType::Named { name, args }
            if name == "NonNullable" && args.len() == 1 && super::may_be_ts_lib_type(name, file, ctx) =>
        {
            object_union_members(&args[0], file, ctx, depth + 1).map(|members| {
                members
                    .into_iter()
                    .filter(|(ty, _)| !matches!(ty, CollectedType::Null | CollectedType::Undefined))
                    .collect()
            })
        }
        CollectedType::Named { name, args } => {
            let (canonical_file, canonical_name) = canonical(name, file, ctx);
            if lookup_interface_including_ambient(ctx, canonical_file.as_str(), &canonical_name).is_some() {
                return Some(vec![(arg.clone(), file.to_owned())]);
            }
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            object_union_members(&body, &body_file, ctx, depth + 1)
        }
        CollectedType::Union(members) => {
            let mut expanded = Vec::new();
            for member in members {
                expanded.extend(object_union_members(member, file, ctx, depth + 1)?);
            }
            Some(expanded)
        }
        CollectedType::IndexedAccess { obj, key } if matches!(key.as_ref(), CollectedType::Number) => {
            let array = array_element(obj, file, ctx, depth + 1)?;
            object_union_members(&array.0, &array.1, ctx, depth + 1)
        }
        CollectedType::IndexedAccess { .. } => {
            let (member, member_file) = reduce(arg, file, ctx, depth + 1)?;
            object_union_members(&member, &member_file, ctx, depth + 1)
        }
        CollectedType::Null | CollectedType::Undefined | CollectedType::Object(_) => {
            Some(vec![(arg.clone(), file.to_owned())])
        }
        CollectedType::Intersection(_)
        | CollectedType::Array(_)
        | CollectedType::Tuple(_)
        | CollectedType::TypeOf(_)
        | CollectedType::Conditional { .. }
        | CollectedType::Mapped { .. }
        | CollectedType::KeyOf(_)
        | CollectedType::Raw(_)
        | CollectedType::String
        | CollectedType::Number
        | CollectedType::Boolean
        | CollectedType::Any
        | CollectedType::Never
        | CollectedType::Unknown
        | CollectedType::Void
        | CollectedType::BigInt
        | CollectedType::Symbol
        | CollectedType::StringLiteral(_)
        | CollectedType::NumberLiteral(_)
        | CollectedType::BoolLiteral(_)
        | CollectedType::TemplateLiteral(_)
        | CollectedType::Function { .. } => Some(vec![(arg.clone(), file.to_owned())]),
    }
}

fn array_element(arg: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Located> {
    if depth > MAX_DEPTH {
        return None;
    }
    match arg {
        CollectedType::AtFile { file, inner } => array_element(inner, file, ctx, depth + 1),
        CollectedType::Named { name, args }
            if name == "NonNullable" && args.len() == 1 && super::may_be_ts_lib_type(name, file, ctx) =>
        {
            array_element(&args[0], file, ctx, depth + 1)
        }
        CollectedType::Named { name, args } => {
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            array_element(&body, &body_file, ctx, depth + 1)
        }
        CollectedType::IndexedAccess { .. } => {
            let (member, member_file) = reduce(arg, file, ctx, depth + 1)?;
            array_element(&member, &member_file, ctx, depth + 1)
        }
        CollectedType::Union(members) => {
            let mut arrays =
                members.iter().filter(|member| !matches!(member, CollectedType::Null | CollectedType::Undefined));
            let one = arrays.next()?;
            if arrays.next().is_some() {
                return None;
            }
            array_element(one, file, ctx, depth + 1)
        }
        CollectedType::Array(inner) => Some((inner.as_ref().clone(), file.to_owned())),
        CollectedType::Object(_)
        | CollectedType::Intersection(_)
        | CollectedType::Tuple(_)
        | CollectedType::TypeOf(_)
        | CollectedType::Conditional { .. }
        | CollectedType::Mapped { .. }
        | CollectedType::KeyOf(_)
        | CollectedType::Raw(_)
        | CollectedType::String
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
        | CollectedType::TemplateLiteral(_)
        | CollectedType::Function { .. } => None,
    }
}

/// The selected members of a discriminated object union, or `None` if the filter cannot be decided.
pub(super) fn extracted_object_members(
    source: &CollectedType,
    filter: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<Vec<Located>> {
    let CollectedType::Object(fields) = filter else { return None };
    let [field] = fields.as_slice() else { return None };
    if field.required {
        return None;
    }
    let CollectedType::StringLiteral(wanted) = &field.collected_type else { return None };
    let mut selected = Vec::new();
    for (member_type, member_file) in object_union_members(source, file, ctx, 0)? {
        if matches!(member_type, CollectedType::Null | CollectedType::Undefined) {
            continue;
        }
        let (actual, actual_file) = member(&member_type, &field.name, &member_file, ctx, 0)?;
        let values = atoms(&actual, &actual_file, ctx, 0)?;
        if !values.is_empty() && values.iter().all(|value| value == &Atom::Str(wanted.to_string())) {
            selected.push((member_type, member_file));
        }
    }
    Some(selected)
}

/// Whether `obj` is an object type, through aliases, whose fields are all known and none is `key`.
fn lacks_member(obj: &CollectedType, key: &str, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match obj {
        CollectedType::AtFile { file, inner } => lacks_member(inner, key, file, ctx, depth + 1),
        // An index signature isn't recorded, so an object with no fields may still have the member.
        CollectedType::Object(fields) => !fields.is_empty() && !fields.iter().any(|f| f.name == key),
        CollectedType::Named { name, args } => alias_body(name, args, file, ctx)
            .is_some_and(|(body, body_file)| lacks_member(&body, key, &body_file, ctx, depth + 1)),
        _ => false,
    }
}

/// The names `keyof target` produces: an interface's members (its ancestors' too), or an object type's fields.
fn keys_of(target: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Vec<Key>> {
    if depth > MAX_DEPTH {
        return None;
    }
    match target {
        CollectedType::AtFile { file, inner } => keys_of(inner, file, ctx, depth + 1),
        CollectedType::Object(fields) => Some(
            fields
                .iter()
                .map(|f| Key {
                    name: f.name.clone(),
                    required: f.required,
                    description: f.description.clone(),
                    tags: Default::default(),
                })
                .collect(),
        ),
        CollectedType::Named { name, args } => {
            let (canonical_file, canonical_name) = canonical(name, file, ctx);
            if let Some(iface) = lookup_interface_including_ambient(ctx, canonical_file.as_str(), &canonical_name) {
                let mut keys = Vec::new();
                interface_keys(iface, ctx, depth + 1, &mut keys);
                return Some(keys);
            }
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            keys_of(&body, &body_file, ctx, depth + 1)
        }
        _ => None,
    }
}

/// An interface's members, then those of every interface it extends that the project declares.
fn interface_keys(iface: &CollectedInterface, ctx: &ResolutionContext, depth: u8, keys: &mut Vec<Key>) {
    for prop in &iface.props {
        if !keys.iter().any(|key| key.name == prop.name) {
            keys.push(Key {
                name: prop.name.clone(),
                required: prop.required,
                description: prop.description.clone(),
                tags: prop.tags.clone(),
            });
        }
    }
    if depth > MAX_DEPTH {
        return;
    }
    for parent in &iface.extends {
        let found = match parent {
            ExtendsRef::SameFile { name, .. } => lookup_interface(ctx, iface.file_path.as_str(), name),
            ExtendsRef::Imported { local_name, .. } => {
                let (file, name) = canonical(local_name, &iface.file_path, ctx);
                lookup_interface(ctx, file.as_str(), &name)
            }
            ExtendsRef::Builtin { .. } => None,
        };
        if let Some(parent) = found {
            interface_keys(parent, ctx, depth + 1, keys);
        }
    }
}

/// The type of member `key` of `obj`, read as `obj[key]` reads it.
pub(super) fn member(
    obj: &CollectedType,
    key: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<Located> {
    member_shared(obj, key, file, ctx, depth).map(Arc::unwrap_or_clone)
}

/// `member`, sharing the answer instead of copying it: csstype's keyword unions have dozens of members, and every style
/// prop of every component reads one.
pub(super) fn member_shared(
    obj: &CollectedType,
    key: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<Arc<Located>> {
    // A read from the top of a walk depends only on the declarations, so a run answers it once (`member_reads`).
    if let (CollectedType::Named { name, args }, 0) = (obj, depth) {
        let name: Box<str> = if args.is_empty() { Box::from(name.as_str()) } else { obj.to_raw_string().into() };
        let cache_key = (Box::from(file.as_str()), name, Box::from(key));
        if let Some(read) = ctx.member_reads.get(&cache_key) {
            super::deps::replay(&read.deps);
            return read.found.clone();
        }
        let (found, deps) = super::deps::capturing(|| member_uncached(obj, key, file, ctx, depth));
        let found = found.map(Arc::new);
        ctx.member_reads.insert(cache_key, MemberRead { found: found.clone(), deps });
        return found;
    }
    member_uncached(obj, key, file, ctx, depth).map(Arc::new)
}

/// The most members a member read's union may have to be shown in place of the read (`Variant["size"]` becomes its
/// literals). A longer one, such as csstype's keyword unions, stays as written, the way TypeScript truncates a long
/// type when it prints one.
pub(super) const MAX_DISPLAYED_MEMBERS: usize = 32;

/// The type an indexed access reads, shared, when it has at most `MAX_DISPLAYED_MEMBERS` union members.
pub(super) fn displayed_member(
    access: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<Arc<Located>> {
    let CollectedType::IndexedAccess { obj, key } = access else { return None };
    let found = member_shared(obj, &literal(key)?, file, ctx, 0)?;
    let members = match unwrap_at_file(&found.0).0 {
        CollectedType::Union(members) => members.len(),
        _ => 1,
    };
    (members <= MAX_DISPLAYED_MEMBERS).then_some(found)
}

/// `(file, type as written, key)` of a cached member read.
pub(crate) type MemberReadKey = (Box<str>, Box<str>, Box<str>);

/// A cached `member` answer and the files it read.
#[derive(Clone)]
pub(crate) struct MemberRead {
    found: Option<Arc<Located>>,
    deps: super::deps::Dependencies,
}

impl MemberRead {
    pub(super) fn deps(&self) -> &super::deps::Dependencies {
        &self.deps
    }
}

fn member_uncached(
    obj: &CollectedType,
    key: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<Located> {
    if depth > MAX_DEPTH {
        return None;
    }
    match obj {
        CollectedType::AtFile { file, inner } => member(inner, key, file, ctx, depth + 1),
        // A property of `A & B` is read from the constituents that declare it, as TypeScript reads it: a constituent
        // with only an index signature (`{ spacing: SpacingToken } & { [token: string]: never }`) adds nothing to a
        // declared name. Declared by several, it is their intersection.
        CollectedType::Intersection(members) => {
            let mut found: Vec<Located> = members.iter().filter_map(|m| member(m, key, file, ctx, depth + 1)).collect();
            match found.len() {
                0 => None,
                1 => found.pop(),
                _ => Some((
                    CollectedType::Intersection(
                        found
                            .into_iter()
                            .map(|(ty, at)| CollectedType::AtFile { file: at, inner: Box::new(ty) })
                            .collect(),
                    ),
                    file.to_owned(),
                )),
            }
        }
        CollectedType::Object(fields) => {
            Some((fields.iter().find(|f| f.name == key)?.collected_type.clone(), file.to_owned()))
        }
        CollectedType::Named { name, args } => {
            let (canonical_file, canonical_name) = canonical(name, file, ctx);
            let found = lookup_interface_including_ambient(ctx, canonical_file.as_str(), &canonical_name)
                .or_else(|| react_interface(ctx, &canonical_file, &canonical_name));
            if let Some(iface) = found {
                return interface_member(iface, args, key, file, ctx, depth + 1);
            }
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            member(&body, key, &body_file, ctx, depth + 1)
        }
        // `{ [P in keyof T]: T[P] | … }["width"]` (csstype's `Fallback<T>`): the value with the key bound, when the keys
        // include it. Over `keyof T` the key's presence is a member read of `T`, which avoids listing all of T's keys.
        CollectedType::Mapped { param, key_type, value_type, .. } => {
            let present = match unwrap_at_file(key_type) {
                (CollectedType::KeyOf(target), target_file) => {
                    member(target, key, target_file.unwrap_or(file), ctx, depth + 1).is_some()
                }
                _ => mapped_keys(key_type, file, ctx, depth + 1)?.iter().any(|k| k.name == key),
            };
            if !present {
                return None;
            }
            let bound = [CollectedType::StringLiteral(key.into())];
            let subst = build_substitution(std::slice::from_ref(param), &bound, file, &mut Vec::new());
            Some((substitute_type(value_type, &subst), file.to_owned()))
        }
        // `A["x"]["y"]`, `(T extends … ? A : B)["x"]`.
        CollectedType::IndexedAccess { .. } | CollectedType::Conditional { .. } => {
            let (reduced, reduced_file) = reduce(obj, file, ctx, depth + 1)?;
            member(&reduced, key, &reduced_file, ctx, depth + 1)
        }
        _ => None,
    }
}

/// Member `key` of an interface referenced with `args`, its type parameters bound to them.
fn interface_member(
    iface: &CollectedInterface,
    args: &[CollectedType],
    key: &str,
    args_file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<Located> {
    interface_prop(iface, args, key, args_file, ctx, depth).map(|prop| (prop.ty, prop.file))
}

/// A member as its interface declares it: the type with the interface's parameters bound, whether it is required, and
/// its documentation.
pub(super) struct DeclaredProp {
    pub(super) ty: CollectedType,
    pub(super) required: bool,
    pub(super) description: String,
    pub(super) file: Utf8PathBuf,
    /// The interface that declares it, which may be an ancestor of the one read.
    pub(super) declared_by: String,
}

/// Whether `name` (as written in `file`) is one of React's interfaces, which `react_declared_prop` can read.
pub(super) fn is_react_interface(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> bool {
    let (canonical_file, canonical_name) = canonical(name, file, ctx);
    react_interface(ctx, &canonical_file, &canonical_name).is_some()
}

/// React's own interface `name` (as written in `file`, `React.AriaAttributes` or `AriaAttributes`), read from its
/// declaration whether or not React's declarations were merged into the run (`html_attributes` other than `full`).
pub(super) fn react_declared_prop(
    name: &str,
    args: &[CollectedType],
    key: &str,
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<DeclaredProp> {
    let (canonical_file, canonical_name) = canonical(name, file, ctx);
    let iface = react_interface(ctx, &canonical_file, &canonical_name)?;
    interface_prop(iface, args, key, file, ctx, 0)
}

fn interface_prop(
    iface: &CollectedInterface,
    args: &[CollectedType],
    key: &str,
    args_file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<DeclaredProp> {
    let params = ctx
        .named_types
        .lookup_interface_type_params_for(iface)
        .or_else(|| ctx.react_types.as_deref()?.lookup_interface_type_params_for(iface));
    let defaults = ctx
        .named_types
        .lookup_interface_param_defaults_for(iface)
        .or_else(|| ctx.react_types.as_deref()?.lookup_interface_param_defaults_for(iface));
    let subst = match params {
        Some(params) if !params.is_empty() && (!args.is_empty() || defaults.is_some()) => {
            Some(bind_args(params, defaults, args, args_file, &iface.file_path, &mut Vec::new()))
        }
        _ => None,
    };
    // A member's type, or a parent's arguments (`extends StandardProperties<TLength>`), bound to `args`.
    let bind = |ty: &CollectedType| match &subst {
        Some(subst) => substitute_type(ty, subst),
        None => ty.clone(),
    };
    if let Some(prop) = iface.props.iter().find(|p| p.name == key) {
        return Some(DeclaredProp {
            ty: bind(&prop.collected_type),
            required: prop.required,
            description: prop.description.clone(),
            file: iface.file_path.clone(),
            declared_by: iface.name.to_string(),
        });
    }
    if depth > MAX_DEPTH {
        return None;
    }
    iface.extends.iter().find_map(|parent| {
        let (name, type_args) = match parent {
            ExtendsRef::SameFile { name, type_args } => (name, type_args),
            ExtendsRef::Imported { local_name, type_args, .. } => (local_name, type_args),
            // React's own interfaces name their parents by the spelling the extractor recognizes as React's
            // (`HTMLAttributes<T> extends AriaAttributes, DOMAttributes<T>`).
            ExtendsRef::Builtin { name, type_args, .. } => (name, type_args),
        };
        let (parent_file, parent_name) = canonical(name, &iface.file_path, ctx);
        let type_args: Vec<CollectedType> = type_args.iter().map(&bind).collect();
        match lookup_interface(ctx, parent_file.as_str(), &parent_name)
            .or_else(|| react_interface(ctx, &parent_file, &parent_name))
        {
            Some(parent) => interface_prop(parent, &type_args, key, &iface.file_path, ctx, depth + 1),
            // An interface may extend an alias (csstype's `StandardLonghandPropertiesFallback = Fallback<…>`).
            None => {
                let parent = CollectedType::Named { name: name.clone(), args: type_args };
                member(&parent, key, &iface.file_path, ctx, depth + 1).map(|(ty, file)| DeclaredProp {
                    ty,
                    required: false,
                    description: String::new(),
                    file,
                    declared_by: name.to_string(),
                })
            }
        }
    })
}

/// A type alias's definition with `args` bound to its parameters, as one type.
pub(super) fn alias_body(
    name: &CompactString,
    args: &[CollectedType],
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<Located> {
    let (alias_file, alias_name) = canonical(name, file, ctx);
    let (matched, alias) = lookup_type_alias(ctx, alias_file.as_str(), &alias_name)?;
    let defaults = ctx.named_types.lookup_type_param_defaults(&alias_file, &matched);
    let alias = match ctx.named_types.lookup_type_alias_params(&alias_file, &matched) {
        Some(params) if !args.is_empty() || defaults.is_some() => {
            substitute_alias(alias, &bind_args(params, defaults, args, file, &alias_file, &mut Vec::new()))
        }
        _ => alias.clone(),
    };
    let body = match alias {
        CollectedTypeAlias::Passthrough { target, .. } => target,
        CollectedTypeAlias::Intersection { members, .. } => CollectedType::Intersection(members),
        CollectedTypeAlias::Union { members, .. } => CollectedType::Union(members),
        CollectedTypeAlias::LiteralUnion { members, .. } => {
            CollectedType::Union(members.into_iter().map(|m| CollectedType::StringLiteral(m.into())).collect())
        }
        _ => return None,
    };
    Some((body, alias_file))
}

/// An interface of React's that curated mode indexes apart from the run's declarations (ADR 0013): reading a member
/// of one reads its declaration, as Full mode does.
fn react_interface<'c>(ctx: &'c ResolutionContext, file: &Utf8Path, name: &str) -> Option<&'c CollectedInterface> {
    ctx.react_index().lookup_interface(file, name)
}

fn canonical(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> (Utf8PathBuf, String) {
    if let Some(found) = resolve_to_canonical(name, file, ctx, &mut Vec::new()) {
        return found;
    }
    // `@types/react`'s UMD global (`export as namespace React`): `React.CSSProperties` without an import.
    match name.strip_prefix("React.") {
        Some(member) => (super::react::resolve_react_types_file(file, ctx).into(), member.to_owned()),
        None => (file.to_owned(), name.to_owned()),
    }
}

/// The string a literal type denotes.
fn literal(ty: &CollectedType) -> Option<String> {
    match ty {
        CollectedType::AtFile { inner, .. } => literal(inner),
        CollectedType::StringLiteral(s) => Some(s.to_string()),
        _ => None,
    }
}

/// The strings a literal or a union of literals denotes.
fn literal_set(ty: &CollectedType) -> Option<Vec<String>> {
    match ty {
        CollectedType::AtFile { inner, .. } => literal_set(inner),
        CollectedType::Union(members) => members.iter().map(literal).collect(),
        other => Some(vec![literal(other)?]),
    }
}

fn unwrap_at_file(ty: &CollectedType) -> (&CollectedType, Option<&Utf8Path>) {
    match ty {
        CollectedType::AtFile { file, inner } => (unwrap_at_file(inner).0, Some(file.as_path())),
        other => (other, None),
    }
}
