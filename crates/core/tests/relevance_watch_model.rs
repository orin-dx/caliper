#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Relevance evidence reads other files (call sites in consumers, a forwarding target's own props, the props type's
//! declarations), so a kept answer is stale unless those reads are dependencies (ADR 0028, Consequences). Over random
//! edit sequences on a generated project (components over an ambient DOM source and project sources, a forwarding
//! wrapper, a consumer page and a story), after each edit a [`WatchSession`] and a lazy session must give every
//! component exactly what a fresh `extract()` of the same files gives, relevance and layers included.

use std::fmt::Write as _;

use caliper_docgen_core::pipeline::{
    CollapseInheritedProps, HtmlAttributeMode, PipelineOptions, WatchSession, extract,
};
use camino::{Utf8Path, Utf8PathBuf};

/// A small deterministic generator: the model is reproducible without a dependency.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % n as u64).unwrap()
    }
}

/// One generated component.
#[derive(Clone)]
struct Component {
    name: String,
    /// `Small` (10 members) or `Large` (40), beside the ambient `DomBits` (33 members).
    large: bool,
    own: usize,
    reads: Vec<String>,
}

/// The generated project; each edit changes it and rewrites one file.
struct Model {
    root: Utf8PathBuf,
    components: Vec<Component>,
    /// Call sites in `Page.tsx`: the component's index and the prop passed.
    sites: Vec<(usize, String)>,
    /// The props the story passes to `C0`.
    story: Vec<String>,
    /// `Consumer.tsx`: the component its `Tag` import names, and whether it renders two `<Tag d2>` sites.
    consumer: (usize, bool),
}

impl Model {
    fn new(root: Utf8PathBuf, rng: &mut Lcg) -> Model {
        let components = (0..4)
            .map(|c| Component {
                name: format!("C{c}"),
                large: rng.below(2) == 0,
                own: rng.below(3),
                reads: (0..rng.below(2)).map(|_| format!("d{}", rng.below(3))).collect(),
            })
            .collect();
        let mut model =
            Model { root, components, sites: Vec::new(), story: vec!["d1".to_owned()], consumer: (1, true) };
        for _ in 0..6 {
            let site = model.random_site(rng);
            model.sites.push(site);
        }
        let fields = |prefix: &str, n: usize| -> String { (0..n).map(|i| format!("{prefix}{i}?: string; ")).collect() };
        let dom = model.root.join("vendor/@types/react");
        std::fs::create_dir_all(&dom).unwrap();
        std::fs::write(dom.join("dom.ts"), format!("export interface DomBits {{ {}}}\n", fields("d", 33))).unwrap();
        std::fs::write(
            model.root.join("sources.ts"),
            format!(
                "export interface Small {{ {}}}\nexport interface Large {{ {}}}\n",
                fields("s", 10),
                fields("l", 40)
            ),
        )
        .unwrap();
        model.write_components();
        model.write_page();
        model.write_story();
        model.write_consumer();
        model
    }

    fn random_site(&self, rng: &mut Lcg) -> (usize, String) {
        let component = rng.below(self.components.len());
        let prefix = if self.components[component].large { "l" } else { "s" };
        // Mostly ambient members, which a collapsed layer names in `used` when passed at the floor.
        let prop = if rng.below(3) > 0 { format!("d{}", rng.below(2)) } else { format!("{prefix}{}", rng.below(3)) };
        (component, prop)
    }

    fn write_components(&self) {
        let mut source = String::from(
            "import type { DomBits } from './vendor/@types/react/dom';\nimport type { Small, Large } from './sources';\n",
        );
        for component in &self.components {
            let source_type = if component.large { "Large" } else { "Small" };
            let own: String = (0..component.own).map(|i| format!("own{i}?: string; ")).collect();
            let body = if component.reads.is_empty() {
                "null".to_owned()
            } else {
                format!("<div>{{props.{}}}</div>", component.reads.join("}{props."))
            };
            writeln!(
                source,
                "export interface {name}Props extends DomBits, {source_type} {{ {own}}}\n\
                 export function {name}(props: {name}Props) {{ return {body}; }}",
                name = component.name,
            )
            .unwrap();
        }
        // A forwarding wrapper: its rest goes one hop on to `C0`, whose own props it shows.
        let target = &self.components[0].name;
        writeln!(
            source,
            "export interface WrapProps extends DomBits {{ label?: string; tone?: string }}\n\
             export function Wrap(props: WrapProps) {{ const {{ label, ...rest }} = props; return <{target} {{...rest}} />; }}"
        )
        .unwrap();
        std::fs::write(self.root.join("Components.tsx"), source).unwrap();
    }

    fn write_page(&self) {
        let names: Vec<&str> = self.components.iter().map(|c| c.name.as_str()).collect();
        let mut page = format!("import {{ {}, Wrap }} from './Components';\n", names.join(", "));
        for (at, (component, prop)) in self.sites.iter().enumerate() {
            writeln!(page, "export const site{at} = <{} {prop}=\"v\" />;", self.components[*component].name).unwrap();
        }
        writeln!(page, "export const wrapped = <Wrap tone=\"v\" />;").unwrap();
        std::fs::write(self.root.join("Page.tsx"), page).unwrap();
    }

    fn write_story(&self) {
        let target = &self.components[0].name;
        let mut story =
            format!("import {{ {target} }} from './Components';\nexport default {{ component: {target} }};\n");
        let props: String = self.story.iter().map(|prop| format!(" {prop}=\"v\"")).collect();
        writeln!(story, "export const Primary = () => <{target}{props} />;").unwrap();
        writeln!(story, "export const Secondary = () => <{target}{props} />;").unwrap();
        std::fs::write(self.root.join("C0.stories.tsx"), story).unwrap();
    }

    /// A second consumer: two sites of `d2` (a prop no other site passes) on whichever component `Tag` imports.
    fn write_consumer(&self) {
        let (target, renders) = self.consumer;
        let mut source = format!("import {{ {} as Tag }} from './Components';\n", self.components[target].name);
        if renders {
            source.push_str("export const a = <Tag d2=\"v\" />;\nexport const b = <Tag d2=\"v\" />;\n");
        }
        std::fs::write(self.root.join("Consumer.tsx"), source).unwrap();
    }

    /// Applies one random edit and returns the file it rewrote.
    fn edit(&mut self, rng: &mut Lcg) -> (&'static str, Utf8PathBuf) {
        match rng.below(8) {
            0 => {
                // Two alike, so a prop can reach the floor of two sites.
                let site = self.random_site(rng);
                self.sites.extend([site.clone(), site]);
                self.write_page();
                ("add a call site", self.root.join("Page.tsx"))
            }
            1 => {
                if !self.sites.is_empty() {
                    let gone = self.sites[rng.below(self.sites.len())].clone();
                    self.sites.retain(|site| *site != gone);
                }
                self.write_page();
                ("remove a call site", self.root.join("Page.tsx"))
            }
            2 => {
                // The forward target's own props change, which changes what `Wrap` forwards.
                self.components[0].own = (self.components[0].own + 1) % 3;
                self.write_components();
                ("change the forward target's own props", self.root.join("Components.tsx"))
            }
            3 => {
                let at = 1 + rng.below(self.components.len() - 1);
                let name = &mut self.components[at].name;
                *name = if name.ends_with('r') { name.trim_end_matches('r').to_owned() } else { format!("{name}r") };
                self.write_components();
                ("rename an export", self.root.join("Components.tsx"))
            }
            4 => {
                let at = rng.below(self.components.len());
                self.components[at].large = !self.components[at].large;
                self.write_components();
                ("edit the props type", self.root.join("Components.tsx"))
            }
            5 => {
                self.consumer.1 = !self.consumer.1;
                self.write_consumer();
                (CONSUMER_JSX, self.root.join("Consumer.tsx"))
            }
            6 => {
                // Only the import changes: the same `<Tag>` sites now count for another component.
                self.consumer.0 = 1 + self.consumer.0 % (self.components.len() - 1);
                self.write_consumer();
                (CONSUMER_IMPORT, self.root.join("Consumer.tsx"))
            }
            _ => {
                let prop = format!("d{}", rng.below(3));
                if let Some(at) = self.story.iter().position(|p| *p == prop) {
                    self.story.remove(at);
                } else {
                    self.story.push(prop);
                }
                self.write_story();
                ("edit the story", self.root.join("C0.stories.tsx"))
            }
        }
    }
}

const CONSUMER_JSX: &str = "edit only a consumer's JSX";
const CONSUMER_IMPORT: &str = "edit only a consumer's import";

fn options(root: &Utf8Path) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.to_owned()],
        use_cache: false,
        html_attributes: HtmlAttributeMode::None,
        collapse_inherited_props: CollapseInheritedProps::All,
        ..PipelineOptions::default()
    }
}

#[test]
fn watch_equals_fresh() {
    let (mut edits, mut moved, mut consumer_moved) = (0, 0, 0);
    for seed in [1_u64, 7, 42] {
        let dir = tempfile::Builder::new().prefix("RelevanceWatch").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let mut rng = Lcg(seed);
        let mut model = Model::new(root.clone(), &mut rng);
        let options = options(&root);
        let watch = WatchSession::new(options.clone());
        drop(watch.initialize());
        let lazy = WatchSession::lazy(options.clone());
        drop(lazy.initialize());
        let fresh = extract(&options);
        // Ask the lazy session for everything, so each edit has kept answers to go stale.
        for key in fresh.components.keys() {
            drop(lazy.component(key));
        }
        let mut previous = fresh;
        let mut history = Vec::new();
        for _ in 0..14 {
            let (edit, file) = model.edit(&mut rng);
            history.push(edit);
            drop(watch.update_file(&file));
            let delta = lazy.update_file(&file);
            let fresh = extract(&options);
            let watched = watch.snapshot();
            let keys: Vec<&String> = fresh.components.keys().collect();
            assert_eq!(watched.components.keys().collect::<Vec<_>>(), keys, "seed {seed} after {history:?}");
            for (key, expected) in &fresh.components {
                assert_eq!(&watched.components[key], expected, "watch: {key}, seed {seed} after {history:?}");
                let answer = lazy.component(key).unwrap_or_else(|| panic!("lazy: no {key}"));
                assert_eq!(&answer.entry, expected, "lazy: {key}, seed {seed} after {history:?}");
            }
            // The model is only as strong as the evidence it moves: count the edits that change a `used` list.
            moved += usize::from(used(&previous) != used(&fresh));
            // A lazy delta names every key whose entry changed, a kept answer re-decided against the new call sites
            // included; after an edit that only moves call sites, it names nothing else.
            let changed: Vec<&String> = fresh
                .components
                .iter()
                .filter(|(key, entry)| previous.components.get(*key) != Some(entry))
                .map(|(key, _)| key)
                .collect();
            let reported: Vec<&String> = delta.updated_keys.iter().collect();
            for key in &changed {
                assert!(reported.contains(key), "lazy delta misses {key}: {reported:?}, seed {seed} after {history:?}");
            }
            if file.ends_with("Page.tsx") || file.ends_with("Consumer.tsx") {
                assert_eq!(reported, changed, "lazy delta after a consumer edit, seed {seed} after {history:?}");
                consumer_moved += usize::from(!changed.is_empty() && [CONSUMER_JSX, CONSUMER_IMPORT].contains(&edit));
            }
            previous = fresh;
            edits += 1;
        }
    }
    assert_eq!(edits, 3 * 14);
    assert!(moved >= 3, "edits changed a `used` list {moved} times: the model does not exercise relevance");
    assert!(consumer_moved >= 2, "consumer-only edits changed an entry {consumer_moved} times");
}

/// Every component's non-empty `used` lists, by component and layer.
fn used(output: &caliper_docgen_core::types::ExtractionOutput) -> Vec<(String, String, Vec<String>)> {
    let mut used = Vec::new();
    for (key, entry) in &output.components {
        for layer in entry.inheritance.iter().filter(|layer| !layer.used.is_empty()) {
            used.push((key.clone(), layer.type_name.clone(), layer.used.clone()));
        }
    }
    used
}
