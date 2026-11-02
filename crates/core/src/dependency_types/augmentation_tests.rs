#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The persisted answer to which files a module's re-exports lead to that augment React.

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::types::options::PipelineOptions;

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
    assert!(
        cold.parsed_data(root.join("node_modules/plain/more.d.ts").as_str()).is_none(),
        "a file the search only passed through is scanned for its re-exports and not kept"
    );

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

const HOPS: &[(&str, &str)] = &[
    ("src/App.tsx", "import 'hops';\n"),
    ("node_modules/hops/package.json", r#"{"name":"hops","version":"1.0.0","types":"index.d.ts"}"#),
    (
        "node_modules/hops/index.d.ts",
        "import { Mid } from './imported.js';\nexport { Mid };\nexport * from './plain.js';\nexport { Named } from './named.js';\n",
    ),
    ("node_modules/hops/plain.d.ts", "export interface Plain { a?: string }\n"),
    ("node_modules/hops/named.d.ts", "export * as nested from './deep.js';\nexport interface Named {}\n"),
    (
        "node_modules/hops/deep.d.ts",
        "export interface Deep {}\ndeclare module 'react' { interface HTMLAttributes<T> { deep?: string } }\n",
    ),
    ("node_modules/hops/imported.d.ts", "export interface Mid {}\nexport * from './last.js';\n"),
    ("node_modules/hops/last.d.ts", "declare namespace React { interface HTMLAttributes<T> { last?: string } }\n"),
];

#[test]
fn an_augmenting_file_is_found_through_files_scanned_without_a_full_parse() {
    let (_tmp, root) = project(HOPS);
    let index = module(&root, "node_modules/hops/index.d.ts");
    let layer = load(import_demand(&root, "hops"), &options(&root), None);

    let mut expected = vec![root.join("node_modules/hops/deep.d.ts"), root.join("node_modules/hops/last.d.ts")];
    expected.sort();
    assert_eq!(layer.augmenters_of(&index), Some(&expected));
    for passed_through in ["plain", "named", "imported"] {
        let file = root.join(format!("node_modules/hops/{passed_through}.d.ts"));
        assert!(layer.parsed_data(file.as_str()).is_none(), "{passed_through} is scanned, not kept");
    }
}

#[test]
fn directories_with_one_resolution_scope_resolve_a_module_import_once() {
    let (_tmp, root) = project(&[
        ("src/a/App.tsx", ""),
        ("src/b/App.tsx", ""),
        ("src/c/App.tsx", ""),
        ("src/c/package.json", r#"{"name":"c"}"#),
        ("node_modules/plain/package.json", r#"{"name":"plain","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/plain/index.d.ts", "export interface Plain {}\n"),
    ]);
    let demand =
        |from: &str| Demand { from: root.join(from), specifier: "plain".into(), name: CompactString::default() };
    let demands = vec![demand("src/a/App.tsx"), demand("src/b/App.tsx"), demand("src/c/App.tsx")];
    let layer = load(demands.clone(), &options(&root), None);

    assert!(layer.covers(&demands), "every directory's demand is recorded as followed");
    let asked: Vec<&str> = layer
        .resolutions
        .keys()
        .filter(|(_, specifier)| specifier.as_ref() == "plain")
        .map(|(dir, _)| dir.as_ref())
        .collect();
    assert_eq!(asked.len(), 2, "src/a and src/b share the root's scope and src/c has its own package.json: {asked:?}");
    assert!(asked.contains(&root.join("src/c").as_str()));
}

#[test]
fn only_a_declaration_of_the_react_namespace_makes_a_file_worth_a_full_parse() {
    for source in [
        "declare module 'react' { interface A {} }",
        "declare module \"react\" {}",
        "declare\n  module\n  'react' {}",
        "declare namespace React { interface A {} }",
        "declare global { namespace React { interface A {} } }",
        "declare module /* c */ 'react' {}",
        "declare namespace/**/React {}",
    ] {
        assert!(may_declare_into_react(source), "{source}");
    }
    for source in [
        "import * as React from 'react';\nexport declare const A: React.FC;",
        "import type { FC } from \"react\";",
        "export interface A { react: string }",
        "",
    ] {
        assert!(!may_declare_into_react(source), "{source}");
    }
}

#[test]
fn only_an_export_of_a_list_or_a_star_makes_a_file_worth_scanning_for_re_exports() {
    for source in [
        "export * from './a';",
        "export * as ns from './a';",
        "export { a } from './a';",
        "export {a};",
        "export type { A } from './a';",
        "export type * from './a';",
        "export\n  {\n  a\n}",
        "export /* c */ { a };",
        "import A from './a';\nexport default A;",
    ] {
        assert!(may_re_export(source), "{source}");
    }
    for source in [
        "export interface A {}",
        "export declare function f(): void;",
        "export type A = string;",
        "export = React;",
        "",
    ] {
        assert!(!may_re_export(source), "{source}");
    }
}
