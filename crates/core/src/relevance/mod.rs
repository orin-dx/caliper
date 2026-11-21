//! Which props of a component evidence puts forward (ADR 0028, section A).
//!
//! [`decide`] is the one place the rules are combined: it takes the [`Evidence`] gathered for the members a
//! component's props type reaches and says, for each member, whether it is a typed row or a name in its source's
//! layer. It is pure and total: no I/O, no
//! name lists, and its output follows the order of the members it is given.
//!
//! [`apply`] is its one caller, where a structural answer becomes output (`pipeline::revision`): the [`Inputs`]
//! gathered with the answer's resolution (so their reads are its dependencies) and the revision's call-site counts
//! (counted again each revision, so an edit to a consumer reaches every component it renders). The checker's merges
//! keep the inheritance layers, and so the `used` lists, of the structural answer.
//!
//! Under `htmlAttributes: full` the answer holds React's attributes of its HTML element apart ([`Attributes`]): [`apply`]
//! decides them with the listed props and lists only those the decision keeps, so the hundreds a component never uses
//! are never listed only to be moved into a layer again.

pub(crate) mod call_sites;
pub(crate) mod forwarding;
pub(crate) mod props_type;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::types::output::{ComponentEntry, InheritedLayer, ParsedProp, PropParent};

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
    /// The sources outside React's types the answer reaches that are ambient by their typings
    /// ([`crate::origin::is_ambient_declaration`]): type names by declaring file. Read with the answer's resolution, so
    /// an edit to a declaring file refreshes it.
    pub(crate) private: BTreeMap<String, BTreeSet<String>>,
}

/// Decides where each member of `entry`'s props goes, from `inputs`, the revision's `call_sites` of the component and
/// the members its listed props and layers reach; keeps the decision on the entry ([`ComponentEntry::relevance`]) and
/// lists by it ([`narrow`]).
///
/// `attributes` are the answer's React attributes held apart ([`Attributes`]): decided as listed props are, and listed
/// only where the decision keeps them.
pub(crate) fn apply(entry: &mut ComponentEntry, inputs: &Inputs, call_sites: &CallSites, attributes: &Attributes) {
    // A row no evidence of use names, and that its declaration does not list, is left out whatever else the evidence
    // holds: it is not built as a member (most of React's attributes, each component).
    let named = named_by_use(inputs.body.as_ref(), call_sites);
    place(entry, inputs, call_sites, attributes, |prop| !named.contains(prop.name.as_str()));
}

/// [`apply`], building no member for a row `quiet_candidate` takes that its declaration does not list
/// ([`props_type::members_but_quiet`]): the caller has checked no evidence of use names it.
fn place(
    entry: &mut ComponentEntry,
    inputs: &Inputs,
    call_sites: &CallSites,
    attributes: &Attributes,
    quiet_candidate: impl Fn(&ParsedProp) -> bool,
) {
    let rows = entry.props.values().chain(&attributes.0).chain(entry.notable_inherited.values()).map(AsRef::as_ref);
    let (members, quiet) =
        props_type::members_but_quiet(&inputs.props_type, rows, &entry.inheritance, &inputs.private, quiet_candidate);
    let shown = decide(&Evidence { members: &members, body: inputs.body.as_ref(), call_sites });
    // A row of a member the answer holds only as a name in a collapsed layer (a source the resolver collapsed past its
    // thresholds, such as Chakra's `SystemProperties`) has no type to list: it is named in the layer's `used` list
    // instead, so a member evidence puts forward is never left unnamed.
    let used: BTreeSet<String> = shown
        .rows
        .iter()
        .filter(|name| {
            !entry.props.contains_key(*name)
                && !attributes.contains(name)
                && !entry.notable_inherited.contains_key(*name)
        })
        .cloned()
        .collect();
    // A `Pick` or `Omit` over keys that are not a closed set of literals leaves which members of a source reach
    // unproven, so no source is narrowed into a layer that may claim a member that does not reach; React's and the
    // DOM's attributes still are, as they are never listed without evidence.
    let ambient: BTreeSet<&str> = members
        .iter()
        .filter(|member| member.ambient)
        .map(|member| member.name.as_str())
        .chain(quiet.iter().filter(|(_, ambient)| *ambient).map(|(name, _)| *name))
        .collect();
    let declined = inputs.props_type.declined;
    // A `Pick` whose key set is not proven selected its members deliberately: none is narrowed, React's included, and
    // the checker's merges take every React prop it finds (`Placement::expanded`).
    let narrows = inputs.narrows && !inputs.props_type.unproven_pick;
    let left = if narrows {
        let collapsed = shown.collapsed.iter().map(String::as_str).chain(quiet.iter().map(|(name, _)| *name));
        sources(entry, attributes, collapsed.filter(|name| !declined || ambient.contains(name)))
    } else {
        Vec::new()
    };
    entry.relevance = Some(std::sync::Arc::new(Placement { left, used, narrows }));
    narrow_with(entry, attributes);
}

/// React's attributes of a component's HTML element layers under `htmlAttributes: full`, held apart from its listing
/// until [`apply`] decides which it lists: each would be a listed prop, and the decision leaves nearly all of them out
/// again. Only an attribute every declaration of which is React's own (`@types/react`) is held, so that holding it
/// changes no step between resolution and [`apply`]: such a prop takes no name from a layer
/// (`listed_props_leave_layers`), names no private source, and is ambient. Any other is listed as before. Sorted by
/// name, each name once, never a name the listing holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Attributes(Vec<Arc<ParsedProp>>);

impl Attributes {
    /// Holds `prop` when every declaration of it is React's own, else gives it back to be listed. A name already held
    /// keeps its first prop, as the listing does.
    pub(crate) fn hold(&mut self, prop: Arc<ParsedProp>) -> Option<Arc<ParsedProp>> {
        let react = |parent: &PropParent| parent.file_name.contains("/@types/react/");
        if !(prop.parent.as_ref().is_some_and(react) && prop.declarations.iter().all(react)) {
            return Some(prop);
        }
        let at = self.0.partition_point(|held| held.name < prop.name);
        if self.0.get(at).is_none_or(|held| held.name != prop.name) {
            self.0.insert(at, prop);
        }
        None
    }

    /// Whether an attribute named `name` is held.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    fn get(&self, name: &str) -> Option<&Arc<ParsedProp>> {
        self.0.binary_search_by(|held| held.name.as_str().cmp(name)).ok().and_then(|at| self.0.get(at))
    }

    /// Lists every held attribute: the answer no decision narrows (no evidence was gathered) lists every prop.
    pub(crate) fn list_all(&self, entry: &mut ComponentEntry) {
        for prop in &self.0 {
            entry.props.entry(prop.name.clone()).or_insert_with(|| Arc::clone(prop));
        }
    }
}

/// A source by type name and file, as a prop's `parent` and a layer name it.
type Source = (String, String);

/// Each of `names` with the source the structural answer gives it, grouped by source: the `parent` of the listed prop,
/// and the layer of each collapsed layer holding the name. A name the answer gives no source (a listed prop with no
/// `parent`) has no entry: its source is unknown, so no lane narrows it. The project's own sources are held too: a
/// generated one (Chakra's `Conditions` in Chakra's own repo) is a layer there as it is when installed.
fn sources<'n>(
    entry: &'n ComponentEntry,
    attributes: &'n Attributes,
    names: impl Iterator<Item = &'n str>,
) -> Vec<(Source, Box<[Box<str>]>)> {
    let mut sourced: BTreeMap<(&str, &str), BTreeSet<&str>> = BTreeMap::new();
    let mut add = |name: &'n str, type_name: &'n str, file_name: &'n str| {
        sourced.entry((type_name, file_name)).or_default().insert(name);
    };
    let names: rustc_hash::FxHashSet<&'n str> = names.collect();
    for name in &names {
        let listed =
            entry.props.get(*name).or_else(|| attributes.get(name)).or_else(|| entry.notable_inherited.get(*name));
        if let Some(parent) = listed.and_then(|prop| prop.parent.as_ref()) {
            add(name, &parent.name, &parent.file_name);
        }
    }
    // Each layer's members are walked once against the set of names, never the layer once per name: a layer of
    // thousands of members with every member left out is linear, not quadratic.
    for layer in &entry.inheritance {
        for name in wanted_members(layer, |name| names.contains(name)) {
            add(name, &layer.type_name, &layer.file_name);
        }
    }
    sourced
        .into_iter()
        .map(|((type_name, file_name), names)| {
            ((type_name.to_owned(), file_name.to_owned()), names.into_iter().map(Box::from).collect())
        })
        .collect()
}

/// The members of `layer` that `wanted` takes, in the layer's order, each member asked once.
fn wanted_members(layer: &InheritedLayer, mut wanted: impl FnMut(&str) -> bool) -> impl Iterator<Item = &str> {
    layer.members.iter().map(String::as_str).filter(move |name| wanted(name))
}

/// What [`decide`] said of one component, kept on its entry so that every merge of the checker's props into it lists by
/// the same decision ([`narrow`]): the decision is made once, from the structural answer's evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// The members evidence leaves out of the listing ([`Shown::collapsed`]), by name under the
    /// source the structural answer gives each, sources in order and each source's names sorted: a listed prop is left
    /// out only when both match, so a prop of another source with the same name, or one the structural answer gave no
    /// source, is never hidden by it. Each source's type name and path is held once per component, not once per name.
    left: Vec<(Source, Box<[Box<str>]>)>,
    /// The rows [`decide`] puts forward that the structural answer holds only as names in a collapsed layer: named in
    /// that layer's `used` list.
    used: BTreeSet<String>,
    /// Whether the listing follows the decision ([`Inputs::narrows`]); off in the expanded view and for a `Pick` whose
    /// keys are not proven.
    narrows: bool,
}

impl Placement {
    /// Whether the answer lists every resolved prop: the expanded view (`collapseInheritedProps: none`), or a `Pick`
    /// whose keys are not proven (what it selects is deliberate). A checker merge then takes every `@types/react` prop
    /// the checker finds.
    pub(crate) fn expanded(&self) -> bool {
        !self.narrows
    }

    /// Whether the decision leaves `name`, declared by `parent`, out of the listing.
    fn leaves(&self, name: &str, parent: &PropParent) -> bool {
        let source = (parent.name.as_str(), parent.file_name.as_str());
        self.left
            .binary_search_by(|((type_name, file_name), _)| (type_name.as_str(), file_name.as_str()).cmp(&source))
            .ok()
            .and_then(|at| self.left.get(at))
            .is_some_and(|(_, names)| names.binary_search_by(|held| (**held).cmp(name)).is_ok())
    }
}

/// Lists `entry` by its kept [`Placement`]: a listed prop (in `props` or `notableInherited`) the decision leaves out
/// moves to the layer of its source, as a name, when the placement left out that name from that same source, whoever
/// declares it: [`decide`] leaves out only ambient or generated members. Its home is the layer of its `parent` (by
/// type name and file), else, for a source of React's, the entry's HTML element layer, else a new layer of its
/// `parent` (a library's private source keeps its own layer). A prop with no `parent` has no known source and stays
/// listed: no name is ever lost. Then each layer's `used` list is written. Without a placement nothing changes. Run by
/// [`apply`] and at the end of each merge of the checker's props, which may list again what the structural answer
/// left out.
pub(crate) fn narrow(entry: &mut ComponentEntry) {
    narrow_with(entry, &Attributes::default());
}

/// [`narrow`], with the held `attributes` taken as listed props: each the placement keeps is listed, and each it leaves
/// out moves to its layer at its place in name order among the listed props left out, as it would had it been listed.
/// Without a placement every one is listed.
fn narrow_with(entry: &mut ComponentEntry, attributes: &Attributes) {
    let Some(placement) = entry.relevance.clone() else {
        attributes.list_all(entry);
        return;
    };
    let ComponentEntry { props, notable_inherited, inheritance, .. } = entry;
    let leaves =
        |name: &str, prop: &ParsedProp| prop.parent.as_ref().is_some_and(|parent| placement.leaves(name, parent));
    // The listed props left out, in the order they join their layers: `props` (the held attributes among them, by
    // name), then `notableInherited`.
    let mut left: Vec<(String, Arc<ParsedProp>)> = Vec::new();
    props.retain(|name, prop| {
        let leaving = leaves(name, prop);
        if leaving {
            left.push((name.clone(), Arc::clone(prop)));
        }
        !leaving
    });
    let (kept, held): (Vec<&Arc<ParsedProp>>, Vec<&Arc<ParsedProp>>) =
        attributes.0.iter().partition(|prop| !leaves(&prop.name, prop));
    props.extend(kept.into_iter().map(|prop| (prop.name.clone(), Arc::clone(prop))));
    let mut left = merged_by_name(left, held.into_iter().map(|prop| (prop.name.clone(), Arc::clone(prop))));
    let from_props = left.len();
    notable_inherited.retain(|name, prop| {
        let leaving = leaves(name, prop);
        if leaving {
            left.push((name.clone(), Arc::clone(prop)));
        }
        !leaving
    });
    debug_assert!(left[..from_props].is_sorted_by(|a, b| a.0 < b.0));
    if !left.is_empty() {
        // The names the layers hold: a prop left out joins a layer only when none holds its name yet
        // (`listed_props_leave_layers`' rule).
        let layered: rustc_hash::FxHashSet<&str> =
            inheritance.iter().flat_map(|layer| layer.members.iter().map(String::as_str)).collect();
        let mut joined: rustc_hash::FxHashSet<&str> = rustc_hash::FxHashSet::default();
        let joining: Vec<usize> = (0..left.len())
            .filter(|at| {
                let (name, prop) = &left[*at];
                prop.parent.is_some() && !layered.contains(name.as_str()) && joined.insert(name.as_str())
            })
            .collect();
        drop((layered, joined));
        for at in joining {
            let (name, prop) = &left[at];
            if let Some(parent) = prop.parent.as_ref() {
                home(inheritance, parent).members.push(name.clone());
            }
        }
    }
    for layer in inheritance.iter_mut() {
        layer.total_props = layer.total_props.max(u32::try_from(layer.members.len()).unwrap_or(u32::MAX));
        layer.used = layer.members.iter().filter(|name| placement.used.contains(*name)).cloned().collect();
    }
}

/// `a` and `b`, each in name order and naming none in common, as one list in name order.
fn merged_by_name<T>(a: Vec<(String, T)>, b: impl IntoIterator<Item = (String, T)>) -> Vec<(String, T)> {
    let mut b = b.into_iter().peekable();
    if b.peek().is_none() {
        return a;
    }
    let mut merged = Vec::with_capacity(a.len());
    let mut a = a.into_iter().peekable();
    loop {
        let from_a = match (a.peek(), b.peek()) {
            (Some((x, _)), Some((y, _))) => x < y,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return merged,
        };
        merged.extend(if from_a { a.next() } else { b.next() });
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
    /// Every declaration of it is ambient ([`crate::origin::is_ambient_declaration`]): React's own, or one its library
    /// marks private. A member a library's API declares too, or with no known declaration, is not. Never decided by the
    /// name.
    pub(crate) ambient: bool,
    /// One of a generated set, not an API: a member of a source the resolver collapsed, of a source reaching with more
    /// than [`LARGE_API_SOURCE`](crate::types::options::LARGE_API_SOURCE) members, or one a mapped type copies from the
    /// keys of an interface of more than [`COLLAPSE_THRESHOLD`](crate::types::options::COLLAPSE_THRESHOLD) members.
    /// Decided by structure and size, never by a name.
    pub(crate) large: bool,
}

/// What a component's function body does with its props, read from the syntax alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Body {
    /// The props the body reads, destructures (with or without a default) or passes by name. What the body says is
    /// certain; its silence is not read: a member it never names keeps the place the props type gives it.
    pub(crate) reads: Vec<String>,
    /// The JSX elements the body writes, in source order.
    pub(crate) elements: Vec<Element>,
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
    /// A component tag that did not resolve: a spread onto it forwards nothing known.
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

/// Where [`decide`] puts each member, both lists in the order of [`Evidence::members`], except that `rows` puts the
/// documented and literal-union members first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Shown {
    /// Typed rows.
    pub(crate) rows: Vec<String>,
    /// Members left as names in their source's layer.
    pub(crate) collapsed: Vec<String>,
}

/// Decides, from the evidence alone, where each member of a component's props goes (ADR 0028, A2):
///
/// 1. The main props are listed in full: every member the props type declares itself (own, picked, overriding,
///    required or defaulted), and every member of a source that is neither ambient nor large ([`Member::large`]), with
///    no story or call site needed.
/// 2. What the component uses directly is listed from any source: a member its body reads, destructures or passes by
///    name, or that a spread forwards to a resolved component's own props. A spread onto a DOM tag is no evidence, and
///    a body that hands its props on where the syntax cannot follow promotes nothing and hides nothing.
/// 3. Every other member of an ambient or large source stays a name in its layer, unless
/// 4. it is passed at the call-site floor (`max(2, 20%)` of the component's call sites).
pub(crate) fn decide(evidence: &Evidence<'_>) -> Shown {
    let body = evidence.body;
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
        let used = reads.contains(name) || forwarded.contains(name);
        if listed_by_declaration(member) || used || at_floor.contains(name) {
            ordered_rows.push((!(member.documented || member.literal_union), member.name.clone()));
        } else {
            shown.collapsed.push(member.name.clone());
        }
    }
    ordered_rows.sort_by_key(|(plain, _)| *plain);
    shown.rows = ordered_rows.into_iter().map(|(_, name)| name).collect();
    shown
}

/// Whether [`decide`] lists `member` from what its declaration and the props type say alone (rule 1): declared by the
/// props type itself, or of an API source. Any other member is listed only on evidence of use naming it (rules 2 and
/// 4), so one that no evidence names is left out; [`apply`] leaves such a member out without building it.
pub(crate) fn listed_by_declaration(member: &Member) -> bool {
    let declared = member.own || member.picked || member.overrides || member.required || member.default_tag;
    let api = !(member.ambient || member.large);
    declared || api
}

/// Every name the evidence of use says something about: read or passed by name in the body, forwarded by a spread, or
/// passed at any call site. A member named nowhere here is placed by [`listed_by_declaration`] alone.
fn named_by_use<'e>(body: Option<&'e Body>, call_sites: &'e CallSites) -> rustc_hash::FxHashSet<&'e str> {
    let mut named: rustc_hash::FxHashSet<&str> = call_sites.passed.iter().map(|(name, _)| name.as_str()).collect();
    if let Some(body) = body {
        named.extend(body.reads.iter().map(String::as_str));
        named.extend(body.forwarded());
    }
    named
}

impl Body {
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
        Collapsed,
    }

    /// A member of a large source (a style system's, say): listed only on evidence.
    fn inherited(name: &str) -> Member {
        Member { name: name.to_owned(), large: true, ..Member::default() }
    }

    /// A member of an API source: neither ambient nor large.
    fn api(name: &str) -> Member {
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

    fn layer(type_name: &str, file_name: &str, members: Vec<String>) -> InheritedLayer {
        InheritedLayer {
            type_name: type_name.to_owned(),
            file_name: file_name.to_owned(),
            omitted: Vec::new(),
            html_element: None,
            total_props: u32::try_from(members.len()).unwrap(),
            members,
            used: Vec::new(),
        }
    }

    #[test]
    fn a_layer_is_walked_once_whatever_number_of_names_it_leaves_out() {
        // Entire finding on change 288: a layer of 4,000 members with every member left out was rescanned once per
        // name (16 million comparisons). Each member is asked once.
        let members: Vec<String> = (0..4_000).map(|i| format!("m{i}")).collect();
        let big = layer("Big", "/p/node_modules/lib/big.d.ts", members.clone());
        let mut asked = 0;
        let wanted: BTreeSet<&str> = members.iter().map(String::as_str).collect();
        let found = wanted_members(&big, |name| {
            asked += 1;
            wanted.contains(name)
        })
        .count();
        assert_eq!((found, asked), (4_000, 4_000));
        let entry = ComponentEntry { inheritance: vec![big], ..button(vec![]) };
        let left = sources(&entry, &Attributes::default(), members.iter().map(String::as_str));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].1.len(), 4_000);
    }

    #[test]
    fn a_project_source_is_narrowed_like_a_dependency_s() {
        // The generated rule holds in the project too (Chakra's own repo): a project source's left-out names are held.
        let entry = ComponentEntry {
            inheritance: vec![layer("Conditions", "/p/src/conditions.ts", strings(&["_hover", "_active"]))],
            ..button(vec![])
        };
        let left = sources(&entry, &Attributes::default(), ["_hover"].into_iter());
        assert_eq!(
            left,
            vec![(("Conditions".to_owned(), "/p/src/conditions.ts".to_owned()), vec!["_hover".into()].into())]
        );
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
        let lists = [(&shown.rows, Placed::Row), (&shown.collapsed, Placed::Collapsed)];
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
    fn a_member_the_body_uses_is_a_row_from_any_source() {
        table(vec![(
            "reads, destructures or passes by name",
            with(
                vec![inherited("a"), ambient("className"), ambient("onFocus"), ambient("id")],
                Some(reading(&["a", "className", "onFocus"])),
            ),
            &[("a", Placed::Row), ("className", Placed::Row), ("onFocus", Placed::Row), ("id", Placed::Collapsed)],
        )]);
    }

    #[test]
    fn every_member_of_an_api_source_is_a_row_without_evidence() {
        let members = || vec![api("loadingText"), api("asChild"), inherited("_hover"), ambient("onClick")];
        let listed: &[(&str, Placed)] = &[
            ("loadingText", Placed::Row),
            ("asChild", Placed::Row),
            ("_hover", Placed::Collapsed),
            ("onClick", Placed::Collapsed),
        ];
        table(vec![
            ("a body that names nothing", with(members(), Some(Body::default())), listed),
            ("no function of its own", with(members(), None), listed),
        ]);
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
    fn an_unknown_body_promotes_nothing_and_hides_nothing() {
        // A spread onto a tag that did not resolve, or no body at all: what is said still holds, and the main props
        // stay listed; nothing of an ambient or large source is listed for want of evidence.
        let said = || reading(&["a"]);
        let unresolved =
            Body { elements: vec![element(Target::Unresolved, vec![Attribute::Spread(SpreadOf::Rest)])], ..said() };
        let members = || {
            vec![
                inherited("a"),
                inherited("b"),
                ambient("c"),
                api("f"),
                Member { required: true, ..inherited("d") },
                Member { default_tag: true, ..ambient("e") },
            ]
        };
        let expected = |a: Placed| -> Vec<(&'static str, Placed)> {
            vec![
                ("a", a),
                ("b", Placed::Collapsed),
                ("c", Placed::Collapsed),
                ("f", Placed::Row),
                ("d", Placed::Row),
                ("e", Placed::Row),
            ]
        };
        table(vec![
            ("spread onto an unresolved tag", with(members(), Some(unresolved)), &expected(Placed::Row)),
            ("no function of its own", with(members(), None), &expected(Placed::Collapsed)),
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
        assert_eq!((plain.rows, plain.collapsed), (strings(&["c", "d"]), strings(&["a", "b"])));
        assert_eq!(
            (familiar.rows, familiar.collapsed),
            (strings(&["onClick", "children"]), strings(&["className", "style"]))
        );
        assert_eq!(Owned::default().decide(), Shown::default());
        let twice = with(vec![inherited("a"), ambient("a")], Some(reading(&["a"]))).decide();
        assert_eq!((twice.rows, twice.collapsed), (strings(&["a"]), Vec::new()));
    }

    const REACT: &str = "/p/node_modules/@types/react/index.d.ts";
    const OWN: &str = "/p/src/button.tsx";

    fn parent(type_name: &str, file_name: &str) -> PropParent {
        PropParent { name: type_name.to_owned(), file_name: file_name.to_owned() }
    }

    fn listed(
        name: &str,
        type_name: &str,
        file_name: &str,
    ) -> (String, std::sync::Arc<crate::types::output::ParsedProp>) {
        let prop = crate::types::output::ParsedProp::new(
            name.to_owned(),
            crate::types::output::PropType::Any,
            false,
            None,
            String::new(),
            BTreeMap::new(),
            Some(parent(type_name, file_name)),
            vec![],
        );
        (name.to_owned(), std::sync::Arc::new(prop))
    }

    fn button(props: Vec<(String, std::sync::Arc<crate::types::output::ParsedProp>)>) -> ComponentEntry {
        ComponentEntry {
            relevance: None,
            display_name: "Button".to_owned(),
            file_path: OWN.into(),
            description: String::new(),
            props: props.into_iter().collect(),
            inheritance: vec![],
            notable_inherited: BTreeMap::new(),
            discriminant_prop: None,
            composes: vec![],
            tags: BTreeMap::new(),
            methods: vec![],
            detected_by: crate::types::DetectionRule::FunctionDeclaration,
            exported: true,
            exports: vec![crate::types::ComponentExport::named("Button")],
        }
    }

    #[test]
    fn a_placement_holds_each_source_once_and_lists_by_name_and_source() {
        // `onClick` and `onBlur` of one React source, `title` of another, and `variant` of the project's own, which the
        // decision lists (a project source is never ambient, and this one is not generated): the placement holds each
        // React source once with its names sorted.
        let mut entry = button(vec![
            listed("onClick", "DOMAttributes", REACT),
            listed("onBlur", "DOMAttributes", REACT),
            listed("title", "HTMLAttributes", REACT),
            listed("variant", "ButtonProps", OWN),
        ]);
        let left = sources(&entry, &Attributes::default(), ["title", "onClick", "onBlur"].into_iter());
        let names = |names: &[&str]| names.iter().map(|name| Box::from(*name)).collect::<Box<[Box<str>]>>();
        assert_eq!(
            left,
            vec![
                (("DOMAttributes".to_owned(), REACT.to_owned()), names(&["onBlur", "onClick"])),
                (("HTMLAttributes".to_owned(), REACT.to_owned()), names(&["title"])),
            ]
        );
        let placement = Placement { left, used: BTreeSet::from(["title".to_owned()]), narrows: true };
        assert!(placement.leaves("onClick", &parent("DOMAttributes", REACT)));
        assert!(!placement.leaves("onClick", &parent("HTMLAttributes", REACT)), "a name of another source");
        assert!(!placement.leaves("onFocus", &parent("DOMAttributes", REACT)), "a name the decision did not leave");
        entry.relevance = Some(std::sync::Arc::new(placement));
        narrow(&mut entry);
        assert_eq!(entry.props.keys().collect::<Vec<_>>(), ["variant"]);
        let layers: Vec<_> =
            entry.inheritance.iter().map(|layer| (layer.type_name.as_str(), &layer.members, &layer.used)).collect();
        assert_eq!(
            layers,
            [
                ("DOMAttributes", &strings(&["onBlur", "onClick"]), &Vec::new()),
                ("HTMLAttributes", &strings(&["title"]), &strings(&["title"])),
            ]
        );
    }

    /// A React attribute of `type_name` in React's own types: held apart under `htmlAttributes: full`.
    fn react_attribute(name: &str, type_name: &str, required: bool, default_tag: bool) -> Arc<ParsedProp> {
        let tags = if default_tag { BTreeMap::from([("default".to_owned(), "x".to_owned())]) } else { BTreeMap::new() };
        let source = parent(type_name, REACT);
        Arc::new(ParsedProp::new(
            name.to_owned(),
            crate::types::output::PropType::String,
            required,
            None,
            String::new(),
            tags,
            Some(source.clone()),
            vec![source],
        ))
    }

    fn sourced(name: &str, source: Option<PropParent>, declarations: Vec<PropParent>) -> Arc<ParsedProp> {
        let prop = ParsedProp::new(
            name.to_owned(),
            crate::types::output::PropType::Any,
            false,
            None,
            String::new(),
            BTreeMap::new(),
            source,
            declarations,
        );
        Arc::new(prop)
    }

    #[test]
    fn only_an_attribute_every_declaration_of_which_is_react_s_is_held() {
        let mut held = Attributes::default();
        assert_eq!(held.hold(react_attribute("title", "HTMLAttributes", false, false)), None);
        let augmented = sourced("css", Some(parent("HTMLAttributes", "/p/src/augment.d.ts")), vec![]);
        assert!(held.hold(augmented).is_some(), "an augmentation of the project is listed as before");
        let library = parent("FocusEvents", "/p/node_modules/lib/events.d.ts");
        let both =
            sourced("onFocus", Some(parent("DOMAttributes", REACT)), vec![parent("DOMAttributes", REACT), library]);
        assert!(held.hold(both).is_some(), "a member a library declares too is listed as before");
        assert!(held.hold(sourced("id", None, vec![parent("HTMLAttributes", REACT)])).is_some(), "no source");
        let other_react = "/p/node_modules/react/index.d.ts";
        assert!(held.hold(sourced("key", Some(parent("Attributes", other_react)), vec![])).is_some(), "not @types");
        assert_eq!(held.hold(react_attribute("title", "DOMAttributes", false, false)), None, "first kept");
        assert_eq!(
            held.0.iter().map(|prop| prop.parent.as_ref().unwrap().name.as_str()).collect::<Vec<_>>(),
            ["HTMLAttributes"]
        );
        assert!(held.contains("title") && !held.contains("css"));
    }

    /// Held attributes and quiet rows change nothing a reader sees: over generated components, deciding with React's
    /// attributes held apart, building no member for a row no use names, gives the entry that listing every attribute
    /// and building every member gives (main's answer): rows, layers' members in order, `used` lists, and the placement
    /// the checker's merges narrow by.
    #[test]
    fn holding_attributes_apart_and_leaving_quiet_rows_unbuilt_changes_no_answer() {
        let mut seed: u64 = 7;
        let mut below = move |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let names: Vec<String> =
            (0..40).map(|i| format!("a{i:02}")).chain(["own0".into(), "children".into()]).collect();
        let mut left_out = 0;
        for case in 0..400 {
            // The answer's own rows, one of them named like an attribute could be.
            let own: Vec<_> = (0..below(4)).map(|i| listed(&format!("own{i}"), "ButtonProps", OWN)).collect();
            let mut entry = button(own);
            let html = layer("ComponentProps", REACT, Vec::new());
            entry.inheritance.push(InheritedLayer { html_element: Some("button".to_owned()), ..html });
            if below(2) == 0 {
                // A collapsed source holding some of the attributes' names.
                let held: Vec<String> = names[..40].iter().filter(|_| below(4) == 0).cloned().collect();
                entry.inheritance.push(layer("SystemProps", "/p/node_modules/lib/system.d.ts", held));
            }
            if below(3) == 0 {
                // A React prop the chain listed itself, of a source the attributes come from too.
                let (name, prop) = listed("a15z", "HTMLAttributes", REACT);
                entry.props.insert(name, prop);
            }
            let mut attributes = Attributes::default();
            for name in &names[..40] {
                let type_name =
                    ["HTMLAttributes", "DOMAttributes", "AriaAttributes"][usize::try_from(below(3)).unwrap()];
                assert_eq!(attributes.hold(react_attribute(name, type_name, below(20) == 0, below(20) == 0)), None);
            }
            let mut pick = |odds: u64| -> Vec<String> { names.iter().filter(|_| below(odds) == 0).cloned().collect() };
            let reads = pick(8);
            let forwarded = pick(10);
            let passed = pick(6);
            let body = (below(3) != 0).then(|| Body {
                reads,
                elements: vec![element(
                    Target::Component(forwarded),
                    vec![Attribute::Spread(SpreadOf::Rest), named(&names[usize::try_from(below(40)).unwrap()])],
                )],
            });
            let sites = u32::try_from(below(12)).unwrap();
            let passed = passed.into_iter().map(|name| (name, u32::try_from(below(u64::from(sites) + 1)).unwrap()));
            let call_sites = CallSites { sites, passed: passed.collect() };
            let inputs = Inputs {
                site: (OWN.into(), "Button".to_owned(), 0),
                props_type: props_type::PropsType::default(),
                body,
                narrows: below(4) != 0,
                private: BTreeMap::new(),
            };
            let mut held = entry.clone();
            apply(&mut held, &inputs, &call_sites, &attributes);
            let mut listed_all = entry.clone();
            attributes.list_all(&mut listed_all);
            place(&mut listed_all, &inputs, &call_sites, &Attributes::default(), |_| false);
            assert_eq!(held, listed_all, "case {case}");
            left_out += held.inheritance.iter().map(|layer| layer.used.len() + layer.members.len()).sum::<usize>();
        }
        assert!(left_out > 400 * 20, "the cases leave attributes out: {left_out}");
    }
}
