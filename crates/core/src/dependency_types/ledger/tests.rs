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
/// every layer of the chain and by the flattened one.
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

/// The interleaving built by hand: in one load an augmentation scan fails to read a file, it becomes readable, and a
/// parse of it succeeds. The load's ledger keeps the failure, and the layer does not keep the parse, so the next load
/// reads the file again and that read ends the failure.
#[test]
fn a_file_whose_scan_failed_and_parse_read_in_one_load_is_not_kept_parsed() {
    let (_tmp, root) = project(SOLO);
    let props = root.join("node_modules/solo-kit/props.d.ts");
    let text = std::fs::read_to_string(&props).unwrap();
    std::fs::write(&props, UNREADABLE).unwrap();
    let options = options(&root);
    let resolvers = crate::resolver::ModuleResolvers::new(&options);
    let loader = Loader::new(&resolvers, &[], None);
    assert!(loader.augmentation_scan(&props, false).is_err(), "the scan's read fails");
    std::fs::write(&props, &text).unwrap();
    assert!(matches!(loader.parse(&props), Ok(Some(_))), "the parse's read succeeds");
    let Loader { parsed, ledger, .. } = loader;
    let ledger = ledger.finish();
    assert!(ledger.failed(props.as_str()), "the failure stands for this load");
    let kept = retained_parses(parsed, &ledger);
    assert!(!kept.contains_key(props.as_str()), "no layer holds the file both failed and parsed");
}

/// A cold load with one unreadable file writes the index for every walk that read all it needed, and leaves out only
/// the one that could not: the next cold load answers the complete walks from it, reading neither re-export stub, and
/// loads what a load without the index loads.
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

/// A chain of 200 package declarations, each extending the next, ends at one file that could not be read: finding the
/// nodes that reach the unread walk visits each node once.
#[test]
fn the_nodes_that_reach_an_unread_walk_are_found_in_one_traversal() {
    let mut files: Vec<(String, String)> = vec![
        (
            "node_modules/chain-kit/package.json".into(),
            r#"{ "name": "chain-kit", "version": "1.0.0", "types": "index.d.ts" }"#.into(),
        ),
        ("node_modules/chain-kit/index.d.ts".into(), "export { D0 } from './d0';\n".into()),
        ("node_modules/chain-kit/leaf.d.ts".into(), "export interface Leaf { leaf: string }\n".into()),
    ];
    for n in 0..200 {
        let (next, from) = if n == 199 {
            ("Leaf".to_owned(), "leaf".to_owned())
        } else {
            (format!("D{}", n + 1), format!("d{}", n + 1))
        };
        let body = format!(
            "import type {{ {next} }} from './{from}';\nexport interface D{n} extends {next} {{ f{n}: string }}\n"
        );
        files.push((format!("node_modules/chain-kit/d{n}.d.ts"), body));
    }
    let files: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let (_tmp, root) = project(&files);
    std::fs::write(root.join("node_modules/chain-kit/leaf.d.ts"), UNREADABLE).unwrap();
    let demands = vec![Demand { from: root.join("src/Card.tsx"), specifier: "chain-kit".into(), name: "D0".into() }];
    let layer = load(demands, &options(&root), None);
    let (walks, visits) = layer.unread_walks_counted();
    assert!(walks.touches(root.join("node_modules/chain-kit/d0.d.ts").as_str()), "the head reaches the unread walk");
    let marks = layer.ledger.marks.len();
    assert!(marks >= 400, "{marks} marks: every claim and demand of the chain");
    assert!(visits <= marks, "{visits} visits for {marks} marks");
}

/// One file could not be read in the first load; each of twelve growths after it loads an unrelated name and leaves
/// the ledger as it was. Asked of every layer, twice, the unread walks are built once in all: each later layer's are
/// its parent's.
#[test]
fn the_unread_walks_are_built_once_per_layer_that_changed_the_ledger() {
    let mut files: Vec<(String, String)> = vec![
        (
            "node_modules/nest-kit/package.json".into(),
            r#"{ "name": "nest-kit", "version": "1.0.0", "types": "index.d.ts" }"#.into(),
        ),
        (
            "node_modules/nest-kit/index.d.ts".into(),
            "import type { Base } from './base';\nexport interface NestProps extends Base { own: string }\n".into(),
        ),
        ("node_modules/nest-kit/base.d.ts".into(), "export interface Base { tone: string }\n".into()),
        (
            "node_modules/many-kit/package.json".into(),
            r#"{ "name": "many-kit", "version": "1.0.0", "types": "index.d.ts" }"#.into(),
        ),
    ];
    let index: String = (0..12).map(|n| format!("export interface P{n} {{ p{n}: string }}\n")).collect();
    files.push(("node_modules/many-kit/index.d.ts".into(), index));
    let files: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let (_tmp, root) = project(&files);
    std::fs::write(root.join("node_modules/nest-kit/base.d.ts"), UNREADABLE).unwrap();
    let demand = |specifier: &str, name: String| Demand {
        from: root.join("src/Card.tsx"),
        specifier: specifier.into(),
        name: name.into(),
    };
    let options = options(&root);
    let now = Moment { at: std::time::Instant::now(), edits: Edits::default() };
    let mut layer = Arc::new(load_at(
        vec![demand("nest-kit", "NestProps".into())],
        &options,
        None,
        Retry::Due(now),
        Stamp { moment: now, load: Load(1) },
    ));
    let mut layers = vec![layer.clone()];
    for n in 0..12 {
        let stamp = Stamp { moment: now, load: Load(n + 2) };
        layer = Arc::new(load_at(
            vec![demand("many-kit", format!("P{n}"))],
            &options,
            Some(&layer),
            Retry::Due(now),
            stamp,
        ));
        layers.push(layer.clone());
    }
    for layer in layers.iter().chain(&layers) {
        assert!(!layer.unread_walks().is_empty());
    }
    assert_eq!(layer.unread_walk_builds(), 1, "one build for the chain");
}

/// The same chain, at the depth limit: the load that asks for the other claims flattens the chain, so it has no
/// parent to ask. It takes in the chain's marks and answers which nodes reach the unread walk with one reverse
/// traversal of them, not one walk of the ledger per claim; each claim still leads there.
#[test]
fn a_flatten_that_meets_many_earlier_claims_traverses_the_marks_once() {
    let mut files: Vec<(String, String)> = vec![(
        "node_modules/chain-kit/package.json".into(),
        r#"{ "name": "chain-kit", "version": "1.0.0", "types": "index.d.ts" }"#.into(),
    )];
    let mut index = String::new();
    for n in 0..200 {
        index.push_str(&format!("export {{ D{n} }} from './d{n}';\n"));
        let (next, from) = if n == 199 {
            ("Leaf".to_owned(), "leaf".to_owned())
        } else {
            (format!("D{}", n + 1), format!("d{}", n + 1))
        };
        let body = format!(
            "import type {{ {next} }} from './{from}';\nexport interface D{n} extends {next} {{ f{n}: string }}\n"
        );
        files.push((format!("node_modules/chain-kit/d{n}.d.ts"), body));
    }
    files.push(("node_modules/chain-kit/index.d.ts".into(), index));
    files.push(("node_modules/chain-kit/leaf.d.ts".into(), "export interface Leaf { leaf: string }\n".into()));
    let files: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let (_tmp, root) = project(&files);
    std::fs::write(root.join("node_modules/chain-kit/leaf.d.ts"), UNREADABLE).unwrap();
    let demand = |n: usize| Demand {
        from: root.join("src/Card.tsx"),
        specifier: "chain-kit".into(),
        name: format!("D{n}").into(),
    };
    let options = options(&root);
    let now = Moment { at: std::time::Instant::now(), edits: Edits::default() };
    let stamp = |load| Stamp { moment: now, load: Load(load) };
    let mut layer = Arc::new(load_at(vec![demand(0)], &options, None, Retry::Due(now), stamp(1)));
    for n in 1..=MAX_CHAIN_DEPTH {
        layer = Arc::new(load_at(vec![demand(n)], &options, Some(&layer), Retry::Due(now), stamp(n as u64 + 1)));
    }
    let rest: Vec<Demand> = (MAX_CHAIN_DEPTH + 1..200).map(demand).collect();
    let flattened = load_at(rest, &options, Some(&layer), Retry::Due(now), stamp(100));
    assert!(flattened.parent.is_none(), "the depth limit flattened the chain");
    assert_eq!(flattened.ledger.traversals, 1, "one reverse traversal, not one walk per claim");
    assert!(!flattened.unread(&[demand(150)]).is_empty(), "a claim met still leads to the unread walk");
}

/// Once the last unread walk is followed again and reads, the chain still holds its (clean) marks; asking which
/// unread walks 1000 live demands lead to visits none of them.
#[test]
fn a_recovered_chain_visits_no_root() {
    let (_tmp, root) = project(SOLO);
    let props = root.join("node_modules/solo-kit/props.d.ts");
    let text = std::fs::read_to_string(&props).unwrap();
    std::fs::write(&props, UNREADABLE).unwrap();
    let demand = |n: usize| Demand {
        from: root.join(format!("src/c{n}/Card.tsx")),
        specifier: "solo-kit".into(),
        name: "SoloProps".into(),
    };
    let failed = Arc::new(load(vec![demand(0)], &options(&root), None));
    std::fs::write(&props, text).unwrap();
    let recovered = load(failed.due_walks(Retry::Always), &options(&root), Some(&failed));
    assert!(recovered.has_marks(), "the recovered walk's marks stay");
    let roots: Vec<Demand> = (0..1000).map(demand).collect();
    let visited = std::cell::Cell::new(0);
    assert_eq!(recovered.unread_from(roots.iter().inspect(|_| visited.set(visited.get() + 1))), Vec::<Demand>::new());
    assert_eq!(visited.get(), 0, "no root visited once no walk is unread");
}

/// A chain of 200 declarations ends at one file that could not be read. A later load asks for each of the other 199
/// by name: each is a claim the first load made, which this load meets without walking it. Whether each reaches the
/// unread walk is the parent's answer, computed once: no walk of the ledger per claim, and each still leads there.
#[test]
fn a_load_that_meets_many_earlier_claims_asks_the_parent_once() {
    let mut files: Vec<(String, String)> = vec![(
        "node_modules/chain-kit/package.json".into(),
        r#"{ "name": "chain-kit", "version": "1.0.0", "types": "index.d.ts" }"#.into(),
    )];
    let mut index = String::new();
    for n in 0..200 {
        index.push_str(&format!("export {{ D{n} }} from './d{n}';\n"));
        let (next, from) = if n == 199 {
            ("Leaf".to_owned(), "leaf".to_owned())
        } else {
            (format!("D{}", n + 1), format!("d{}", n + 1))
        };
        let body = format!(
            "import type {{ {next} }} from './{from}';\nexport interface D{n} extends {next} {{ f{n}: string }}\n"
        );
        files.push((format!("node_modules/chain-kit/d{n}.d.ts"), body));
    }
    files.push(("node_modules/chain-kit/index.d.ts".into(), index));
    files.push(("node_modules/chain-kit/leaf.d.ts".into(), "export interface Leaf { leaf: string }\n".into()));
    let files: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let (_tmp, root) = project(&files);
    std::fs::write(root.join("node_modules/chain-kit/leaf.d.ts"), UNREADABLE).unwrap();
    let demand = |n: usize| Demand {
        from: root.join("src/Card.tsx"),
        specifier: "chain-kit".into(),
        name: format!("D{n}").into(),
    };
    let options = options(&root);
    let now = Moment { at: std::time::Instant::now(), edits: Edits::default() };
    let first =
        Arc::new(load_at(vec![demand(0)], &options, None, Retry::Due(now), Stamp { moment: now, load: Load(1) }));
    let later = load_at(
        (1..200).map(demand).collect(),
        &options,
        Some(&first),
        Retry::Due(now),
        Stamp { moment: now, load: Load(2) },
    );
    assert_eq!(later.ledger.traversals, 0, "no walk of the ledger per earlier claim");
    assert_eq!(first.unread_walk_builds(), 1, "the parent's answer, built once");
    assert!(!later.unread(&[demand(150)]).is_empty(), "a claim met still leads to the unread walk");
}

/// A barrel walks an `export *` cycle, then an unreadable target, then finds `NameProps` in a later file: a partial
/// mapping to that file. Once the target reads and declares `NameProps` first, the cycle still keeps the answer out of
/// `exports`, and the newest walk's outcome supersedes the older layer's mapping: the resolver is not sent to the
/// later file.
#[test]
fn the_newest_walk_of_a_barrel_name_supersedes_an_older_partial_mapping() {
    let (_tmp, root) = project(&[
        ("node_modules/cyc-kit/package.json", r#"{ "name": "cyc-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
        (
            "node_modules/cyc-kit/index.d.ts",
            "export * from './loop';\nexport * from './first';\nexport * from './second';\n",
        ),
        ("node_modules/cyc-kit/loop.d.ts", "export * from './index';\n"),
        ("node_modules/cyc-kit/first.d.ts", "export interface NameProps { first: string }\n"),
        ("node_modules/cyc-kit/second.d.ts", "export interface NameProps { second: string }\n"),
    ]);
    let first = root.join("node_modules/cyc-kit/first.d.ts");
    let text = std::fs::read_to_string(&first).unwrap();
    std::fs::write(&first, UNREADABLE).unwrap();
    let demand = Demand { from: root.join("src/Card.tsx"), specifier: "cyc-kit".into(), name: "NameProps".into() };
    let (index, second) = (root.join("node_modules/cyc-kit/index.d.ts"), root.join("node_modules/cyc-kit/second.d.ts"));
    let options = options(&root);
    let failed = Arc::new(load(vec![demand.clone()], &options, None));
    assert_eq!(failed.export_for_resolver(&index, "NameProps"), Some(Some((second.clone(), "NameProps".into()))));

    let card = "import type { NameProps } from 'cyc-kit';\ninterface CardProps extends NameProps { own: string }\n\
                export function Card(props: CardProps) { return null; }\n";
    write(&root, "src/Card.tsx", card);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    std::fs::write(&first, text).unwrap();
    let recovered = load(failed.due_walks(Retry::Always), &options, Some(&failed));
    assert_ne!(
        recovered.export_for_resolver(&index, "NameProps"),
        Some(Some((second, "NameProps".into()))),
        "the older layer's mapping no longer stands"
    );
}

/// A module's augmentation search could not read two files. A retry reads one of them again and still fails on the
/// other: an answer resolved before the retry no longer holds, though the walk's latest mark (one file, a new retry
/// time) would say it did. So does an answer whose files were read again and failed again, by any load.
#[test]
fn an_answer_holds_only_while_every_file_it_could_not_read_is_unchanged() {
    let augments = "import 'react';\ndeclare module 'react' { interface CSSProperties { one?: string } }\nexport {};\n";
    let (_tmp, root) = project(&[
        ("node_modules/aug2-kit/package.json", r#"{"name":"aug2-kit","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/aug2-kit/index.d.ts", "export * from './one';\nexport * from './two';\n"),
        ("node_modules/aug2-kit/one.d.ts", augments),
        ("node_modules/aug2-kit/two.d.ts", augments),
    ]);
    let (one, two) = (root.join("node_modules/aug2-kit/one.d.ts"), root.join("node_modules/aug2-kit/two.d.ts"));
    std::fs::write(&one, UNREADABLE).unwrap();
    std::fs::write(&two, UNREADABLE).unwrap();
    let demand = Demand { from: root.join("src/App.tsx"), specifier: "aug2-kit".into(), name: "".into() };
    let options = options(&root);
    let start = std::time::Instant::now();
    let at = |at, edits| Moment { at, edits: Edits(edits) };
    let stamp = |moment, load| Stamp { moment, load: Load(load) };
    let first =
        Arc::new(load_timed(vec![demand.clone()], &options, None, Retry::Due(at(start, 0)), stamp(at(start, 0), 1), {
            Arc::new(move || start)
        }));
    let footprint = first.footprint(&first.unread(std::slice::from_ref(&demand))).unwrap();
    assert!(first.footprint_holds(&footprint, at(start, 0)));
    assert!(!first.footprint_holds(&footprint, at(start + DEGRADED_RETRY, 0)), "not past its retry");
    assert!(!first.footprint_holds(&footprint, at(start, 1)), "not past an edit");

    let again = load_at(vec![demand.clone()], &options, Some(&first), Retry::Always, stamp(at(start, 0), 2));
    assert!(!again.footprint_holds(&footprint, at(start, 0)), "both read again and failed again: a new failure");

    std::fs::write(&one, augments).unwrap();
    let retry = at(start + DEGRADED_RETRY, 0);
    let clock: Clock = Arc::new(move || retry.at);
    let recovered = load_timed(vec![demand.clone()], &options, Some(&first), Retry::Due(retry), stamp(retry, 3), clock);
    assert!(recovered.is_dependency_file(&one), "the retry loads the file it read again");
    let latest = recovered.footprint(&recovered.unread(std::slice::from_ref(&demand))).unwrap();
    assert!(recovered.footprint_holds(&latest, retry), "the walk's latest mark alone still looks unexpired");
    assert!(!recovered.footprint_holds(&footprint, retry), "the earlier answer does not hold: one of its files reads");
}

/// The eager revision: two components of one directory share a demand whose package declaration extends a type from a
/// file that could not be read. Once it reads, an edit to one of them follows the shared walk again, and both answer as
/// a fresh session does.
#[test]
fn an_eager_edit_resolves_again_every_component_that_reached_a_walk_it_followed_again() {
    let tag = "import type { NestProps } from 'nest-kit';\ninterface TagProps extends NestProps { label: string }\n\
               export function Tag(props: TagProps) { return null; }\n";
    let (_tmp, root) = project(&[NESTED, &[("src/Tag.tsx", tag)]].concat());
    let base = root.join("node_modules/nest-kit/base.d.ts");
    let text = std::fs::read_to_string(&base).unwrap();
    std::fs::write(&base, UNREADABLE).unwrap();
    let session = WatchSession::new(options(&root));
    drop(session.initialize());
    assert_eq!(names(&session.component("Tag")), ["label", "own"]);
    std::fs::write(&base, text).unwrap();
    edit(&session, &root.join("src/Card.tsx"));

    let fresh = WatchSession::new(options(&root));
    drop(fresh.initialize());
    for name in ["Card", "Tag"] {
        assert_eq!(session.component(name), fresh.component(name), "{name}");
    }
    assert_eq!(names(&session.component("Tag")), ["label", "own", "tone"]);
}

/// The eager revision: `Card` imports `SoloProps` through a barrel that is only `export * from './props'`, and
/// `props.d.ts` could not be read. The resolver recorded the barrel, not the target it never read. The file reads
/// on disk and `Plain`, unrelated, is edited: the retry recovers the walk, and `Card`, which demanded it, is resolved
/// again and has the prop, as a fresh session does.
#[test]
fn an_eager_retry_of_a_pure_barrel_walk_resolves_its_demander_again() {
    let card = "import type { SoloProps } from 'solo-kit';\ninterface CardProps extends SoloProps { own: string }\n\
                export function Card(props: CardProps) { return null; }\n";
    let plain = "export function Plain(props: { title: string }) { return null; }\n";
    let (_tmp, root) = project(&[SOLO, &[("src/card/Card.tsx", card), ("src/plain/Plain.tsx", plain)]].concat());
    let props = root.join("node_modules/solo-kit/props.d.ts");
    let text = std::fs::read_to_string(&props).unwrap();
    std::fs::write(&props, UNREADABLE).unwrap();
    let session = WatchSession::new(options(&root));
    drop(session.initialize());
    assert_eq!(names(&session.component("Card")), ["own"]);
    std::fs::write(&props, text).unwrap();
    edit(&session, &root.join("src/plain/Plain.tsx"));

    let fresh = WatchSession::new(options(&root));
    drop(fresh.initialize());
    assert_eq!(names(&session.component("Card")), ["own", "tone"]);
    assert_eq!(session.component("Card"), fresh.component("Card"));
}

/// The eager revision: two components of one directory import `SoloProps`, whose file could not be read; the loader
/// keeps one demand for the directory, recorded under one of the two files. Each file in turn drops its import while
/// the file becomes readable: the other component, which still imports it, answers as a fresh session does.
#[test]
fn an_eager_edit_that_drops_a_shared_import_still_retries_it_for_the_other_file() {
    let importer = |name: &str| {
        format!(
            "import type {{ SoloProps }} from 'solo-kit';\ninterface {name}Props extends SoloProps {{ own: string }}\n\
             export function {name}(props: {name}Props) {{ return null; }}\n"
        )
    };
    let dropped = |name: &str| format!("export function {name}(props: {{ own: string }}) {{ return null; }}\n");
    for (edited, other) in [("First", "Second"), ("Second", "First")] {
        let (first, second) = (importer("First"), importer("Second"));
        let (_tmp, root) = project(
            &[SOLO, &[("src/pair/First.tsx", first.as_str()), ("src/pair/Second.tsx", second.as_str())]].concat(),
        );
        let props = root.join("node_modules/solo-kit/props.d.ts");
        let text = std::fs::read_to_string(&props).unwrap();
        std::fs::write(&props, UNREADABLE).unwrap();
        let session = WatchSession::new(options(&root));
        drop(session.initialize());
        std::fs::write(&props, text).unwrap();
        let file = root.join(format!("src/pair/{edited}.tsx"));
        std::fs::write(&file, dropped(edited)).unwrap();
        drop(session.update_file(&file));

        let fresh = WatchSession::new(options(&root));
        drop(fresh.initialize());
        assert_eq!(names(&session.component(other)), ["own", "tone"], "{edited} dropped its import");
        assert_eq!(session.component(other), fresh.component(other), "{edited} dropped its import");
    }
}

/// The eager revision: `First` and `Second`, one directory, both import `aug-kit`, whose React augmentation could not
/// be read. One drops its import while the other still needs it; then the other drops the last import just as the
/// augmentation becomes readable. No file imports it now: `Box` has no augmented attribute, as a fresh run over the
/// edited project, in either order.
#[test]
fn an_eager_walk_whose_last_importer_drops_it_is_not_followed_again() {
    let importer = |name: &str| format!("import * as Aug from 'aug-kit';\nexport const aug{name} = Aug;\n");
    for (first_drop, last_drop) in [("First", "Second"), ("Second", "First")] {
        let (one, two) = (importer("First"), importer("Second"));
        let (_tmp, root) = react_project(&[
            ("src/pair/First.ts", one.as_str()),
            ("src/pair/Second.ts", two.as_str()),
            ("src/box/Box.tsx", BOX),
        ]);
        let session = WatchSession::new(react_options(&root));
        drop(session.initialize());
        let first = root.join(format!("src/pair/{first_drop}.ts"));
        std::fs::write(&first, "export const none = 1;\n").unwrap();
        drop(session.update_file(&first));
        std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), AUGMENTATION).unwrap();
        let last = root.join(format!("src/pair/{last_drop}.ts"));
        std::fs::write(&last, "export const none = 2;\n").unwrap();
        drop(session.update_file(&last));

        let fresh = WatchSession::new(react_options(&root));
        drop(fresh.initialize());
        assert!(!names(&fresh.component("Box")).contains(&"tw".to_owned()));
        assert!(!names(&session.component("Box")).contains(&"tw".to_owned()), "{first_drop} dropped first");
        assert!(session.component("Box") == fresh.component("Box"), "{first_drop} dropped first");
    }
}

/// The eager revision, roots: `setup.ts`, the only importer of `aug-kit`, is deleted while the augmentation cannot be
/// read; the augmentation reads and an unrelated file is edited: no live demand leads to the walk, so it is not
/// followed and `Box` has no augmented attribute, as a fresh run. Restoring `setup.ts` makes the walk live again.
#[test]
fn an_eager_walk_whose_importer_was_deleted_is_not_followed_until_it_returns() {
    let setup = "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n";
    let plain = "export function Plain(props: { title: string }) { return null; }\n";
    let (_tmp, root) =
        react_project(&[("src/setup.ts", setup), ("src/box/Box.tsx", BOX), ("src/plain/Plain.tsx", plain)]);
    let session = WatchSession::new(react_options(&root));
    drop(session.initialize());
    std::fs::remove_file(root.join("src/setup.ts")).unwrap();
    drop(session.update_file(&root.join("src/setup.ts")));
    std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), AUGMENTATION).unwrap();
    edit(&session, &root.join("src/plain/Plain.tsx"));
    let fresh = WatchSession::new(react_options(&root));
    drop(fresh.initialize());
    assert!(!names(&session.component("Box")).contains(&"tw".to_owned()), "no live demand reaches the augmentation");
    assert!(session.component("Box") == fresh.component("Box"));

    std::fs::write(root.join("src/setup.ts"), setup).unwrap();
    drop(session.update_file(&root.join("src/setup.ts")));
    let fresh = WatchSession::new(react_options(&root));
    drop(fresh.initialize());
    assert!(names(&session.component("Box")).contains(&"tw".to_owned()), "restored, the walk is live again");
    assert!(session.component("Box") == fresh.component("Box"));
}

/// Reporting failed reads costs a healthy layer nothing: with no failed read in the chain, 1000 live demands are not
/// iterated. With one, they are.
#[test]
fn a_healthy_layer_reports_failed_reads_without_iterating_the_live_demands() {
    let (_tmp, root) = project(SOLO);
    let demand = |n: usize| Demand {
        from: root.join(format!("src/c{n}/Card.tsx")),
        specifier: "solo-kit".into(),
        name: "SoloProps".into(),
    };
    let roots: Vec<Demand> = (0..1000).map(demand).collect();
    let healthy = load(vec![demand(0)], &options(&root), None);
    let visited = std::cell::Cell::new(0);
    let counted = || roots.iter().inspect(|_| visited.set(visited.get() + 1));
    assert_eq!(healthy.read_warnings_reached(counted()), Vec::<Diagnostic>::new());
    assert_eq!(visited.get(), 0, "no live demand iterated");
    std::fs::write(root.join("node_modules/solo-kit/props.d.ts"), UNREADABLE).unwrap();
    let failed = load(vec![demand(0)], &PipelineOptions { use_cache: false, ..options(&root) }, None);
    assert_eq!(failed.read_warnings_reached(counted()).len(), 1);
    assert_eq!(visited.get(), 1000);
}

/// After 1000 failed reads recover, reporting them costs an eager edit nothing: the layer's current failures are kept
/// once per layer, empty, so no read of the history is looked up and no live demand is iterated.
#[test]
fn a_recovered_layer_reports_failed_reads_without_reading_the_history() {
    let reexports: String = (0..1000).map(|n| format!("export * from './m{n}';\n")).collect();
    let (_tmp, root) = project(&[
        ("node_modules/many-kit/package.json", r#"{ "name": "many-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
        ("node_modules/many-kit/index.d.ts", &reexports),
    ]);
    let module = |n: usize| root.join(format!("node_modules/many-kit/m{n}.d.ts"));
    (0..1000).for_each(|n| std::fs::write(module(n), UNREADABLE).unwrap());
    let options = PipelineOptions { use_cache: false, ..options(&root) };
    let demand = Demand { from: root.join("src/Card.tsx"), specifier: "many-kit".into(), name: "Absent".into() };
    let failed = Arc::new(load(vec![demand.clone()], &options, None));
    assert_eq!(failed.read_warnings().len(), 1000);
    (0..1000).for_each(|n| std::fs::write(module(n), format!("export interface M{n} {{}}\n")).unwrap());
    let recovered = load(failed.missing(std::slice::from_ref(&demand)), &options, Some(&failed));
    let roots: Vec<Demand> = vec![demand; 1000];
    let visited = std::cell::Cell::new(0);
    drop(recovered.unread_walks());
    WARNING_READS.with(|reads| reads.set(0));
    for _edit in 0..3 {
        let counted = roots.iter().inspect(|_| visited.set(visited.get() + 1));
        assert_eq!(recovered.read_warnings_reached(counted), Vec::<Diagnostic>::new());
    }
    assert_eq!(WARNING_READS.with(std::cell::Cell::get), 0, "no read looked up");
    assert_eq!(visited.get(), 0, "no live demand iterated");
}

/// An augmentation search past its file bound: the package's barrel re-exports `bad` (64 files once it reads) and
/// `mid`, which re-exports `late`, an augmentation of React. While `bad` cannot be read, the search reaches `late`
/// and `Box` has `tw`, provisionally; once `bad` reads, its files fill the bound first and a fresh run never reaches
/// `late`. The complete search replaces the partial one: after an unrelated edit `Box` equals a fresh run, without
/// `tw`.
#[test]
fn an_augmentation_a_partial_search_reached_past_its_bound_is_not_kept() {
    let fillers: String = (0..64).map(|n| format!("export * from './f{n}';\n")).collect();
    let mut files: Vec<(String, String)> = vec![
        (
            "node_modules/scan-kit/package.json".into(),
            r#"{ "name": "scan-kit", "version": "1.0.0", "types": "index.d.ts" }"#.into(),
        ),
        ("node_modules/scan-kit/index.d.ts".into(), "export * from './bad';\nexport * from './mid';\n".into()),
        ("node_modules/scan-kit/mid.d.ts".into(), "export * from './late';\n".into()),
        ("node_modules/scan-kit/late.d.ts".into(), AUGMENTATION.into()),
        ("src/setup.ts".into(), "import * as Scan from 'scan-kit';\nexport const scan = Scan;\n".into()),
        ("src/box/Box.tsx".into(), BOX.into()),
        ("src/plain/Plain.tsx".into(), "export function Plain(props: { title: string }) { return null; }\n".into()),
    ];
    files
        .extend((0..64).map(|n| (format!("node_modules/scan-kit/f{n}.d.ts"), format!("export interface F{n} {{}}\n"))));
    let files: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let (_tmp, root) = react_project(&files);
    let bad = root.join("node_modules/scan-kit/bad.d.ts");
    std::fs::write(&bad, UNREADABLE).unwrap();
    let session = WatchSession::new(react_options(&root));
    drop(session.initialize());
    assert!(names(&session.component("Box")).contains(&"tw".to_owned()), "what the partial search read, for now");
    std::fs::write(&bad, &fillers).unwrap();
    edit(&session, &root.join("src/plain/Plain.tsx"));

    let fresh = WatchSession::new(react_options(&root));
    drop(fresh.initialize());
    assert!(!names(&fresh.component("Box")).contains(&"tw".to_owned()), "a fresh run's search stops before `late`");
    assert!(!names(&session.component("Box")).contains(&"tw".to_owned()), "the partial search's `late` is not kept");
    assert!(session.component("Box") == fresh.component("Box"));
}

/// A search that cannot read one file keeps every augmentation it can read: the barrel re-exports `bad`, unreadable,
/// and `mid`, which re-exports `late`, an augmentation of React. `Box` has `tw` while `bad` cannot be read, and once
/// it reads (re-exporting nothing) and an edit follows, `Box` equals a fresh run, `tw` and nothing else added.
#[test]
fn an_augmentation_past_an_unreadable_file_is_kept() {
    let (_tmp, root) = react_project(&[
        ("node_modules/scan-kit/package.json", r#"{ "name": "scan-kit", "version": "1.0.0", "types": "index.d.ts" }"#),
        ("node_modules/scan-kit/index.d.ts", "export * from './bad';\nexport * from './mid';\n"),
        ("node_modules/scan-kit/mid.d.ts", "export * from './late';\n"),
        ("node_modules/scan-kit/late.d.ts", AUGMENTATION),
        ("src/setup.ts", "import * as Scan from 'scan-kit';\nexport const scan = Scan;\n"),
        ("src/box/Box.tsx", BOX),
        ("src/plain/Plain.tsx", "export function Plain(props: { title: string }) { return null; }\n"),
    ]);
    let bad = root.join("node_modules/scan-kit/bad.d.ts");
    std::fs::write(&bad, UNREADABLE).unwrap();
    let session = WatchSession::new(react_options(&root));
    drop(session.initialize());
    let while_unread = names(&session.component("Box"));
    assert!(while_unread.contains(&"tw".to_owned()), "`late` reads though `bad` does not");
    std::fs::write(&bad, "export interface Bad {}\n").unwrap();
    edit(&session, &root.join("src/plain/Plain.tsx"));

    let fresh = WatchSession::new(react_options(&root));
    drop(fresh.initialize());
    assert!(names(&fresh.component("Box")).contains(&"tw".to_owned()));
    assert_eq!(names(&session.component("Box")), names(&fresh.component("Box")));
    assert_eq!(names(&session.component("Box")), while_unread, "the complete search adds nothing stale");
    assert!(session.component("Box") == fresh.component("Box"));
}

/// A flatten copies the chain's files, an augmentation of React among them; a load that adds only an unrelated type
/// adds no React declaration, so nothing that read React's augmentations is dropped. The load that first read the
/// augmentation did add one.
#[test]
fn a_flatten_adds_no_react_declaration_it_only_copied() {
    let (_tmp, root) = react_project(&[("src/setup.ts", "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n")]);
    std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), AUGMENTATION).unwrap();
    write(
        &root,
        "node_modules/other-kit/package.json",
        r#"{ "name": "other-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
    );
    let other: String = (0..=MAX_CHAIN_DEPTH).map(|n| format!("export interface O{n} {{ o{n}: string }}\n")).collect();
    write(&root, "node_modules/other-kit/index.d.ts", &other);
    let options = react_options(&root);
    let setup = Demand { from: root.join("src/setup.ts"), specifier: "aug-kit".into(), name: "".into() };
    let other = |n: usize| Demand {
        from: root.join("src/setup.ts"),
        specifier: "other-kit".into(),
        name: format!("O{n}").into(),
    };
    let empty = Arc::new(load(vec![other(0)], &options, None));
    let mut layer = Arc::new(load(vec![setup], &options, Some(&empty)));
    assert!(layer.changes_react_declarations(&empty), "the load that read the augmentation added it");
    for n in 1..MAX_CHAIN_DEPTH {
        layer = Arc::new(load(vec![other(n)], &options, Some(&layer)));
    }
    let flattened = load(vec![other(MAX_CHAIN_DEPTH)], &options, Some(&layer));
    assert!(flattened.parent.is_none(), "the depth limit flattened the chain");
    assert!(!flattened.changes_react_declarations(&layer), "copied, not added");
}

/// The eager revision: `Card` sits behind a file that could not be read, which then reads on disk, and `Plain`,
/// unrelated, is deleted: a deletion makes the walk due as an edit does, and `Card` answers as a fresh session does.
#[test]
fn an_eager_deletion_follows_every_unread_walk_again() {
    let card = "import type { SoloProps } from 'solo-kit';\ninterface CardProps extends SoloProps { own: string }\n\
                export function Card(props: CardProps) { return null; }\n";
    let plain = "export function Plain(props: { title: string }) { return null; }\n";
    let (_tmp, root) = project(&[SOLO, &[("src/card/Card.tsx", card), ("src/plain/Plain.tsx", plain)]].concat());
    let props = root.join("node_modules/solo-kit/props.d.ts");
    let text = std::fs::read_to_string(&props).unwrap();
    std::fs::write(&props, UNREADABLE).unwrap();
    let session = WatchSession::new(options(&root));
    drop(session.initialize());
    std::fs::write(&props, text).unwrap();
    std::fs::remove_file(root.join("src/plain/Plain.tsx")).unwrap();
    drop(session.update_file(&root.join("src/plain/Plain.tsx")));

    let fresh = WatchSession::new(options(&root));
    drop(fresh.initialize());
    assert_eq!(names(&session.component("Card")), ["own", "tone"]);
    assert_eq!(session.component("Card"), fresh.component("Card"));
}

/// The eager revision: `Tag` reaches a walk that could not read a file, which then reads on disk. An edit to a file
/// that reaches no unread walk (`Plain`) follows every unread walk again, as an edit makes each one due in a lazy
/// session: `Tag` answers as a fresh session does.
#[test]
fn an_eager_edit_to_any_file_follows_every_unread_walk_again() {
    let tag = "import type { NestProps } from 'nest-kit';\ninterface TagProps extends NestProps { label: string }\n\
               export function Tag(props: TagProps) { return null; }\n";
    let plain = "export function Plain(props: { title: string }) { return null; }\n";
    let (_tmp, root) = project(&[NESTED, &[("src/tag/Tag.tsx", tag), ("src/plain/Plain.tsx", plain)]].concat());
    let base = root.join("node_modules/nest-kit/base.d.ts");
    let text = std::fs::read_to_string(&base).unwrap();
    std::fs::write(&base, UNREADABLE).unwrap();
    let session = WatchSession::new(options(&root));
    drop(session.initialize());
    assert_eq!(names(&session.component("Tag")), ["label", "own"]);
    std::fs::write(&base, text).unwrap();
    edit(&session, &root.join("src/plain/Plain.tsx"));

    let fresh = WatchSession::new(options(&root));
    drop(fresh.initialize());
    assert_eq!(names(&session.component("Tag")), ["label", "own", "tone"]);
    for name in ["Card", "Tag", "Plain"] {
        assert_eq!(session.component(name), fresh.component(name), "{name}");
    }
}

/// Two hundred directories of one resolution scope import a module whose augmentation search could not read two
/// files: one search, and its list of two files is held once for all 200 unread walks.
#[test]
fn the_demands_of_one_augmentation_search_share_its_list_of_unread_files() {
    let augments = "import 'react';\ndeclare module 'react' { interface CSSProperties { one?: string } }\nexport {};\n";
    let (_tmp, root) = project(&[
        ("node_modules/aug2-kit/package.json", r#"{"name":"aug2-kit","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/aug2-kit/index.d.ts", "export * from './one';\nexport * from './two';\n"),
        ("node_modules/aug2-kit/one.d.ts", augments),
        ("node_modules/aug2-kit/two.d.ts", augments),
    ]);
    std::fs::write(root.join("node_modules/aug2-kit/one.d.ts"), UNREADABLE).unwrap();
    std::fs::write(root.join("node_modules/aug2-kit/two.d.ts"), UNREADABLE).unwrap();
    let demands: Vec<Demand> = (0..200)
        .map(|n| Demand { from: root.join(format!("src/d{n}/App.tsx")), specifier: "aug2-kit".into(), name: "".into() })
        .collect();
    let layer = load(demands, &options(&root), None);
    let counts = layer.suspect_counts();
    assert_eq!(counts.len(), 200, "one unread walk per directory");
    assert!(counts.iter().all(|count| *count == 2), "{counts:?}");
    assert_eq!(layer.distinct_suspect_lists(), 1, "one list for the search");
}

/// A slow load: it starts at `start` and its read fails 29 s later. The walk is due again 30 s after the failure, not
/// after the load started: asked at `start + 31 s` it is not followed again, at `start + 59 s` it is.
#[test]
fn the_retry_counts_from_the_failure_not_from_the_start_of_its_load() {
    let (_tmp, root) = project(SOLO);
    std::fs::write(root.join("node_modules/solo-kit/props.d.ts"), UNREADABLE).unwrap();
    let demand = Demand { from: root.join("src/Card.tsx"), specifier: "solo-kit".into(), name: "SoloProps".into() };
    let start = std::time::Instant::now();
    let moment = Moment { at: start, edits: Edits::default() };
    let failure: Clock = Arc::new(move || start + std::time::Duration::from_secs(29));
    let stamp = Stamp { moment, load: Load(1) };
    let layer = load_timed(vec![demand.clone()], &options(&root), None, Retry::Due(moment), stamp, failure);
    let at = |secs| Retry::Due(Moment { at: start + std::time::Duration::from_secs(secs), edits: Edits::default() });
    assert_eq!(layer.missing_at(std::slice::from_ref(&demand), at(31)), Vec::<Demand>::new(), "not due yet");
    assert_eq!(layer.missing_at(std::slice::from_ref(&demand), at(59)), [demand]);
}

const AUGMENTATION: &str = "import type {} from 'react';\n\
                            declare module 'react' { interface HTMLAttributes<T> { tw?: string } }\n";
const BOX: &str = "import * as React from 'react';\n\
                   export const Box = (props: React.ComponentProps<'div'> & { own?: boolean }) => <div />;\n";

/// A project inside the crate, so React's own types resolve from the workspace's `node_modules`, with `aug-kit`'s React
/// augmentation unreadable.
fn react_project(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    write(
        &root,
        "node_modules/aug-kit/package.json",
        r#"{ "name": "aug-kit", "version": "1.0.0", "types": "index.d.ts" }"#,
    );
    std::fs::create_dir_all(root.join("node_modules/aug-kit")).unwrap();
    std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), UNREADABLE).unwrap();
    for (path, body) in files {
        write(&root, path, body);
    }
    (tmp, root)
}

fn react_options(root: &Utf8Path) -> PipelineOptions {
    PipelineOptions { html_attributes: crate::pipeline::HtmlAttributeMode::Full, ..options(root) }
}

/// The eager revision: only `setup.ts` imports `aug-kit`, whose React augmentation could not be read. It reads on
/// disk, and an edit to `Plain` follows the walk again and loads it: `Box`, untouched and never reaching `setup.ts`,
/// is resolved again and has the augmented attribute, as a fresh session does.
#[test]
fn an_eager_retry_that_loads_an_augmentation_resolves_every_reader_of_augmentations_again() {
    let plain = "export function Plain(props: { title: string }) { return null; }\n";
    let setup = "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n";
    let (_tmp, root) =
        react_project(&[("src/setup.ts", setup), ("src/box/Box.tsx", BOX), ("src/plain/Plain.tsx", plain)]);
    let session = WatchSession::new(react_options(&root));
    drop(session.initialize());
    assert!(!names(&session.component("Box")).contains(&"tw".to_owned()));
    std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), AUGMENTATION).unwrap();
    edit(&session, &root.join("src/plain/Plain.tsx"));

    let fresh = WatchSession::new(react_options(&root));
    drop(fresh.initialize());
    assert!(names(&fresh.component("Box")).contains(&"tw".to_owned()), "the fixture reaches the augmentation");
    assert!(session.component("Box") == fresh.component("Box"), "{:?}", names(&session.component("Box")));
}

/// The eager revision: `Box` is the only importer of `aug-kit`, whose augmentation could not be read. One edit removes
/// the import while the file becomes readable: the walk of the removed import is not followed again, so `Box` has no
/// augmented attribute, as a fresh session over the edited project.
#[test]
fn an_eager_edit_that_removes_the_only_import_does_not_follow_its_walk_again() {
    let importer = format!("import * as Aug from 'aug-kit';\nexport const aug = Aug;\n{BOX}");
    let (_tmp, root) = react_project(&[("src/box/Box.tsx", importer.as_str())]);
    let session = WatchSession::new(react_options(&root));
    drop(session.initialize());
    std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), AUGMENTATION).unwrap();
    std::fs::write(root.join("src/box/Box.tsx"), BOX).unwrap();
    drop(session.update_file(&root.join("src/box/Box.tsx")));

    let fresh = WatchSession::new(react_options(&root));
    drop(fresh.initialize());
    assert!(!names(&fresh.component("Box")).contains(&"tw".to_owned()), "nothing imports the augmentation now");
    assert!(session.component("Box") == fresh.component("Box"), "{:?}", names(&session.component("Box")));
}

/// Once a walk is followed again and reads, its newest mark is clean: the chain has no due walk, though the layer
/// below still holds the unread mark it superseded.
#[test]
fn a_walk_followed_again_that_reads_is_no_longer_due() {
    let (_tmp, root) = project(SOLO);
    let props = root.join("node_modules/solo-kit/props.d.ts");
    std::fs::write(&props, UNREADABLE).unwrap();
    let demand = Demand { from: root.join("src/Card.tsx"), specifier: "solo-kit".into(), name: "SoloProps".into() };
    let failed = Arc::new(load(vec![demand.clone()], &options(&root), None));
    assert_eq!(failed.due_walks(Retry::Always), std::slice::from_ref(&demand));
    std::fs::write(&props, "export interface SoloProps { tone: string }\n").unwrap();
    let recovered = load(failed.due_walks(Retry::Always), &options(&root), Some(&failed));
    assert_eq!(recovered.due_walks(Retry::Always), Vec::<Demand>::new());
}

/// A re-export stub an augmentation search only scans (it names neither React nor `"react"`) and could not read is
/// reported until a load reads it again, and not after: the scan's read clears it as a parse's does.
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

/// Two files of one resolution scope import a module for its own sake, and an augmentation it leads to takes a type
/// from a file that could not be read. Only the first demand to ask searches the module and loads the augmentation, so
/// the other reaches the unread walk through it: each demand leads to the same unread one, whichever searched.
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

/// An edit to `file`, which makes every walk past an unreadable file due: nothing is read again before the retry or an
/// edit ([`Retry`]).
fn edit(session: &WatchSession, file: &Utf8Path) {
    std::fs::write(file, std::fs::read_to_string(file).unwrap() + "// edited\n").unwrap();
    drop(session.update_file(file));
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
/// file is not kept, so once the file reads the next request has the inherited prop, the same as a fresh session.
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
    edit(&session, &root.join("src/Card.tsx"));
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
    let now = Moment { at: std::time::Instant::now(), edits: Edits::default() };
    let (due, stamp) = (Retry::Due(now), Stamp { moment: now, load: Load(1) });
    let first = Arc::new(load_at(vec![demand("NestProps")], &options, None, due, stamp));
    assert_eq!(first.unread(&[demand("NestProps")]), std::slice::from_ref(&base));

    let later = load_at(vec![demand("OtherProps")], &options, Some(&first), due, stamp);
    assert_eq!(later.reads, 0, "the unread demand is met, not followed again");
    assert_eq!(later.unread(&[demand("OtherProps")]), [base], "and it still leads to the unread walk");
    let always = load_at(vec![demand("OtherProps")], &options, Some(&first), Retry::Always, stamp);
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
    edit(&session, &root.join("src/tag/Tag.tsx"));
    let tag = session.component("Tag");

    let fresh = WatchSession::lazy(options(&root));
    drop(fresh.initialize());
    let fresh_tag = fresh.component("Tag");
    assert_eq!(names(&fresh_tag), ["label", "own", "tone"]);
    assert_eq!(tag, fresh_tag);
}

/// The model's package: a barrel of `export *` targets with a re-export stub, a named re-export of a declaration that
/// extends a type from a nested file, a chain of two more below it, and Radix's mapped `Primitive` behind the last
/// `export *`. A second barrel, `cyc`, walks a re-export cycle before `first` and finds `NameProps` in `second` while
/// `first`, which declares it too, cannot be read. Each declaration file can be made unreadable, `cyc/first` alone of
/// the second barrel's.
const MODEL_PACKAGE: &[(&str, &str)] = &[
    ("node_modules/mk/package.json", r#"{ "name": "mk", "version": "1.0.0", "types": "index.d.ts" }"#),
    (
        "node_modules/mk/index.d.ts",
        "export * from './a';\nexport * from './stub';\nexport { Nest } from './nest';\nexport * from './prim';\n",
    ),
    ("node_modules/mk/a.d.ts", "export interface AProps { a: string }\n"),
    ("node_modules/mk/stub.d.ts", "export * from './b';\n"),
    ("node_modules/mk/b.d.ts", "export interface BProps { b: string }\n"),
    (
        "node_modules/mk/cyc/index.d.ts",
        "export * from './loop';\nexport * from './first';\nexport * from './second';\n",
    ),
    ("node_modules/mk/cyc/loop.d.ts", "export * from './index';\n"),
    ("node_modules/mk/cyc/first.d.ts", "export interface NameProps { first: string }\n"),
    ("node_modules/mk/cyc/second.d.ts", "export interface NameProps { second: string }\n"),
    (
        "node_modules/mk/nest.d.ts",
        "import type { Deep } from './deep';\nexport interface Nest extends Deep { n: string }\n",
    ),
    (
        "node_modules/mk/deep.d.ts",
        "import type { Leaf } from './leaf';\nexport interface Deep extends Leaf { d: string }\n",
    ),
    ("node_modules/mk/leaf.d.ts", "export interface Leaf { l: string }\n"),
    (
        "node_modules/mk/prim.d.ts",
        "import * as React from 'react';\n\
         declare const NODES: readonly [\"a\", \"div\"];\n\
         type Primitives = { [E in (typeof NODES)[number]]: PrimitiveForwardRefComponent<E> };\n\
         type PrimitivePropsWithRef<E extends React.ElementType> = Own<E> & { asChild?: boolean };\n\
         interface Own<E> { tag?: E }\n\
         interface PrimitiveForwardRefComponent<E extends React.ElementType> \
         extends React.ForwardRefExoticComponent<PrimitivePropsWithRef<E>> {}\n\
         declare const Primitive: Primitives;\n\
         export { Primitive };\n",
    ),
];

/// A project file that imports nothing from the package: an edit to it is unrelated to every model component.
const MODEL_PLAIN: &str = "export function Plain(props: { title: string }) { return null; }\n";

/// The model's resolution config. A config edit rewrites it with the same meaning, so the oracles need no copy of it,
/// and the session takes its rebuild path.
const MODEL_TSCONFIG: &str = "{ \"compilerOptions\": {} }\n";

/// Each component, its source, and the package files its walk reads when every file is readable: its demands' export
/// walks, the declarations they extend, and the augmentation search of the module its file imports. `Direct` loads
/// `BProps` through the stub (`mk/stub`), and its walk of `Missing` (exported by nothing) resolves every target of
/// the barrel, so a later barrel walk of `BProps` past an unreadable `a.d.ts` adds only a partial mapping. `Q` reads
/// `typeof Primitive.div` past the same.
const MODEL_COMPONENTS: &[(&str, &str, &[&str])] = &[
    (
        "A",
        "import type { AProps } from 'mk';\ninterface AOwn extends AProps { own_a: string }\n\
         export function A(props: AOwn) { return null; }\n",
        &["index", "a", "stub", "b", "nest", "prim"],
    ),
    (
        "B",
        "import type { BProps } from 'mk';\ninterface BOwn extends BProps { own_b: string }\n\
         export function B(props: BOwn) { return null; }\n",
        &["index", "a", "stub", "b", "nest", "prim"],
    ),
    (
        "N",
        "import type { Nest } from 'mk';\ninterface NOwn extends Nest { own_n: string }\n\
         export function N(props: NOwn) { return null; }\n",
        &["index", "a", "stub", "b", "nest", "deep", "leaf", "prim"],
    ),
    (
        "All",
        "import type { AProps, BProps, Nest } from 'mk';\n\
         interface AllOwn extends AProps, BProps, Nest { own_all: string }\n\
         export function All(props: AllOwn) { return null; }\n",
        &["index", "a", "stub", "b", "nest", "deep", "leaf", "prim"],
    ),
    (
        "Direct",
        "import type { BProps } from 'mk/stub';\nimport type { Missing, Nest } from 'mk';\n\
         interface DirectOwn extends BProps, Nest { own_direct: string }\n\
         export function Direct(props: DirectOwn) { return null; }\n",
        &["index", "a", "stub", "b", "nest", "deep", "leaf", "prim"],
    ),
    (
        "Cyc",
        "import type { NameProps } from 'mk/cyc';\ninterface CycOwn extends NameProps { own_cyc: string }\n\
         export function Cyc(props: CycOwn) { return null; }\n",
        &["cyc/index", "cyc/loop", "cyc/first", "cyc/second"],
    ),
    (
        "Q",
        "import { Primitive } from 'mk';\nimport * as React from 'react';\n\
         export function Q(props: React.ComponentProps<typeof Primitive.div> & { own_q?: string }) { return null; }\n",
        &["index", "a", "stub", "b", "nest", "prim"],
    ),
];

fn model_source(body: &str, edit: usize) -> String {
    format!("{body}// edit {edit}\n")
}

fn model_project() -> (tempfile::TempDir, Utf8PathBuf) {
    let (tmp, root) = project(MODEL_PACKAGE);
    write(&root, "tsconfig.json", MODEL_TSCONFIG);
    write(&root, "src/plain/Plain.tsx", MODEL_PLAIN);
    for (name, body, _) in MODEL_COMPONENTS {
        write(&root, &format!("src/{name}.tsx"), &model_source(body, 0));
    }
    (tmp, root)
}

/// Exactly `unreadable` of the package's declaration files unreadable (or, with `empty`, declaring nothing), the rest
/// as written.
fn set_unreadable(root: &Utf8Path, unreadable: &std::collections::BTreeSet<&str>, empty: bool) {
    for (path, body) in MODEL_PACKAGE {
        let Some(name) = path.strip_prefix("node_modules/mk/").and_then(|name| name.strip_suffix(".d.ts")) else {
            continue;
        };
        match (unreadable.contains(name), empty) {
            (true, false) => std::fs::write(root.join(path), UNREADABLE).unwrap(),
            (true, true) => std::fs::write(root.join(path), "export {};\n").unwrap(),
            (false, _) => std::fs::write(root.join(path), body).unwrap(),
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

fn prop_names(answer: &ComponentAnswer) -> Vec<String> {
    let mut names: Vec<String> = answer.entry.props.keys().cloned().collect();
    names.sort();
    names
}

/// The model's two oracles, each a fresh session with the cache off over a project of its own, memoized by component
/// and file set: the answer with exactly those files unreadable, and the props with those files declaring nothing
/// (what the readable files give, decided without any code that handles a failed read).
struct ModelOracle {
    _tmp: tempfile::TempDir,
    root: Utf8PathBuf,
    answers: FxHashMap<(String, Vec<String>, bool), String>,
}

impl ModelOracle {
    fn new() -> Self {
        let (tmp, root) = model_project();
        ModelOracle { _tmp: tmp, root, answers: FxHashMap::default() }
    }

    fn ask(&mut self, component: &str, files: &[String], empty: bool) -> String {
        let root = self.root.clone();
        self.answers
            .entry((component.to_owned(), files.to_vec(), empty))
            .or_insert_with(|| {
                set_unreadable(&root, &files.iter().map(String::as_str).collect(), empty);
                let fresh = WatchSession::lazy(PipelineOptions { use_cache: false, ..options(&root) });
                drop(fresh.initialize());
                let answer = fresh.component(component).unwrap();
                match empty {
                    true => format!("{:?}", prop_names(&answer)),
                    false => comparable(&root, &answer),
                }
            })
            .clone()
    }
}

/// One model session: its project, what is unreadable in it, and every invariant that failed, by rule.
struct ModelRun<'o> {
    oracle: &'o mut ModelOracle,
    _tmp: tempfile::TempDir,
    root: Utf8PathBuf,
    session: WatchSession,
    label: String,
    log: Vec<String>,
    failures: &'o mut Vec<String>,
}

impl ModelRun<'_> {
    fn fail(&mut self, rule: &str, detail: String) {
        self.failures.push(format!("[{rule}] {}: {detail}\n  {}", self.label, self.log.join("\n  ")));
    }

    /// A request, checked against both oracles: the answer is a fresh session's over exactly the files it reports
    /// unreadable, and its props are what the readable files give.
    fn request(&mut self, name: &str) -> Vec<String> {
        let answer = self.session.component(name).unwrap();
        self.session.wait_for_dependency_load();
        self.log.push(format!("request {name}"));
        let warned = warned_names(&self.root, &answer);
        let mut once = warned.clone();
        once.dedup();
        if warned != once {
            self.fail("reported once", format!("{warned:?}"));
        }
        let expected = self.oracle.ask(name, &warned, false);
        if comparable(&self.root, &answer) != expected {
            self.fail("fresh session", format!("{name} warned {warned:?}"));
        }
        // A re-export cycle's answer reaches the resolver only as a partial mapping: with the unreadable target
        // declaring nothing, a fresh run maps nothing, so that oracle does not say what `Cyc`'s readable files give.
        let readable = self.oracle.ask(name, &warned, true);
        if name != "Cyc" && format!("{:?}", prop_names(&answer)) != readable {
            self.fail("readable files", format!("{name} {:?}, expected {readable}", prop_names(&answer)));
        }
        let again = self.session.component(name).unwrap();
        let again_warned = warned_names(&self.root, &again);
        if comparable(&self.root, &again) != self.oracle.ask(name, &again_warned, false) {
            self.fail("asked again", name.to_owned());
        }
        warned
    }

    /// A project edit, or a config edit when `file` is the tsconfig: every component whose answer changes is in the
    /// delta, a provisional one included.
    fn edit(&mut self, file: &Utf8Path, body: &str) {
        let answers = |run: &Self| -> Vec<String> {
            MODEL_COMPONENTS
                .iter()
                .map(|(name, _, _)| comparable(&run.root, &run.session.component(name).unwrap()))
                .collect()
        };
        let before = answers(self);
        std::fs::write(file, body).unwrap();
        let update = self.session.update_file(file);
        self.log.push(format!("edit {}", file.file_name().unwrap_or_default()));
        let after = answers(self);
        for ((component, _, _), (before, after)) in MODEL_COMPONENTS.iter().zip(before.iter().zip(&after)) {
            if before != after && !update.updated_keys.iter().any(|key| key == component) {
                self.fail("delta", format!("{component} changed, not in the delta"));
            }
        }
    }
}

/// A change the model's eager block makes to its project, by name: it returns the file it changed.
type ModelChange = (&'static str, fn(&Utf8Path) -> Utf8PathBuf);

/// A model-based test of what a lazy session holds while package files go unreadable and readable again: seeded
/// sequences of files made unreadable (bytes that are not UTF-8) or readable, requests, project edits and config edits
/// (the rebuild path), and every ordered pair of requests under each single unreadable file followed by a config edit
/// once it reads. After every request:
///
/// 1. the answer equals a fresh session's (cache off) with exactly the files the session reports unreadable made
///    unreadable, and its props equal a fresh session's with those files declaring nothing: what the readable files
///    give, through every resolver path (a direct import, a barrel, a qualified `typeof X.y`);
/// 2. each file is reported once, and a request made while every file reads, after an edit since a file last changed,
///    reports none of the files its own walk reads: the edit made every walk past an unreadable file due, and no
///    provisional answer outlives it;
/// 3. after every edit, project or config, every component whose answer changed is in the delta;
/// 4. once every file reads, every component equals the cold answer, with no warning, in this session and in the next
///    one over the same cache: nothing from a degraded answer was memoized or persisted.
///
/// Every failure is collected with its rule, seed or pair, and the steps so far.
#[test]
fn a_session_over_files_that_go_unreadable_answers_as_a_fresh_one_over_what_it_could_read() {
    let names: Vec<&str> = MODEL_PACKAGE
        .iter()
        .filter_map(|(path, _)| path.strip_prefix("node_modules/mk/")?.strip_suffix(".d.ts"))
        .filter(|name| !name.starts_with("cyc/") || *name == "cyc/first")
        .collect();
    let mut oracle = ModelOracle::new();
    let mut failures: Vec<String> = Vec::new();
    for seed in [1_u64, 2, 3, 5, 8, 13, 21, 34] {
        let (tmp, root) = model_project();
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = |bound: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let mut unreadable: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let session = WatchSession::lazy(options(&root));
        let mut log = Vec::new();
        if next(2) == 0 {
            unreadable.insert(names[next(names.len())]);
            set_unreadable(&root, &unreadable, false);
            log.push(format!("unreadable before start: {unreadable:?}"));
        }
        drop(session.initialize());
        let label = format!("seed {seed}");
        let mut run = ModelRun { oracle: &mut oracle, _tmp: tmp, root, session, label, log, failures: &mut failures };
        let mut edited_since_change = true;
        for step in 0..40 {
            match next(5) {
                0 => {
                    let name = names[next(names.len())];
                    if !unreadable.remove(name) {
                        unreadable.insert(name);
                    }
                    set_unreadable(&run.root, &unreadable, false);
                    edited_since_change = false;
                    run.log.push(format!("unreadable: {unreadable:?}"));
                }
                1 => {
                    let (name, body, _) = MODEL_COMPONENTS[next(MODEL_COMPONENTS.len())];
                    let file = run.root.join(format!("src/{name}.tsx"));
                    run.edit(&file, &model_source(body, step + 1));
                    edited_since_change = true;
                }
                2 => {
                    let file = run.root.join("tsconfig.json");
                    run.edit(&file, &format!("{MODEL_TSCONFIG}{}", "\n".repeat(step + 1)));
                    edited_since_change = true;
                }
                _ => {
                    let (name, _, reads) = MODEL_COMPONENTS[next(MODEL_COMPONENTS.len())];
                    let all_read = unreadable.is_empty() && edited_since_change;
                    let warned = run.request(name);
                    let reread: Vec<&String> = warned.iter().filter(|file| reads.contains(&file.as_str())).collect();
                    if all_read && !reread.is_empty() {
                        run.fail("read again", format!("{name} {reread:?}"));
                    }
                }
            }
        }
        unreadable.clear();
        set_unreadable(&run.root, &unreadable, false);
        let (name, body, _) = MODEL_COMPONENTS[0];
        let file = run.root.join(format!("src/{name}.tsx"));
        run.edit(&file, &model_source(body, 99));
        for (name, _, _) in MODEL_COMPONENTS {
            drop(run.session.component(name));
            run.session.wait_for_dependency_load();
        }
        let next_session = WatchSession::lazy(options(&run.root));
        drop(next_session.initialize());
        for (name, _, _) in MODEL_COMPONENTS {
            let cold = run.oracle.ask(name, &[], false);
            let answers =
                [("this session", run.session.component(name)), ("the next session", next_session.component(name))];
            for (which, answer) in answers {
                let answer = answer.unwrap();
                if !warned_names(&run.root, &answer).is_empty() || comparable(&run.root, &answer) != cold {
                    run.fail("all readable", format!("{which}: {name}"));
                }
            }
        }
    }
    // Every ordered pair of requests under each single unreadable file, then the file reads again and a config edit
    // rebuilds the session: an event order the seeds need not hit.
    for file in &names {
        for (first, _, _) in MODEL_COMPONENTS {
            for (second, _, _) in MODEL_COMPONENTS.iter().filter(|(second, _, _)| second != first) {
                let (tmp, root) = model_project();
                let unreadable: std::collections::BTreeSet<&str> = [*file].into();
                set_unreadable(&root, &unreadable, false);
                let session = WatchSession::lazy(options(&root));
                drop(session.initialize());
                let label = format!("pair {file}: {first}, {second}");
                let mut run = ModelRun {
                    oracle: &mut oracle,
                    _tmp: tmp,
                    root,
                    session,
                    label,
                    log: Vec::new(),
                    failures: &mut failures,
                };
                let degraded = !run.request(first).is_empty();
                // A pair the unreadable file degraded neither of has nothing for the edits below to recover.
                if !degraded && run.request(second).is_empty() {
                    continue;
                }
                set_unreadable(&run.root, &std::collections::BTreeSet::new(), false);
                run.log.push(format!("{file} reads"));
                // A project edit makes the walk due: the retry's answer for a degraded `first` is a fresh session's.
                if degraded {
                    let (_, body, _) = MODEL_COMPONENTS.iter().find(|(name, _, _)| name == first).unwrap();
                    run.edit(&run.root.join(format!("src/{first}.tsx")), &model_source(body, 1));
                    run.request(first);
                }
                let tsconfig = run.root.join("tsconfig.json");
                run.edit(&tsconfig, &format!("{MODEL_TSCONFIG}\n"));
            }
        }
    }
    // The eager revision, under each single unreadable file: once it reads, one change follows every due walk again,
    // and every component answers as a fresh run over the changed project does (the footprint rule: a retry of any
    // walk a component's resolution consulted, through a barrel whose target it never read included, resolves it
    // again). The changes: an edit to `Plain`, which reaches no package; its deletion; and `A` or `All` dropping its
    // import of `AProps`, which the other still makes from the same directory (one demand, recorded under either).
    let changes: [ModelChange; 4] = [
        ("edit Plain", |root| {
            let plain = root.join("src/plain/Plain.tsx");
            std::fs::write(&plain, format!("{MODEL_PLAIN}// edited\n")).unwrap();
            plain
        }),
        ("delete Plain", |root| {
            let plain = root.join("src/plain/Plain.tsx");
            std::fs::remove_file(&plain).unwrap();
            plain
        }),
        ("A drops its import", |root| {
            let file = root.join("src/A.tsx");
            std::fs::write(&file, "export function A(props: { own_a: string }) { return null; }\n").unwrap();
            file
        }),
        ("All drops its import", |root| {
            let file = root.join("src/All.tsx");
            std::fs::write(&file, "export function All(props: { own_all: string }) { return null; }\n").unwrap();
            file
        }),
    ];
    // Two-step changes: one made while the file cannot be read, then one after it reads. `Cyc`, the only importer of
    // `mk/cyc`, drops its import and restores it (the restored demand starts a fresh walk); or drops it for good (its
    // failed read is no longer reported, and nothing retries it) and an unrelated edit follows every due walk. `A` and
    // `All` both drop `AProps`, whose file other walks still read.
    let drop_cyc: fn(&Utf8Path) -> Utf8PathBuf = |root| {
        let file = root.join("src/Cyc.tsx");
        std::fs::write(&file, "export function Cyc(props: { own_cyc: string }) { return null; }\n").unwrap();
        file
    };
    let restore_cyc: fn(&Utf8Path) -> Utf8PathBuf = |root| {
        let file = root.join("src/Cyc.tsx");
        let (_, body, _) = MODEL_COMPONENTS.iter().find(|(name, _, _)| *name == "Cyc").unwrap();
        std::fs::write(&file, model_source(body, 0)).unwrap();
        file
    };
    let two_steps: [(&str, Vec<ModelChange>, Vec<ModelChange>); 3] = [
        ("Cyc drops, then restores its import", vec![("drop", drop_cyc)], vec![("restore", restore_cyc)]),
        ("Cyc, the only importer, drops its import", vec![("drop", drop_cyc)], vec![changes[0]]),
        ("A and All drop their import", vec![("drop A", changes[2].1), ("drop All", changes[3].1)], vec![changes[0]]),
    ];
    let one_step = changes.iter().map(|(name, apply)| (*name, Vec::new(), vec![(*name, *apply)]));
    let all: Vec<(&str, Vec<ModelChange>, Vec<ModelChange>)> = one_step.chain(two_steps).collect();
    for file in &names {
        for (change, before, after) in &all {
            let (_tmp, root) = model_project();
            set_unreadable(&root, &[*file].into(), false);
            let session = WatchSession::new(options(&root));
            drop(session.initialize());
            for (_, apply) in before {
                drop(session.update_file(&apply(&root)));
            }
            set_unreadable(&root, &std::collections::BTreeSet::new(), false);
            for (_, apply) in after {
                drop(session.update_file(&apply(&root)));
            }
            let fresh = WatchSession::new(PipelineOptions { use_cache: false, ..options(&root) });
            let fresh_output = fresh.initialize();
            let io = |output: &crate::types::ExtractionOutput| -> Vec<Option<String>> {
                let io = |d: &&crate::types::Diagnostic| d.code == DiagnosticCode::IoError;
                output.diagnostics.iter().filter(io).map(|d| d.file.clone()).collect()
            };
            if io(&session.current_output().unwrap()) != io(&fresh_output) {
                failures.push(format!("[eager warnings] {file}, {change}: a failed read is reported"));
            }
            for (name, _, _) in MODEL_COMPONENTS {
                let answer = session.component(name).unwrap();
                let cold = comparable(&root, &fresh.component(name).unwrap());
                if !warned_names(&root, &answer).is_empty() || comparable(&root, &answer) != cold {
                    failures.push(format!("[eager retry] {file}, {change}: {name} is not a fresh run's"));
                }
            }
        }
    }
    // Roots: a project with React's types, where `setup.ts` alone imports `aug-kit`, whose augmentation cannot be
    // read. Each change removes the importer (deleted, or its import dropped) while the file cannot be read; the file
    // then reads and an unrelated edit follows. No live demand reaches the walk: `Box` answers as a fresh run, without
    // the augmented attribute. Restoring the importer makes the walk live again.
    for (change, gone) in [("setup.ts deleted", true), ("setup.ts drops its import", false)] {
        let setup = "import * as Aug from 'aug-kit';\nexport const aug = Aug;\n";
        let files = [("src/setup.ts", setup), ("src/box/Box.tsx", BOX), ("src/plain/Plain.tsx", MODEL_PLAIN)];
        let (_tmp, root) = react_project(&files);
        let session = WatchSession::new(react_options(&root));
        drop(session.initialize());
        match gone {
            true => std::fs::remove_file(root.join("src/setup.ts")).unwrap(),
            false => std::fs::write(root.join("src/setup.ts"), "export const aug = 1;\n").unwrap(),
        }
        drop(session.update_file(&root.join("src/setup.ts")));
        std::fs::write(root.join("node_modules/aug-kit/index.d.ts"), AUGMENTATION).unwrap();
        edit(&session, &root.join("src/plain/Plain.tsx"));
        let fresh = WatchSession::new(react_options(&root));
        drop(fresh.initialize());
        if session.component("Box") != fresh.component("Box") {
            failures.push(format!("[eager roots] {change}: Box is not a fresh run's"));
        }
        // The importer returns: the walk is live again, followed, and `Box` has the attribute, as a fresh run.
        std::fs::write(root.join("src/setup.ts"), setup).unwrap();
        drop(session.update_file(&root.join("src/setup.ts")));
        let fresh = WatchSession::new(react_options(&root));
        drop(fresh.initialize());
        let restored = session.component("Box");
        if restored != fresh.component("Box")
            || !restored.as_ref().is_some_and(|box_| box_.entry.props.contains_key("tw"))
        {
            failures.push(format!("[eager roots] {change}, then restored: Box lacks the augmentation a fresh run has"));
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
