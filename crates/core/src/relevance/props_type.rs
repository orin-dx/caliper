//! The evidence a component's props type gives about each member it reaches (ADR 0028, slice T2): the props type's
//! own members, the keys a `Pick` names, the members that redeclare or narrow a base member, the required and defaulted
//! ones, and the JSDoc and literal-union flags [`decide`](super::decide) only orders rows by.
//!
//! [`members`] is the one function that builds it, from the same inputs in both lanes: what the props type writes
//! ([`PropsType`], read from its syntax, so the same for both), and the lane's answer (its listed props and its
//! collapsed layers). The checker lane's props carry the checker's `parent` and `declarations` (ADR 0026 S7a), the
//! structural lane's the resolver's; where neither names a base, a collapsed layer that holds the member does. A member
//! only a layer names is known by its name alone in both lanes, so it is never required or documented here.

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};

use super::Member;
use crate::types::collected::{CollectedInterface, CollectedType, CollectedTypeAlias, ComponentMapping, ExtendsRef};
use crate::types::output::{InheritedLayer, ParsedProp, PropParent, PropType};

/// How deep [`PropsType::read`] follows aliases of `Pick` keys, so a cyclic alias ends.
const MAX_ALIAS_DEPTH: u8 = 8;

/// A declaration the props type names, as the caller finds it by name from the component's file.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Declaration<'a> {
    /// An interface.
    Interface(&'a CollectedInterface),
    /// A type alias.
    Alias(&'a CollectedTypeAlias),
}

/// What a component's props type says itself, read once from its syntax.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PropsType {
    /// The interface the props type is, when it is one: a member whose `parent` it is was declared here.
    interface: Option<(String, String)>,
    /// The members the props type writes itself: its interface's own members and the fields of its type literals.
    own: BTreeSet<String>,
    /// The keys a `Pick` in the props type itself names. An `Omit` names nothing kept.
    picked: BTreeSet<String>,
    /// The props the signature gives a default: a destructuring default or `defaultProps`.
    defaulted: BTreeSet<String>,
    /// The members the interfaces it extends or intersects declare outside the ambient declarations.
    inherited: BTreeSet<String>,
}

/// Finds a name written in a file: the declaration it names and the file declaring it, through imports.
pub(crate) trait Lookup<'a>: Fn(&str, &Utf8Path) -> Option<(Declaration<'a>, Utf8PathBuf)> {}

impl<'a, F: Fn(&str, &Utf8Path) -> Option<(Declaration<'a>, Utf8PathBuf)>> Lookup<'a> for F {}

impl PropsType {
    /// The members a forwarding target's props type reaches outside the ambient declarations, as `forwarding` takes them:
    /// its own, its picked keys and those of the interfaces it extends or intersects, in name order. Read from the
    /// syntax, so the same in both lanes, and known before the target is resolved.
    pub(crate) fn reachable_members(&self) -> Vec<Member> {
        let names: BTreeSet<&String> = self.own.iter().chain(&self.picked).chain(&self.inherited).collect();
        names.into_iter().map(|name| Member { name: name.clone(), own: true, ..Member::default() }).collect()
    }

    /// Reads the props type of `mapping`, finding each name it writes with `lookup` from the file that writes it (the
    /// component's, then the declaring file of each declaration it reaches). The members of a base it extends or
    /// intersects are inherited: the lanes report them, and only [`PropsType::reachable_members`] reads them.
    pub(crate) fn read<'a>(mapping: &ComponentMapping, lookup: impl Lookup<'a>) -> Self {
        let mut read = Self { defaulted: mapping.param_defaults.keys().cloned().collect(), ..Self::default() };
        let from = mapping.file_path.as_path();
        // `props: Pick<Base, Keys>` is the mapping's own name and arguments, not a declaration to look up.
        read.pick(&mapping.props_type_name, &mapping.props_type_args, from, &lookup);
        match lookup(&mapping.props_type_name, from) {
            Some((Declaration::Interface(interface), _)) => {
                read.interface = Some((interface.name.to_string(), interface.file_path.to_string()));
                read.own.extend(interface.props.iter().map(|prop| prop.name.clone()));
                read.bases(interface, &lookup, 0);
            }
            Some((Declaration::Alias(alias), declared)) => match alias {
                CollectedTypeAlias::Pick { keys, .. } => read.keys(keys, &declared, &lookup, 0),
                // A union's arms are each the props type (a discriminated union's fields are its own).
                CollectedTypeAlias::Intersection { members, .. } | CollectedTypeAlias::Union { members, .. } => {
                    members.iter().for_each(|member| read.part(member, &declared, &lookup));
                }
                CollectedTypeAlias::Passthrough { target: part, .. }
                | CollectedTypeAlias::Partial { base: part, .. }
                | CollectedTypeAlias::Required { base: part, .. } => read.part(part, &declared, &lookup),
                CollectedTypeAlias::Omit { .. } | CollectedTypeAlias::LiteralUnion { .. } => {}
            },
            None => {}
        }
        read
    }

    /// The members `interface`'s bases declare outside the ambient declarations, through their own bases, each base
    /// found from the file that names it; and the keys a `Pick` among them names.
    fn bases<'a>(&mut self, interface: &CollectedInterface, lookup: &impl Lookup<'a>, depth: u8) {
        for base in &interface.extends {
            let (ExtendsRef::SameFile { name, type_args } | ExtendsRef::Imported { local_name: name, type_args, .. }) =
                base
            else {
                continue;
            };
            self.pick(name, type_args, &interface.file_path, lookup);
            self.inherit(name, &interface.file_path, lookup, depth);
        }
    }

    /// The members the interface `name` written in `from` declares and inherits, unless it is ambient.
    fn inherit<'a>(&mut self, name: &str, from: &Utf8Path, lookup: &impl Lookup<'a>, depth: u8) {
        if depth >= MAX_ALIAS_DEPTH {
            return;
        }
        if let Some((Declaration::Interface(base), declared)) = lookup(name, from)
            && !crate::origin::declaration_origin(declared.as_str()).is_ambient()
        {
            self.inherited.extend(base.props.iter().map(|prop| prop.name.clone()));
            self.bases(base, lookup, depth + 1);
        }
    }

    /// One part of the props type written in `from`: a type literal's fields are its own, an intersection's parts are
    /// parts, a `Pick` names its keys; any other name is a base.
    fn part<'a>(&mut self, part: &CollectedType, from: &Utf8Path, lookup: &impl Lookup<'a>) {
        match part {
            CollectedType::Object(fields) | CollectedType::Callable { fields, .. } => {
                self.own.extend(fields.iter().map(|field| field.name.clone()));
            }
            CollectedType::Intersection(parts) | CollectedType::Union(parts) => {
                parts.iter().for_each(|part| self.part(part, from, lookup));
            }
            CollectedType::Named { name, args } => {
                self.pick(name, args, from, lookup);
                self.inherit(name, from, lookup, 0);
            }
            _ => {}
        }
    }

    /// The keys of `name<args>` written in `from` when it is `Pick<Base, Keys>`.
    fn pick<'a>(&mut self, name: &str, args: &[CollectedType], from: &Utf8Path, lookup: &impl Lookup<'a>) {
        if let ("Pick", [_, keys]) = (name, args) {
            self.keys(keys, from, lookup, 0);
        }
    }

    /// The string literal keys `keys` written in `from` names, through unions and aliases, each alias found from the
    /// file that names it.
    fn keys<'a>(&mut self, keys: &CollectedType, from: &Utf8Path, lookup: &impl Lookup<'a>, depth: u8) {
        match keys {
            CollectedType::StringLiteral(key) => {
                self.picked.insert(key.to_string());
            }
            CollectedType::Union(keys) => keys.iter().for_each(|key| self.keys(key, from, lookup, depth)),
            CollectedType::Named { name, args } if args.is_empty() && depth < MAX_ALIAS_DEPTH => {
                let Some((Declaration::Alias(alias), declared)) = lookup(name, from) else { return };
                match alias {
                    CollectedTypeAlias::LiteralUnion { members, .. } => self.picked.extend(members.iter().cloned()),
                    CollectedTypeAlias::Passthrough { target, .. } => self.keys(target, &declared, lookup, depth + 1),
                    CollectedTypeAlias::Union { members, .. } => {
                        members.iter().for_each(|key| self.keys(key, &declared, lookup, depth + 1));
                    }
                    CollectedTypeAlias::Pick { .. }
                    | CollectedTypeAlias::Omit { .. }
                    | CollectedTypeAlias::Intersection { .. }
                    | CollectedTypeAlias::Partial { .. }
                    | CollectedTypeAlias::Required { .. } => {}
                }
            }
            _ => {}
        }
    }

    /// Whether `parent` is the interface the props type is.
    fn declares(&self, parent: &PropParent) -> bool {
        self.interface.as_ref().is_some_and(|(name, file)| *name == parent.name && *file == parent.file_name)
    }
}

/// The props-type evidence of every member a lane's answer reaches: each prop it lists (`rows`, with its
/// `notableInherited` ones) in order, then each member of its collapsed `layers` it does not list. `ambient` says
/// whether a declaring file is ambient (ADR 0028: by path, never by name); a member with no declaring file is not.
///
/// A member is own when the props type writes it, and overrides when, besides, a base declares it: its `parent` or a
/// declaration is not the props type, or a collapsed layer holds it.
pub(crate) fn members<'a>(
    props_type: &PropsType,
    rows: impl IntoIterator<Item = &'a ParsedProp>,
    layers: &[InheritedLayer],
) -> Vec<Member> {
    let ambient = |file: &str| crate::origin::declaration_origin(file).is_ambient();
    let layered: BTreeSet<&str> = layers.iter().flat_map(|layer| layer.members.iter().map(String::as_str)).collect();
    let mut listed = BTreeSet::new();
    let mut members = Vec::new();
    for prop in rows {
        if !listed.insert(prop.name.as_str()) {
            continue;
        }
        let own = props_type.own.contains(&prop.name) || prop.parent.as_ref().is_some_and(|p| props_type.declares(p));
        let based = layered.contains(prop.name.as_str())
            || prop.parent.iter().chain(&prop.declarations).any(|parent| !props_type.declares(parent));
        let given = member(props_type, &prop.name);
        members.push(Member {
            own,
            overrides: own && based,
            required: prop.required,
            default_tag: given.default_tag || prop.default_value.is_some() || prop.tags.contains_key("default"),
            documented: !prop.description.is_empty(),
            literal_union: is_literal_union(&prop.prop_type),
            // The checker states `parent` only for an interface; a declaration in a type literal is still in a file.
            ambient: prop.parent.iter().chain(&prop.declarations).any(|parent| ambient(&parent.file_name)),
            ..given
        });
    }
    for layer in layers {
        for name in layer.members.iter().filter(|name| !listed.contains(name.as_str())) {
            let own = props_type.own.contains(name);
            members.push(Member {
                own,
                overrides: own,
                ambient: ambient(&layer.file_name),
                ..member(props_type, name)
            });
        }
    }
    members
}

/// The evidence the props type gives `name` whatever the lane says: picked, defaulted by the signature.
fn member(props_type: &PropsType, name: &str) -> Member {
    Member {
        name: name.to_owned(),
        picked: props_type.picked.contains(name),
        default_tag: props_type.defaulted.contains(name),
        ..Member::default()
    }
}

/// Whether `prop_type` is a union of literals or enum members (`null` and `undefined` beside them allowed), or a
/// variant union.
fn is_literal_union(prop_type: &PropType) -> bool {
    let members = match prop_type {
        PropType::LiteralUnion { .. } => return true,
        PropType::Union(members) => members,
        _ => return false,
    };
    let literal = |member: &PropType| {
        matches!(
            member,
            PropType::StringLiteral(_)
                | PropType::NumberLiteral(_)
                | PropType::BoolLiteral(_)
                | PropType::BigIntLiteral(_)
                | PropType::EnumMember { .. }
        )
    };
    members.iter().any(literal)
        && members.iter().all(|member| literal(member) || matches!(member, PropType::Null | PropType::Undefined))
}

#[cfg(test)]
mod tests {
    use camino::{Utf8Path, Utf8PathBuf};

    use super::*;
    use crate::extractor::parse_file;
    use crate::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
    use crate::relevance::{Body, Evidence, Shown, decide};
    use crate::types::collected::SourceData;
    use crate::types::output::ComponentEntry;

    /// A source of 33 members, `tone` required: collapsed in the `All` mode.
    fn big() -> String {
        let fields: String = (0..31).map(|i| format!("p{i}?: string; ")).collect();
        format!(
            "export interface Big {{ size?: string; tone: string; {fields}}}\n\
             interface Small {{ need: string; maybe?: string }}\n\
             interface Narrows extends Big {{ size?: 'sm' | 'md' }}\n\
             type More = 'p6' | 'p7';\n\
             type Keys = 'p3' | 'p4' | More;\n\
             type Either = {{ kind: 'a'; a?: string }} | {{ kind: 'b'; b?: string }};\n\
             interface Picks extends Pick<Big, Keys> {{ x?: string }}\n"
        )
    }

    const COMPONENTS: &str = "export function ByExtends(props: Narrows) { return null; }\n\
        export function ByIntersection(props: Big & { size?: 'sm' }) { return null; }\n\
        export function ByPick(props: Pick<Big, 'p1' | 'p2'> & { own?: boolean }) { return null; }\n\
        export function ByOmit({ p5 = 'x' }: Omit<Big, 'p1'> & { own?: boolean }) { return null; }\n\
        export function ByRequired(props: Small & { own?: boolean }) { return null; }\n\
        export function ByPickAlone(props: Pick<Big, 'tone'>) { return null; }\n\
        export function ByPickedBase(props: Picks) { return null; }\n\
        export function ByUnion(props: Either) { return null; }\n";

    fn lookup<'a>(data: &'a SourceData, path: &Utf8Path, name: &str) -> Option<(Declaration<'a>, Utf8PathBuf)> {
        let key = format!("{path}:{name}");
        data.interfaces
            .get(&key)
            .map(Declaration::Interface)
            .or_else(|| data.type_aliases.get(&key).map(Declaration::Alias))
            .map(|declaration| (declaration, path.to_owned()))
    }

    /// The props type of each component of `text`, read from its syntax.
    fn props_types(path: &Utf8Path, text: &str) -> Vec<(String, PropsType)> {
        let data = parse_file(path, text);
        let read =
            |mapping: &ComponentMapping| PropsType::read(mapping, |name: &str, _: &Utf8Path| lookup(&data, path, name));
        data.component_mappings.iter().map(|mapping| (mapping.component_name.clone(), read(mapping))).collect()
    }

    fn props_type(component: &str) -> PropsType {
        let path = Utf8Path::new("/fixtures/Components.tsx");
        let found = props_types(path, &format!("{}{COMPONENTS}", big()));
        found.into_iter().find(|(name, _)| name == component).map(|(_, read)| read).unwrap()
    }

    fn names(set: &BTreeSet<String>) -> Vec<&str> {
        set.iter().map(String::as_str).collect()
    }

    /// What `decide` makes of the props type's evidence alone, the body known to read nothing.
    fn decided(members: Vec<Member>) -> Shown {
        let call_sites = &crate::relevance::NO_CALL_SITES;
        decide(&Evidence { members: &members, body: Some(&Body::default()), call_sites })
    }

    #[test]
    fn a_member_that_narrows_a_base_member_is_a_row_when_its_source_collapses() {
        // A source only the checker read: the lane put the redeclared `size` in the collapsed layer with the rest.
        let layer = InheritedLayer {
            type_name: "Big".to_owned(),
            file_name: "/fixtures/Components.tsx".to_owned(),
            omitted: Vec::new(),
            html_element: None,
            total_props: 3,
            members: vec!["size".to_owned(), "tone".to_owned(), "p0".to_owned()],
            used: Vec::new(),
        };
        let members = members(&props_type("ByIntersection"), [], &[layer]);
        let size = members.iter().find(|member| member.name == "size").unwrap();
        assert!(size.own && size.overrides, "{size:?}");
        let shown = decided(members);
        assert_eq!(shown.rows, ["size"]);
        assert_eq!(shown.collapsed, ["tone", "p0"]);
    }

    #[test]
    fn a_pick_names_its_keys_and_an_omit_names_nothing() {
        let pick = props_type("ByPick");
        assert_eq!((names(&pick.picked), names(&pick.own)), (vec!["p1", "p2"], vec!["own"]));
        let omit = props_type("ByOmit");
        assert_eq!((names(&omit.picked), names(&omit.own), names(&omit.defaulted)), (vec![], vec!["own"], vec!["p5"]));
        assert_eq!(names(&props_type("ByPickAlone").picked), ["tone"]);
        let base = props_type("ByPickedBase");
        assert_eq!((names(&base.picked), names(&base.own)), (vec!["p3", "p4", "p6", "p7"], vec!["x"]));
        assert_eq!(names(&props_type("ByUnion").own), ["a", "b", "kind"]);
        let extends = props_type("ByExtends");
        assert_eq!(
            (names(&extends.own), extends.interface.map(|(name, _)| name)),
            (vec!["size"], Some("Narrows".into()))
        );
    }

    /// Each component of the project as the structural lane answers, and as the checker lane does: the structural
    /// answer with the checker's `parent` and `declarations` merged over it (ADR 0026 S7a). The checker names a base
    /// interface first where an intersection redeclares one of its members beside it (TypeScript's first declaration),
    /// so a member a type literal writes and a collapsed source holds has that source as `parent` in the checker lane.
    fn lanes() -> Vec<(String, PropsType, ComponentEntry, ComponentEntry)> {
        let dir = tempfile::Builder::new().prefix("PropsTypeModel").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let path = root.join("Components.tsx");
        let text = format!("{}{COMPONENTS}", big());
        std::fs::write(&path, &text).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![root.clone()],
            cache_dir: Some(root.join(".cache")),
            html_attributes: HtmlAttributeMode::None,
            collapse_inherited_props: CollapseInheritedProps::All,
            ..PipelineOptions::default()
        };
        let output = extract(&options);
        props_types(&path, &text)
            .into_iter()
            .map(|(name, read)| {
                let structural = output.components[&name].clone();
                let mut checker = structural.clone();
                for prop in checker.props.values_mut() {
                    let Some(layer) = structural.inheritance.iter().find(|layer| layer.members.contains(&prop.name))
                    else {
                        continue;
                    };
                    if prop.parent.is_none() {
                        let base = PropParent { name: layer.type_name.clone(), file_name: layer.file_name.clone() };
                        *prop = std::sync::Arc::new(ParsedProp::new(
                            prop.name.clone(),
                            prop.prop_type.clone(),
                            prop.required,
                            prop.default_value.clone(),
                            prop.description.clone(),
                            prop.tags.clone(),
                            Some(base.clone()),
                            vec![base],
                        ));
                    }
                }
                (name, read, structural, checker)
            })
            .collect()
    }

    fn evidence(read: &PropsType, entry: &ComponentEntry) -> Vec<Member> {
        let rows = entry.props.values().chain(entry.notable_inherited.values()).map(AsRef::as_ref);
        members(read, rows, &entry.inheritance)
    }

    #[test]
    fn both_lanes_give_the_same_evidence_for_pick_omit_override_and_required() {
        for (name, read, structural, checker) in lanes() {
            let (from_structural, from_checker) = (evidence(&read, &structural), evidence(&read, &checker));
            assert_eq!(from_structural, from_checker, "{name}");
            let shown = decided(from_structural);
            let row = |prop: &str| shown.rows.iter().any(|row| row == prop);
            match name.as_str() {
                "ByExtends" | "ByIntersection" => {
                    assert!(row("size") && !row("tone") && !row("p0"), "{name}: {shown:?}")
                }
                "ByPick" => assert_eq!(shown.rows, ["own", "p1", "p2"], "{name}"),
                "ByOmit" => {
                    assert!(row("own") && row("p5") && !row("p1") && !row("p0"), "{name}: {shown:?}");
                    assert!(!shown.collapsed.iter().any(|member| member == "p1"), "{name}: {shown:?}");
                }
                "ByRequired" => {
                    assert_eq!(
                        (shown.rows, shown.collapsed),
                        (vec!["need".to_owned(), "own".to_owned()], vec!["maybe".to_owned()])
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_doc_comment_or_literal_union_is_reported_but_promotes_nothing_alone() {
        let union = PropType::Union(vec![PropType::StringLiteral("a".into()), PropType::Undefined]);
        let documented = ParsedProp::new(
            "tone".into(),
            union,
            false,
            None,
            "The tone.".into(),
            Default::default(),
            Some(PropParent { name: "Base".into(), file_name: "/base.ts".into() }),
            Vec::new(),
        );
        let members = members(&PropsType::default(), [&documented], &[]);
        assert!(members[0].documented && members[0].literal_union && !members[0].own, "{members:?}");
        assert_eq!(decided(members).collapsed, ["tone"]);
        assert!(!is_literal_union(&PropType::Union(vec![PropType::String, PropType::StringLiteral("a".into())])));
        // A declaration in a type literal has no `parent` in the checker lane; its file still says ambient.
        let react = PropParent { name: "__type".into(), file_name: "/node_modules/@types/react/index.d.ts".into() };
        let handler = ParsedProp::new(
            "onClick".into(),
            PropType::Any,
            false,
            None,
            String::new(),
            Default::default(),
            None,
            vec![react],
        );
        assert!(super::members(&PropsType::default(), [&handler], &[])[0].ambient);
    }

    /// Two files: the component's, and one declaring its props type and what that type names.
    fn across_files(component: &str, declarations: &str) -> PropsType {
        let (here, there) = (Utf8Path::new("/fixtures/Button.tsx"), Utf8Path::new("/fixtures/props.ts"));
        let (component, declarations) = (parse_file(here, component), parse_file(there, declarations));
        // Only the props type is imported; every other name is found in the file that writes it.
        let lookup = |name: &str, from: &Utf8Path| {
            let (data, path) = match (from == here, name) {
                (true, "Props") => (&declarations, there),
                (true, _) => (&component, here),
                (false, _) => (&declarations, there),
            };
            lookup(data, path, name)
        };
        PropsType::read(&component.component_mappings[0], lookup)
    }

    #[test]
    fn a_pick_of_an_imported_props_type_finds_its_keys_where_the_props_type_is_declared() {
        let read = across_files(
            "import type { Props } from './props';\nexport function Button(props: Props) { return null; }\n",
            "interface Base { a?: string; b?: string; c?: string }\ntype Keys = 'a' | 'b';\n\
             export type Props = Pick<Base, Keys>;\n",
        );
        assert_eq!(read.picked, BTreeSet::from(["a".to_owned(), "b".to_owned()]));
    }

    #[test]
    fn a_forwarding_target_that_only_extends_a_base_reaches_the_bases_members() {
        let read = across_files(
            "import type { Props } from './props';\nexport function Inner(props: Props) { return null; }\n",
            "interface Base { tone?: string; size?: string }\nexport interface Props extends Base {}\n",
        );
        let names: Vec<String> = read.reachable_members().into_iter().map(|member| member.name).collect();
        assert_eq!(names, ["size", "tone"]);
    }
}
