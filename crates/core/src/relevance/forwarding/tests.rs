use super::*;
use crate::extractor::parse_file;
use crate::import_map::ReExportStep;
use crate::relevance::{Body, Shown, decide};
use crate::types::collected::ComponentMapping;
use crate::types::global::GlobalSourceData;

/// A project parsed from in-memory files under `/p`.
struct Fixture {
    global: GlobalSourceData,
    imports: ImportResolutionMap,
    /// Each component's props, by the component's file and name, as a lane would resolve them.
    props: Vec<(&'static str, &'static str, Vec<Member>)>,
}

impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let mut global = GlobalSourceData::default();
        for (path, source) in files {
            let path = Utf8PathBuf::from(format!("/p/{path}"));
            global.merge(&path, parse_file(&path, source));
        }
        let imports = ImportResolutionMap::build(&global);
        Self { global, imports, props: Vec::new() }
    }

    fn props(mut self, file: &'static str, component: &'static str, members: Vec<Member>) -> Self {
        self.props.push((file, component, members));
        self
    }

    fn component(&self, file: &str, name: &str) -> &ComponentMapping {
        let path = format!("/p/{file}");
        self.global
            .component_mappings
            .iter()
            .find(|component| component.file_path == path && component.component_name == name)
            .unwrap_or_else(|| panic!("no component {name} in {file}"))
    }

    fn forwarding(&self, file: &str, name: &str) -> Forwarding {
        let project = Project { components: &self.global.component_mappings, imports: &self.imports };
        forwarding(self.component(file, name), project, self).expect("the component has a body")
    }

    /// What `decide` makes of `name`'s members with the forwarding input this module gives and the reads the
    /// extractor recorded.
    fn decide(&self, file: &str, name: &str, members: Vec<Member>) -> Shown {
        let forwarding = self.forwarding(file, name);
        let facts = self.component(file, name).body.clone().unwrap_or_default();
        let body = Body { reads: facts.reads, elements: forwarding.elements };
        let call_sites = &super::super::NO_CALL_SITES;
        decide(&super::super::Evidence { members: &members, body: Some(&body), call_sites })
    }
}

impl Lane for Fixture {
    /// Relative specifiers resolve to `.tsx`, then `.ts`, files of the fixture; re-exports are followed through the
    /// import map as the resolver follows them. A package specifier does not resolve.
    fn declaration(&self, from: &Utf8Path, specifier: &str, export: &str) -> Option<(Utf8PathBuf, String)> {
        let relative = specifier.strip_prefix("./")?;
        let directory = from.parent()?;
        let file = ["tsx", "ts"]
            .iter()
            .map(|extension| directory.join(format!("{relative}.{extension}")))
            .find(|file| self.global.re_export_map.contains_key(file) || self.global.import_map.contains_key(file))?;
        if let Some(declared) = self.imports.local_export(&file, export) {
            return Some((file, declared));
        }
        match self.imports.resolve_reexport_chain(&file, export) {
            Some(ReExportStep::Named { source_specifier, source_name }) => {
                self.declaration(&file, &source_specifier, &source_name)
            }
            // `export * from`: the first module that declares the name, as the resolver walks wildcards.
            Some(ReExportStep::Wildcards(specifiers)) => specifiers
                .iter()
                .filter_map(|specifier| self.declaration(&file, specifier, export))
                .find(|(declaring, declared)| {
                    self.imports.local_export(declaring, export).is_some() || declared != export
                })
                .or_else(|| Some((file, export.to_owned()))),
            None => Some((file, export.to_owned())),
        }
    }

    fn members(&self, component: &ComponentMapping) -> Option<Vec<Member>> {
        self.props.iter().find_map(|(file, name, members)| {
            (component.file_path == format!("/p/{file}") && component.component_name == *name).then(|| members.clone())
        })
    }
}

fn own(name: &str) -> Member {
    Member { name: name.to_owned(), own: true, ..Member::default() }
}

/// A member of a large source: listed only on evidence, which is what forwarding gives.
fn inherited(name: &str) -> Member {
    Member { name: name.to_owned(), large: true, ..Member::default() }
}

fn ambient(name: &str) -> Member {
    Member { ambient: true, ..own(name) }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

const INNER: &str = "interface InnerProps { tone?: string; size?: string; variant?: string }\n\
                     export function Inner(props: InnerProps) { return <div />; }\n";

/// `Inner`'s props as a lane gives them: three of its own, one inherited, one ambient.
fn inner_members() -> Vec<Member> {
    vec![own("tone"), own("size"), inherited("base"), ambient("onClick"), own("variant")]
}

/// `Outer`'s props: its own `label`, and `Inner`'s props plus one more, all inherited.
fn outer_members() -> Vec<Member> {
    vec![own("label"), inherited("tone"), inherited("size"), inherited("variant"), inherited("other")]
}

fn outer(jsx: &str) -> String {
    format!(
        "import {{ Inner }} from './inner';\n\
         export function Outer({{ label, ...rest }}: OuterProps) {{ return {jsx}; }}\n"
    )
}

fn project(outer_source: &str) -> Fixture {
    Fixture::new(&[("inner.tsx", INNER), ("outer.tsx", outer_source)]).props("inner.tsx", "Inner", inner_members())
}

#[test]
fn a_rest_spread_onto_a_component_shows_its_own_props_except_those_set_after_it() {
    let fixture = project(&outer("<Inner {...rest} size=\"sm\" />"));
    let shown = fixture.decide("outer.tsx", "Outer", outer_members());
    assert_eq!(shown.rows, ["label", "tone", "variant"]);
    assert_eq!(shown.collapsed, ["size", "other"]);
}

#[test]
fn an_attribute_set_before_the_spread_is_an_overridable_default_and_kept() {
    let fixture = project(&outer("<Inner size=\"sm\" {...rest} />"));
    let shown = fixture.decide("outer.tsx", "Outer", outer_members());
    assert_eq!(shown.rows, ["label", "tone", "size", "variant"]);
    assert_eq!(shown.collapsed, ["other"]);
}

#[test]
fn a_spread_forwards_one_hop_of_the_targets_own_non_ambient_props() {
    // `Inner` itself spreads onto `Deep`: `Outer`'s spread reaches `Inner`'s props, not `Deep`'s.
    let inner = "import { Deep } from './deep';\n\
                 export function Inner({ tone, ...rest }: InnerProps) { return <Deep {...rest} />; }\n";
    let deep = "export function Deep(props: DeepProps) { return <div />; }\n";
    let fixture =
        Fixture::new(&[("inner.tsx", inner), ("deep.tsx", deep), ("outer.tsx", &outer("<Inner {...rest} />"))])
            .props("inner.tsx", "Inner", inner_members())
            .props("deep.tsx", "Deep", vec![own("depth")]);
    let forwarding = fixture.forwarding("outer.tsx", "Outer");
    assert_eq!(forwarding.elements[0].target, Target::Component(strings(&["tone", "size", "base", "variant"])));
    let mut members = outer_members();
    members.extend([inherited("depth"), inherited("base"), inherited("onClick")]);
    let shown = fixture.decide("outer.tsx", "Outer", members);
    assert_eq!(shown.rows, ["label", "tone", "size", "variant", "base"]);
    assert_eq!(shown.collapsed, ["other", "depth", "onClick"]);
}

#[test]
fn an_attribute_valued_with_an_own_prop_adds_nothing() {
    for jsx in ["<Inner tone={label} />", "<Inner {...rest} tone={label} />"] {
        let fixture = project(&outer(jsx));
        let forwarding = fixture.forwarding("outer.tsx", "Outer");
        let named = forwarding.elements[0].attributes.iter().find(|attribute| matches!(attribute, Attribute::Named(_)));
        assert_eq!(named, Some(&Attribute::Named("tone".to_owned())), "{jsx}");
        let shown = fixture.decide("outer.tsx", "Outer", outer_members());
        assert!(shown.collapsed.contains(&"tone".to_owned()), "{jsx}: {shown:?}");
    }
}

#[test]
fn a_spread_onto_a_dom_tag_is_ignored() {
    let fixture = project(&outer("<button {...rest} />"));
    assert_eq!(fixture.forwarding("outer.tsx", "Outer").elements[0].target, Target::Dom);
    let shown = fixture.decide("outer.tsx", "Outer", outer_members());
    assert_eq!(shown.rows, ["label"]);
    assert_eq!(shown.collapsed, ["tone", "size", "variant", "other"]);
}

#[test]
fn the_tag_resolves_by_its_import_not_by_its_name() {
    // A renamed import resolves; a component of the same name elsewhere that the file does not import does not.
    let renamed = "import { Inner as Box } from './inner';\n\
                   export function Outer({ label, ...rest }: OuterProps) { return <Box {...rest} />; }\n";
    let shadowed = "const Inner = makeStyled('div');\n\
                    export function Outer({ label, ...rest }: OuterProps) { return <Inner {...rest} />; }\n";
    let fixture = Fixture::new(&[("inner.tsx", INNER), ("renamed.tsx", renamed), ("shadowed.tsx", shadowed)]).props(
        "inner.tsx",
        "Inner",
        inner_members(),
    );
    let own_props = Target::Component(strings(&["tone", "size", "base", "variant"]));
    assert_eq!(fixture.forwarding("renamed.tsx", "Outer").elements[0].target, own_props);
    assert_eq!(fixture.forwarding("shadowed.tsx", "Outer").elements[0].target, Target::Unresolved);
}

#[test]
fn a_tag_an_inner_binding_shadows_is_unknown_not_the_module_component() {
    // `Item` is a component of the file and an import of another; each body renders a different `Item` of its own.
    let item = "export function Item(props: ItemProps) { return <li />; }\n";
    let bodies = [
        "export function FromProp({ label, ...rest }: P) { const Item = rest.as; return <Item {...rest} />; }",
        "export function FromParam({ Item, ...rest }: P) { return <Item {...rest} />; }",
        "export function FromInner({ label, ...rest }: P) { function Item() { return null; } return <Item {...rest} />; }",
        "export function FromMember({ label, ...rest }: P) { const Item = rest.parts; return <Item.Row {...rest} />; }",
    ];
    for import in [false, true] {
        let mut source = if import { "import { Item } from './item';\n".to_owned() } else { item.to_owned() };
        source.push_str(&bodies.join("\n"));
        let files = [("item.tsx", item), ("outer.tsx", source.as_str())];
        let fixture = Fixture::new(&files[usize::from(!import)..]).props("item.tsx", "Item", vec![own("value")]).props(
            "outer.tsx",
            "Item",
            vec![own("value")],
        );
        for component in ["FromProp", "FromParam", "FromInner", "FromMember"] {
            let forwarding = fixture.forwarding("outer.tsx", component);
            let element = forwarding.elements.iter().find(|element| element.attributes.len() == 1).expect("the spread");
            assert_eq!(element.target, Target::Unresolved, "{component}, imported: {import}");
            let shown = fixture.decide("outer.tsx", component, vec![inherited("value")]);
            assert_eq!(shown.collapsed, ["value"], "{component}, imported: {import}");
        }
    }
}

#[test]
fn a_sub_component_its_module_also_exports_by_name_stays_unknown() {
    // The extractor gives `MenuItem` only its own export, not `default.Item`: the tag is unknown, never guessed.
    let menu = "function Menu(props: MenuProps) { return <div />; }\n\
                export function MenuItem(props: ItemProps) { return <div />; }\n\
                Menu.Item = MenuItem;\n\
                export default Menu;\n";
    let consumer = "import Menu from './menu';\n\
                    export function Outer({ label, ...rest }: OuterProps) { return <Menu.Item {...rest} />; }\n";
    let fixture = Fixture::new(&[("menu.tsx", menu), ("outer.tsx", consumer)])
        .props("menu.tsx", "Menu", vec![own("open")])
        .props("menu.tsx", "MenuItem", vec![own("item")]);
    assert_eq!(fixture.forwarding("outer.tsx", "Outer").elements[0].target, Target::Unresolved);
}

#[test]
fn a_local_component_resolves_by_its_binding() {
    let source =
        format!("{INNER}export function Outer({{ label, ...rest }}: OuterProps) {{ return <Inner {{...rest}} />; }}\n");
    let fixture = Fixture::new(&[("both.tsx", &source)]).props("both.tsx", "Inner", inner_members());
    assert_eq!(
        fixture.forwarding("both.tsx", "Outer").elements[0].target,
        Target::Component(strings(&["tone", "size", "base", "variant"]))
    );
}

#[test]
fn a_tag_resolves_through_a_barrel_and_a_namespace_import() {
    let barrel = "export { Inner } from './inner';\n";
    let consumer = "import * as UI from './index';\n\
                    export function Outer({ label, ...rest }: OuterProps) { return <UI.Inner {...rest} />; }\n";
    let fixture = Fixture::new(&[("inner.tsx", INNER), ("index.ts", barrel), ("outer.tsx", consumer)]).props(
        "inner.tsx",
        "Inner",
        inner_members(),
    );
    assert_eq!(
        fixture.forwarding("outer.tsx", "Outer").elements[0].target,
        Target::Component(strings(&["tone", "size", "base", "variant"]))
    );
}

#[test]
fn a_tag_resolves_through_a_wildcard_barrel() {
    let barrel = "export * from './other';\nexport * from './inner';\n";
    let consumer = "import { Inner } from './index';\n\
                    export function Outer({ label, ...rest }: OuterProps) { return <Inner {...rest} />; }\n";
    let other = "export function Other(props: OtherProps) { return <div />; }\n";
    let fixture =
        Fixture::new(&[("inner.tsx", INNER), ("other.tsx", other), ("index.ts", barrel), ("outer.tsx", consumer)])
            .props("inner.tsx", "Inner", inner_members());
    assert_eq!(
        fixture.forwarding("outer.tsx", "Outer").elements[0].target,
        Target::Component(strings(&["tone", "size", "base", "variant"]))
    );
}

#[test]
fn a_target_that_only_inherits_its_props_forwards_them() {
    // `interface ButtonProps extends BaseProps {}`: nothing declared by the props type itself, all of it forwarded.
    let fixture = Fixture::new(&[("inner.tsx", INNER), ("outer.tsx", &outer("<Inner {...rest} />"))]).props(
        "inner.tsx",
        "Inner",
        vec![inherited("tone"), inherited("size"), ambient("onClick")],
    );
    assert_eq!(
        fixture.forwarding("outer.tsx", "Outer").elements[0].target,
        Target::Component(strings(&["tone", "size"]))
    );
    let shown = fixture.decide("outer.tsx", "Outer", outer_members());
    assert_eq!(shown.rows, ["label", "tone", "size"]);
    assert_eq!(shown.collapsed, ["variant", "other"]);
}

#[test]
fn an_attached_sub_component_resolves_by_its_export_path() {
    // `Menu.Item = Row` beside `export default Menu`: the tag `Menu.Item` is `default.Item`, the component `Row`.
    let menu = "function Menu(props: MenuProps) { return <div />; }\n\
                function Row(props: RowProps) { return <div />; }\n\
                function Item(props: ItemProps) { return <div />; }\n\
                Menu.Item = Row;\n\
                export { Item };\n\
                export default Menu;\n";
    let consumer = "import Menu from './menu';\n\
                    export function Outer({ label, ...rest }: OuterProps) { return <Menu.Item {...rest} />; }\n";
    let fixture = Fixture::new(&[("menu.tsx", menu), ("outer.tsx", consumer)])
        .props("menu.tsx", "Menu", vec![own("open")])
        .props("menu.tsx", "Row", vec![own("row")])
        .props("menu.tsx", "Item", vec![own("item")]);
    assert_eq!(fixture.forwarding("outer.tsx", "Outer").elements[0].target, Target::Component(strings(&["row"])));
}

#[test]
fn a_spread_onto_a_tag_that_does_not_resolve_is_unknown() {
    let package = "import { Ext } from 'some-package';\n\
                   export function Outer({ label, ...rest }: OuterProps) { return <Ext {...rest} />; }\n";
    let factory = "import { styled } from './styled';\n\
                   export function Outer({ label, ...rest }: OuterProps) { return <styled.button {...rest} />; }\n";
    let fixture = Fixture::new(&[
        ("package.tsx", package),
        ("factory.tsx", factory),
        ("styled.ts", "export const styled = {};\n"),
    ]);
    for file in ["package.tsx", "factory.tsx"] {
        assert_eq!(fixture.forwarding(file, "Outer").elements[0].target, Target::Unresolved, "{file}");
        let shown = fixture.decide(file, "Outer", outer_members());
        assert_eq!(shown.rows, ["label"], "{file}");
        assert_eq!(shown.collapsed, ["tone", "size", "variant", "other"], "{file}");
    }
}

#[test]
fn a_component_whose_props_did_not_resolve_is_unknown() {
    // The tag resolves, but the lane has no props for `Inner`: nothing is guessed about what it takes.
    let fixture = Fixture::new(&[("inner.tsx", INNER), ("outer.tsx", &outer("<Inner {...rest} />"))]);
    assert_eq!(fixture.forwarding("outer.tsx", "Outer").elements[0].target, Target::Unresolved);
    assert_eq!(fixture.decide("outer.tsx", "Outer", outer_members()).collapsed, ["tone", "size", "variant", "other"]);
}

#[test]
fn a_body_that_hands_its_props_on_is_unknown() {
    let source = "import { Inner } from './inner';\n\
                  export function Outer(props: OuterProps) { track(props); return <Inner {...props} />; }\n";
    let fixture = project(source);
    let shown = fixture.decide("outer.tsx", "Outer", outer_members());
    // What the body says (the spread onto `Inner`) still holds; handing the props on promotes nothing else.
    assert_eq!(shown.rows, ["label", "tone", "size", "variant"]);
    assert_eq!(shown.collapsed, ["other"]);
}

#[test]
fn the_output_is_deterministic_and_follows_source_and_member_order() {
    let fixture = project(&outer("<><Inner {...rest} /><span /><Inner variant=\"x\" {...rest} /></>"));
    let first = fixture.forwarding("outer.tsx", "Outer");
    assert_eq!(first, fixture.forwarding("outer.tsx", "Outer"));
    let targets: Vec<&Target> = first.elements.iter().map(|element| &element.target).collect();
    let inner = Target::Component(strings(&["tone", "size", "base", "variant"]));
    assert_eq!(targets, [&inner, &Target::Dom, &inner]);
}
