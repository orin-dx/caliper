#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The two lanes report the same relevance (ADR 0028, slice T7). Over generated components (own members, a project
//! source of either side of the collapse threshold reached whole or through `Pick` or `Omit`, an ambient DOM source
//! declared under `@types/react` or `@react-types/shared`, a body reading some members, call sites passing some at
//! several counts), the structural lane alone and the checker lane (the structural answer with the checker's merged over
//! it) must give each layer the same `used` names: one decision function, `relevance::decide`, asked with the same
//! evidence in both, and its ambient bit decided by the declaring path.

use std::fmt::Write as _;
use std::time::Duration;

use caliper_docgen_core::pipeline::{CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, extract};
use caliper_docgen_core::types::ComponentEntry;
use caliper_docgen_core::types::options::CollapseContext;
use caliper_docgen_core::types::semantic::merge_semantic_in;
use caliper_docgen_semantic::{SemanticRequest, resolve, targets_of};
use camino::Utf8PathBuf;

/// A small deterministic generator: the model is reproducible without a dependency.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % n as u64).unwrap()
    }
}

/// One generated project: `count` components over sources of 10 and 40 members.
fn project(seed: u64, count: usize) -> (tempfile::TempDir, Utf8PathBuf, Vec<String>) {
    let dir = tempfile::Builder::new().prefix("RelevanceModel").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    let fields = |prefix: &str, n: usize| -> String { (0..n).map(|i| format!("{prefix}{i}?: string; ")).collect() };
    std::fs::write(
        root.join("sources.ts"),
        format!("export interface Small {{ {}}}\nexport interface Large {{ {}}}\n", fields("s", 10), fields("l", 40)),
    )
    .unwrap();
    // Ambient by path: React's declarations and Spectrum's shared DOM ones.
    for (dir, file, name) in
        [("vendor/@types/react", "dom.ts", "ReactDom"), ("vendor/@react-types/shared/src", "dom.d.ts", "SharedDom")]
    {
        std::fs::create_dir_all(root.join(dir)).unwrap();
        std::fs::write(root.join(dir).join(file), format!("export interface {name} {{ {}}}\n", fields("d", 33)))
            .unwrap();
    }
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
    )
    .unwrap();
    let mut rng = Lcg(seed);
    let mut components = String::from(
        "import type { Small, Large } from './sources';\n\
         import type { ReactDom } from './vendor/@types/react/dom';\n\
         import type { SharedDom } from './vendor/@react-types/shared/src/dom';\n",
    );
    let mut page = String::new();
    let mut names = Vec::new();
    for c in 0..count {
        let name = format!("C{c}");
        let (source, prefix, size) = if rng.below(2) == 0 { ("Small", "s", 10) } else { ("Large", "l", 40) };
        let base = match rng.below(3) {
            0 => source.to_owned(),
            1 => format!("Pick<{source}, '{prefix}0' | '{prefix}1'>"),
            _ => format!("Omit<{source}, '{prefix}0'>"),
        };
        let own: Vec<String> = (0..rng.below(3)).map(|i| format!("own{i}")).collect();
        let required = rng.below(2) == 0;
        let mut literal = String::new();
        for (i, prop) in own.iter().enumerate() {
            let mark = if required && i == 0 { "" } else { "?" };
            write!(literal, "{prop}{mark}: string; ").unwrap();
        }
        let dom = if rng.below(2) == 0 { "ReactDom" } else { "SharedDom" };
        let reads: Vec<String> = (0..rng.below(3))
            .map(|_| match rng.below(2) {
                0 => format!("{prefix}{}", rng.below(size)),
                _ => format!("d{}", rng.below(3)),
            })
            .collect();
        let body = if reads.is_empty() {
            "null".to_owned()
        } else {
            format!("<div>{{props.{}}}</div>", reads.join("}{props."))
        };
        writeln!(components, "export function {name}(props: {base} & {dom} & {{ {literal}}}) {{ return {body}; }}")
            .unwrap();
        writeln!(page, "import {{ {name} }} from './Components';").unwrap();
        let sites = rng.below(6);
        for site in 0..sites {
            let passed = match rng.below(2) {
                0 => format!("{prefix}{}", (site + rng.below(2)) % size),
                _ => format!("d{}", rng.below(2)),
            };
            writeln!(page, "export const use{name}x{site} = <{name} {passed}=\"v\" />;").unwrap();
        }
        names.push(name);
    }
    std::fs::write(root.join("Components.tsx"), components).unwrap();
    std::fs::write(root.join("Page.tsx"), page).unwrap();
    (dir, root, names)
}

#[test]
fn both_lanes_report_the_same_relevance_for_the_same_member() {
    let (mut compared, mut named) = (0, 0);
    for seed in [1, 7, 42] {
        let (_dir, root, names) = project(seed, 12);
        for collapse in [CollapseInheritedProps::All, CollapseInheritedProps::None] {
            let options = PipelineOptions {
                src_dirs: vec![root.clone()],
                use_cache: false,
                collapse_inherited_props: collapse,
                html_attributes: HtmlAttributeMode::None,
                ..PipelineOptions::default()
            };
            let structural = extract(&options);
            let mut request = SemanticRequest::new(root.clone(), targets_of(&structural));
            request.request_timeout = Duration::from_secs(60);
            let outcome = resolve(request);
            assert_eq!(outcome.fallbacks, [], "the checker answers every component");
            let mut checker = structural.clone();
            merge_semantic_in(&mut checker, outcome, Default::default(), CollapseContext::of(&options));
            for name in &names {
                let case = format!("{name} seed={seed} {collapse:?}");
                let (from_structural, from_checker) = (&structural.components[name], &checker.components[name]);
                // A component whose members all stay in a collapsed layer names none: an empty list is an answer.
                named += usize::from(!used(from_structural).is_empty());
                assert_eq!(used(from_checker), used(from_structural), "{case}");
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 3 * 2 * 12);
    assert!(named > 0, "ambient members were put forward: {named} of {compared}");
}

/// Each layer's non-empty `used` list, by the layer's type name.
fn used(entry: &ComponentEntry) -> Vec<(String, Vec<String>)> {
    let mut used: Vec<(String, Vec<String>)> = entry
        .inheritance
        .iter()
        .filter(|layer| !layer.used.is_empty())
        .map(|layer| (layer.type_name.clone(), layer.used.clone()))
        .collect();
    used.sort();
    used
}
