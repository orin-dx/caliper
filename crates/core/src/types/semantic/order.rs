//! Declaration order for a union the checker answered.
//!
//! The checker builds a union's members sorted by kind, then by value, so `"sm" | "md" | "lg"` comes back as
//! `"lg" | "md" | "sm"`; the order the author wrote is not in its answer, and it is the order a Storybook control lists
//! its options in. The structural pass reads the same union from the type node and keeps the order it was written in
//! (`PropType::Union`: "the members in declaration order"). So where both resolved the same union, the structural
//! order is the declared one.
//!
//! It applies when the structural pass resolved the prop: a union in a file the extractor parsed, or one from a
//! dependency that was loaded. A union the structural pass left opaque, or a prop it has no entry for, keeps the
//! checker's order and carries no marker. Only a reorder is made: no member is added, dropped or changed. The checker's
//! answer is not in the source's order, so oxc's syntax would have to supply it once the structural pass is gone, and
//! the checker names no position to find a declaration's type node by (ADR 0026, S7a).

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
    let bare = |name: &str| name.rsplit('.').next().unwrap_or(name).to_owned();
    let position = |member: &PropType| {
        declared.iter().position(|candidate| candidate == member).or_else(|| match member {
            // `boolean` is `true | false` to the checker; the source may have written either.
            PropType::Boolean => declared.iter().position(|candidate| matches!(candidate, PropType::BoolLiteral(_))),
            // The source may name a type through a namespace (`React.Ref`); the checker names the declaration.
            PropType::Named { name, .. } => declared.iter().position(
                |candidate| matches!(candidate, PropType::Named { name: written, .. } if bare(written) == bare(name)),
            ),
            _ => None,
        })
    };
    let places: Vec<Option<usize>> = members.iter().map(position).collect();
    if places.iter().all(Option::is_none) {
        return members;
    }
    let (mut known, unknown): (Vec<_>, Vec<_>) =
        members.into_iter().zip(places).partition(|(_, place)| place.is_some());
    known.sort_by_key(|(_, place)| *place);
    known.into_iter().chain(unknown).map(|(member, _)| member).collect()
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

    #[test]
    fn a_prop_the_structural_pass_left_opaque_is_not_reordered() {
        let checker = union(&["b", "a"]);
        assert_eq!(in_declared_order(checker.clone(), &PropType::Any), checker);
        assert_eq!(in_declared_order(checker.clone(), &lit("a")), checker);
    }
}
