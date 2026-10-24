#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Refreshes `tests/fixtures/recording.json`, the answers the replay-tsc test double gives, from a real
//! `typescript@7.1.0-beta`. Run after changing which calls the backend makes:
//!
//! ```sh
//! cargo run -p caliper-docgen-semantic --example record_fixture -- <typescript package dir>
//! ```
//!
//! It copies `tests/fixtures/project` to a scratch directory, runs the structural pass and the backend on it with
//! recording on, and writes the calls with the scratch path replaced by `<ROOT>` (see `tests/bin/replay_tsc.rs`).

use caliper_docgen_core::pipeline::{extract, PipelineOptions};
use caliper_docgen_semantic::{targets_of, SemanticRequest};
use camino::Utf8PathBuf;

fn main() {
    let package_dir = Utf8PathBuf::from(std::env::args().nth(1).expect("the typescript package directory"));
    let version = {
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(package_dir.join("package.json")).unwrap()).unwrap();
        json["version"].as_str().unwrap().to_string()
    };
    println!("recording against typescript {version}");

    let crate_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Upper-case letters in the name, so the root and its lower-cased `path` form differ.
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        .unwrap()
        .join(format!("CaliperRecord{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for name in ["Button.tsx", "tsconfig.json"] {
        std::fs::copy(crate_dir.join("tests/fixtures/project").join(name), root.join(name)).unwrap();
    }

    let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
    let recording = root.join("recording.json");
    let mut request = SemanticRequest::new(root.clone(), targets_of(&output));
    request.tsc_path = Some(package_dir.join("bin/tsc"));
    request.expected_versions = Box::leak(vec![&*Box::leak(version.into_boxed_str())].into_boxed_slice());
    request.record_to = Some(recording.clone());
    let outcome = caliper_docgen_semantic::resolve(request);
    println!("{} resolved, {} fell back, {:?}", outcome.components.len(), outcome.fallbacks.len(), outcome.diagnostics);
    assert!(outcome.diagnostics.is_empty(), "the real checker was unavailable");

    let text = std::fs::read_to_string(&recording).unwrap();
    let text = text.replace(root.as_str(), "<ROOT>").replace(&root.as_str().to_lowercase(), "<root>");
    assert!(!text.to_lowercase().contains("/users/"), "the recording leaks a home directory");
    let target = crate_dir.join("tests/fixtures/recording.json");
    std::fs::write(&target, text).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    println!("wrote {target}");
}
