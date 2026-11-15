#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The two lanes collapse alike. Over `loadDependencies` × `collapseInheritedProps` × `htmlAttributes`, a small project
//! whose components inherit sources of every origin (the project, a package loaded as a dependency or not loaded),
//! size (each side of the 30-prop ambient and the 200-prop thresholds) and wrapper (none, `Omit`, `Pick`) is resolved
//! by the structural lane alone and by the checker lane (the structural answer with the checker's merged over it).
//! Both must give each component the same inheritance layers, and, wherever the structural lane can read the source,
//! the same props. A fourth origin is a package source only the checker can see: a package component's props reached
//! through `PropsOf<typeof Widget>` (an `infer` conditional, as `ComponentProps` is), which the structural lane does not follow; there the checker supplies the
//! source's reaching members and the layers must be what the structural lane would give (the one decision,
//! `CollapseInheritedProps::collapses`, asked with the members that reach the component, counted alike in both lanes).

use std::collections::BTreeSet;
use std::time::Duration;

use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::ExtractionOutput;
use caliper_docgen_core::types::options::CollapseContext;
use caliper_docgen_core::types::semantic::merge_semantic_in;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

/// (source name, member prefix, member count, declared by the package).
const SOURCES: &[(&str, &str, usize, bool)] = &[
    ("GlobalDOMEvents", "g", 31, true),
    ("DOMAttributes", "d", 30, true),
    ("BigApi", "b", 201, true),
    ("EdgeApi", "e", 200, true),
    ("ProjectBig", "pb", 201, false),
    ("ProjectEdge", "pe", 200, false),
    ("SpaceProps", "sp", 31, false),
];

fn members(prefix: &str, count: usize) -> String {
    (0..count).map(|i| format!("{prefix}{i}?: string; ")).collect()
}

/// The project in a scratch directory: `node_modules/pkg` declares the package sources, `sources.ts` the project's,
/// and `Components.tsx` one exported component per source and wrapper.
fn project() -> (tempfile::TempDir, Utf8PathBuf) {
    let dir = tempfile::Builder::new().prefix("CollapseModel").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let pkg = root.join("node_modules/pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("package.json"), r#"{ "name": "pkg", "version": "1.0.0", "types": "index.d.ts" }"#)
        .unwrap();
    let declare = |package: bool| -> String {
        SOURCES
            .iter()
            .filter(|source| source.3 == package)
            .map(|(name, prefix, count, _)| {
                let mut text = format!("export interface {name} {{ {}}}\n", members(prefix, *count));
                if package {
                    text.push_str(&format!(
                        "export interface Widget{name}Props extends {name} {{ w?: boolean }}\n\
                         export declare function Widget{name}(props: Widget{name}Props): null;\n"
                    ));
                }
                text
            })
            .collect()
    };
    std::fs::write(pkg.join("index.d.ts"), declare(true)).unwrap();
    std::fs::write(root.join("sources.ts"), declare(false)).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
    )
    .unwrap();
    // A conditional type the structural lane does not evaluate, as `ComponentProps<typeof X>` is for it.
    let mut components = String::from("type PropsOf<T> = T extends (props: infer P) => unknown ? P : never;\n");
    for (name, prefix, _, package) in SOURCES {
        let from = if *package { "pkg" } else { "./sources" };
        components.push_str(&format!("import type {{ {name} }} from '{from}';\n"));
        if *package {
            components.push_str(&format!("import {{ Widget{name} }} from 'pkg';\n"));
            components.push_str(&format!(
                "export function Only{name}(props: PropsOf<typeof Widget{name}>) {{ return null; }}\n"
            ));
        }
        components.push_str(&format!("interface {name}Props extends {name} {{ own?: boolean }}\n"));
        components.push_str(&format!("export function With{name}(props: {name}Props) {{ return null; }}\n"));
        components.push_str(&format!(
            "export function Omit{name}(props: Omit<{name}, '{prefix}0'> & {{ own?: boolean }}) {{ return null; }}\n"
        ));
        components.push_str(&format!(
            "export function Pick{name}(props: Pick<{name}, '{prefix}0' | '{prefix}1'> & {{ own?: boolean }}) {{ \
             return null; }}\n"
        ));
    }
    std::fs::write(root.join("Components.tsx"), components).unwrap();
    (dir, root)
}

type Layers = BTreeSet<(String, String, u32, Vec<String>)>;

fn layers(output: &ExtractionOutput, key: &str) -> Layers {
    output
        .components
        .get(key)
        .unwrap_or_else(|| panic!("{key} was not extracted"))
        .inheritance
        .iter()
        .map(|layer| (layer.type_name.clone(), layer.file_name.clone(), layer.total_props, layer.omitted.clone()))
        .collect()
}

fn props(output: &ExtractionOutput, key: &str) -> BTreeSet<String> {
    output.components[key].props.keys().cloned().collect()
}

#[test]
fn the_structural_and_the_checker_lane_give_the_same_layers_and_props() {
    let (_dir, root) = project();
    let mut checked = 0;
    for load_dependencies in [true, false] {
        for collapse in
            [CollapseInheritedProps::Dependencies, CollapseInheritedProps::All, CollapseInheritedProps::None]
        {
            for html_attributes in [HtmlAttributeMode::Full, HtmlAttributeMode::None] {
                let options = PipelineOptions {
                    src_dirs: vec![root.clone()],
                    use_cache: false,
                    load_dependencies,
                    collapse_inherited_props: collapse,
                    html_attributes,
                    ..PipelineOptions::default()
                };
                let structural = extract(&options);
                let mut request = SemanticRequest::new(root.clone(), targets_of(&structural));
                request.request_timeout = Duration::from_secs(60);
                let outcome = resolve(request);
                assert_eq!(outcome.fallbacks, [], "the checker answers every component");
                let mut checker = structural.clone();
                merge_semantic_in(&mut checker, outcome, Default::default(), CollapseContext::of(&options));

                for (name, _, _, package) in SOURCES {
                    for key in [format!("With{name}"), format!("Omit{name}"), format!("Pick{name}")] {
                        let case = format!("{key} load={load_dependencies} {collapse:?} {html_attributes:?}");
                        assert_eq!(layers(&checker, &key), layers(&structural, &key), "layers of {case}");
                        // A package type the structural lane does not load is read by the checker alone.
                        if load_dependencies || !package {
                            assert_eq!(props(&checker, &key), props(&structural, &key), "props of {case}");
                        }
                        checked += 1;
                    }
                    if !package {
                        continue;
                    }
                    // Only the checker sees this source: the layers are the ones the rule gives a loaded dependency's
                    // source of this declared size, the same the structural lane gives `With{name}`.
                    let key = format!("Only{name}");
                    let case = format!("{key} load={load_dependencies} {collapse:?} {html_attributes:?}");
                    let (_, _, count, _) = SOURCES.iter().find(|s| s.0 == *name).unwrap();
                    let ambient = name.ends_with("DOMEvents") || name.ends_with("DOMAttributes");
                    let threshold = if ambient { 30 } else { 200 };
                    let collapses = load_dependencies && collapse != CollapseInheritedProps::None && *count > threshold;
                    let pkg = root.join("node_modules/pkg/index.d.ts").to_string();
                    let expected: Layers = if collapses {
                        [(name.to_string(), pkg, u32::try_from(*count).unwrap(), Vec::new())].into()
                    } else {
                        Layers::new()
                    };
                    assert_eq!(layers(&checker, &key), expected, "layers of {case}");
                    if load_dependencies {
                        assert_eq!(layers(&checker, &key), layers(&checker, &format!("With{name}")), "{case}");
                    }
                    let listed = props(&checker, &key);
                    assert!(listed.contains("w"), "{case}: {listed:?}");
                    assert_eq!(listed.len(), if collapses { 1 } else { count + 1 }, "props of {case}");
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 2 * 3 * 2 * (SOURCES.len() * 3 + 4));
}

/// A 700-member source reached through `Pick` and `Omit`: it collapses by the members that reach the component, in the
/// structural lane (`Huge` imported) and in the checker lane alone (`PropsOf<typeof WidgetHuge>`). `Pick` of 2 is listed;
/// `Omit` of 1 collapses (699 reach); `Omit` of 600 is listed (100); 200 reaching is listed and 201 collapses; the same
/// interface reached twice counts once.
#[test]
fn a_source_collapses_by_the_members_that_reach_the_component() {
    let dir = tempfile::Builder::new().prefix("CollapseReach").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let pkg = root.join("node_modules/pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("package.json"), r#"{ "name": "pkg", "version": "1.0.0", "types": "index.d.ts" }"#)
        .unwrap();
    std::fs::write(
        pkg.join("index.d.ts"),
        format!(
            "export interface Huge {{ {}}}\nexport interface WidgetHugeProps extends Huge {{ w?: boolean }}\n\
             export declare function WidgetHuge(props: WidgetHugeProps): null;\n",
            members("h", 700)
        ),
    )
    .unwrap();
    let keys = |range: std::ops::Range<usize>| range.map(|i| format!("'h{i}'")).collect::<Vec<_>>().join(" | ");
    // (component, wrapper of `S`, members that reach)
    let cases: Vec<(&str, String, usize)> = vec![
        ("Pick2", format!("Pick<S, {}>", keys(0..2)), 2),
        ("Omit1", format!("Omit<S, {}>", keys(0..1)), 699),
        ("Omit600", format!("Omit<S, {}>", keys(0..600)), 100),
        ("Leaves200", format!("Omit<S, {}>", keys(0..500)), 200),
        ("Leaves201", format!("Omit<S, {}>", keys(0..499)), 201),
        ("Twice", "S & Omit<S, 'h0'>".to_string(), 700),
    ];
    let mut text = String::from(
        "import type { Huge } from 'pkg';\nimport { WidgetHuge } from 'pkg';\n\
         type PropsOf<T> = T extends (props: infer P) => unknown ? P : never;\ntype Seen = PropsOf<typeof WidgetHuge>;\n",
    );
    for (name, wrapper, _) in &cases {
        let structural = wrapper.replace('S', "Huge");
        let checker_only = wrapper.replace('S', "Seen");
        text.push_str(&format!("export function Struct{name}(props: {structural}) {{ return null; }}\n"));
        text.push_str(&format!("export function Only{name}(props: {checker_only}) {{ return null; }}\n"));
    }
    std::fs::write(root.join("Components.tsx"), text).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
    )
    .unwrap();
    let options = PipelineOptions { src_dirs: vec![root.clone()], use_cache: false, ..PipelineOptions::default() };
    let structural = extract(&options);
    let mut request = SemanticRequest::new(root.clone(), targets_of(&structural));
    request.request_timeout = Duration::from_secs(60);
    let outcome = resolve(request);
    let mut checker = structural.clone();
    merge_semantic_in(&mut checker, outcome, Default::default(), CollapseContext::of(&options));

    for (name, _, reaching) in &cases {
        let collapses = *reaching > 200;
        for (lane, output, key) in [
            ("structural", &structural, format!("Struct{name}")),
            ("checker", &checker, format!("Struct{name}")),
            ("checker only", &checker, format!("Only{name}")),
        ] {
            let entry = &output.components[&key];
            let huge: Vec<_> = entry.inheritance.iter().filter(|layer| layer.type_name == "Huge").collect();
            let listed = entry.props.keys().filter(|prop| prop.starts_with('h')).count();
            if collapses {
                assert_eq!(huge.len(), 1, "{lane} {key}: one Huge layer");
                assert_eq!(huge[0].total_props as usize, *reaching, "{lane} {key}: what the layer holds");
            } else {
                assert!(huge.is_empty(), "{lane} {key}: no Huge layer");
                assert_eq!(listed, *reaching, "{lane} {key}: listed");
            }
        }
    }
}

/// A scratch project with the package `pkg` declaring `package` and the project files `project`, read by the
/// structural lane and by the checker lane (the structural answer with the checker's merged over it), under the default
/// options.
fn both_lanes(package: &str, project: &[(&str, String)]) -> (tempfile::TempDir, ExtractionOutput, ExtractionOutput) {
    let dir = tempfile::Builder::new().prefix("CollapseLanes").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let pkg = root.join("node_modules/pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("package.json"), r#"{ "name": "pkg", "version": "1.0.0", "types": "index.d.ts" }"#)
        .unwrap();
    std::fs::write(pkg.join("index.d.ts"), package).unwrap();
    for (file, text) in project {
        std::fs::write(root.join(file), text).unwrap();
    }
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
    )
    .unwrap();
    let options = PipelineOptions { src_dirs: vec![root.clone()], use_cache: false, ..PipelineOptions::default() };
    let structural = extract(&options);
    let mut request = SemanticRequest::new(root.clone(), targets_of(&structural));
    request.request_timeout = Duration::from_secs(60);
    let outcome = resolve(request);
    assert_eq!(outcome.fallbacks, [], "the checker answers every component");
    let mut checker = structural.clone();
    merge_semantic_in(&mut checker, outcome, Default::default(), CollapseContext::of(&options));
    (dir, structural, checker)
}

/// The checker answers a prop's JSDoc when it is asked (`described` cleared), for a plain interface and for one only
/// it can reach. It answers `""` for every property of a TypeScript file today, so a source only the checker reads
/// (`OnlyBox` in the finding B test) has no description: the whole-project merge hides this elsewhere by keeping the
/// structural description (`SemanticTarget::described`).
#[test]
#[ignore = "the checker (tsrs 0.24 getDocumentationComment) answers \"\" for every property of a .ts file"]
fn the_checker_answers_a_props_jsdoc() {
    let dir = tempfile::Builder::new().prefix("Jsdoc").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::write(
        root.join("local.ts"),
        "export interface LocalSmall { /** The class. */ className?: string; /** L. */ l?: string }\n\
         export declare function WidgetLocal(props: LocalSmall): null;\n",
    )
    .unwrap();
    std::fs::write(
        root.join("C.tsx"),
        "import { WidgetLocal, type LocalSmall } from './local';\n\
         type PropsOf<T> = T extends (props: infer P) => unknown ? P : never;\n\
         export function OnlyLocal(props: PropsOf<typeof WidgetLocal>) { return null; }\n\
         export function Direct(props: LocalSmall) { return null; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
    )
    .unwrap();
    let options = PipelineOptions { src_dirs: vec![root.clone()], use_cache: false, ..PipelineOptions::default() };
    let structural = extract(&options);
    let mut targets = targets_of(&structural);
    for target in &mut targets {
        target.described.clear();
    }
    let mut request = SemanticRequest::new(root.clone(), targets);
    request.request_timeout = Duration::from_secs(60);
    let outcome = resolve(request);
    for component in &outcome.components {
        for prop in &component.props {
            let expected = if prop.name == "className" { "The class." } else { "L." };
            assert_eq!(prop.description, expected, "{} {}", component.key, prop.name);
        }
    }
}

/// Finding A: a `Pick` names its keys on purpose, so the props it picks are listed however large their source, also
/// when only the checker reads the source (`PropsOf<typeof WidgetHuge>`), and through an `Omit`, a `Partial` or an
/// intersection around it.
#[test]
fn finding_a_picked_keys_over_a_large_source_stay_listed_in_both_lanes() {
    let keys = |range: std::ops::Range<usize>| range.map(|i| format!("'h{i}'")).collect::<Vec<_>>().join(" | ");
    // (component, wrapper of `S`, props listed)
    let cases: Vec<(&str, String, usize)> = vec![
        ("Pick250", format!("Pick<S, {}>", keys(0..250)), 250),
        ("OmitOfPick300", format!("Omit<Pick<S, {}>, 'h0'>", keys(0..300)), 299),
        ("PartialPick250", format!("Partial<Pick<S, {}>>", keys(0..250)), 250),
        ("PickAndOwn250", format!("Pick<S, {}> & {{ own?: boolean }}", keys(0..250)), 250),
        // A project alias whose whole body is the `Pick`.
        ("NamedPick250", "NamedS".to_string(), 250),
    ];
    let mut text = String::from(
        "import type { Huge } from 'pkg';\nimport { WidgetHuge } from 'pkg';\n\
         type PropsOf<T> = T extends (props: infer P) => unknown ? P : never;\ntype Seen = PropsOf<typeof WidgetHuge>;\n",
    );
    for base in ["Huge", "Seen"] {
        text.push_str(&format!("type Named{base} = Pick<{base}, {}>;\n", keys(0..250)));
    }
    for (name, wrapper, _) in &cases {
        let structural = wrapper.replace('S', "Huge");
        let checker_only = wrapper.replace('S', "Seen");
        text.push_str(&format!("export function Struct{name}(props: {structural}) {{ return null; }}\n"));
        text.push_str(&format!("export function Only{name}(props: {checker_only}) {{ return null; }}\n"));
    }
    let package = format!(
        "export interface Huge {{ {}}}\nexport interface WidgetHugeProps extends Huge {{ w?: boolean }}\n\
         export declare function WidgetHuge(props: WidgetHugeProps): null;\n",
        members("h", 700)
    );
    let (_dir, structural, checker) = both_lanes(&package, &[("Components.tsx", text)]);
    for (name, _, listed) in &cases {
        for (lane, output, key) in [
            ("structural", &structural, format!("Struct{name}")),
            ("checker", &checker, format!("Struct{name}")),
            ("checker only", &checker, format!("Only{name}")),
        ] {
            // The checker cannot see a `Pick` a project alias is (`Converter::composes`), and the structural lane
            // cannot read `Seen`: this one case collapses (a known limit, not asserted).
            if *name == "NamedPick250" && lane == "checker only" {
                continue;
            }
            let entry = &output.components[&key];
            assert!(entry.inheritance.iter().all(|layer| layer.type_name != "Huge"), "{lane} {key}: no Huge layer");
            let shown = entry.props.keys().filter(|prop| prop.starts_with('h')).count();
            assert_eq!(shown, *listed, "{lane} {key}: listed");
        }
        if *name != "NamedPick250" {
            assert_eq!(props(&checker, &format!("Only{name}")), props(&structural, &format!("Struct{name}")), "{name}");
        }
    }
}

/// Finding B: a collapsed source's notable prop (`className`) keeps the default the component gives it, its JSDoc and
/// its optionality (a default makes it optional; `Partial`, `Required`) in `notableInherited`, the same in both lanes, whether the structural lane
/// collapsed the source (the checker's answer merged into its layer) or only the checker read it.
#[test]
fn finding_b_a_notable_prop_of_a_collapsed_source_keeps_its_default_in_both_lanes() {
    let big = |name: &str| {
        format!(
            "export interface {name} {{ /** The class. */ className: string; {}}}\n\
             export interface Widget{name}Props extends {name} {{ w?: boolean }}\n\
             export declare function Widget{name}(props: Widget{name}Props): null;\n",
            members("b", 201)
        )
    };
    let text = String::from(
        "import type { BigBox } from './sources';\nimport { WidgetHugeBox } from 'pkg';\n\
         type PropsOf<T> = T extends (props: infer P) => unknown ? P : never;\n\
         interface BoxProps extends BigBox { own?: boolean }\n\
         export function Box({ className = 'base' }: BoxProps) { return null; }\n\
         export function PartialBox({ className = 'base' }: Partial<BigBox>) { return null; }\n\
         export function RequiredBox(props: Required<Partial<BigBox>>) { return null; }\n\
         export function OnlyBox({ className = 'base' }: PropsOf<typeof WidgetHugeBox>) { return null; }\n\
         export function PickedAndLayered(props: Pick<BigBox, 'className'> & Omit<BigBox, 'b0'>) { return null; }\n",
    );
    let (_dir, structural, checker) =
        both_lanes(&big("HugeBox"), &[("sources.ts", big("BigBox")), ("Components.tsx", text)]);
    type Notable = (String, bool, Option<String>, String);
    let notable = |output: &ExtractionOutput, key: &str| -> Option<Notable> {
        let entry = &output.components[key];
        assert!(!entry.props.contains_key("className"), "{key}: className is the layer's");
        entry.notable_inherited.get("className").map(|prop| {
            let source = prop.parent.as_ref().map(|parent| parent.name.clone()).unwrap_or_default();
            let default = prop.default_value.as_ref().map(|value| value.value.clone());
            (source, prop.required, default, prop.description.clone())
        })
    };
    let expect = |source: &str, required: bool, default: Option<&str>| {
        Some((source.to_string(), required, default.map(str::to_string), "The class.".to_string()))
    };
    for (key, expected) in [
        ("Box", expect("BigBox", false, Some("'base'"))),
        ("PartialBox", expect("BigBox", false, Some("'base'"))),
        ("RequiredBox", expect("BigBox", true, None)),
    ] {
        assert!(layers(&structural, key).iter().any(|layer| layer.0 == "BigBox"), "{key}: a BigBox layer");
        assert_eq!(notable(&structural, key), expected, "structural {key}");
        assert_eq!(notable(&checker, key), expected, "checker {key}");
    }
    // Picked by name and held by a layer through the `Omit`: the structural lane lists it, so the checker lane does.
    for (lane, output) in [("structural", &structural), ("checker", &checker)] {
        let entry = &output.components["PickedAndLayered"];
        assert!(entry.inheritance.iter().any(|layer| layer.type_name == "BigBox"), "{lane}: a BigBox layer");
        assert!(entry.props.contains_key("className"), "{lane}: className listed");
        assert!(!entry.notable_inherited.contains_key("className"), "{lane}: className not moved");
    }
    // Only the checker reads this source; its JSDoc is `the_checker_answers_a_props_jsdoc`'s.
    let only = notable(&checker, "OnlyBox").map(|(source, required, default, _)| (source, required, default));
    assert_eq!(only, Some(("HugeBox".to_string(), false, Some("'base'".to_string()))), "checker only OnlyBox");
}
