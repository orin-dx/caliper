#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A lazy session answers each key exactly as a batch extraction of the same files publishes it (ADR 0016).
//!
//! `parity_over_fixtures` runs every fixture. `parity_over_sources` is the same check over directories named by
//! `CALIPER_PARITY_SRC` (colon-separated), for the large corpora:
//!   CALIPER_PARITY_SRC=/path/a:/path/b cargo test -p caliper-docgen-core --test lazy_parity --release -- --ignored
use caliper_docgen_core::pipeline::{extract, ComponentAnswer, HtmlAttributeMode, PipelineOptions, WatchSession};
use caliper_docgen_core::types::{Diagnostic, ExtractionOutput};
use camino::Utf8PathBuf;

fn workspace_root() -> Utf8PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    Utf8PathBuf::from_path_buf(root.to_owned()).unwrap()
}

/// What a batch extraction attaches to `key`: its diagnostics, scoped to that key alone.
fn batch_diagnostics(output: &ExtractionOutput, key: &str) -> Vec<Diagnostic> {
    output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.affects.iter().any(|affected| affected == key))
        .map(|diagnostic| Diagnostic { affects: vec![key.to_owned()], ..diagnostic.clone() })
        .collect()
}

/// `got` is `expected`; when it is not, name what differs instead of printing both in full.
fn assert_same(what: &str, got: Option<ComponentAnswer>, expected: ComponentAnswer) {
    if got.as_ref() == Some(&expected) {
        return;
    }
    let Some(got) = got else { panic!("{what}: no answer") };
    let names = |a: &ComponentAnswer| a.entry.props.keys().cloned().collect::<Vec<_>>();
    let (got_props, expected_props) = (names(&got), names(&expected));
    let lines = |a: &ComponentAnswer| format!("{:#?}", a.entry).lines().map(str::to_owned).collect::<Vec<_>>();
    let (got_lines, expected_lines) = (lines(&got), lines(&expected));
    let differing: Vec<String> = got_lines
        .iter()
        .zip(&expected_lines)
        .filter(|(g, e)| g != e)
        .take(8)
        .map(|(g, e)| format!("  lazy  {g}\n  batch {e}"))
        .collect();
    panic!(
        "{what}\nprops: {} lazy, {} batch; only lazy {:?}; only batch {:?}\nentry lines {} vs {}\n{}\ndiagnostics: {} lazy, {} batch\nlazy-only {:?}\nbatch-only {:?}",
        got_props.len(),
        expected_props.len(),
        got_props.iter().filter(|p| !expected_props.contains(p)).take(8).collect::<Vec<_>>(),
        expected_props.iter().filter(|p| !got_props.contains(p)).take(8).collect::<Vec<_>>(),
        got_lines.len(),
        expected_lines.len(),
        differing.join("\n"),
        got.diagnostics.len(),
        expected.diagnostics.len(),
        got.diagnostics.iter().filter(|d| !expected.diagnostics.contains(d)).map(|d| &d.message).take(5).collect::<Vec<_>>(),
        expected.diagnostics.iter().filter(|d| !got.diagnostics.contains(d)).map(|d| &d.message).take(5).collect::<Vec<_>>(),
    );
}

fn assert_parity(options: &PipelineOptions, label: &str) {
    let batch = extract(options);
    let session = WatchSession::lazy(options.clone());
    let initial = session.initialize();
    assert!(initial.components.is_empty(), "{label}: a lazy session resolves nothing on initialize");
    let indexed = session.lazy_stats().expect("lazy stats");
    assert_eq!(indexed.resolved, 0, "{label}: nothing is resolved before it is asked for");

    let keys: Vec<String> = batch.components.keys().cloned().collect();
    let listed = session.component_keys();
    if options.load_dependencies {
        // Listing never loads a dependency, so it can include a re-exported imported component with nothing to show.
        assert!(keys.iter().all(|key| listed.contains(key)), "{label}: every key a batch run publishes is listed");
        for extra in listed.iter().filter(|key| !keys.contains(key)) {
            assert_eq!(session.component(extra), None, "{label}: {extra} is listed but has nothing to document");
        }
    } else {
        assert_eq!(listed, keys, "{label}: the keys a batch run publishes");
    }

    let answers = session.components(&keys);
    for (key, answer) in keys.iter().zip(answers) {
        let expected = ComponentAnswer {
            key: key.clone(),
            entry: batch.components[key].clone(),
            diagnostics: batch_diagnostics(&batch, key),
        };
        assert_same(&format!("{label}: {key}"), answer, expected);
    }
    assert_eq!(session.component("NoSuchComponent"), None, "{label}: an unknown key");

    // What a request loads depends on what an earlier one did, so the order must not change an answer: a second
    // session asks for one component at a time, last key first.
    let sequential = WatchSession::lazy(options.clone());
    drop(sequential.initialize());
    for key in keys.iter().rev() {
        let expected = ComponentAnswer {
            key: key.clone(),
            entry: batch.components[key].clone(),
            diagnostics: batch_diagnostics(&batch, key),
        };
        assert_same(&format!("{label}: {key}, asked one at a time in reverse"), sequential.component(key), expected);
    }
}

fn fixture_options(dir: &Utf8PathBuf, cache: &tempfile::TempDir) -> PipelineOptions {
    PipelineOptions {
        src_dirs: vec![dir.clone()],
        tsconfig_path: Some(workspace_root().join("fixtures").join("tsconfig.json")),
        cache_dir: Some(Utf8PathBuf::from_path_buf(cache.path().to_owned()).unwrap()),
        ..Default::default()
    }
}

fn fixtures() -> Vec<Utf8PathBuf> {
    let mut dirs: Vec<Utf8PathBuf> = std::fs::read_dir(workspace_root().join("fixtures"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| Utf8PathBuf::from_path_buf(entry.path()).unwrap())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn parity_over_fixtures() {
    for dir in fixtures() {
        let cache = tempfile::TempDir::new().unwrap();
        assert_parity(&fixture_options(&dir, &cache), dir.file_name().unwrap());
    }
}

#[test]
fn parity_over_fixtures_with_dependencies_and_full_html_attributes() {
    for dir in fixtures() {
        let cache = tempfile::TempDir::new().unwrap();
        let options = PipelineOptions {
            load_dependencies: true,
            html_attributes: HtmlAttributeMode::Full,
            ..fixture_options(&dir, &cache)
        };
        assert_parity(&options, &format!("{} (dependencies, full)", dir.file_name().unwrap()));
    }
}

#[test]
#[ignore = "reads the directories named by CALIPER_PARITY_SRC"]
fn parity_over_sources() {
    let sources = std::env::var("CALIPER_PARITY_SRC").expect("CALIPER_PARITY_SRC");
    for src in sources.split(':').filter(|src| !src.is_empty()) {
        for (load_dependencies, html_attributes) in
            [(false, HtmlAttributeMode::Curated), (true, HtmlAttributeMode::Full)]
        {
            let cache = tempfile::TempDir::new().unwrap();
            let options = PipelineOptions {
                src_dirs: vec![Utf8PathBuf::from(src)],
                cache_dir: Some(Utf8PathBuf::from_path_buf(cache.path().to_owned()).unwrap()),
                load_dependencies,
                html_attributes,
                ..Default::default()
            };
            assert_parity(&options, &format!("{src} (dependencies: {load_dependencies}, {html_attributes:?})"));
        }
    }
}
