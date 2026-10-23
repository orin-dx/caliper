//! The type-level evaluation a props base needs before its props can be listed, when the operands are known: the branch
//! a conditional takes for a literal (`T extends keyof Recipes ? … : …` with `T = "button"`), a member read through
//! an indexed access (`Recipes["button"]["__type"]`, generic interface members included), and the keys a mapped type
//! ranges over (`{ [K in keyof Variant]?: … }`). Anything that needs a checker (an unknown operand, a generic left
//! unbound, an inferred type) evaluates to `None`, and the caller keeps its existing handling.

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
        CollectedType::IndexedAccess { obj, key } => member(obj, &literal(key)?, file, ctx, depth + 1),
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

/// Whether the literal `check` is assignable to `extends_type`; `None` when either side isn't known.
fn extends(
    check: &CollectedType,
    extends_type: &CollectedType,
    file: &Utf8Path,
    ctx: &ResolutionContext,
    depth: u8,
) -> Option<bool> {
    let value = literal(check)?;
    match unwrap_at_file(extends_type) {
        (CollectedType::KeyOf(target), target_file) => {
            let keys = keys_of(target, target_file.unwrap_or(file), ctx, depth + 1)?;
            Some(keys.iter().any(|key| key.name == value))
        }
        (CollectedType::String, _) => Some(true),
        (other, _) => Some(literal_set(other)?.contains(&value)),
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
fn member(obj: &CollectedType, key: &str, file: &Utf8Path, ctx: &ResolutionContext, depth: u8) -> Option<Located> {
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
            if let Some(iface) = lookup_interface_including_ambient(ctx, canonical_file.as_str(), &canonical_name) {
                return interface_member(iface, args, key, file, ctx, depth + 1);
            }
            let (body, body_file) = alias_body(name, args, file, ctx)?;
            member(&body, key, &body_file, ctx, depth + 1)
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
    if let Some(prop) = iface.props.iter().find(|p| p.name == key) {
        let ty = match ctx.named_types.lookup_interface_type_params_for(iface) {
            Some(params) if !params.is_empty() && !args.is_empty() => {
                substitute_type(&prop.collected_type, &build_substitution(params, args, args_file, &mut Vec::new()))
            }
            _ => prop.collected_type.clone(),
        };
        return Some((ty, iface.file_path.clone()));
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
        let (file, name) = canonical(name, &iface.file_path, ctx);
        let parent = lookup_interface(ctx, file.as_str(), &name)?;
        interface_member(parent, type_args, key, &iface.file_path, ctx, depth + 1)
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

fn canonical(name: &str, file: &Utf8Path, ctx: &ResolutionContext) -> (Utf8PathBuf, String) {
    resolve_to_canonical(name, file, ctx, &mut Vec::new()).unwrap_or_else(|| (file.to_owned(), name.to_owned()))
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
