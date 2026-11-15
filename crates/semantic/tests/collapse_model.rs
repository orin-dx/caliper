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
