#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The persisted answer to which files a module's re-exports lead to that augment React.

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::pipeline::PipelineOptions;

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
        ..Default::default()
    }
}

/// The demand a file that imports `specifier` for its own sake makes.
fn import_demand(root: &Utf8Path, specifier: &str) -> Vec<Demand> {
    vec![Demand { from: root.join("src/App.tsx"), specifier: specifier.into(), name: CompactString::default() }]
}

fn module(root: &Utf8Path, path: &str) -> PathKey {
    PathKey::from(root.join(path).as_path())
}

fn index_file(root: &Utf8Path) -> IndexFile {
    let path = root
        .join("cache")
        .read_dir_utf8()
        .unwrap()
        .map(|entry| entry.unwrap().path().to_owned())
        .find(|path| path.file_name().is_some_and(|name| name.starts_with("dependency-index-")))
        .expect("the run writes the index");
    rmp_serde::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

const PLAIN: &[(&str, &str)] = &[
    ("src/App.tsx", "import 'plain';\n"),
    ("node_modules/plain/package.json", r#"{"name":"plain","version":"1.0.0","types":"index.d.ts"}"#),
    ("node_modules/plain/index.d.ts", "export * from './more.js';\n"),
    ("node_modules/plain/more.d.ts", "export interface More { a?: string }\n"),
];

#[test]
fn a_module_that_nothing_augments_is_persisted_so_the_next_run_does_not_scan_it_again() {
    let (_tmp, root) = project(PLAIN);
    let plain = module(&root, "node_modules/plain/index.d.ts");

    let cold = load(import_demand(&root, "plain"), &options(&root), None);
    assert_eq!(cold.augmenters_of(&plain), Some(&Vec::new()));
    assert!(cold.parsed_data(root.join("node_modules/plain/more.d.ts").as_str()).is_some(), "the cold run scanned it");

    let rows: Vec<AugmenterRow> =
        index_file(&root).packages.into_iter().flat_map(|package| package.augmenters).collect();
    assert_eq!(rows, [(std::borrow::Borrow::<str>::borrow(&plain).to_owned(), Vec::new(), Vec::new())]);

    let warm = load(import_demand(&root, "plain"), &options(&root), None);
    assert_eq!(warm.augmenters_of(&plain), Some(&Vec::new()));
    assert!(warm.parsed.is_empty(), "the warm run answered from the index and parsed nothing");
}

const REEXPORTING: &[(&str, &str)] = &[
    ("src/App.tsx", "import 'reexp';\n"),
    ("node_modules/reexp/package.json", r#"{"name":"reexp","version":"1.0.0","types":"index.d.ts"}"#),
    ("node_modules/reexp/index.d.ts", "export * from 'inner';\n"),
    ("node_modules/inner/package.json", r#"{"name":"inner","version":"1.0.0","types":"index.d.ts"}"#),
    (
        "node_modules/inner/index.d.ts",
        "export interface Inner {}\ndeclare module 'react' { interface HTMLAttributes<T> { tw?: string } }\n",
    ),
];

#[test]
fn upgrading_a_package_a_module_re_exports_from_voids_the_persisted_answer() {
    let (_tmp, root) = project(REEXPORTING);
    let reexp = module(&root, "node_modules/reexp/index.d.ts");
    let inner = root.join("node_modules/inner/index.d.ts");

    let first = load(import_demand(&root, "reexp"), &options(&root), None);
    assert_eq!(first.augmenters_of(&reexp), Some(&vec![inner.clone()]));
    let rows: Vec<AugmenterRow> =
        index_file(&root).packages.into_iter().flat_map(|package| package.augmenters).collect();
    assert_eq!(rows.len(), 1);
    let crossed: Vec<&str> = rows[0].2.iter().map(|(dir, _, _)| dir.as_str()).collect();
    assert_eq!(crossed, [root.join("node_modules/inner").as_str()], "the row names the other package it read");

    // Unchanged, the warm run trusts the row.
    let warm = load(import_demand(&root, "reexp"), &options(&root), None);
    assert_eq!(warm.augmenters_of(&reexp), Some(&vec![inner.clone()]));

    // `inner` is upgraded to a version that no longer augments React; `reexp` itself is untouched.
    write(&root, "node_modules/inner/index.d.ts", "export interface Inner {}\n");
    write(&root, "node_modules/inner/package.json", r#"{"name":"inner","version":"1.0.10","types":"index.d.ts"}"#);
    let upgraded = load(import_demand(&root, "reexp"), &options(&root), None);
    assert_eq!(upgraded.augmenters_of(&reexp), Some(&Vec::new()));
}

/// With no re-exports the scan resolves nothing, so the empty answer is the only thing the run learned.
#[test]
fn an_empty_answer_alone_is_enough_to_write_the_index() {
    let (_tmp, root) = project(&[
        ("src/App.tsx", "import 'bare';\n"),
        ("node_modules/bare/package.json", r#"{"name":"bare","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/bare/index.d.ts", "export interface Bare { a?: string }\n"),
    ]);
    let bare = module(&root, "node_modules/bare/index.d.ts");
    drop(load(import_demand(&root, "bare"), &options(&root), None));
    let rows: Vec<AugmenterRow> =
        index_file(&root).packages.into_iter().flat_map(|package| package.augmenters).collect();
    assert_eq!(rows, [(std::borrow::Borrow::<str>::borrow(&bare).to_owned(), Vec::new(), Vec::new())]);

    let warm = load(import_demand(&root, "bare"), &options(&root), None);
    assert!(warm.parsed.is_empty(), "the warm run parsed nothing");
}
