//! Which component a JSX spread in a component body forwards the props to (ADR 0028, section A, slice T3).
//!
//! The extractor records what a body writes (`ComponentMapping::body`): each element's tag as written and its
//! attributes in source order. [`forwarding`] turns those facts into the elements [`decide`](super::decide) reads: a
//! host tag is [`Target::Dom`], a component tag is resolved by symbol and import to the [`ComponentMapping`] it names
//! and becomes [`Target::Component`] with that component's own non-ambient props, and anything else is
//! [`Target::Unresolved`]. Resolution follows bindings, never names: the tag's binding in its file, the import that
//! binds it, the re-exports to the declaring file, the declaration's binding there, and for a member tag
//! (`Menu.Item`) the export path ADR 0027 records. A tag that does not end at a component mapping is unresolved, so
//! a spread onto it leaves the body's evidence unknown; it is never matched by display name.
//!
//! `decide` then applies the rules: one hop of the target's props, an attribute after the spread fixed and
//! subtracted, one before it an overridable default and kept, an attribute valued with an own prop adding nothing,
//! a spread onto a DOM tag passing nothing.

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};

use super::{Attribute, Element, Member, SpreadOf, Target};
use crate::import_map::ImportResolutionMap;
use crate::types::collected::{
    BodyFacts, ComponentExport, ComponentMapping, JsxAttributeFact, JsxElementFact, JsxTag, SpreadOf as SpreadFact,
};

#[cfg(test)]
mod tests;

/// What the lane that calls [`forwarding`] answers: the two questions this module does not answer from the
/// extractor's facts alone.
pub(crate) trait Lane {
    /// The file that declares what module `specifier`, imported by `from`, exports as `export`, and the name it
    /// declares it under, following re-exports: the resolver's import resolution and re-export walk. `None` when the
    /// specifier does not resolve.
    fn declaration(&self, from: &Utf8Path, specifier: &str, export: &str) -> Option<(Utf8PathBuf, String)>;

    /// The members `component`'s props type reaches, as the lane hands them to [`decide`](super::decide); `None`
    /// when its props did not resolve.
    fn members(&self, component: &ComponentMapping) -> Option<Vec<Member>>;
}

/// The forwarding input of [`Body`](super::Body): the elements a component body writes with their tags resolved, and
/// whether the props escape where the syntax cannot follow them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Forwarding {
    /// The body's elements in source order, for [`Body::elements`](super::Body::elements). A component tag that no
    /// spread of the props reaches is left [`Target::Unresolved`]: `decide` reads a target only through such a spread.
    pub(crate) elements: Vec<Element>,
    /// For [`Body::handed_on`](super::Body::handed_on): the body's silence about a prop is unknown.
    pub(crate) handed_on: bool,
}

/// Where the project's components and imports are looked up.
#[derive(Clone, Copy)]
pub(crate) struct Project<'a> {
    /// Every component mapping of the project.
    pub(crate) components: &'a [ComponentMapping],
    /// The project's import bindings and re-exports.
    pub(crate) imports: &'a ImportResolutionMap,
}

/// The forwarding input for `component`'s body; `None` when the component has no function of its own.
pub(crate) fn forwarding(component: &ComponentMapping, project: Project<'_>, lane: &impl Lane) -> Option<Forwarding> {
    let body: &BodyFacts = component.body.as_ref()?;
    let mut resolved: Vec<(&JsxTag, Target)> = Vec::new();
    let elements = body
        .elements
        .iter()
        .map(|element: &JsxElementFact| {
            let attributes: Vec<Attribute> = element.attributes.iter().map(attribute).collect();
            // Only a spread of the props reads the target's props; the lane resolves none it does not need.
            if !matches!(element.tag, JsxTag::Intrinsic(_)) && !attributes.iter().any(Attribute::spreads_props) {
                return Element { target: Target::Unresolved, attributes };
            }
            let target = match resolved.iter().find(|(tag, _)| *tag == &element.tag) {
                Some((_, target)) => target.clone(),
                None => {
                    let target = target(&component.file_path, &element.tag, project, lane);
                    resolved.push((&element.tag, target.clone()));
                    target
                }
            };
            Element { target, attributes }
        })
        .collect();
    Some(Forwarding { elements, handed_on: body.handed_on })
}

/// The extractor's attribute fact as `decide` reads it.
fn attribute(fact: &JsxAttributeFact) -> Attribute {
    match fact {
        JsxAttributeFact::Named { name, prop } => Attribute::Named { name: name.clone(), prop: prop.clone() },
        JsxAttributeFact::Spread(of) => Attribute::Spread(match of {
            SpreadFact::Props => SpreadOf::Props,
            SpreadFact::Rest => SpreadOf::Rest,
            SpreadFact::Other => SpreadOf::Other,
        }),
    }
}

/// What the tag written in `file` resolves to.
fn target(file: &Utf8Path, tag: &JsxTag, project: Project<'_>, lane: &impl Lane) -> Target {
    let path: Vec<&str> = match tag {
        JsxTag::Intrinsic(_) => return Target::Dom,
        // Bound in an inner scope (`const Item = props.as`): whatever it holds at run time, not a module binding.
        JsxTag::Scoped(_) => return Target::Unresolved,
        JsxTag::Identifier(name) => vec![name.as_str()],
        JsxTag::Member(path) => path.split('.').collect(),
    };
    let Some(component) = component(file, &path, project, lane) else { return Target::Unresolved };
    match lane.members(component) {
        Some(members) => Target::Component(own_props(&members)),
        None => Target::Unresolved,
    }
}

/// The component's own props: the members its props type reaches that are not ambient (not the layered DOM and ARIA
/// sets), declared there or inherited from a project type, in member order, each once. Keeping only the members the
/// props type declares itself would read a target that extends a base as forwarding nothing, and hide what it takes.
fn own_props(members: &[Member]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    members
        .iter()
        .filter(|member| !member.ambient && seen.insert(member.name.as_str()))
        .map(|member| member.name.clone())
        .collect()
}

/// The component mapping the tag `path` (`Button`, `Menu.Item`, `UI.Button`) written in `file` names.
fn component<'a>(
    file: &Utf8Path,
    path: &[&str],
    project: Project<'a>,
    lane: &impl Lane,
) -> Option<&'a ComponentMapping> {
    let (&root, mut rest) = path.split_first()?;
    let (declaring_file, binding) = match project.imports.find_import(file, root) {
        Some(import) if import.is_type_only => return None,
        Some(import) if import.exported_name == "*" => {
            // `import * as UI from "./ui"`, then `<UI.Button>`: the namespace's member is the module's export.
            let (&export, members) = rest.split_first()?;
            rest = members;
            lane.declaration(file, &import.specifier, export)?
        }
        Some(import) => lane.declaration(file, &import.specifier, &import.exported_name)?,
        None => (file.to_owned(), root.to_owned()),
    };
    let declared = declared(project.components, &declaring_file, &binding)?;
    if rest.is_empty() {
        return Some(declared);
    }
    // An attached sub-component: the mapping its module exports through one of the parent's own exports.
    project.components.iter().find(|candidate| {
        candidate.file_path == declaring_file
            && candidate.exports.iter().any(|export| {
                export.members.iter().map(String::as_str).eq(rest.iter().copied())
                    && declared.exports.contains(&ComponentExport::named(export.name.clone()))
            })
    })
}

/// The component `file` declares under its binding `binding`; an anonymous default export, which has no binding, is
/// found by its `default` export.
fn declared<'a>(components: &'a [ComponentMapping], file: &Utf8Path, binding: &str) -> Option<&'a ComponentMapping> {
    let in_file = || components.iter().filter(move |component| component.file_path == file);
    in_file()
        .find(|component| (component.bound || component.local_name.is_some()) && component.binding_name() == binding)
        .or_else(|| {
            (binding == "default")
                .then(|| in_file().find(|component| component.exports.contains(&ComponentExport::named("default"))))
                .flatten()
        })
}
