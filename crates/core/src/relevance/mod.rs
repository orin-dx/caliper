//! Which props of a component evidence puts forward (ADR 0028, section A).
//!
//! [`decide`] is the one place the rules are combined: it takes the [`Evidence`] each lane gathers for the members a
//! component's props type reaches and says, for each member, whether it is a typed row, a name in its collapsed
//! layer's `used` list, left in that layer, or decided as today because the body says nothing. It is pure and total:
//! no I/O, no name lists, and its output follows the order of the members it is given.
//!
//! Both lanes reach it through [`annotate`] (ADR 0028, slice T7): the structural resolver after it resolves a
//! component, the checker's merges after they replace its props. The inputs are the same for both: the
//! lane-independent [`Inputs`] gathered once by the resolver, and the members the entry's props and layers reach
//! ([`props_type::members`]). Story `args` are absent evidence: stories files are not discovered.

pub(crate) mod call_sites;
pub(crate) mod forwarding;
pub(crate) mod props_type;

use std::collections::{BTreeMap, BTreeSet};

use crate::types::output::{ComponentEntry, Relevance};

/// The evidence about one component that does not depend on the lane, gathered once by the structural resolver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// What the props type writes itself.
    pub(crate) props_type: props_type::PropsType,
    /// What the component's body does with its props; `None` when it has no function of its own.
    pub(crate) body: Option<Body>,
    /// The props consumers pass at its JSX call sites.
    pub(crate) call_sites: CallSites,
}

/// Decides where each member of `entry`'s props goes, from its [`Inputs`] and the members its listed props and layers
/// reach, and writes [`ComponentEntry::relevance`]. Every lane calls it after it last changes the props or layers, so
/// the decision is one function of the same inputs. Without inputs the relevance is empty: unknown, never guessed.
pub(crate) fn annotate(entry: &mut ComponentEntry) {
    let Some(inputs) = entry.relevance_evidence.0.clone() else {
        entry.relevance.clear();
        return;
    };
    let rows = entry.props.values().chain(entry.notable_inherited.values()).map(AsRef::as_ref);
    let members = props_type::members(&inputs.props_type, rows, &entry.inheritance, crate::types::ambient::is_dom_file);
    let evidence =
        Evidence { members, body: inputs.body.clone(), call_sites: inputs.call_sites.clone(), story_args: Vec::new() };
    let listed = |name: &str| entry.props.contains_key(name) || entry.notable_inherited.contains_key(name);
    entry.relevance = relevance_of(&decide(&evidence), listed);
}

/// The relevance of each listed name, and of each unlisted one evidence puts forward.
fn relevance_of(shown: &Shown, listed: impl Fn(&str) -> bool) -> BTreeMap<String, Relevance> {
    let lists = [
        (&shown.rows, Relevance::Row),
        (&shown.used, Relevance::Used),
        (&shown.collapsed, Relevance::Collapsed),
        (&shown.as_today, Relevance::AsToday),
    ];
    let mut relevance = BTreeMap::new();
    for (names, placed) in lists {
        let forward = matches!(placed, Relevance::Row | Relevance::Used);
        for name in names.iter().filter(|name| forward || listed(name)) {
            relevance.entry(name.clone()).or_insert(placed);
        }
    }
    relevance
}

/// Everything known about the members one component's props type reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Evidence {
    /// The members, in the order the props type reaches them; [`Shown`] keeps this order. A name repeated is placed
    /// once, by its first member.
    pub(crate) members: Vec<Member>,
    /// What the component's own function body does with its props; `None` when it has no function of its own.
    pub(crate) body: Option<Body>,
    /// The props passed at the component's JSX call sites across the project.
    pub(crate) call_sites: CallSites,
    /// The names set in the `args` of the component's stories. Any one of them counts: stories have no floor.
    pub(crate) story_args: Vec<String>,
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
    /// Declared by `@types/react`, or reaching the component only through a spread onto a DOM tag. Computed by the
    /// lane from the declaring path, never from the name.
    pub(crate) ambient: bool,
}

/// What a component's function body does with its props, read from the syntax alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Body {
    /// The props the body reads. Complete only when `handed_on` is false.
    pub(crate) reads: Vec<String>,
    /// The props destructured with a default value.
    pub(crate) defaulted: Vec<String>,
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
    /// `name={value}`, with the component's own prop the value is, when it is exactly one.
    Named {
        /// The attribute's name.
        name: String,
        /// The component's prop the value is, if it is exactly one. Kept from the extractor's facts; [`decide`] gives
        /// it no weight (an attribute valued with an own prop adds nothing).
        prop: Option<String>,
    },
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
    /// Ambient members named in their collapsed layer's `used` list.
    pub(crate) used: Vec<String>,
    /// Members left in their collapsed layer.
    pub(crate) collapsed: Vec<String>,
    /// Members with no signal whose body evidence is incomplete: listed or collapsed as today, so missing evidence
    /// never hides more than today does.
    pub(crate) as_today: Vec<String>,
}

/// Decides, from the evidence alone, where each member of a component's props goes.
pub(crate) fn decide(evidence: &Evidence) -> Shown {
    let body = evidence.body.as_ref();
    let complete = body.is_some_and(Body::is_complete);
    let facts = |names: fn(&Body) -> &[String]| -> BTreeSet<&str> {
        body.map(|body| names(body).iter().map(String::as_str).collect()).unwrap_or_default()
    };
    let (reads, defaulted) = (facts(|body| &body.reads), facts(|body| &body.defaulted));
    let forwarded = body.map(Body::forwarded).unwrap_or_default();
    let passed: BTreeSet<&str> = evidence
        .call_sites
        .passed
        .iter()
        .filter(|(_, count)| evidence.call_sites.at_floor(*count))
        .map(|(name, _)| name.as_str())
        .chain(evidence.story_args.iter().map(String::as_str))
        .collect();
    let mut shown = Shown::default();
    let mut ordered_rows = Vec::new();
    let mut placed = BTreeSet::new();
    for member in &evidence.members {
        let name = member.name.as_str();
        if !placed.insert(name) {
            continue;
        }
        let read = reads.contains(name);
        let defaulted = member.default_tag || defaulted.contains(name);
        let declared = member.own || member.picked || member.overrides || member.required || defaulted;
        let list = if declared || passed.contains(name) || (!member.ambient && (read || forwarded.contains(name))) {
            ordered_rows.push((!(member.documented || member.literal_union), member.name.clone()));
            continue;
        } else if member.ambient && read {
            &mut shown.used
        } else if !complete {
            &mut shown.as_today
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
                        Attribute::Named { name, .. } => Some(name.as_str()),
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
        AsToday,
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

    fn named(name: &str, prop: Option<&str>) -> Attribute {
        Attribute::Named { name: name.to_owned(), prop: prop.map(str::to_owned) }
    }

    fn element(target: Target, attributes: Vec<Attribute>) -> Element {
        Element { target, attributes }
    }

    fn with(members: Vec<Member>, body: Option<Body>) -> Evidence {
        Evidence { members, body, ..Evidence::default() }
    }

    fn placed(shown: &Shown, name: &str) -> Option<Placed> {
        let lists = [
            (&shown.rows, Placed::Row),
            (&shown.used, Placed::Used),
            (&shown.collapsed, Placed::Collapsed),
            (&shown.as_today, Placed::AsToday),
        ];
        lists.into_iter().find(|(list, _)| list.iter().any(|n| n == name)).map(|(_, placed)| placed)
    }

    /// One case of a table: its label, its evidence, and where each named member must go.
    type Case<'a> = (&'a str, Evidence, &'a [(&'a str, Placed)]);

    /// Runs each case and checks where every named member went.
    fn table(cases: Vec<Case<'_>>) {
        for (case, evidence, expected) in cases {
            let shown = decide(&evidence);
            for (name, want) in expected {
                assert_eq!(placed(&shown, name), Some(*want), "{case}: {name}");
            }
        }
    }

    #[test]
    fn an_inherited_member_the_body_never_reads_stays_in_its_layer() {
        let evidence = with(vec![inherited("tone"), inherited("size")], Some(reading(&["size"])));
        let shown = decide(&evidence);
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
        let tagged = Member { default_tag: true, ..inherited("b") };
        let destructured = Body { defaulted: strings(&["c"]), ..Body::default() };
        table(vec![(
            "required, @default, destructuring default",
            with(vec![required, tagged, inherited("c"), inherited("d")], Some(destructured)),
            &[("a", Placed::Row), ("b", Placed::Row), ("c", Placed::Row), ("d", Placed::Collapsed)],
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
        assert_eq!(decide(&evidence).rows, ["u", "d", "plain"]);
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
                with(members(), Some(spread(vec![named("tone", None), rest(), named("size", None)]))),
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
            elements: vec![element(Target::Component(strings(&["tone"])), vec![named("tone", Some("size"))])],
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
    fn a_prop_passed_at_a_fifth_of_the_call_sites_is_a_typed_row() {
        let passed = |sites: u32, count: u32| Evidence {
            members: vec![inherited("a"), ambient("b")],
            body: Some(Body::default()),
            call_sites: CallSites { sites, passed: vec![("a".to_owned(), count), ("b".to_owned(), count)] },
            story_args: Vec::new(),
        };
        table(vec![
            ("2 of 10", passed(10, 2), &[("a", Placed::Row), ("b", Placed::Row)]),
            ("1 of 10", passed(10, 1), &[("a", Placed::Collapsed), ("b", Placed::Collapsed)]),
            ("1 of 2: under the floor of 2", passed(2, 1), &[("a", Placed::Collapsed)]),
            ("2 of 11: under 20%", passed(11, 2), &[("a", Placed::Collapsed)]),
            ("3 of 11", passed(11, 3), &[("a", Placed::Row)]),
        ]);
    }

    #[test]
    fn any_story_arg_is_a_typed_row() {
        let evidence = Evidence {
            members: vec![inherited("a"), ambient("b"), inherited("c")],
            body: Some(Body::default()),
            call_sites: CallSites { sites: 50, passed: Vec::new() },
            story_args: strings(&["a", "b"]),
        };
        table(vec![("one story", evidence, &[("a", Placed::Row), ("b", Placed::Row), ("c", Placed::Collapsed)])]);
    }

    #[test]
    fn an_unknown_body_is_never_guessed() {
        let said = || Body { defaulted: strings(&["e"]), ..reading(&["a"]) };
        let handed_on = Body { handed_on: true, ..said() };
        let unresolved =
            Body { elements: vec![element(Target::Unresolved, vec![Attribute::Spread(SpreadOf::Rest)])], ..said() };
        let members = || {
            vec![
                inherited("a"),
                inherited("b"),
                ambient("c"),
                Member { required: true, ..inherited("d") },
                inherited("e"),
            ]
        };
        // What an incomplete body says still holds; only its silence is unknown, ambient or not.
        let incomplete: &[(&str, Placed)] = &[
            ("a", Placed::Row),
            ("b", Placed::AsToday),
            ("c", Placed::AsToday),
            ("d", Placed::Row),
            ("e", Placed::Row),
        ];
        let none: &[(&str, Placed)] = &[
            ("a", Placed::AsToday),
            ("b", Placed::AsToday),
            ("c", Placed::AsToday),
            ("d", Placed::Row),
            ("e", Placed::AsToday),
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
        let plain = decide(&evidence(["a", "b", "c", "d"]));
        let familiar = decide(&evidence(["className", "style", "onClick", "children"]));
        assert_eq!((plain.rows, plain.used, plain.collapsed), (strings(&["c"]), strings(&["d"]), strings(&["a", "b"])));
        assert_eq!(familiar.rows, ["onClick"]);
        assert_eq!(familiar.used, ["children"]);
        assert_eq!(familiar.collapsed, ["className", "style"]);
        assert_eq!(decide(&Evidence::default()), Shown::default());
        let twice = decide(&with(vec![inherited("a"), ambient("a")], Some(reading(&["a"]))));
        assert_eq!((twice.rows, twice.used), (strings(&["a"]), Vec::new()));
    }
}
