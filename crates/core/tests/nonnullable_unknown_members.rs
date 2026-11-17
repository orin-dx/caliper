#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! `NonNullable<…>` reduces only when every member that stays is known not to hold `null` or `undefined`: a type
//! parameter, a cyclic alias and an alias past the depth limit keep the call, a known object or intersection reduces,
//! and a lazy session prints what a batch run prints.

use caliper_docgen_core::pipeline::{PipelineOptions, WatchSession, extract};
use camino::Utf8PathBuf;

const PRELUDE: &str = r#"
type Fallback<T> = { [P in keyof T]: T[P] | NonNullable<T[P]>[] };
type Conditional<T> = T extends infer U ? U | { _hover: U } : never;
type Maybe = string | undefined;
type Loop = Again | string;
type Again = Loop;
type Both = { a: 1 } & { b: 2 };
type Loose = unknown & Maybe;
interface Plain { x: number }
interface Shape<X> { generic?: X | undefined; prim?: number | undefined; plain?: Plain | undefined; cyc?: Loop | undefined; both?: Both | undefined; loose?: Loose | undefined }
interface Styles<X> extends Fallback<Shape<X>> {}
"#;

fn source(prop: &str) -> String {
    format!(
        "{PRELUDE}\nexport function Box<X>(props: {{ {prop}: Conditional<Styles<X>[\"{prop}\"]> }}) {{ return null; }}"
    )
}

fn options(root: &Utf8PathBuf) -> PipelineOptions {
    PipelineOptions { src_dirs: vec![root.clone()], cache_dir: Some(root.join(".cache")), ..Default::default() }
}

fn project(prop: &str) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();
    std::fs::write(root.join("Box.tsx"), source(prop)).unwrap();
    (tmp, root)
}

fn text_of(prop: &str) -> String {
    let (_tmp, root) = project(prop);
    let output = extract(&options(&root));
    output.components.values().next().expect("one component").props[prop].prop_type.raw_string()
}

#[test]
fn a_type_parameter_keeps_the_call() {
    assert_eq!(text_of("generic"), "Conditional<X | undefined | NonNullable<X | undefined>[]>");
}

#[test]
fn a_primitive_member_reduces() {
    assert_eq!(text_of("prim"), "Conditional<number | undefined | number[]>");
}

#[test]
fn an_interface_member_reduces() {
    assert_eq!(text_of("plain"), "Conditional<Plain | undefined | Plain[]>");
}

#[test]
fn a_cyclic_alias_past_the_depth_limit_keeps_the_call() {
    assert_eq!(text_of("cyc"), "Conditional<Loop | undefined | NonNullable<Loop | undefined>[]>");
}

#[test]
fn an_intersection_of_objects_reduces() {
    assert_eq!(text_of("both"), "Conditional<Both | undefined | Both[]>");
}

#[test]
fn an_intersection_whose_members_may_all_be_nullish_keeps_the_call() {
    assert_eq!(text_of("loose"), "Conditional<Loose | undefined | NonNullable<Loose | undefined>[]>");
}

#[test]
fn a_lazy_session_prints_what_a_batch_run_prints() {
    for prop in ["generic", "prim", "plain", "cyc", "both", "loose"] {
        let (_tmp, root) = project(prop);
        let options = options(&root);
        let batch = extract(&options);
        let (key, entry) = batch.components.iter().next().expect("one component");
        let session = WatchSession::lazy(options.clone());
        drop(session.initialize());
        let answer = session.component(key).expect("the lazy session answers the key");
        assert_eq!(
            answer.entry.props[prop].prop_type.raw_string(),
            entry.props[prop].prop_type.raw_string(),
            "{prop}: lazy vs batch"
        );
    }
}
