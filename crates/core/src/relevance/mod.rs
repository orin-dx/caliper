//! Which props of a component evidence puts forward (ADR 0028, section A).
//!
//! [`decide`] is the one place the rules are combined: it takes the [`Evidence`] gathered for the members a
//! component's props type reaches and says, for each member, whether it is a typed row, a name in its collapsed
//! layer's `used` list, left in that layer, or unknown because the body says nothing. It is pure and total: no I/O, no
//! name lists, and its output follows the order of the members it is given.
//!
//! [`apply`] is its one caller, where a structural answer becomes output (`pipeline::revision`): the [`Inputs`]
//! gathered with the answer's resolution (so their reads are its dependencies) and the revision's call-site counts
//! (counted again each revision, so an edit to a consumer reaches every component it renders). The checker's merges
//! keep the inheritance layers, and so the `used` lists, of the structural answer.

pub(crate) mod call_sites;
pub(crate) mod forwarding;
pub(crate) mod props_type;

use std::collections::BTreeSet;

use crate::types::output::{ComponentEntry, InheritedLayer, PropParent};

/// The evidence about one component gathered with its resolution: what its props type writes and what its body does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// The component's place in the project, which its call-site count is looked up by.
    pub(crate) site: call_sites::SiteKey,
    /// What the props type writes itself.
    pub(crate) props_type: props_type::PropsType,
    /// What the component's body does with its props; `None` when it has no function of its own.
    pub(crate) body: Option<Body>,
    /// Whether the listing follows the decision ([`narrow`]): off when every inherited prop is asked for expanded
    /// (`collapseInheritedProps: none`, ADR 0028's `relevance: 'all'`); the `used` lists are written either way.
    pub(crate) narrows: bool,
}

/// Decides where each member of `entry`'s props goes, from `inputs`, the revision's `call_sites` of the component and
/// the members its listed props and layers reach; keeps the decision on the entry ([`ComponentEntry::relevance`]) and
/// lists by it ([`narrow`]).
pub(crate) fn apply(entry: &mut ComponentEntry, inputs: &Inputs, call_sites: &CallSites) {
    let rows = entry.props.values().chain(entry.notable_inherited.values()).map(AsRef::as_ref);
    let members = props_type::members(&inputs.props_type, rows, &entry.inheritance);
    let shown = decide(&Evidence { members: &members, body: inputs.body.as_ref(), call_sites });
    // A collapsed ambient member the signature defaults (`{ className = 'x' }`) is read by the body, which a row does
    // not record for a name left in its layer: it is `used` too. Nothing else in `rows` is added.
    let defaulted =
        members.iter().filter(|member| member.ambient && member.default_tag).map(|member| member.name.clone());
    // A row of a member the answer holds only as a name in a collapsed layer has no type to list: it is named in the
    // layer's `used` list instead, so a member evidence puts forward is never left unnamed.
    let untyped = shown
        .rows
        .iter()
        .filter(|name| !entry.props.contains_key(*name) && !entry.notable_inherited.contains_key(*name))
        .cloned();
    let used: BTreeSet<String> = shown.used.iter().cloned().chain(defaulted).chain(untyped).collect();
    let left =
        if inputs.narrows { shown.collapsed.iter().chain(&shown.used).cloned().collect() } else { BTreeSet::new() };
    entry.relevance = Some(std::sync::Arc::new(Placement { left, used }));
    narrow(entry);
}

/// What [`decide`] said of one component, kept on its entry so that every merge of the checker's props into it lists by
/// the same decision ([`narrow`]): the decision is made once, from the structural answer's evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// The members evidence leaves out of the listing ([`Shown::collapsed`] and [`Shown::used`]): each is a name in
    /// the layer of its source, never a row.
    left: BTreeSet<String>,
    /// The members of `left` named in their layer's `used` list.
    used: BTreeSet<String>,
}

/// Lists `entry` by its kept [`Placement`]: a listed prop (in `props` or `notableInherited`) the decision leaves out
/// and that a source outside the project declares (a dependency's, or React's and the DOM's) moves to the layer of
/// its source, as a name. Its home is the layer of its `parent` (by type name and file), else,
/// for an ambient source, the entry's HTML element layer, else a new layer of its `parent`. A prop with no `parent`
/// has no known source and stays listed: no name is ever lost. Then each layer's `used` list is written. Without a
/// placement (no evidence was gathered) nothing changes. Run by [`apply`] and at the end of each merge of the
/// checker's props, which may list again what the structural answer left out.
pub(crate) fn narrow(entry: &mut ComponentEntry) {
    let Some(placement) = entry.relevance.clone() else { return };
    let ComponentEntry { props, notable_inherited, inheritance, .. } = entry;
    for listing in [props, notable_inherited] {
        listing.retain(|name, prop| {
            if !placement.left.contains(name) {
                return true;
            }
            // Only a source outside the project is narrowed: a dependency's or an ambient one, decided by the
            // declaring path. The project's own sources stay as the collapse rule lists them.
            let Some(parent) = prop.parent.as_ref() else { return true };
            if crate::origin::declaration_origin(&parent.file_name) == crate::origin::Origin::Project {
                return true;
            }
            if !inheritance.iter().any(|layer| layer.members.contains(name)) {
                home(inheritance, parent).members.push(name.clone());
            }
            false
        });
    }
    for layer in inheritance.iter_mut() {
        layer.total_props = layer.total_props.max(u32::try_from(layer.members.len()).unwrap_or(u32::MAX));
        layer.used = layer.members.iter().filter(|name| placement.used.contains(*name)).cloned().collect();
    }
}

/// The layer a prop declared in `parent` is left in ([`narrow`]): made when the entry has none.
fn home<'l>(layers: &'l mut Vec<InheritedLayer>, parent: &PropParent) -> &'l mut InheritedLayer {
    let ambient = crate::origin::declaration_origin(&parent.file_name).is_ambient();
    let at = layers
        .iter()
        .position(|layer| layer.type_name == parent.name && layer.file_name == parent.file_name)
        .or_else(|| {
            ambient
                .then(|| {
                    layers.iter().position(|layer| {
                        layer.html_element.is_some() && crate::origin::declaration_origin(&layer.file_name).is_ambient()
                    })
                })
                .flatten()
        });
    let at = at.unwrap_or_else(|| {
        layers.push(InheritedLayer {
            type_name: parent.name.clone(),
            file_name: parent.file_name.clone(),
            omitted: Vec::new(),
            html_element: None,
            total_props: 0,
            members: Vec::new(),
            used: Vec::new(),
        });
        layers.len() - 1
    });
    &mut layers[at]
}

/// Everything known about the members one component's props type reaches.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Evidence<'a> {
    /// The members, in the order the props type reaches them; [`Shown`] keeps this order. A name repeated is placed
    /// once, by its first member.
    pub(crate) members: &'a [Member],
    /// What the component's own function body does with its props; `None` when it has no function of its own.
    pub(crate) body: Option<&'a Body>,
    /// The props passed at the component's JSX call sites across the project.
    pub(crate) call_sites: &'a CallSites,
}

/// One member the props type reaches, with the signals its declaration gives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Member {
    /// The member's name. Only compared with other names in the same [`Evidence`], never with a list.
    pub(crate) name: String,
    /// The props type declares it itself, not through an inherited source.
    pub(crate) own: bool,
    /// A key a `Pick<X, …>` names.
    pub(crate) picked: bool,
    /// It redeclares or narrows a base member.
    pub(crate) overrides: bool,
    /// It is not optional.
    pub(crate) required: bool,
    /// Its declaration carries a `@default` tag.
    pub(crate) default_tag: bool,
    /// It carries a JSDoc comment: orders the rows, never shows a member.
    pub(crate) documented: bool,
    /// Its type is a union of literals: orders the rows, never shows a member.
    pub(crate) literal_union: bool,
    /// Declared by React's or the shared DOM declarations ([`crate::origin::Origin::is_ambient`]): decided by
    /// the declaring path, never by the name.
    pub(crate) ambient: bool,
}

/// What a component's function body does with its props, read from the syntax alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Body {
    /// The props the body reads. Complete only when [`Body::is_complete`].
    pub(crate) reads: Vec<String>,
    /// The JSX elements the body writes, in source order.
    pub(crate) elements: Vec<Element>,
    /// The props object or a rest binding escapes in a way the syntax cannot follow: what the body says still holds,
    /// but its silence about a prop is unknown, never guessed.
    pub(crate) handed_on: bool,
}

/// One JSX element a component body writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Element {
    /// What the tag resolves to.
    pub(crate) target: Target,
    /// Its attributes in source order, so a named attribute's place before or after a spread is kept.
    pub(crate) attributes: Vec<Attribute>,
}

/// What a JSX tag resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A host element: a spread onto it passes only ambient DOM attributes.
    Dom,
    /// A component resolved by symbol and import, with the names of its own non-ambient props.
    Component(Vec<String>),
    /// A component tag that did not resolve: a spread onto it leaves the body's evidence unknown.
    Unresolved,
}

/// One attribute of a JSX element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Attribute {
    /// `name={value}`: set after a spread, it fixes `name`; its value adds nothing.
    Named(String),
    /// `{...value}`.
    Spread(SpreadOf),
}

/// What a JSX spread attribute spreads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpreadOf {
    /// The props object, whole.
    Props,
    /// A rest binding of the props object.
    Rest,
    /// Any other expression: no evidence about the props.
    Other,
}

/// How often each prop is passed at a component's JSX call sites.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallSites {
    /// The number of call sites.
    pub(crate) sites: u32,
    /// Each prop passed at them, with the number of sites passing it.
    pub(crate) passed: Vec<(String, u32)>,
}

/// No call sites: no evidence.
pub(crate) static NO_CALL_SITES: CallSites = CallSites { sites: 0, passed: Vec::new() };

impl CallSites {
    /// Whether `count` of the sites reach the floor of `max(2, 20%)` of them.
    fn at_floor(&self, count: u32) -> bool {
        count >= 2 && u64::from(count) * 5 >= u64::from(self.sites)
    }
}

/// Where [`decide`] puts each member, every list in the order of [`Evidence::members`], except that `rows` puts the
/// documented and literal-union members first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Shown {
    /// Typed rows.
    pub(crate) rows: Vec<String>,
    /// Ambient members named in their collapsed layer's `used` list: passed at the floor or read.
    pub(crate) used: Vec<String>,
    /// Members left in their collapsed layer.
    pub(crate) collapsed: Vec<String>,
    /// Members with no signal whose body evidence is incomplete or absent: listed or collapsed by the collapse rule
    /// alone, so missing evidence never hides more than that rule does.
    pub(crate) unknown: Vec<String>,
}

/// Decides, from the evidence alone, where each member of a component's props goes.
pub(crate) fn decide(evidence: &Evidence<'_>) -> Shown {
    let body = evidence.body;
    let complete = body.is_some_and(Body::is_complete);
    let reads: BTreeSet<&str> = body.map(|body| body.reads.iter().map(String::as_str).collect()).unwrap_or_default();
    let forwarded = body.map(Body::forwarded).unwrap_or_default();
    let at_floor: BTreeSet<&str> = evidence
        .call_sites
        .passed
        .iter()
        .filter(|(_, count)| evidence.call_sites.at_floor(*count))
        .map(|(name, _)| name.as_str())
        .collect();
    let mut shown = Shown::default();
    let mut ordered_rows = Vec::new();
    let mut placed = BTreeSet::new();
    for member in evidence.members {
        let name = member.name.as_str();
        if !placed.insert(name) {
            continue;
        }
        let declared = member.own || member.picked || member.overrides || member.required || member.default_tag;
        // Passed at the floor puts a member forward as a typed row, ambient or not: consumers set it, so a story needs
        // its control. Read or forwarded puts a non-ambient member forward as a row; an ambient member the body only
        // reads is a name in its layer's `used` list (ADR 0028, Combination).
        let passed = at_floor.contains(name);
        let read = reads.contains(name);
        let list = if declared || passed || (!member.ambient && (read || forwarded.contains(name))) {
            ordered_rows.push((!(member.documented || member.literal_union), member.name.clone()));
            continue;
        } else if member.ambient && read {
            &mut shown.used
        } else if !complete && !member.ambient {
            // The body's silence is unknown: listed or collapsed as the collapse rule alone says. An ambient member
            // (React's and the DOM's attributes, which every element has) is never listed for want of evidence: it
            // stays a name in its layer, so a component without call sites lists no hundreds of DOM attributes.
            &mut shown.unknown
        } else {
            &mut shown.collapsed
        };
        list.push(member.name.clone());
    }
    ordered_rows.sort_by_key(|(plain, _)| *plain);
    shown.rows = ordered_rows.into_iter().map(|(_, name)| name).collect();
    shown
}

impl Body {
    /// Whether the body's evidence is complete, so that a prop it never mentions is known to be unused: nothing handed
    /// on, and no spread of the props onto a tag that did not resolve. What an incomplete body does say (a read, a
    /// default, a forward) is still certain; only its silence is unknown.
    fn is_complete(&self) -> bool {
        !self.handed_on
            && !self.elements.iter().any(|element| {
                element.target == Target::Unresolved
                    && element.attributes.iter().any(|attribute| attribute.spreads_props())
            })
    }

    /// The props a spread of the props object or a rest binding passes one hop on to a component: the component's own
    /// props, minus those an attribute after the spread fixes. Spreads onto DOM tags pass nothing.
    fn forwarded(&self) -> BTreeSet<&str> {
        let mut forwarded = BTreeSet::new();
        for element in &self.elements {
            let Target::Component(own) = &element.target else { continue };
            for (at, attribute) in element.attributes.iter().enumerate() {
                if !attribute.spreads_props() {
                    continue;
                }
                let fixed: BTreeSet<&str> = element.attributes[at + 1..]
                    .iter()
                    .filter_map(|after| match after {
                        Attribute::Named(name) => Some(name.as_str()),
                        Attribute::Spread(_) => None,
                    })
                    .collect();
                forwarded.extend(own.iter().map(String::as_str).filter(|name| !fixed.contains(name)));
            }
        }
        forwarded
    }
}

impl Attribute {
    /// Whether the attribute spreads the props object or a rest binding of it.
    fn spreads_props(&self) -> bool {
        matches!(self, Self::Spread(SpreadOf::Props | SpreadOf::Rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a member went, for the tables.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Placed {
        Row,
        Used,
        Collapsed,
        Unknown,
    }

    fn inherited(name: &str) -> Member {
        Member { name: name.to_owned(), ..Member::default() }
    }

    fn ambient(name: &str) -> Member {
        Member { ambient: true, ..inherited(name) }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn reading(names: &[&str]) -> Body {
        Body { reads: strings(names), ..Body::default() }
    }

    fn named(name: &str) -> Attribute {
        Attribute::Named(name.to_owned())
    }

    fn element(target: Target, attributes: Vec<Attribute>) -> Element {
        Element { target, attributes }
    }

    /// The evidence of one case, owned: [`Evidence`] borrows it.
    #[derive(Default)]
    struct Owned {
        members: Vec<Member>,
        body: Option<Body>,
        call_sites: CallSites,
    }

    impl Owned {
        fn decide(&self) -> Shown {
            decide(&Evidence { members: &self.members, body: self.body.as_ref(), call_sites: &self.call_sites })
        }
    }

    fn with(members: Vec<Member>, body: Option<Body>) -> Owned {
        Owned { members, body, ..Owned::default() }
    }

    fn placed(shown: &Shown, name: &str) -> Option<Placed> {
        let lists = [
            (&shown.rows, Placed::Row),
            (&shown.used, Placed::Used),
            (&shown.collapsed, Placed::Collapsed),
            (&shown.unknown, Placed::Unknown),
        ];
        lists.into_iter().find(|(list, _)| list.iter().any(|n| n == name)).map(|(_, placed)| placed)
    }

    /// One case of a table: its label, its evidence, and where each named member must go.
    type Case<'a> = (&'a str, Owned, &'a [(&'a str, Placed)]);

    /// Runs each case and checks where every named member went.
    fn table(cases: Vec<Case<'_>>) {
        for (case, evidence, expected) in cases {
            let shown = evidence.decide();
            for (name, want) in expected {
                assert_eq!(placed(&shown, name), Some(*want), "{case}: {name}");
            }
        }
    }

    #[test]
    fn an_inherited_member_the_body_never_reads_stays_in_its_layer() {
        let evidence = with(vec![inherited("tone"), inherited("size")], Some(reading(&["size"])));
        let shown = evidence.decide();
        assert_eq!(shown.collapsed, ["tone"]);
        assert_eq!(shown.rows, ["size"]);
    }

    #[test]
    fn a_member_the_props_type_declares_itself_is_always_a_row() {
        let own = |member: Member| Member { own: true, ..member };
        table(vec![
            ("never read", with(vec![own(inherited("a"))], Some(Body::default())), &[("a", Placed::Row)]),
            ("unknown body", with(vec![own(inherited("a"))], None), &[("a", Placed::Row)]),
            ("ambient", with(vec![own(ambient("a"))], Some(Body::default())), &[("a", Placed::Row)]),
        ]);
    }

    #[test]
    fn a_picked_key_is_a_row_and_its_unpicked_siblings_are_not() {
        let picked = Member { picked: true, ..inherited("a") };
        table(vec![(
            "Pick<X, 'a'>",
            with(vec![picked, inherited("b")], Some(Body::default())),
            &[("a", Placed::Row), ("b", Placed::Collapsed)],
        )]);
    }

    #[test]
    fn a_member_that_overrides_or_narrows_a_base_member_is_a_row() {
        let overriding = |member: Member| Member { overrides: true, ..member };
        table(vec![
            ("inherited", with(vec![overriding(inherited("a"))], Some(Body::default())), &[("a", Placed::Row)]),
            ("ambient", with(vec![overriding(ambient("a"))], Some(Body::default())), &[("a", Placed::Row)]),
        ]);
    }

    #[test]
    fn a_required_or_defaulted_member_is_a_row() {
        let required = Member { required: true, ..inherited("a") };
        let defaulted = Member { default_tag: true, ..inherited("b") };
        table(vec![(
            "required, defaulted (a `@default` tag or a destructuring default)",
            with(vec![required, defaulted, inherited("d")], Some(Body::default())),
            &[("a", Placed::Row), ("b", Placed::Row), ("d", Placed::Collapsed)],
        )]);
    }

    #[test]
    fn a_doc_comment_or_literal_union_orders_rows_but_never_shows_a_member_alone() {
        let documented = |name: &str| Member { documented: true, ..inherited(name) };
        let union = |name: &str| Member { literal_union: true, ..inherited(name) };
        table(vec![(
            "alone",
            with(vec![documented("a"), union("b"), Member { documented: true, ..ambient("c") }], Some(Body::default())),
            &[("a", Placed::Collapsed), ("b", Placed::Collapsed), ("c", Placed::Collapsed)],
        )]);
        let evidence = with(vec![inherited("plain"), union("u"), documented("d")], Some(reading(&["plain", "u", "d"])));
        assert_eq!(evidence.decide().rows, ["u", "d", "plain"]);
    }

    #[test]
    fn a_rest_spread_onto_a_dom_tag_is_no_evidence() {
        let body =
            Body { elements: vec![element(Target::Dom, vec![Attribute::Spread(SpreadOf::Rest)])], ..Body::default() };
        table(vec![(
            "<div {...rest} />",
            with(vec![inherited("a"), ambient("onClick")], Some(body)),
            &[("a", Placed::Collapsed), ("onClick", Placed::Collapsed)],
        )]);
    }

    #[test]
    fn a_spread_onto_a_component_forwards_its_own_props_one_hop() {
        let inner = || Target::Component(strings(&["tone", "size", "variant"]));
        let spread =
            |attributes: Vec<Attribute>| Body { elements: vec![element(inner(), attributes)], ..Body::default() };
        let members = || vec![inherited("tone"), inherited("size"), inherited("variant"), inherited("other")];
        let rest = || Attribute::Spread(SpreadOf::Rest);
        table(vec![
            (
                "<Inner {...rest} />",
                with(members(), Some(spread(vec![rest()]))),
                &[("tone", Placed::Row), ("size", Placed::Row), ("other", Placed::Collapsed)],
            ),
            (
                "<Inner {...props} />",
                with(members(), Some(spread(vec![Attribute::Spread(SpreadOf::Props)]))),
                &[("variant", Placed::Row), ("other", Placed::Collapsed)],
            ),
            (
                "<Inner tone=\"x\" {...rest} size=\"y\" />: before kept, after subtracted",
                with(members(), Some(spread(vec![named("tone"), rest(), named("size")]))),
                &[("tone", Placed::Row), ("size", Placed::Collapsed), ("variant", Placed::Row)],
            ),
            (
                "<Inner {...other} />",
                with(members(), Some(spread(vec![Attribute::Spread(SpreadOf::Other)]))),
                &[("tone", Placed::Collapsed)],
            ),
        ]);
    }

    #[test]
    fn an_attribute_valued_with_an_own_prop_adds_nothing() {
        let body = Body {
            elements: vec![element(Target::Component(strings(&["tone"])), vec![named("tone")])],
            ..Body::default()
        };
        table(vec![(
            "<Inner tone={size} />",
            with(vec![inherited("size"), inherited("tone")], Some(body)),
            &[("size", Placed::Collapsed), ("tone", Placed::Collapsed)],
        )]);
    }

    #[test]
    fn a_read_member_is_a_row_unless_ambient_then_a_used_name() {
        table(vec![(
            "reads",
            with(vec![inherited("a"), ambient("className"), ambient("id")], Some(reading(&["a", "className"]))),
            &[("a", Placed::Row), ("className", Placed::Used), ("id", Placed::Collapsed)],
        )]);
    }

    #[test]
    fn a_prop_passed_at_a_fifth_of_the_call_sites_is_a_row_ambient_or_not() {
        let passed = |sites: u32, count: u32| Owned {
            members: vec![inherited("a"), ambient("b")],
            body: Some(Body::default()),
            call_sites: CallSites { sites, passed: vec![("a".to_owned(), count), ("b".to_owned(), count)] },
        };
        table(vec![
            ("2 of 10", passed(10, 2), &[("a", Placed::Row), ("b", Placed::Row)]),
            ("1 of 10", passed(10, 1), &[("a", Placed::Collapsed), ("b", Placed::Collapsed)]),
            ("1 of 2: under the floor of 2", passed(2, 1), &[("a", Placed::Collapsed)]),
            ("2 of 11: under 20%", passed(11, 2), &[("a", Placed::Collapsed)]),
            ("3 of 11", passed(11, 3), &[("a", Placed::Row), ("b", Placed::Row)]),
        ]);
    }

    #[test]
    fn an_unknown_body_is_never_guessed() {
        let said = || reading(&["a"]);
        let handed_on = Body { handed_on: true, ..said() };
        let unresolved =
            Body { elements: vec![element(Target::Unresolved, vec![Attribute::Spread(SpreadOf::Rest)])], ..said() };
        let members = || {
            vec![
                inherited("a"),
                inherited("b"),
                ambient("c"),
                Member { required: true, ..inherited("d") },
                Member { default_tag: true, ..inherited("e") },
            ]
        };
        // What an incomplete body says still holds; only its silence is unknown. An ambient member is never listed
        // for want of evidence: it stays in its layer.
        let incomplete: &[(&str, Placed)] = &[
            ("a", Placed::Row),
            ("b", Placed::Unknown),
            ("c", Placed::Collapsed),
            ("d", Placed::Row),
            ("e", Placed::Row),
        ];
        let none: &[(&str, Placed)] = &[
            ("a", Placed::Unknown),
            ("b", Placed::Unknown),
            ("c", Placed::Collapsed),
            ("d", Placed::Row),
            ("e", Placed::Row),
        ];
        table(vec![
            ("handed on", with(members(), Some(handed_on)), incomplete),
            ("spread onto an unresolved tag", with(members(), Some(unresolved)), incomplete),
            ("no function of its own", with(members(), None), none),
        ]);
    }

    #[test]
    fn names_carry_no_meaning_and_order_follows_the_members() {
        let evidence = |names: [&str; 4]| {
            with(
                vec![inherited(names[0]), ambient(names[1]), inherited(names[2]), ambient(names[3])],
                Some(reading(&[names[2], names[3]])),
            )
        };
        let plain = evidence(["a", "b", "c", "d"]).decide();
        let familiar = evidence(["className", "style", "onClick", "children"]).decide();
        assert_eq!((plain.rows, plain.used, plain.collapsed), (strings(&["c"]), strings(&["d"]), strings(&["a", "b"])));
        assert_eq!(familiar.rows, ["onClick"]);
        assert_eq!(familiar.used, ["children"]);
        assert_eq!(familiar.collapsed, ["className", "style"]);
        assert_eq!(Owned::default().decide(), Shown::default());
        let twice = with(vec![inherited("a"), ambient("a")], Some(reading(&["a"]))).decide();
        assert_eq!((twice.rows, twice.used), (strings(&["a"]), Vec::new()));
    }
}
