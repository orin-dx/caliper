#![allow(clippy::unwrap_used, clippy::expect_used)]
//! What a load could not read, as the layer's one record of it: the warnings it reports, the demands it follows again
//! and the answers a session may keep. Every unreadable file here holds bytes that are not UTF-8, which no user can
//! read, root included.

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::pipeline::{CollapseInheritedProps, ComponentAnswer, PipelineOptions, WatchSession};

fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    for (path, body) in files {
        write(&root, path, body);
    }
    (tmp, root)
}

fn write(root: &Utf8Path, path: &str, body: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn options(root: &Utf8Path) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.join("src")],
        cache_dir: Some(root.join("cache")),
        load_dependencies: true,
        collapse_inherited_props: CollapseInheritedProps::None,
        ..Default::default()
    }
}

const UNREADABLE: [u8; 3] = [0xff, 0xfe, 0xfd];

/// The files the layer's reported diagnostics name, in order.
fn reported(layer: &DependencyLayer) -> Vec<Option<String>> {
    layer.panic_diagnostics().into_iter().map(|diagnostic| diagnostic.file).collect()
}

const SOLO: &[(&str, &str)] = &[
    ("node_modules/solo-kit/package.json", r#"{ "name": "solo-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
    ("node_modules/solo-kit/index.d.ts", "export * from './props';\n"),
    ("node_modules/solo-kit/props.d.ts", "export interface SoloProps { tone: string }\n"),
];

/// A file that stays unreadable while a session's layer grows past the depth that flattens it is reported once, by
/// every layer of the chain and by the flattened one. On the base each growth that retried the file added a copy of the
/// warning, and flattening kept them all.
#[test]
fn a_file_that_stays_unreadable_is_reported_once_however_often_the_layer_grows() {
    let (_tmp, root) = project(SOLO);
    let props = root.join("node_modules/solo-kit/props.d.ts");
    std::fs::write(&props, UNREADABLE).unwrap();
    let demands =
        vec![Demand { from: root.join("src/Card.tsx"), specifier: "solo-kit".into(), name: "SoloProps".into() }];

    let mut layer = Arc::new(load(demands.clone(), &options(&root), None));
    assert_eq!(reported(&layer), [Some(props.to_string())]);
    for growth in 1..=(MAX_CHAIN_DEPTH + 3) {
        let again = layer.missing(&demands);
        assert_eq!(again, demands, "growth {growth}: the demand is followed again");
        layer = Arc::new(load(again, &options(&root), Some(&layer)));
        assert_eq!(reported(&layer), [Some(props.to_string())], "growth {growth}");
    }
}

/// Within one load a read of a file that failed stands, even when another reader of the same file (a scan and a parse
/// hold a cell each) then reads it: what the first reader's walk derived is incomplete, so the file stays failed and
/// its warning stays until the next load reads it again. The layer below failed on it too, so the successful read is
/// the one that would end that earlier failure.
#[test]
fn a_failed_read_stands_for_its_load_when_another_reader_then_reads_the_file() {
    let (_tmp, root) = project(SOLO);
    let props = root.join("node_modules/solo-kit/props.d.ts");
    let text = std::fs::read_to_string(&props).unwrap();
    std::fs::write(&props, UNREADABLE).unwrap();
    let demands =
        vec![Demand { from: root.join("src/Card.tsx"), specifier: "solo-kit".into(), name: "SoloProps".into() }];
    let options = options(&root);
    let below = Arc::new(load(demands, &options, None));
    assert_eq!(reported(&below), [Some(props.to_string())]);

    let resolvers = crate::resolver::ModuleResolvers::new(&options);
    let loader = Loader::new(&resolvers, &[], Some(&below));
    assert!(loader.read(&props).is_err());
    std::fs::write(&props, &text).unwrap();
    assert_eq!(loader.read(&props).ok().flatten().as_deref(), Some(text.as_str()), "the other reader reads it");
    let ledger = loader.ledger.finish();
    assert!(matches!(ledger.reads.get(props.as_str()), Some(Read::Failed(_))), "the failure stands for this load");
}

/// A cold load with one unreadable file writes the index for every walk that read all it needed, and leaves out only
/// the one that could not: the next cold load answers the complete walks from it, reading neither re-export stub, and
/// loads what a load without the index loads. On the base no index was written when any read failed, so the next load
/// walked every stub again.
#[test]
fn a_cold_load_with_an_unreadable_file_indexes_every_walk_it_completed() {
    let kit: &[(&str, &str)] = &[
        ("node_modules/mix-kit/package.json", r#"{ "name": "mix-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
        (
            "node_modules/mix-kit/index.d.ts",
            "export * from './a-stub';\nexport { BProps } from './b';\nexport * from './c-stub';\n",
        ),
        ("node_modules/mix-kit/a-stub.d.ts", "export * from './a';\n"),
        ("node_modules/mix-kit/a.d.ts", "export interface AProps { a: string }\n"),
        ("node_modules/mix-kit/b.d.ts", "export interface BProps { b: string }\n"),
        ("node_modules/mix-kit/c-stub.d.ts", "export * from './c';\n"),
        ("node_modules/mix-kit/c.d.ts", "export interface CProps { c: string }\n"),
    ];
    let (_tmp, root) = project(kit);
    let demands: Vec<Demand> = ["AProps", "BProps", "CProps"]
        .into_iter()
        .map(|name| Demand { from: root.join("src/Card.tsx"), specifier: "mix-kit".into(), name: name.into() })
        .collect();
    let options = options(&root);
    let index = || -> Vec<ExportRow> {
        let bytes = std::fs::read(index_path(&options).0).unwrap();
        let index: IndexFile = rmp_serde::from_slice(&bytes).unwrap();
        let short = |path: &str| path.trim_start_matches(&format!("{root}/node_modules/mix-kit/")).to_owned();
        let mut rows: Vec<_> = index
            .packages
            .into_iter()
            .flat_map(|package| package.exports)
            .map(|(module, name, found)| (short(&module), name, found.map(|(file, name)| (short(&file), name))))
            .collect();
        rows.sort();
        rows
    };
    let files = |layer: &DependencyLayer| {
        let mut files: Vec<String> =
            layer.files.iter().map(|file| std::borrow::Borrow::<str>::borrow(file).to_owned()).collect();
        files.sort();
        files
    };
    let b = root.join("node_modules/mix-kit/b.d.ts");
    let text = std::fs::read_to_string(&b).unwrap();

    let readable = load(demands.clone(), &options, None);
    let (unread, complete): (Vec<_>, Vec<_>) = index().into_iter().partition(|(_, name, _)| name == "BProps");
    let found = |file: &str, name: &str| Some((file.to_owned(), name.to_owned()));
    let row = |module: &str| (module.to_owned(), "BProps".to_owned(), found("b.d.ts", "BProps"));
    assert_eq!(unread, [row("b.d.ts"), row("index.d.ts")]);
    // Each walk keeps the answer of every module it passed: the barrel, the stub and the declaring file.
    assert_eq!(complete.len(), 8, "{complete:?}");
    std::fs::remove_file(index_path(&options).0).unwrap();

    std::fs::write(&b, UNREADABLE).unwrap();
    let first = load(demands.clone(), &options, None);
    assert_eq!(reported(&first), [Some(b.to_string())]);
    assert_eq!(index(), complete, "every complete walk is indexed, the unread one is not");
    let second = load(demands.clone(), &options, None);
    assert_eq!(first.reads - second.reads, 2, "the stubs are not read again");
    let oracle = load(demands.clone(), &PipelineOptions { use_cache: false, ..options.clone() }, None);
    assert_eq!(files(&second), files(&oracle));
    assert_eq!(reported(&second), [Some(b.to_string())]);

    std::fs::write(&b, text).unwrap();
    let recovered = load(demands, &options, None);
    assert_eq!(files(&recovered), files(&readable), "once it reads, the next load equals the cold one");
    assert_eq!(reported(&recovered), Vec::<Option<String>>::new());
}

/// A re-export stub an augmentation search only scans (it names neither React nor `"react"`) and could not read is
/// reported until a load reads it again, and not after: the scan's read clears it as a parse's does. On the base only a
/// parse did, so the warning outlived the read.
#[test]
fn a_stub_that_reads_again_through_a_scan_is_no_longer_reported() {
    let (_tmp, root) = project(&[
        ("src/App.tsx", "import 'aug-kit';\n"),
        ("node_modules/aug-kit/package.json", r#"{"name":"aug-kit","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/aug-kit/index.d.ts", "export * from './stub';\n"),
        ("node_modules/aug-kit/stub.d.ts", "export * from './inner';\n"),
        (
            "node_modules/aug-kit/inner.d.ts",
            "import 'react';\ndeclare module 'react' { interface CSSProperties { tone?: string } }\nexport {};\n",
        ),
    ]);
    let stub = root.join("node_modules/aug-kit/stub.d.ts");
    let text = std::fs::read_to_string(&stub).unwrap();
    let demands =
        vec![Demand { from: root.join("src/App.tsx"), specifier: "aug-kit".into(), name: CompactString::default() }];

    std::fs::write(&stub, UNREADABLE).unwrap();
    let failed = Arc::new(load(demands.clone(), &options(&root), None));
    assert_eq!(reported(&failed), [Some(stub.to_string())]);
    std::fs::write(&stub, text).unwrap();

    let again = failed.missing(&demands);
    assert_eq!(again, demands, "the module demand is followed again");
    let recovered = load(again, &options(&root), Some(&failed));
    assert_eq!(reported(&recovered), Vec::<Option<String>>::new(), "the scan read the stub");
    assert!(recovered.parsed_data(stub.as_str()).is_none(), "only scanned, never parsed");
    let inner = root.join("node_modules/aug-kit/inner.d.ts");
    assert!(!failed.is_dependency_file(&inner), "the search did not reach the augmentation past the stub");
    assert!(recovered.is_dependency_file(&inner), "the augmentation past the stub is loaded");
}

/// Two files of one resolution scope import a module for its own sake, and an augmentation it leads to takes a type from
/// a file that could not be read. Only the first demand to ask searches the module and loads the augmentation, so the
/// other reaches the unread walk through it: each demand leads to the same unread one, whichever searched.
#[test]
fn every_demand_of_a_scope_leads_to_what_the_search_its_first_demand_made_left_unread() {
    let (_tmp, root) = project(&[
        ("node_modules/aug-kit/package.json", r#"{"name":"aug-kit","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/aug-kit/index.d.ts", "export * from './inner';\n"),
        (
            "node_modules/aug-kit/inner.d.ts",
            "import type { Tone } from './tone';\n\
             declare module 'react' { interface CSSProperties { tone?: Tone } }\nexport {};\n",
        ),
        ("node_modules/aug-kit/tone.d.ts", "export type Tone = 'info' | 'warn';\n"),
    ]);
    std::fs::write(root.join("node_modules/aug-kit/tone.d.ts"), UNREADABLE).unwrap();
    let module_demand =
        |dir: &str| Demand { from: root.join(dir).join("App.tsx"), specifier: "aug-kit".into(), name: "".into() };
    let demands = vec![module_demand("src/a"), module_demand("src/b")];
    let tone =
        Demand { from: root.join("node_modules/aug-kit/inner.d.ts"), specifier: "./tone".into(), name: "Tone".into() };

    let layer = load(demands.clone(), &options(&root), None);
    assert_eq!(layer.missing(&demands), [], "the search itself read every file");
    for demand in &demands {
        assert_eq!(layer.unread(std::slice::from_ref(demand)), std::slice::from_ref(&tone), "{}", demand.from);
    }
}

fn names(answer: &Option<ComponentAnswer>) -> Vec<String> {
    let mut names: Vec<String> = answer.as_ref().unwrap().entry.props.keys().cloned().collect();
    names.sort();
    names
}

fn warned(answer: &Option<ComponentAnswer>) -> Vec<Option<String>> {
    let mut files: Vec<Option<String>> = answer
        .as_ref()
        .unwrap()
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::IoError)
        .map(|diagnostic| diagnostic.file.clone())
        .collect();
    files.sort();
    files
}

const NESTED: &[(&str, &str)] = &[
    ("node_modules/nest-kit/package.json", r#"{ "name": "nest-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
    (
        "node_modules/nest-kit/index.d.ts",
        "import type { Base } from './base';\nexport interface NestProps extends Base { own: string }\n",
    ),
    ("node_modules/nest-kit/base.d.ts", "export interface Base { tone: string }\n"),
    (
        "src/Card.tsx",
        "import type { NestProps } from 'nest-kit';\ninterface CardProps extends NestProps { title: string }\n\
         export function Card(props: CardProps) { return null; }\n",
    ),
];

/// A readable package declaration that extends a type from a file that could not be read: the failed demand is the
/// package file's, not the project's, and the project's demand was followed. The session's answer without the nested
/// file is not kept, so once the file reads the next request has the inherited prop, the same as a fresh session. On
/// the base the retry guard looked only at the project file's demands, and the memo kept `["own", "title"]`.
#[test]
fn a_prop_a_package_file_inherits_from_an_unreadable_file_appears_once_the_file_reads() {
    let (_tmp, root) = project(NESTED);
    let base = root.join("node_modules/nest-kit/base.d.ts");
    let text = std::fs::read_to_string(&base).unwrap();

    let session = WatchSession::lazy(options(&root));
    drop(session.initialize());
    std::fs::write(&base, UNREADABLE).unwrap();
    let card = session.component("Card");
    assert_eq!(names(&card), ["own", "title"], "answered from what was read");
    assert_eq!(warned(&card), [Some(base.to_string())]);
    let card = session.component("Card");
    assert_eq!(names(&card), ["own", "title"], "asked again while it stays unreadable");
    assert_eq!(warned(&card), [Some(base.to_string())]);
    std::fs::write(&base, text).unwrap();
    let card = session.component("Card");

    let fresh = WatchSession::lazy(options(&root));
    drop(fresh.initialize());
    let fresh_card = fresh.component("Card");
    assert_eq!(names(&fresh_card), ["own", "title", "tone"]);
    assert_eq!(card, fresh_card, "the session answers as a fresh one once the file reads");
}

/// A later load claims another declaration that imports the same type from the same unreadable file: it meets the
/// demand an earlier load left unread and, while that retry is not due, does not read the file again; it still leads
/// to the unread walk, so what it answers is not kept. A load that retries every unread walk reads the file again.
#[test]
fn a_load_meets_an_unread_demand_that_is_not_due_without_reading_its_file_again() {
    let (_tmp, root) = project(&[
        ("node_modules/nest-kit/package.json", r#"{ "name": "nest-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
        (
            "node_modules/nest-kit/index.d.ts",
            "import type { Base } from './base';\nexport interface NestProps extends Base { own: string }\n\
             export interface OtherProps extends Base { other: string }\n",
        ),
        ("node_modules/nest-kit/base.d.ts", "export interface Base { tone: string }\n"),
    ]);
    std::fs::write(root.join("node_modules/nest-kit/base.d.ts"), UNREADABLE).unwrap();
    let demand =
        |name: &str| Demand { from: root.join("src/Card.tsx"), specifier: "nest-kit".into(), name: name.into() };
    let base =
        Demand { from: root.join("node_modules/nest-kit/index.d.ts"), specifier: "./base".into(), name: "Base".into() };
    let options = options(&root);
    let due = Retry::Due { now: std::time::Instant::now(), edits: 0 };
    let first = Arc::new(load_at(vec![demand("NestProps")], &options, None, due));
    assert_eq!(first.unread(&[demand("NestProps")]), std::slice::from_ref(&base));

    let later = load_at(vec![demand("OtherProps")], &options, Some(&first), due);
    assert_eq!(later.reads, 0, "the unread demand is met, not followed again");
    assert_eq!(later.unread(&[demand("OtherProps")]), [base], "and it still leads to the unread walk");
    let always = load_at(vec![demand("OtherProps")], &options, Some(&first), Retry::Always);
    assert_eq!(always.reads, 1, "a load that retries every unread walk reads the file again");
}

/// A second component reaches the same package declaration in a later load, which finds it claimed by the first and
/// does not walk it again: it still leads to the walk that could not read the nested file, so its answer is not kept
/// either, and once the file reads it has the inherited prop.
#[test]
fn a_component_that_meets_a_declaration_an_earlier_load_left_unread_is_not_kept_either() {
    let tag = "import type { NestProps } from 'nest-kit';\ninterface TagProps extends NestProps { label: string }\n\
               export function Tag(props: TagProps) { return null; }\n";
    // Another directory, so its demand is another than `Card`'s: a demand is keyed by its directory.
    let (_tmp, root) = project(&[NESTED, &[("src/tag/Tag.tsx", tag)]].concat());
    let base = root.join("node_modules/nest-kit/base.d.ts");
    let text = std::fs::read_to_string(&base).unwrap();

    let session = WatchSession::lazy(options(&root));
    drop(session.initialize());
    std::fs::write(&base, UNREADABLE).unwrap();
    assert_eq!(names(&session.component("Card")), ["own", "title"]);
    let tag = session.component("Tag");
    assert_eq!(names(&tag), ["label", "own"], "answered from what was read");
    assert_eq!(warned(&tag), [Some(base.to_string())]);
    std::fs::write(&base, text).unwrap();
    let tag = session.component("Tag");

    let fresh = WatchSession::lazy(options(&root));
    drop(fresh.initialize());
    let fresh_tag = fresh.component("Tag");
    assert_eq!(names(&fresh_tag), ["label", "own", "tone"]);
    assert_eq!(tag, fresh_tag);
}

/// The model's package: a barrel of `export *` targets with a re-export stub, a named re-export of a declaration that
/// extends a type from a nested file, and a chain of two more below it. Each of its files can be made unreadable.
const MODEL_PACKAGE: &[(&str, &str)] = &[
    ("node_modules/mk/package.json", r#"{ "name": "mk", "version": "1.0.0", "types": "index.d.ts" }"#),
    ("node_modules/mk/index.d.ts", "export * from './a';\nexport * from './stub';\nexport { Nest } from './nest';\n"),
    ("node_modules/mk/a.d.ts", "export interface AProps { a: string }\n"),
    ("node_modules/mk/stub.d.ts", "export * from './b';\n"),
    ("node_modules/mk/b.d.ts", "export interface BProps { b: string }\n"),
    (
        "node_modules/mk/nest.d.ts",
        "import type { Deep } from './deep';\nexport interface Nest extends Deep { n: string }\n",
    ),
    (
        "node_modules/mk/deep.d.ts",
        "import type { Leaf } from './leaf';\nexport interface Deep extends Leaf { d: string }\n",
    ),
    ("node_modules/mk/leaf.d.ts", "export interface Leaf { l: string }\n"),
];

/// Each component, what it imports, and the package files its walk reads when every file is readable: its demands'
/// export walks, the declarations they extend, and the augmentation search of the module its file imports.
const MODEL_COMPONENTS: &[(&str, &str, &[&str])] = &[
    ("A", "AProps", &["index", "a", "stub", "b", "nest"]),
    ("B", "BProps", &["index", "a", "stub", "b", "nest"]),
    ("N", "Nest", &["index", "a", "stub", "b", "nest", "deep", "leaf"]),
    ("All", "AProps, BProps, Nest", &["index", "a", "stub", "b", "nest", "deep", "leaf"]),
];

fn model_source(name: &str, imports: &str, edit: usize) -> String {
    let own = name.to_lowercase();
    format!(
        "import type {{ {imports} }} from 'mk';\ninterface {name}Own extends {imports} {{ own_{own}: string }}\n\
         export function {name}(props: {name}Own) {{ return null; }}\n// edit {edit}\n"
    )
}

fn model_project() -> (tempfile::TempDir, Utf8PathBuf) {
    let (tmp, root) = project(MODEL_PACKAGE);
    for (name, imports, _) in MODEL_COMPONENTS {
        write(&root, &format!("src/{name}.tsx"), &model_source(name, imports, 0));
    }
    (tmp, root)
}

/// Exactly `unreadable` of the package's declaration files unreadable, the rest as written.
fn set_unreadable(root: &Utf8Path, unreadable: &std::collections::BTreeSet<&str>) {
    for (path, body) in MODEL_PACKAGE {
        let Some(name) = path.strip_prefix("node_modules/mk/").and_then(|name| name.strip_suffix(".d.ts")) else {
            continue;
        };
        match unreadable.contains(name) {
            true => std::fs::write(root.join(path), UNREADABLE).unwrap(),
            false => std::fs::write(root.join(path), body).unwrap(),
        }
    }
}

/// The package files an answer's read warnings name, by their short name, one entry per warning.
fn warned_names(root: &Utf8Path, answer: &ComponentAnswer) -> Vec<String> {
    let prefix = format!("{root}/node_modules/mk/");
    let mut names: Vec<String> = answer
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::IoError)
        .map(|diagnostic| {
            let file = diagnostic.file.as_deref().unwrap_or_default();
            file.trim_start_matches(&prefix).trim_end_matches(".d.ts").to_owned()
        })
        .collect();
    names.sort();
    names
}

/// An answer without its read warnings, its paths written from `root`, for comparing across projects.
fn comparable(root: &Utf8Path, answer: &ComponentAnswer) -> String {
    let mut answer = answer.clone();
    answer.diagnostics.retain(|diagnostic| diagnostic.code != DiagnosticCode::IoError);
    format!("{answer:?}").replace(root.as_str(), "<root>")
}

/// A model-based test of what a lazy session holds while package files go unreadable and readable again: a seeded
/// sequence of files made unreadable (bytes that are not UTF-8) or readable, requests and project edits. After every
/// request:
///
/// 1. the answer equals a fresh session's (cache off) with exactly the files the session reports unreadable made
///    unreadable: the cold answer when it reports none, the cold answer minus exactly what those files held otherwise;
/// 2. each file is reported once, and a request made while every file reads reports none of the files its own walk
///    reads (it read them again);
/// 3. once every file reads, every component equals the cold answer, with no warning, in this session and in the next
///    one over the same cache: nothing from a degraded answer was memoized or persisted.
///
/// A failure prints the seed and the steps so far.
#[test]
fn a_session_over_files_that_go_unreadable_answers_as_a_fresh_one_over_what_it_could_read() {
    let names: Vec<&str> = MODEL_PACKAGE
        .iter()
        .filter_map(|(path, _)| path.strip_prefix("node_modules/mk/")?.strip_suffix(".d.ts"))
        .collect();
    let (_oracle_tmp, oracle_root) = model_project();
    let mut oracle_answers: FxHashMap<(String, Vec<String>), String> = FxHashMap::default();
    let mut oracle = |component: &str, unreadable: &[String]| -> String {
        let key = (component.to_owned(), unreadable.to_vec());
        oracle_answers
            .entry(key)
            .or_insert_with(|| {
                set_unreadable(&oracle_root, &unreadable.iter().map(String::as_str).collect());
                let fresh = WatchSession::lazy(PipelineOptions { use_cache: false, ..options(&oracle_root) });
                drop(fresh.initialize());
                comparable(&oracle_root, &fresh.component(component).unwrap())
            })
            .clone()
    };
    for seed in [1_u64, 2, 3, 5, 8, 13, 21, 34] {
        let (_tmp, root) = model_project();
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = |bound: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let mut unreadable: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut log: Vec<String> = Vec::new();
        let session = WatchSession::lazy(options(&root));
        if next(2) == 0 {
            unreadable.insert(names[next(names.len())]);
            set_unreadable(&root, &unreadable);
            log.push(format!("unreadable before start: {unreadable:?}"));
        }
        drop(session.initialize());
        for step in 0..40 {
            let context = |log: &[String]| format!("seed {seed}, step {step}:\n{}", log.join("\n"));
            match next(4) {
                0 => {
                    let name = names[next(names.len())];
                    if !unreadable.remove(name) {
                        unreadable.insert(name);
                    }
                    set_unreadable(&root, &unreadable);
                    log.push(format!("unreadable: {unreadable:?}"));
                }
                1 => {
                    let (name, imports, _) = MODEL_COMPONENTS[next(MODEL_COMPONENTS.len())];
                    let file = root.join(format!("src/{name}.tsx"));
                    std::fs::write(&file, model_source(name, imports, step + 1)).unwrap();
                    drop(session.update_file(&file));
                    log.push(format!("edit {name}"));
                }
                _ => {
                    let (name, _, reads) = MODEL_COMPONENTS[next(MODEL_COMPONENTS.len())];
                    let all_read = unreadable.is_empty();
                    let answer = session.component(name).unwrap();
                    session.wait_for_dependency_load();
                    log.push(format!("request {name}"));
                    let warned = warned_names(&root, &answer);
                    let mut once = warned.clone();
                    once.dedup();
                    assert_eq!(warned, once, "each file is reported once: {}", context(&log));
                    assert_eq!(comparable(&root, &answer), oracle(name, &warned), "{}", context(&log));
                    if all_read {
                        let reread: Vec<&String> =
                            warned.iter().filter(|file| reads.contains(&file.as_str())).collect();
                        assert_eq!(reread, Vec::<&String>::new(), "read again: {}", context(&log));
                    }
                }
            }
        }
        unreadable.clear();
        set_unreadable(&root, &unreadable);
        for (name, _, _) in MODEL_COMPONENTS {
            drop(session.component(name));
            session.wait_for_dependency_load();
        }
        let next_session = WatchSession::lazy(options(&root));
        drop(next_session.initialize());
        for (name, _, _) in MODEL_COMPONENTS {
            let cold = oracle(name, &[]);
            let context = format!("seed {seed}, all readable, {name}:\n{}", log.join("\n"));
            let answer = session.component(name).unwrap();
            assert_eq!(warned_names(&root, &answer), Vec::<String>::new(), "{context}");
            assert_eq!(comparable(&root, &answer), cold, "this session: {context}");
            let answer = next_session.component(name).unwrap();
            assert_eq!(warned_names(&root, &answer), Vec::<String>::new(), "{context}");
            assert_eq!(comparable(&root, &answer), cold, "the next session: {context}");
        }
    }
}
