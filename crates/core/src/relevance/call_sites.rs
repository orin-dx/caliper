//! The props consumers pass to each component, counted at its JSX call sites and read from its stories' `args`
//! (ADR 0028, section A, slice T6).
//!
//! A tag is resolved to a component by symbol and import, never by its display name: a tag naming a binding of a
//! component its own file declares is that component; any other is asked of the import machinery
//! ([`Canonical`]: where the binding the file imports is declared, through re-exports), and the answer is matched to
//! the component that file declares under that binding, or exports as `default`. A member tag (`Menu.Item`) is the
//! sub-component attached to the resolved component's export (ADR 0027's [`ComponentExport`] path), or, for a namespace
//! import, the member the namespace's module declares. A tag that does not resolve is counted nowhere.
//!
//! Only named attributes count; a spread passes names the syntax does not know. A component with fewer than
//! [`MIN_SITES`] resolved sites yields no call-site evidence at all ([`CallSites::default`]), and a prop is used when it
//! is passed at the floor [`CallSites::used`] applies. Stories have no floor: every key of an `args` object counts.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::FxHashMap;

use super::CallSites;
use crate::types::{ComponentExport, ComponentMapping, JsxAttributeFact, JsxTag, JsxUsage, JsxUsageAttribute};

/// The fewest resolved call sites a component needs for its counts to be evidence.
pub(crate) const MIN_SITES: u32 = 3;

/// Where a binding a file imports is declared, following imports and re-exports: `(declaring file, declared name)`,
/// the declared name being the binding there, or `default` for an anonymous default export. `None` when the name is not
/// imported or does not resolve. A dotted name (`Ns.Member`) asks for the member of a namespace import. The pipeline
/// answers it with the resolver's canonical resolution.
pub(crate) trait Canonical: FnMut(&Utf8Path, &str) -> Option<(Utf8PathBuf, String)> {}

impl<F: FnMut(&Utf8Path, &str) -> Option<(Utf8PathBuf, String)>> Canonical for F {}

/// The JSX one file writes outside every component body (`SourceData::jsx_usages`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileUsages<'a> {
    /// The file.
    pub(crate) file: &'a Utf8Path,
    /// Its elements.
    pub(crate) usages: &'a [JsxUsage],
}

/// The `args` one stories file sets (`extractor::story_args`), with the tag-like path its meta's `component` names.
#[cfg_attr(not(test), expect(dead_code, reason = "stories files are not discovered; story args stay absent evidence"))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileStories<'a> {
    /// The stories file.
    pub(crate) file: &'a Utf8Path,
    /// What `meta.component` names, as written (`Button`, `Menu.Item`).
    pub(crate) component: &'a str,
    /// The keys of the meta's and every story's `args`.
    pub(crate) args: &'a [String],
}

/// Counts, for each of `components`, its call sites across `files` and the elements inside every component body, and
/// the props passed at them. The result is parallel to `components`; one with fewer than [`MIN_SITES`] resolved sites
/// gets [`CallSites::default`]: no evidence. `passed` is sorted by name.
pub(crate) fn count<'a>(
    components: &'a [ComponentMapping],
    files: &[FileUsages<'a>],
    canonical: impl Canonical,
) -> Vec<CallSites> {
    let mut resolver = Resolver::new(components, canonical);
    let mut counts: Vec<(u32, BTreeMap<&'a str, u32>)> = vec![Default::default(); components.len()];
    let mut add = |at: usize, names: BTreeSet<&'a str>| {
        let (sites, passed) = &mut counts[at];
        *sites += 1;
        for name in names {
            *passed.entry(name).or_default() += 1;
        }
    };
    for file in files {
        for usage in file.usages {
            let Some(at) = resolver.tag(file.file, &usage.tag) else { continue };
            add(at, usage.attributes.iter().filter_map(JsxUsageAttribute::name).collect());
        }
    }
    for component in components {
        for element in component.body.iter().flat_map(|body| &body.elements) {
            let Some(at) = resolver.tag(&component.file_path, &element.tag) else { continue };
            add(at, element.attributes.iter().filter_map(JsxAttributeFact::name).collect());
        }
    }
    counts
        .into_iter()
        .map(|(sites, passed)| match sites >= MIN_SITES {
            true => CallSites { sites, passed: passed.into_iter().map(|(name, n)| (name.to_owned(), n)).collect() },
            false => CallSites::default(),
        })
        .collect()
}

/// The `args` keys set in stories for each of `components`, parallel to it and sorted: every key counts, with no floor.
/// A stories file whose `component` does not resolve gives nothing.
#[cfg_attr(not(test), expect(dead_code, reason = "stories files are not discovered; story args stay absent evidence"))]
pub(crate) fn story_args<'a>(
    components: &'a [ComponentMapping],
    stories: &[FileStories<'a>],
    canonical: impl Canonical,
) -> Vec<Vec<String>> {
    let mut resolver = Resolver::new(components, canonical);
    let mut args: Vec<BTreeSet<&str>> = vec![BTreeSet::new(); components.len()];
    for story in stories {
        if let Some(at) = resolver.path(story.file, story.component) {
            args[at].extend(story.args.iter().map(String::as_str));
        }
    }
    args.into_iter().map(|names| names.into_iter().map(str::to_owned).collect()).collect()
}

impl CallSites {
    /// The props passed at the floor: at 20% or more of the sites, and at two at least.
    #[cfg_attr(not(test), expect(dead_code, reason = "decide applies the floor itself"))]
    pub(crate) fn used(&self) -> Vec<&str> {
        self.passed.iter().filter(|(_, count)| self.at_floor(*count)).map(|(name, _)| name.as_str()).collect()
    }
}

impl JsxUsageAttribute {
    fn name(&self) -> Option<&str> {
        match self {
            Self::Named(name) => Some(name),
            Self::Spread => None,
        }
    }
}

impl JsxAttributeFact {
    fn name(&self) -> Option<&str> {
        match self {
            Self::Named { name, .. } => Some(name),
            Self::Spread(_) => None,
        }
    }
}

/// Resolves tags to components, by symbol and import.
struct Resolver<'c, F> {
    components: &'c [ComponentMapping],
    /// The components each file declares, by index.
    by_file: BTreeMap<&'c Utf8Path, Vec<usize>>,
    canonical: F,
    /// Answers already given, per file and tag path.
    memo: FxHashMap<(&'c Utf8Path, &'c str), Option<usize>>,
}

impl<'c, F: Canonical> Resolver<'c, F> {
    fn new(components: &'c [ComponentMapping], canonical: F) -> Self {
        let mut by_file: BTreeMap<&Utf8Path, Vec<usize>> = BTreeMap::new();
        for (at, component) in components.iter().enumerate() {
            by_file.entry(&component.file_path).or_default().push(at);
        }
        Self { components, by_file, canonical, memo: FxHashMap::default() }
    }

    /// The component a tag in `file` is, if it is one that resolves.
    fn tag(&mut self, file: &'c Utf8Path, tag: &'c JsxTag) -> Option<usize> {
        match tag {
            // A tag bound in an inner scope holds whatever it holds at run time: no module binding to resolve.
            JsxTag::Intrinsic(_) | JsxTag::Scoped(_) => None,
            JsxTag::Identifier(path) | JsxTag::Member(path) => self.path(file, path),
        }
    }

    /// The component a dotted path written in `file` names: the file's own binding, the binding an import reaches, or
    /// the member of a namespace import.
    fn path(&mut self, file: &'c Utf8Path, path: &'c str) -> Option<usize> {
        let key = (file, path);
        if let Some(known) = self.memo.get(&key) {
            return *known;
        }
        let parts: Vec<&str> = path.split('.').collect();
        let found = self.declared(file, parts[0], &parts[1..], false).or_else(|| {
            let (declaring, name) = (self.canonical)(file, parts[0])?;
            self.declared(&declaring, &name, &parts[1..], true)
        });
        let found = found.or_else(|| {
            let (namespace, rest) = parts.split_at_checked(2)?;
            let (declaring, name) = (self.canonical)(file, &namespace.join("."))?;
            self.declared(&declaring, &name, rest, true)
        });
        self.memo.insert(key, found);
        found
    }

    /// The component `file` declares under the binding `name`, or, when the name is an answer of [`Canonical`], exports
    /// under it (an anonymous `default`); then the sub-component attached to its export along `members`.
    fn declared(&self, file: &Utf8Path, name: &str, members: &[&str], exported: bool) -> Option<usize> {
        let in_file = self.by_file.get(file)?;
        let exports_it =
            |c: &ComponentMapping| exported && c.exports.iter().any(|e| is_export(e, name, &[] as &[&str]));
        let base = *in_file.iter().find(|&&at| {
            let component = &self.components[at];
            (component.bound && component.binding_name() == name) || exports_it(component)
        })?;
        if members.is_empty() {
            return Some(base);
        }
        let exports = &self.components[base].exports;
        in_file.iter().copied().find(|&at| {
            self.components[at].exports.iter().any(|attached| {
                exports.iter().any(|export| {
                    let path: Vec<&str> =
                        export.members.iter().map(String::as_str).chain(members.iter().copied()).collect();
                    is_export(attached, &export.name, &path)
                })
            })
        })
    }
}

/// Whether `export` is `name` reached through `members`.
fn is_export(export: &ComponentExport, name: &str, members: &[impl AsRef<str>]) -> bool {
    export.name == name
        && export.members.len() == members.len()
        && export.members.iter().zip(members).all(|(a, b)| a == b.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyFacts, DetectionRule, JsxElementFact, SpreadOf, TypeName};

    fn component(file: &str, binding: &str, exports: &[ComponentExport]) -> ComponentMapping {
        let mut mapping = ComponentMapping::new(
            binding.to_owned(),
            TypeName::default(),
            file.into(),
            0,
            0,
            DetectionRule::FunctionDeclaration,
        );
        mapping.exports = exports.to_vec();
        mapping
    }

    fn usage(tag: JsxTag, named: &[&str], spread: bool) -> JsxUsage {
        let mut attributes: Vec<_> = named.iter().map(|name| JsxUsageAttribute::Named((*name).to_owned())).collect();
        if spread {
            attributes.push(JsxUsageAttribute::Spread);
        }
        JsxUsage { tag, attributes }
    }

    fn id(name: &str) -> JsxTag {
        JsxTag::Identifier(name.to_owned())
    }

    /// No import resolves.
    fn nothing(_: &Utf8Path, _: &str) -> Option<(Utf8PathBuf, String)> {
        None
    }

    /// Resolves the listed `(file, name)` imports.
    fn imports<'t>(table: &'t [(&'t str, &'t str, &'t str, &'t str)]) -> impl Canonical + 't {
        move |file: &Utf8Path, name: &str| {
            table
                .iter()
                .find(|(f, n, _, _)| file == *f && name == *n)
                .map(|(_, _, to, as_)| ((*to).into(), (*as_).to_owned()))
        }
    }

    fn sites(n: usize, tag: &JsxTag, named: &[&str]) -> Vec<JsxUsage> {
        (0..n).map(|_| usage(tag.clone(), named, false)).collect()
    }

    #[test]
    fn a_prop_passed_at_a_quarter_of_the_sites_is_used_and_one_at_a_tenth_is_not() {
        let button = component("/b.tsx", "Button", &[ComponentExport::named("Button")]);
        let mut usages = sites(5, &id("Button"), &["tone"]);
        usages.extend(sites(2, &id("Button"), &["size"]));
        usages.extend(sites(13, &id("Button"), &[]));
        let counted = count(&[button], &[FileUsages { file: "/b.tsx".into(), usages: &usages }], nothing);
        assert_eq!(counted[0].sites, 20);
        assert_eq!(counted[0].used(), ["tone"]);
        assert_eq!(counted[0].passed, [("size".to_owned(), 2), ("tone".to_owned(), 5)]);
    }

    #[test]
    fn fewer_than_three_resolved_sites_give_no_evidence() {
        let button = component("/b.tsx", "Button", &[]);
        let usages = sites(2, &id("Button"), &["tone"]);
        let counted = count(&[button], &[FileUsages { file: "/b.tsx".into(), usages: &usages }], nothing);
        assert_eq!(counted[0], CallSites::default());
    }

    #[test]
    fn spreads_are_not_counted_and_a_repeated_attribute_counts_once() {
        let button = component("/b.tsx", "Button", &[]);
        let usages: Vec<_> = (0..3).map(|_| usage(id("Button"), &["tone", "tone"], true)).collect();
        let counted = count(&[button], &[FileUsages { file: "/b.tsx".into(), usages: &usages }], nothing);
        assert_eq!(counted[0].passed, [("tone".to_owned(), 3)]);
    }

    #[test]
    fn an_imported_tag_resolves_through_the_import_to_the_declaring_binding() {
        let components = [
            component("/ui/button.tsx", "Button", &[ComponentExport::named("Button")]),
            component("/other/button.tsx", "Button", &[ComponentExport::named("Button")]),
        ];
        let usages = sites(3, &id("Btn"), &["tone"]);
        let table = [("/app.tsx", "Btn", "/ui/button.tsx", "Button")];
        let counted = count(&components, &[FileUsages { file: "/app.tsx".into(), usages: &usages }], imports(&table));
        assert_eq!((counted[0].sites, counted[1].sites), (3, 0), "by import, not by display name");
    }

    #[test]
    fn unresolved_tags_count_nowhere() {
        let button = component("/ui/button.tsx", "Button", &[ComponentExport::named("Button")]);
        let mut usages = sites(3, &id("Button"), &["a"]); // same display name, not imported, not declared here
        usages.extend(sites(3, &JsxTag::Intrinsic("button".to_owned()), &["a"]));
        usages.extend(sites(3, &JsxTag::Member("Button.Item".to_owned()), &["a"]));
        usages.extend(sites(3, &id("this"), &["a"]));
        let table = [("/app.tsx", "Gone", "/missing.tsx", "Gone")];
        usages.extend(sites(3, &id("Gone"), &["a"]));
        let counted = count(&[button], &[FileUsages { file: "/app.tsx".into(), usages: &usages }], imports(&table));
        assert_eq!(counted[0], CallSites::default());
    }

    #[test]
    fn a_tag_an_inner_binding_shadows_is_no_site_of_the_module_component() {
        let button = component("/b.tsx", "Button", &[ComponentExport::named("Button")]);
        let usages = sites(3, &JsxTag::Scoped("Button".to_owned()), &["tone"]);
        let counted = count(&[button], &[FileUsages { file: "/b.tsx".into(), usages: &usages }], nothing);
        assert_eq!(counted[0], CallSites::default());
    }

    #[test]
    fn an_anonymous_default_export_resolves_by_its_export() {
        let mut card = component("/card.tsx", "card", &[ComponentExport::named("default")]);
        card.bound = false;
        let usages = sites(3, &id("Card"), &["a"]);
        let table = [("/app.tsx", "Card", "/card.tsx", "default")];
        let counted = count(&[card], &[FileUsages { file: "/app.tsx".into(), usages: &usages }], imports(&table));
        assert_eq!(counted[0].sites, 3);
    }

    #[test]
    fn a_member_tag_is_the_sub_component_attached_to_the_export() {
        let menu = ComponentExport::named("Menu");
        let components = [
            component("/menu.tsx", "Menu", std::slice::from_ref(&menu)),
            component("/menu.tsx", "MenuItem", &[menu.member("Item")]),
        ];
        let usages = sites(3, &JsxTag::Member("M.Item".to_owned()), &["a"]);
        let table = [("/app.tsx", "M", "/menu.tsx", "Menu")];
        let counted = count(&components, &[FileUsages { file: "/app.tsx".into(), usages: &usages }], imports(&table));
        assert_eq!((counted[0].sites, counted[1].sites), (0, 3));
    }

    #[test]
    fn a_member_of_a_namespace_import_is_what_its_module_declares() {
        let root = component("/dialog.tsx", "DialogRoot", &[ComponentExport::named("Root")]);
        let usages = sites(3, &JsxTag::Member("Dialog.Root".to_owned()), &["open"]);
        let table = [("/app.tsx", "Dialog.Root", "/dialog.tsx", "DialogRoot")];
        let counted = count(&[root], &[FileUsages { file: "/app.tsx".into(), usages: &usages }], imports(&table));
        assert_eq!(counted[0].used(), ["open"]);
    }

    #[test]
    fn elements_inside_component_bodies_are_call_sites() {
        let mut card = component("/card.tsx", "Card", &[]);
        let element = JsxElementFact {
            tag: id("Button"),
            attributes: vec![
                JsxAttributeFact::Named { name: "tone".to_owned(), prop: None },
                JsxAttributeFact::Spread(SpreadOf::Rest),
            ],
        };
        card.body = Some(BodyFacts { elements: vec![element; 3], ..BodyFacts::default() });
        let components = [card, component("/button.tsx", "Button", &[ComponentExport::named("Button")])];
        let table = [("/card.tsx", "Button", "/button.tsx", "Button")];
        let counted = count(&components, &[], imports(&table));
        assert_eq!(counted[1].passed, [("tone".to_owned(), 3)]);
    }

    #[test]
    fn any_story_arg_is_used_with_no_floor() {
        let components = [component("/button.tsx", "Button", &[ComponentExport::named("Button")])];
        let (one, two) = (["tone".to_owned()], ["size".to_owned(), "tone".to_owned()]);
        let stories = [
            FileStories { file: "/button.stories.tsx".into(), component: "Button", args: &one },
            FileStories { file: "/x.stories.tsx".into(), component: "Button", args: &two },
            FileStories { file: "/y.stories.tsx".into(), component: "Unknown", args: &two },
        ];
        let table = [
            ("/button.stories.tsx", "Button", "/button.tsx", "Button"),
            ("/x.stories.tsx", "Button", "/button.tsx", "Button"),
        ];
        assert_eq!(story_args(&components, &stories, imports(&table)), [["size", "tone"]]);
    }
}
