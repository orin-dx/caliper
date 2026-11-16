#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The two lanes report the same relevance (ADR 0028, slice T7). Over generated components (own members, a project
//! source of either side of the collapse threshold reached whole or through `Pick` or `Omit`, an ambient DOM source
//! declared under `@types/react` or `@react-types/shared`, a body reading some members, call sites passing some at
//! several counts), the structural lane alone and the checker lane (the structural answer with the checker's merged over
//! it) must list the same rows and give each layer the same `used` names: one decision function, `relevance::decide`,
//! asked with the same evidence in both, and its ambient bit decided by the declaring path.
//!
//! A second model checks the rule itself (ADR 0028, A2): over generated bodies that read, destructure (with or without
//! a default) or pass by name some members of an ambient DOM source, and spread the rest onto a DOM tag, both lanes list
//! exactly the props type's own members, its API source's members and the DOM members the body names.

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
                named += usize::from(rows(from_structural).iter().any(|row| row.starts_with('d')));
                assert_eq!(rows(from_checker), rows(from_structural), "{case}");
                assert_eq!(used(from_checker), used(from_structural), "{case}");
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 3 * 2 * 12);
    assert!(named > 0, "ambient members were put forward: {named} of {compared}");
}

/// The rows a component lists: `props` and `notableInherited`, sorted.
fn rows(entry: &ComponentEntry) -> Vec<String> {
    let mut rows: Vec<String> = entry.props.keys().chain(entry.notable_inherited.keys()).cloned().collect();
    rows.sort();
    rows
}

/// How a generated body names one member of the ambient source.
#[derive(Debug, Clone, Copy)]
enum Names {
    /// `rest.dN` in an expression.
    Reads,
    /// `{ dN }` in the parameter.
    Destructures,
    /// `{ dN = 'v' }` in the parameter.
    Defaults,
    /// `<Child v={rest.dN} />`.
    Passes,
}

#[test]
fn the_main_props_and_the_members_the_body_names_are_listed_and_nothing_a_dom_spread_passes() {
    let mut checked = 0;
    for seed in [3, 11, 29] {
        let dir = tempfile::Builder::new().prefix("RelevanceRule").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let fields = |prefix: &str, n: usize| -> String { (0..n).map(|i| format!("{prefix}{i}?: string; ")).collect() };
        std::fs::write(root.join("sources.ts"), format!("export interface Api {{ {}}}\n", fields("a", 6))).unwrap();
        std::fs::create_dir_all(root.join("vendor/@types/react")).unwrap();
        std::fs::write(
            root.join("vendor/@types/react/dom.ts"),
            format!("export interface ReactDom {{ {}}}\n", fields("d", 40)),
        )
        .unwrap();
        std::fs::write(
            root.join("tsconfig.json"),
            r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
        )
        .unwrap();
        let mut rng = Lcg(seed);
        let mut text = String::from(
            "import type { Api } from './sources';\n\
             import type { ReactDom } from './vendor/@types/react/dom';\n\
             export function Child(props: { v?: string }) { return null; }\n",
        );
        let mut expected = Vec::new();
        for c in 0..10 {
            let name = format!("C{c}");
            let mut named: Vec<(usize, Names)> = Vec::new();
            for _ in 0..rng.below(4) {
                let member = rng.below(40);
                if named.iter().all(|(seen, _)| *seen != member) {
                    let how = [Names::Reads, Names::Destructures, Names::Defaults, Names::Passes][rng.below(4)];
                    named.push((member, how));
                }
            }
            let mut params = String::new();
            let mut body = String::new();
            for (member, how) in &named {
                match how {
                    Names::Reads => write!(body, "{{String(rest.d{member})}}").unwrap(),
                    Names::Destructures => write!(params, "d{member}, ").unwrap(),
                    Names::Defaults => write!(params, "d{member} = 'v', ").unwrap(),
                    Names::Passes => write!(body, "<Child v={{rest.d{member}}} />").unwrap(),
                }
            }
            writeln!(
                text,
                "export function {name}({{ {params}...rest }}: Api & ReactDom & {{ own?: string }}) \
                 {{ return <div {{...rest}}>{body}</div>; }}"
            )
            .unwrap();
            let mut rows: Vec<String> = (0..6).map(|i| format!("a{i}")).collect();
            rows.push("own".to_owned());
            rows.extend(named.iter().map(|(member, _)| format!("d{member}")));
            rows.sort();
            expected.push((name, rows));
        }
        std::fs::write(root.join("Components.tsx"), text).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![root.clone()],
            use_cache: false,
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
        for (name, want) in &expected {
            for (lane, output) in [("structural", &structural), ("checker", &checker)] {
                assert_eq!(&rows(&output.components[name]), want, "{name} seed={seed} {lane}");
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 30);
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

/// Both lanes over `files` (each `(path, source)` under a new project), with `collapse`: the structural answer and the
/// checker's merged over it.
fn both_lanes(
    files: &[(&str, String)],
    collapse: CollapseInheritedProps,
) -> (tempfile::TempDir, [caliper_docgen_core::types::ExtractionOutput; 2]) {
    let dir = tempfile::Builder::new().prefix("RelevanceProbe").tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "bundler", "jsx": "preserve", "strict": true, "skipLibCheck": true, "noEmit": true, "types": [] } }"#,
    )
    .unwrap();
    for (path, source) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
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
    (dir, [structural, checker])
}

/// `n` optional string members `{prefix}0` to `{prefix}{n-1}`.
fn fields(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("{prefix}{i}?: string; ")).collect()
}

#[test]
fn ambient_is_decided_per_declaration_and_the_generated_rule_holds_in_the_project() {
    // Pattern evaluation of PR 490, probe (ix) and the Step 1 changes, over generated components:
    // - a library file declaring both a `@private` copy of DOM members (`GlobalEvents`, 40) and documented API
    //   (`DomApi`: `id`, `excludeFromTabOrder`): only the private interface is ambient, as in `@react-types/shared`;
    // - a member a small API interface declares that React's `ReactDom` declares too (`d5` in `FocusApi`): API;
    // - a project mapped copy over the keys of a 40-key interface (`Conds`, Chakra's `Conditions`): generated, a layer
    //   in the project too; a project source of at most 200 members stays listed (`Small`).
    let mut checked = 0;
    for seed in [5, 13, 31] {
        let mut rng = Lcg(seed);
        let shared = format!(
            "export interface DomApi {{ /** The element's id. */ id?: string; excludeFromTabOrder?: boolean; }}\n\
             /**\n * Events every element has.\n *\n * @private\n */\n// NOTES, as React Aria writes them between the doc comment and the interface:\n\
             //   - Drag and drop events are omitted for now.\n\
             //   - Keyboard and focus events are supported directly on focusable elements.\n\
             //   - Media events should be handled directly on the media element.\n\
             export interface GlobalEvents {{ {} }}\n\
             export interface FocusApi {{ d5?: string; }}\n",
            fields("e", 40)
        );
        let mut text = String::from(
            "import type { DomApi, GlobalEvents, FocusApi } from './lib/shared/dom';\n\
             import type { ReactDom } from './vendor/@types/react/dom';\n\
             export interface Conds { ",
        );
        text.push_str(&fields("c", 40));
        text.push_str("}\nexport interface Small { s0?: string; s1?: string; }\n");
        text.push_str("export type CondProps = { [K in keyof Conds]?: string };\n");
        let mut expected = Vec::new();
        for c in 0..8 {
            let name = format!("P{c}");
            let mut parts = vec!["{ own?: string }"];
            let mut rows = vec!["own".to_owned()];
            let mut body = String::new();
            if rng.below(2) == 0 {
                parts.push("DomApi & GlobalEvents");
                rows.extend(["excludeFromTabOrder".to_owned(), "id".to_owned()]);
                // A read of a private member is a row, as a read of React's is.
                let read = rng.below(40);
                write!(body, "{{String(props.e{read})}}").unwrap();
                rows.push(format!("e{read}"));
            }
            if rng.below(2) == 0 {
                parts.push("FocusApi & ReactDom");
                rows.push("d5".to_owned());
            }
            if rng.below(2) == 0 {
                parts.push("CondProps & Small");
                rows.extend(["s0".to_owned(), "s1".to_owned()]);
            }
            let props = parts.join(" & ");
            writeln!(text, "export function {name}(props: {props}) {{ return <div {{...props}}>{body}</div>; }}")
                .unwrap();
            rows.sort();
            expected.push((name, rows));
        }
        let files = [
            ("lib/shared/dom.ts", shared),
            ("vendor/@types/react/dom.ts", format!("export interface ReactDom {{ {}}}\n", fields("d", 40))),
            ("Components.tsx", text),
        ];
        let (_dir, lanes) = both_lanes(&files, CollapseInheritedProps::Dependencies);
        for (name, want) in &expected {
            for (lane, output) in ["structural", "checker"].iter().zip(&lanes) {
                assert_eq!(&rows(&output.components[name]), want, "{name} seed={seed} {lane}");
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 24);
}
