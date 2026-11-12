//! Declaration order for a union the checker answered.
//!
//! The checker builds a union's members sorted by kind, then by value, so `"sm" | "md" | "lg"` comes back as
//! `"lg" | "md" | "sm"`; the order the author wrote is not in its answer, and it is the order a Storybook control lists
//! its options in. Two sources give it back, and the merge picks between them (`declared_order` in the parent module):
//!
//! - **oxc's syntax** ([`in_written_order`]): the union the prop's declaration writes, read from the file the checker
//!   names (`crate::extractor::written`). It orders the top-level union only, and a member written as a name the file
//!   does not declare (an import) places nothing: the members it stands for keep their place after the placed ones.
//! - **the structural resolver** ([`in_declared_order`]): where the structural pass resolved the same union, its order
//!   is the declared one, nested unions included. It goes with the resolver (ADR 0026, S7e).
//!
//! Only a reorder is made: no member is added, dropped or changed. A union neither source orders keeps the checker's
//! order (by kind, then value): not alphabetical, and not marked.

use crate::extractor::written::{MemberKind, WrittenMember};
use crate::types::output::{ObjectField, PropType};

/// `checker` with the members of every union it shares with `declared` put in `declared`'s order.
pub(super) fn in_declared_order(checker: PropType, declared: &PropType) -> PropType {
    match (checker, declared) {
        (PropType::Union(members), PropType::Union(order)) => PropType::Union(reorder(members, order)),
        // A union of string literals the structural pass keeps as their values (`type Size = "sm" | "lg"`).
        (PropType::Union(members), PropType::LiteralUnion { members: values, .. }) => {
            let order: Vec<PropType> = values.iter().map(|value| PropType::StringLiteral(value.clone())).collect();
            PropType::Union(reorder(members, &order))
        }
        (PropType::Array(inner), PropType::Array(declared)) => {
            PropType::Array(Box::new(in_declared_order(*inner, declared)))
        }
        (PropType::Readonly(inner), PropType::Readonly(declared)) => {
            PropType::Readonly(Box::new(in_declared_order(*inner, declared)))
        }
        (PropType::Intersection(members), PropType::Intersection(declared)) if members.len() == declared.len() => {
            PropType::Intersection(zip_ordered(members, declared))
        }
        (PropType::Tuple(elements), PropType::Tuple(declared)) if elements.len() == declared.len() => PropType::Tuple(
            elements
                .into_iter()
                .zip(declared)
                .map(|(mut element, declared)| {
                    element.element_type = in_declared_order(element.element_type, &declared.element_type);
                    element
                })
                .collect(),
        ),
        (PropType::Object(fields), PropType::Object(declared)) => PropType::Object(ordered_fields(fields, declared)),
        (
            PropType::Function { type_params, params, return_type },
            PropType::Function { params: declared_params, return_type: declared_return, .. },
        ) if params.len() == declared_params.len() => PropType::Function {
            type_params,
            params: params
                .into_iter()
                .zip(declared_params)
                .map(|(mut param, declared)| {
                    param.param_type = in_declared_order(param.param_type, &declared.param_type);
                    param
                })
                .collect(),
            return_type: Box::new(in_declared_order(*return_type, declared_return)),
        },
        (other, _) => other,
    }
}

/// Whether `declared`, the structural type, gives a place to any member of `checker`'s top-level union: then its order
/// is the declared one, whether or not it differs from the checker's.
pub(super) fn places_any(checker: &PropType, declared: &PropType) -> bool {
    let PropType::Union(members) = checker else { return false };
    match declared {
        PropType::Union(order) => members.iter().any(|member| position(member, order).is_some()),
        PropType::LiteralUnion { members: values, .. } => members.iter().any(|member| match member {
            PropType::StringLiteral(value) => values.contains(value),
            _ => false,
        }),
        _ => false,
    }
}

/// How many of a union's members the written order placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Placed {
    All,
    Some,
}

/// `checker`'s union with the members `written` states put in its order, then the others in their order in `checker`.
/// A member is placed by the written member that states it (a literal, a keyword, a name it answers to), else by kind
/// when the written union has one member of that kind and the checker one unplaced member of it. `None` when `checker`
/// is not a union or `written` places none of its members.
pub(super) fn in_written_order(checker: &PropType, written: &[WrittenMember]) -> Option<(PropType, Placed)> {
    let PropType::Union(members) = checker else { return None };
    let mut places: Vec<Option<usize>> =
        members.iter().map(|member| written.iter().position(|candidate| states(candidate, member))).collect();
    for index in 0..members.len() {
        if places[index].is_some() {
            continue;
        }
        let Some(kind) = kind_of(&members[index]) else { continue };
        let unplaced_of_kind = members
            .iter()
            .zip(&places)
            .filter(|(member, place)| place.is_none() && kind_of(member) == Some(kind))
            .count();
        let mut slots = written.iter().enumerate().filter(|(_, candidate)| **candidate == WrittenMember::Kind(kind));
        if let (Some((slot, _)), None) = (slots.next(), slots.next())
            && unplaced_of_kind == 1
        {
            places[index] = Some(slot);
        }
    }
    if places.iter().all(Option::is_none) {
        return None;
    }
    let placed = if places.iter().all(Option::is_some) { Placed::All } else { Placed::Some };
    let (mut known, unknown): (Vec<_>, Vec<_>) =
        members.iter().cloned().zip(places).partition(|(_, place)| place.is_some());
    known.sort_by_key(|(_, place)| *place);
    Some((PropType::Union(known.into_iter().chain(unknown).map(|(member, _)| member).collect()), placed))
}

/// Whether the written member `written` states the checker's member `member`.
fn states(written: &WrittenMember, member: &PropType) -> bool {
    match written {
        // `boolean` is `true | false` to the checker; the source may have written either.
        WrittenMember::Type(written) => {
            written == member || (*member == PropType::Boolean && matches!(written, PropType::BoolLiteral(_)))
        }
        WrittenMember::Name(name) => answers_to(member, name),
        WrittenMember::Kind(_) => false,
    }
}

/// Whether the checker's member is the type the source names `written` (`React.ReactNode`, `Size.Small`), by the name
/// without the namespace it was written through: the checker names the declaration.
fn answers_to(member: &PropType, written: &str) -> bool {
    let bare = |name: &str| name.rsplit('.').next().unwrap_or(name).to_owned();
    let written_bare = bare(written);
    match member {
        PropType::Named { name, .. } | PropType::Recursive { name, .. } => bare(name) == written_bare,
        PropType::EnumMember { enum_name, member, .. } => {
            written.strip_suffix(member.as_str()).and_then(|head| head.strip_suffix('.')).map(bare)
                == Some(bare(enum_name))
        }
        PropType::Ref { name, .. } => *name == written_bare,
        PropType::ReactNode => {
            matches!(written_bare.as_str(), "ReactNode" | "ReactElement") || written == "JSX.Element"
        }
        PropType::CssProperties => written_bare == "CSSProperties",
        PropType::ElementType => written_bare == "ElementType",
        _ => false,
    }
}

/// The kind a member written only by its kind has on the checker's side.
fn kind_of(member: &PropType) -> Option<MemberKind> {
    match member {
        PropType::Function { .. } => Some(MemberKind::Function),
        PropType::Callable { .. } => Some(MemberKind::Callable),
        PropType::Object(_) => Some(MemberKind::Object),
        PropType::Array(_) => Some(MemberKind::Array),
        PropType::Tuple(_) => Some(MemberKind::Tuple),
        PropType::Readonly(_) => Some(MemberKind::Readonly),
        PropType::Intersection(_) => Some(MemberKind::Intersection),
        PropType::TemplateLiteral(_) => Some(MemberKind::Template),
        _ => None,
    }
}

fn zip_ordered(members: Vec<PropType>, declared: &[PropType]) -> Vec<PropType> {
    members.into_iter().zip(declared).map(|(member, declared)| in_declared_order(member, declared)).collect()
}

fn ordered_fields(fields: Vec<ObjectField>, declared: &[ObjectField]) -> Vec<ObjectField> {
    fields
        .into_iter()
        .map(|mut field| {
            let counterpart = declared.iter().find(|candidate| {
                candidate.name == field.name && candidate.index_key.is_some() == field.index_key.is_some()
            });
            if let Some(counterpart) = counterpart {
                field.prop_type = in_declared_order(field.prop_type, &counterpart.prop_type);
            }
            field
        })
        .collect()
}

/// The members the declaration also has, in its order, then the members it does not have, in the checker's order: the
/// checker's place for them is its sort, not the source's, and a member the structural pass dropped is most often the
/// `undefined` or `null` written last. Unchanged when the declaration has none of them.
fn reorder(members: Vec<PropType>, declared: &[PropType]) -> Vec<PropType> {
    let places: Vec<Option<usize>> = members.iter().map(|member| position(member, declared)).collect();
    if places.iter().all(Option::is_none) {
        return members;
    }
    let (mut known, unknown): (Vec<_>, Vec<_>) =
        members.into_iter().zip(places).partition(|(_, place)| place.is_some());
    known.sort_by_key(|(_, place)| *place);
    known.into_iter().chain(unknown).map(|(member, _)| member).collect()
}

/// The place of the checker's `member` in the structural union `declared`.
fn position(member: &PropType, declared: &[PropType]) -> Option<usize> {
    let bare = |name: &str| name.rsplit('.').next().unwrap_or(name).to_owned();
    declared.iter().position(|candidate| candidate == member).or_else(|| match member {
        // `boolean` is `true | false` to the checker; the source may have written either.
        PropType::Boolean => declared.iter().position(|candidate| matches!(candidate, PropType::BoolLiteral(_))),
        // The source may name a type through a namespace (`React.Ref`); the checker names the declaration.
        PropType::Named { name, .. } => declared.iter().position(
            |candidate| matches!(candidate, PropType::Named { name: written, .. } if bare(written) == bare(name)),
        ),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(text: &str) -> PropType {
        PropType::StringLiteral(text.to_string())
    }

    fn union(members: &[&str]) -> PropType {
        PropType::Union(members.iter().map(|member| lit(member)).collect())
    }

    #[test]
    fn a_union_takes_the_order_it_was_declared_in() {
        assert_eq!(
            in_declared_order(union(&["lg", "md", "sm"]), &union(&["sm", "md", "lg"])),
            union(&["sm", "md", "lg"])
        );
    }

    #[test]
    fn a_union_inside_an_object_a_function_or_an_array_is_reordered_too() {
        let field = |name: &str, prop_type| ObjectField::new(name, prop_type, true, "");
        let checker = PropType::Object(vec![
            field("tags", PropType::Array(Box::new(union(&["b", "a"])))),
            field(
                "on",
                PropType::Function {
                    type_params: vec![],
                    params: vec![crate::types::FunctionParam::new(None, union(&["y", "x"]))],
                    return_type: Box::new(union(&["2", "1"])),
                },
            ),
        ]);
        let declared = PropType::Object(vec![
            field("tags", PropType::Array(Box::new(union(&["a", "b"])))),
            field(
                "on",
                PropType::Function {
                    type_params: vec![],
                    params: vec![crate::types::FunctionParam::new(None, union(&["x", "y"]))],
                    return_type: Box::new(union(&["1", "2"])),
                },
            ),
        ]);

        assert_eq!(in_declared_order(checker, &declared), declared);
    }

    #[test]
    fn a_member_the_declaration_lacks_goes_after_the_ones_it_has() {
        assert_eq!(in_declared_order(union(&["lg", "md", "sm"]), &union(&["sm", "md"])), union(&["sm", "md", "lg"]));
        // `p: "on" | "off" | null | undefined` on a required prop: the structural pass drops `undefined`, the checker
        // sorts it first.
        let checker = PropType::Union(vec![PropType::Undefined, lit("on"), lit("off"), PropType::Null]);
        let declared = PropType::Union(vec![lit("on"), lit("off"), PropType::Null]);
        assert_eq!(
            in_declared_order(checker, &declared),
            PropType::Union(vec![lit("on"), lit("off"), PropType::Null, PropType::Undefined])
        );
        // The checker keeps `string & {}` as a member; the structural result writes `string`.
        let checker = PropType::Union(vec![
            lit("green"),
            lit("red"),
            PropType::Intersection(vec![PropType::String, PropType::Object(vec![])]),
        ]);
        let declared = PropType::Union(vec![lit("red"), lit("green"), PropType::String]);
        assert_eq!(
            in_declared_order(checker, &declared),
            PropType::Union(vec![
                lit("red"),
                lit("green"),
                PropType::Intersection(vec![PropType::String, PropType::Object(vec![])]),
            ])
        );
    }

    #[test]
    fn a_union_of_string_literals_kept_as_values_gives_their_order() {
        let declared = PropType::LiteralUnion {
            members: vec!["small".into(), "medium".into(), "large".into()],
            has_default: false,
        };
        assert_eq!(
            in_declared_order(union(&["large", "medium", "small"]), &declared),
            union(&["small", "medium", "large"])
        );
    }

    #[test]
    fn a_named_member_is_found_by_its_name_without_the_namespace_it_was_written_through() {
        let named = |name: &str| PropType::Named { name: name.into(), args: vec![] };
        assert_eq!(
            in_declared_order(
                PropType::Union(vec![named("ForwardRefExoticComponent"), named("NamedExoticComponent")]),
                &PropType::Union(vec![named("React.NamedExoticComponent"), named("React.ForwardRefExoticComponent")]),
            ),
            PropType::Union(vec![named("NamedExoticComponent"), named("ForwardRefExoticComponent")])
        );
    }

    #[test]
    fn a_declaration_that_shares_no_member_changes_nothing() {
        assert_eq!(in_declared_order(union(&["b", "a"]), &union(&["x", "y"])), union(&["b", "a"]));
    }

    #[test]
    fn a_declaration_with_more_members_than_the_checker_kept_still_orders_the_ones_it_has() {
        assert_eq!(in_declared_order(union(&["b", "a"]), &union(&["a", "x", "b"])), union(&["a", "b"]));
    }

    #[test]
    fn boolean_takes_the_place_of_the_first_boolean_literal() {
        let checker = PropType::Union(vec![PropType::Boolean, lit("a")]);
        let declared = PropType::Union(vec![lit("a"), PropType::BoolLiteral(true), PropType::BoolLiteral(false)]);
        assert_eq!(in_declared_order(checker, &declared), PropType::Union(vec![lit("a"), PropType::Boolean]));
    }

    fn name(text: &str) -> WrittenMember {
        WrittenMember::Name(text.into())
    }

    fn written(members: &[&str]) -> Vec<WrittenMember> {
        members.iter().map(|member| WrittenMember::Type(lit(member))).collect()
    }

    #[test]
    fn the_written_union_gives_the_order_of_the_members_it_states() {
        assert_eq!(
            in_written_order(&union(&["lg", "md", "sm"]), &written(&["sm", "md", "lg"])),
            Some((union(&["sm", "md", "lg"]), Placed::All))
        );
    }

    #[test]
    fn a_member_written_as_an_imported_name_places_nothing_and_its_members_go_last() {
        // `p: 'z' | Size | 'a'` with `Size` imported: the checker flattened it to "b" and "c".
        let written = vec![WrittenMember::Type(lit("z")), name("Size"), WrittenMember::Type(lit("a"))];
        assert_eq!(
            in_written_order(&union(&["a", "b", "c", "z"]), &written),
            Some((union(&["z", "a", "b", "c"]), Placed::Some))
        );
        assert_eq!(in_written_order(&union(&["b", "c"]), &[name("Size"), name("Other")]), None);
    }

    #[test]
    fn a_member_written_by_kind_is_placed_when_its_kind_is_unique() {
        let function =
            PropType::Function { type_params: vec![], params: vec![], return_type: Box::new(PropType::Void) };
        let checker = PropType::Union(vec![PropType::ReactNode, function.clone()]);
        let written = vec![WrittenMember::Kind(MemberKind::Function), name("React.ReactNode")];
        assert_eq!(
            in_written_order(&checker, &written),
            Some((PropType::Union(vec![function, PropType::ReactNode]), Placed::All))
        );
        // Two objects written and two answered: which is which is not known, so neither is placed.
        let object = |field: &str| PropType::Object(vec![ObjectField::new(field, PropType::String, true, "")]);
        let checker = PropType::Union(vec![object("b"), object("a"), lit("x")]);
        let written = vec![
            WrittenMember::Kind(MemberKind::Object),
            WrittenMember::Kind(MemberKind::Object),
            WrittenMember::Type(lit("x")),
        ];
        assert_eq!(
            in_written_order(&checker, &written),
            Some((PropType::Union(vec![lit("x"), object("b"), object("a")]), Placed::Some))
        );
    }

    #[test]
    fn an_enum_member_and_boolean_answer_to_what_the_source_wrote() {
        let member = |name: &str, value: &str| PropType::EnumMember {
            enum_name: "Size".into(),
            member: name.into(),
            value: crate::types::output::EnumMemberValue::String(value.into()),
        };
        let checker = PropType::Union(vec![member("Large", "large"), member("Small", "small"), PropType::Boolean]);
        let written = vec![
            WrittenMember::Type(PropType::BoolLiteral(false)),
            name("Size.Small"),
            name("Size.Large"),
            WrittenMember::Type(PropType::BoolLiteral(true)),
        ];
        assert_eq!(
            in_written_order(&checker, &written),
            Some((
                PropType::Union(vec![PropType::Boolean, member("Small", "small"), member("Large", "large")]),
                Placed::All
            ))
        );
    }

    #[test]
    fn a_type_that_is_no_union_is_not_ordered() {
        assert_eq!(in_written_order(&lit("a"), &written(&["a", "b"])), None);
    }

    #[test]
    fn a_prop_the_structural_pass_left_opaque_is_not_reordered() {
        let checker = union(&["b", "a"]);
        assert_eq!(in_declared_order(checker.clone(), &PropType::Any), checker);
        assert_eq!(in_declared_order(checker.clone(), &lit("a")), checker);
    }
}
