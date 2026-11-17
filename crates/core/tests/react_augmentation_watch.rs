#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A watch session re-resolves HTML components when a project file that augments React is edited, added or removed, and
//! answers as a cold run over the same files does, for the eager and the lazy session alike.

use std::collections::BTreeSet;

use caliper_docgen_core::pipeline::{HtmlAttributeMode, PipelineOptions, WatchSession, extract};
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

const COMPONENT: &str =
    "import * as React from 'react';\nexport const Box = (props: React.ComponentProps<'div'>) => <div />;\n";
const OTHER: &str = "export function Other(props: { a: string }) { return null; }\n";

fn augmenting(member: &str) -> String {
    format!(
        "import type {{}} from 'react';\ndeclare module 'react' {{ interface HTMLAttributes<T> {{ {member}?: string }} }}\n"
    )
}

fn project() -> (TempDir, Utf8PathBuf) {
    let tmp = TempDir::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
    write(&root, "src/C.tsx", COMPONENT);
    write(&root, "src/Other.tsx", OTHER);
    (tmp, root)
}

fn write(root: &Utf8Path, name: &str, source: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

fn options(root: &Utf8Path) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![root.join("src")],
        cache_dir: Some(root.join(".cache")),
        html_attributes: HtmlAttributeMode::Full,
        ..Default::default()
    }
}

fn session(root: &Utf8Path, lazy: bool) -> WatchSession {
    if lazy { WatchSession::lazy(options(root)) } else { WatchSession::new(options(root)) }
}

fn names(session: &WatchSession, key: &str) -> BTreeSet<String> {
    session.component(key).expect("an answer").entry.props.keys().cloned().collect()
}

fn matches_cold_run(session: &WatchSession, root: &Utf8Path, lazy: bool) {
    let cold = extract(&options(root));
    assert_eq!(session.component("Box").unwrap().entry, cold.components["Box"], "lazy: {lazy}");
}

#[test]
fn editing_an_augmenting_file_re_resolves_the_components_that_read_it() {
    for lazy in [false, true] {
        let (_tmp, root) = project();
        write(&root, "src/a.d.ts", &augmenting("tw"));
        let session = session(&root, lazy);
        drop(session.initialize());
        let before = names(&session, "Box");
        assert!(before.contains("tw") && !before.contains("tw2"), "lazy: {lazy}");

        write(&root, "src/a.d.ts", &augmenting("tw2"));
        let update = session.update_file(&root.join("src/a.d.ts"));
        let after = names(&session, "Box");
        assert!(after.contains("tw2") && !after.contains("tw"), "lazy: {lazy}");
        assert_eq!(update.affected_files, [root.join("src/C.tsx")], "lazy: {lazy}");
        matches_cold_run(&session, &root, lazy);
    }
}

#[test]
fn adding_an_augmenting_file_is_picked_up() {
    for lazy in [false, true] {
        let (_tmp, root) = project();
        let session = session(&root, lazy);
        drop(session.initialize());
        assert!(!names(&session, "Box").contains("tw"), "lazy: {lazy}");
        assert_eq!(names(&session, "Other"), BTreeSet::from(["a".to_owned()]));

        write(&root, "src/a.d.ts", &augmenting("tw"));
        let update = session.update_file(&root.join("src/a.d.ts"));
        assert!(names(&session, "Box").contains("tw"), "lazy: {lazy}");
        assert_eq!(update.affected_files, [root.join("src/C.tsx")], "a component that reads no HTML is untouched");
        matches_cold_run(&session, &root, lazy);
    }
}

#[test]
fn removing_an_augmenting_file_drops_its_members() {
    for lazy in [false, true] {
        let (_tmp, root) = project();
        write(&root, "src/a.d.ts", &augmenting("tw"));
        let session = session(&root, lazy);
        drop(session.initialize());
        assert!(names(&session, "Box").contains("tw"), "lazy: {lazy}");

        std::fs::remove_file(root.join("src/a.d.ts")).unwrap();
        session.update_file(&root.join("src/a.d.ts"));
        assert!(!names(&session, "Box").contains("tw"), "lazy: {lazy}");
        matches_cold_run(&session, &root, lazy);
    }
}

#[test]
fn a_file_that_starts_augmenting_react_is_picked_up() {
    for lazy in [false, true] {
        let (_tmp, root) = project();
        write(&root, "src/a.d.ts", "export interface Unrelated { x?: string }\n");
        let session = session(&root, lazy);
        drop(session.initialize());
        assert!(!names(&session, "Box").contains("tw"), "lazy: {lazy}");

        write(&root, "src/a.d.ts", &augmenting("tw"));
        session.update_file(&root.join("src/a.d.ts"));
        assert!(names(&session, "Box").contains("tw"), "lazy: {lazy}");
        matches_cold_run(&session, &root, lazy);
    }
}
