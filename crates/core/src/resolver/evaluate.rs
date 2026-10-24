//! The type-level evaluation a props base needs before its props can be listed, when the operands are known: the branch
//! a conditional takes for a literal (`T extends keyof Recipes ? … : …` with `T = "button"`), a member read through
//! an indexed access (`Recipes["button"]["__type"]`, generic interface members included), and the keys a mapped type
//! ranges over (`{ [K in keyof Variant]?: … }`). Anything that needs a checker (an unknown operand, a generic left
//! unbound, an inferred type) evaluates to `None`, and the caller keeps its existing handling.
//!
//! Its `depth` counts only its own steps: it reads declarations and never re-enters the resolver, so callers start it
//! at 0 instead of passing the resolver's depth, which a deep chain (csstype's `Fallback<…>` under Chakra's style
//! props) would exhaust first.

use camino::{Utf8Path, Utf8PathBuf};
use compact_str::CompactString;

use crate::types::*;

use super::import::{lookup_interface, lookup_interface_including_ambient, lookup_type_alias, resolve_to_canonical};
use super::substitute::{build_substitution, substitute_alias, substitute_type};
use super::{ResolutionContext, MAX_DEPTH};

/// A type, and the file its names resolve in.
pub(super) type Located = (CollectedType, Utf8PathBuf);

/// One key a mapped type ranges over, with what its source member says (a homomorphic `keyof X` keeps X's
/// optionality and documentation).
pub(super) struct Key {
    pub(super) name: String,
    pub(super) required: bool,
    pub(super) description: String,
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
            let branch = if extends(check, extends_type, file, ctx, depth)? { true_type } else { false_type };
            Some((branch.as_ref().clone(), file.to_owned()))
        }
        // Same depth: `member` counts its own steps, and at 0 it answers from the run's cache.
        CollectedType::IndexedAccess { obj, key } => member(obj, &literal(key)?, file, ctx, depth),
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
    match constraint {
        CollectedType::AtFile { file, inner } => mapped_keys(inner, file, ctx, depth + 1),
        CollectedType::KeyOf(target) => keys_of(target, file, ctx, depth + 1),
        other => Some(
            literal_set(other)?
                .into_iter()
                .map(|name| Key { name, required: true, description: String::new() })
                .collect(),
        ),
    }
}

/// Whether `check` is assignable to `extends_type`; `None` when either side isn't known. `check` is a literal, or a
/// member read whose type is a set of literals (`Factory["variant"] extends string`): a member read is not a naked
/// type parameter, so the conditional doesn't distribute and the whole set must be assignable.
fn extends(
    check: &CollectedType,
    extends_type: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<bool> {
    let values = check_values(check, file, ctx, depth + 1)?;
    let literal_value = |value: &CheckValue| match value {
        CheckValue::Literal(s) => Some(s.clone()),
        CheckValue::String | CheckValue::Undefined => None,
    };
    match unwrap_at_file(extends_type) {
        (CollectedType::KeyOf(target), target_file) => {
            let keys = keys_of(target, target_file.unwrap_or(file), ctx, depth + 1)?;
            Some(values.iter().all(|value| literal_value(value).is_some_and(|v| keys.iter().any(|key| key.name == v))))
        }
        (CollectedType::String, _) => Some(values.iter().all(|value| !matches!(value, CheckValue::Undefined))),
        (other, _) => {
            let allowed = literal_set(other)?;
            Some(values.iter().all(|value| literal_value(value).is_some_and(|v| allowed.contains(&v))))
        }
    }
}

/// One type a conditional's check can stand for.
enum CheckValue {
    Literal(String),
    String,
    /// A member the object type doesn't declare, which reads as nothing assignable to a string.
    Undefined,
}

/// What a conditional's check stands for: one literal, or the types a member read's type lists.
fn check_values(check: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Vec<CheckValue>> {
    match check {
        CollectedType::AtFile { file, inner } => check_values(inner, file, ctx, depth + 1),
        CollectedType::IndexedAccess { obj, key } => match reduce(check, file, ctx, depth + 1) {
            Some((member, member_file)) => string_values(&member, &member_file, ctx, depth + 1),
            // `Payload["variant"]` on a payload object that declares no `variant` (Mantine's factories).
            None if lacks_member(obj, &literal(key)?, file, ctx, depth + 1) => Some(vec![CheckValue::Undefined]),
            None => None,
        },
        other => Some(vec![CheckValue::Literal(literal(other)?)]),
    }
}

/// The string types `ty` is a union of, following aliases; `None` when any member is something else.
fn string_values(ty: &CollectedType, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Vec<CheckValue>> {
    if depth > MAX_DEPTH {
        return None;
    }
    match ty {
        CollectedType::AtFile { file, inner } => string_values(inner, file, ctx, depth + 1),
        CollectedType::StringLiteral(s) => Some(vec![CheckValue::Literal(s.to_string())]),
        CollectedType::String => Some(vec![CheckValue::String]),
        CollectedType::Union(members) => {
            let mut all = Vec::new();
            for member in members {
                all.extend(string_values(member, file, ctx, depth + 1)?);
            }
            Some(all)
        }
        CollectedType::Named { name, args } => {
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            string_values(&body, &body_file, ctx, depth + 1)
        }
        _ => None,
    }
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
                .map(|f| Key { name: f.name.clone(), required: f.required, description: f.description.clone() })
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
            keys.push(Key { name: prop.name.clone(), required: prop.required, description: prop.description.clone() });
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
    // A read from the top of a walk depends only on the declarations, so a run answers it once (`member_reads`).
    if let (CollectedType::Named { name, args }, 0) = (obj, depth) {
        let name: Box<str> = if args.is_empty() { Box::from(name.as_str()) } else { obj.to_raw_string().into() };
        let cache_key = (Box::from(file.as_str()), name, Box::from(key));
        if let Some(read) = ctx.member_reads.get(&cache_key) {
            super::deps::replay(&read.deps);
            return read.found.clone();
        }
        let (found, deps) = super::deps::capturing(|| member_uncached(obj, key, file, ctx, depth));
        ctx.member_reads.insert(cache_key, MemberRead { found: found.clone(), deps });
        return found;
    }
    member_uncached(obj, key, file, ctx, depth)
}

/// `(file, type as written, key)` of a cached member read.
pub(crate) type MemberReadKey = (Box<str>, Box<str>, Box<str>);

/// A cached `member` answer and the files it read.
pub(crate) struct MemberRead {
    found: Option<Located>,
    deps: super::deps::Dependencies,
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
    let params = ctx
        .named_types
        .lookup_interface_type_params_for(iface)
        .or_else(|| ctx.react_types.as_deref()?.lookup_interface_type_params_for(iface));
    let subst = match params {
        Some(params) if !params.is_empty() && !args.is_empty() => {
            Some(build_substitution(params, args, args_file, &mut Vec::new()))
        }
        _ => None,
    };
    // A member's type, or a parent's arguments (`extends StandardProperties<TLength>`), bound to `args`.
    let bind = |ty: &CollectedType| match &subst {
        Some(subst) => substitute_type(ty, subst),
        None => ty.clone(),
    };
    if let Some(prop) = iface.props.iter().find(|p| p.name == key) {
        return Some((bind(&prop.collected_type), iface.file_path.clone()));
    }
    if depth > MAX_DEPTH {
        return None;
    }
    iface.extends.iter().find_map(|parent| {
        let (name, type_args) = match parent {
            ExtendsRef::SameFile { name, type_args } => (name, type_args),
            ExtendsRef::Imported { local_name, type_args, .. } => (local_name, type_args),
            ExtendsRef::Builtin { .. } => return None,
        };
        let (parent_file, parent_name) = canonical(name, &iface.file_path, ctx);
        let type_args: Vec<CollectedType> = type_args.iter().map(&bind).collect();
        match lookup_interface(ctx, parent_file.as_str(), &parent_name)
            .or_else(|| react_interface(ctx, &parent_file, &parent_name))
        {
            Some(parent) => interface_member(parent, &type_args, key, &iface.file_path, ctx, depth + 1),
            // An interface may extend an alias (csstype's `StandardLonghandPropertiesFallback = Fallback<…>`).
            None => {
                let parent = CollectedType::Named { name: name.clone(), args: type_args };
                member(&parent, key, &iface.file_path, ctx, depth + 1)
            }
        }
    })
}

/// A type alias's definition with `args` bound to its parameters, as one type.
fn alias_body(
    name: &CompactString,
    args: &[CollectedType],
    file: &Utf8Path,
    ctx: &ResolutionContext,
) -> Option<Located> {
    let (alias_file, alias_name) = canonical(name, file, ctx);
    let (matched, alias) = lookup_type_alias(ctx, alias_file.as_str(), &alias_name)?;
    let alias = match ctx.named_types.lookup_type_alias_params(&alias_file, &matched) {
        Some(params) if !args.is_empty() => {
            substitute_alias(alias, &build_substitution(params, args, file, &mut Vec::new()))
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
    ctx.react_types.as_deref()?.lookup_interface(file, name)
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
