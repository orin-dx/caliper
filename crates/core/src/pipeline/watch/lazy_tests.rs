//! The lazy session (ADR 0016): each answer is a batch extraction's, a request resolves only what it needs, and an edit
//! drops exactly the memoized components it can change. `tests/lazy_parity.rs` holds the parity check over every
//! fixture and the large corpora.

use camino::{Utf8Path, Utf8PathBuf};

use super::WatchSession;
use crate::pipeline::{extract, ComponentAnswer, PipelineOptions};
use crate::types::Diagnostic;

fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    write(&root, files);
    (tmp, root)
}

fn write(root: &Utf8Path, files: &[(&str, &str)]) {
    for (path, body) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

fn options(root: &Utf8Path, load_dependencies: bool) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.join("src")],
        cache_dir: Some(root.join("cache")),
        load_dependencies,
        ..Default::default()
    }
}

/// The session is shared across threads by hosts that link the library.
#[test]
fn a_session_and_its_answers_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WatchSession>();
    assert_send_sync::<ComponentAnswer>();
}

#[test]
fn a_component_is_found_by_file_and_name_as_by_key() {
    let item = "export function Item(props: { a: string }) { return null; }\n";
    let (_tmp, root) = project(&[("src/a/Item.tsx", item), ("src/b/Item.tsx", item), ("src/Card.tsx", CARD)]);
    for lazy in [false, true] {
        let options = options(&root, false);
        let session = if lazy { WatchSession::lazy(options) } else { WatchSession::new(options) };
        drop(session.initialize());

        let second = root.join("src/b/Item.tsx");
        let key = format!("Item ({second})");
        assert_eq!(session.component_in(&second, "Item"), session.component(&key), "lazy: {lazy}");
        assert_eq!(session.component_in(&second, "Item").map(|a| a.key), Some(key));
        assert_eq!(session.component_in(&root.join("src/a/Item.tsx"), "Item").map(|a| a.key), Some("Item".to_owned()));
        assert_eq!(session.component_in(&root.join("src/Card.tsx"), "Item"), None);
        assert_eq!(session.component_in(&root.join("src/Missing.tsx"), "Card"), None);
        assert_eq!(session.docgen_info("Card").map(|d| d["displayName"].clone()), Some("Card".into()));
    }
}

const TYPES: &str = "export interface BaseProps { size?: 'sm' | 'lg' }\n";
const BUTTON: &str = "import type { BaseProps } from './types';\n\
    export function Button(props: BaseProps & { label: string }) { return null; }\n";
const CARD: &str = "export function Card(props: { title: string }) { return null; }\n";

fn prop_names(answer: &Option<ComponentAnswer>) -> Vec<&str> {
    answer.as_ref().map(|a| a.entry.props.keys().map(String::as_str).collect()).unwrap_or_default()
}

/// Every key a batch run publishes answers with the batch entry and the diagnostics batch attaches to the key.
fn assert_matches_batch(session: &WatchSession, options: &PipelineOptions) {
    let batch = extract(options);
    let keys: Vec<String> = batch.components.keys().cloned().collect();
    assert_eq!(session.component_keys(), keys);
    for (key, answer) in keys.iter().zip(session.components(&keys)) {
        let diagnostics: Vec<Diagnostic> = batch
            .diagnostics
            .iter()
            .filter(|d| d.affects.contains(key))
            .map(|d| Diagnostic { affects: vec![key.clone()], ..d.clone() })
            .collect();
        let expected = ComponentAnswer { key: key.clone(), entry: batch.components[key].clone(), diagnostics };
        assert_eq!(answer, Some(expected), "{key}");
    }
}

#[test]
fn initialize_indexes_every_component_and_resolves_none() {
    let (_tmp, root) = project(&[("src/types.ts", TYPES), ("src/Button.tsx", BUTTON), ("src/Card.tsx", CARD)]);
    let session = WatchSession::lazy(options(&root, false));

    assert_eq!(session.component("Button"), None, "nothing is answered before initialize");
    assert!(session.initialize().components.is_empty());

    let stats = session.lazy_stats().unwrap();
    assert_eq!((stats.files, stats.components, stats.resolved), (3, 2, 0));
    assert_eq!(session.component_keys(), ["Button", "Card"]);
    assert_eq!(session.lazy_stats().unwrap().resolved, 0, "listing keys resolves nothing");
}

#[test]
fn a_request_resolves_one_component_and_memoizes_it() {
    let (_tmp, root) = project(&[("src/types.ts", TYPES), ("src/Button.tsx", BUTTON), ("src/Card.tsx", CARD)]);
    let session = WatchSession::lazy(options(&root, false));
    drop(session.initialize());

    let button = session.component("Button");

    assert_eq!(prop_names(&button), ["label", "size"]);
    assert_eq!(session.lazy_stats().unwrap().resolved, 1, "Card was not asked for");
    assert_eq!(session.component("Button"), button);
    assert_eq!(session.lazy_stats().unwrap().resolved, 1, "the second request is answered from the memo");
    assert_eq!(session.component("Missing"), None);
    assert_matches_batch(&session, &options(&root, false));
}

#[test]
fn editing_a_file_a_component_read_drops_only_that_component_and_the_answer_follows() {
    let (_tmp, root) = project(&[("src/types.ts", TYPES), ("src/Button.tsx", BUTTON), ("src/Card.tsx", CARD)]);
    let options = options(&root, false);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    session.components(&["Button".to_owned(), "Card".to_owned()]);
    assert_eq!(session.lazy_stats().unwrap().resolved, 2);

    write(&root, &[("src/types.ts", "export interface BaseProps { size?: 'sm' | 'lg'; tone?: 'a' | 'b' }\n")]);
    let update = session.update_file(&root.join("src/types.ts"));

    assert_eq!(update.revision, 2);
    assert_eq!(update.updated_keys, ["Button"]);
    assert!(update.removed_keys.is_empty());
    assert_eq!(update.affected_files, [root.join("src/Button.tsx")]);
    assert_eq!(session.lazy_stats().unwrap().resolved, 1, "Card keeps its memo");
    assert_eq!(prop_names(&session.component("Button")), ["label", "size", "tone"]);
    assert_matches_batch(&session, &options);
}

#[test]
fn an_edit_to_a_components_own_file_drops_it_and_a_removed_component_is_reported() {
    let (_tmp, root) = project(&[("src/types.ts", TYPES), ("src/Button.tsx", BUTTON), ("src/Card.tsx", CARD)]);
    let options = options(&root, false);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    session.components(&["Button".to_owned(), "Card".to_owned()]);

    write(
        &root,
        &[("src/Card.tsx", "export function Card(props: { title: string; open?: boolean }) { return null; }\n")],
    );
    let update = session.update_file(&root.join("src/Card.tsx"));
    assert_eq!(update.updated_keys, ["Card"]);
    assert_eq!(prop_names(&session.component("Card")), ["open", "title"]);

    std::fs::remove_file(root.join("src/Card.tsx")).unwrap();
    let update = session.update_file(&root.join("src/Card.tsx"));
    assert_eq!(update.removed_keys, ["Card"]);
    assert!(update.updated_keys.is_empty());
    assert_eq!(session.component("Card"), None);
    assert_matches_batch(&session, &options);
}

#[test]
fn a_same_named_component_in_another_file_moves_the_keys() {
    let item = "export function Item(props: { a: string }) { return null; }\n";
    let (_tmp, root) = project(&[("src/a/Item.tsx", item)]);
    let options = options(&root, false);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    assert_eq!(session.component_keys(), ["Item"]);
    session.component("Item");

    write(&root, &[("src/b/Item.tsx", "export function Item(props: { b: string }) { return null; }\n")]);
    let update = session.update_file(&root.join("src/b/Item.tsx"));

    let second = format!("Item ({})", root.join("src/b/Item.tsx"));
    assert_eq!(update.updated_keys, ["Item".to_owned(), second.clone()]);
    assert_eq!(prop_names(&session.component("Item")), ["a"]);
    assert_eq!(prop_names(&session.component(&second)), ["b"]);
    assert_matches_batch(&session, &options);
}

#[test]
fn ensure_fresh_reports_the_keys_an_edit_changed_and_the_next_answer_is_current() {
    let (_tmp, root) = project(&[("src/types.ts", TYPES), ("src/Button.tsx", BUTTON), ("src/Card.tsx", CARD)]);
    let options = options(&root, false);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    // The first call applies again every file modified shortly before the session started, dropping their memos.
    drop(session.ensure_fresh());
    session.components(&["Button".to_owned(), "Card".to_owned()]);

    write(&root, &[("src/types.ts", "export interface BaseProps { size?: 'sm' | 'lg'; tone?: string }\n")]);
    let update = session.ensure_fresh();

    assert_eq!(update.updated_keys, ["Button"]);
    assert_eq!(prop_names(&session.component("Button")), ["label", "size", "tone"]);
    assert_eq!(session.ensure_fresh().updated_keys, Vec::<String>::new());
    assert_matches_batch(&session, &options);
}

#[test]
fn an_eager_session_answers_as_a_lazy_one_does() {
    let (_tmp, root) = project(&[("src/types.ts", TYPES), ("src/Button.tsx", BUTTON), ("src/Card.tsx", CARD)]);
    let options = options(&root, false);
    let eager = WatchSession::new(options.clone());
    let lazy = WatchSession::lazy(options);
    drop(eager.initialize());
    drop(lazy.initialize());

    assert_eq!(eager.component_keys(), lazy.component_keys());
    for key in ["Button", "Card", "Missing"] {
        assert_eq!(eager.component(key), lazy.component(key), "{key}");
    }
    assert_eq!(eager.lazy_stats(), None);
}

fn two_packages() -> (tempfile::TempDir, Utf8PathBuf) {
    project(&[
        ("node_modules/pkg-a/package.json", r#"{ "name": "pkg-a", "version": "1.0.0", "types": "index.d.ts" }"#),
        ("node_modules/pkg-a/index.d.ts", "export interface AProps { a?: string }\n"),
        ("node_modules/pkg-b/package.json", r#"{ "name": "pkg-b", "version": "1.0.0", "types": "index.d.ts" }"#),
        ("node_modules/pkg-b/index.d.ts", "export interface BProps { b?: number }\n"),
        ("src/A.tsx", "import type { AProps } from 'pkg-a';\nexport function A(props: AProps) { return null; }\n"),
        ("src/B.tsx", "import type { BProps } from 'pkg-b';\nexport function B(props: BProps) { return null; }\n"),
    ])
}

#[test]
fn a_request_loads_only_the_dependency_names_its_component_reaches() {
    let (_tmp, root) = two_packages();
    let options = options(&root, true);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    assert_eq!(session.lazy_stats().unwrap().dependency_files, 0, "nothing loaded at the start");

    assert_eq!(prop_names(&session.component("A")), ["a"]);
    assert_eq!(session.lazy_stats().unwrap().dependency_files, 1, "pkg-b is not read for A");

    assert_eq!(prop_names(&session.component("B")), ["b"]);
    assert_eq!(session.lazy_stats().unwrap().dependency_files, 2);
    assert_matches_batch(&session, &options);
}

#[test]
fn a_new_import_in_an_edited_file_loads_its_names_on_the_next_request() {
    let (_tmp, root) = two_packages();
    let options = options(&root, true);
    let session = WatchSession::lazy(options.clone());
    drop(session.initialize());
    session.component("A");

    write(
        &root,
        &[(
            "src/A.tsx",
            "import type { AProps } from 'pkg-a';\nimport type { BProps } from 'pkg-b';\n\
             export function A(props: AProps & BProps) { return null; }\n",
        )],
    );
    let update = session.update_file(&root.join("src/A.tsx"));

    assert_eq!(update.updated_keys, ["A"]);
    assert_eq!(session.lazy_stats().unwrap().dependency_files, 1, "an edit loads nothing by itself");
    assert_eq!(prop_names(&session.component("A")), ["a", "b"]);
    assert_eq!(session.lazy_stats().unwrap().dependency_files, 2);
    assert_matches_batch(&session, &options);
}
