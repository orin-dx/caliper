#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
use caliper_docgen_core::pipeline::{extract, PipelineOptions};
/// Snapshot tests for extraction output.
///
/// These lock the exact JSON output for every fixture so that any refactor
/// step that accidentally changes observable behavior shows up immediately
/// as a snapshot diff rather than a silent regression.
///
/// To update snapshots after an intentional change:
///   INSTA_UPDATE=always cargo test -p caliper-docgen-core --test snapshots
///
/// To review pending snapshots interactively:
///   cargo insta review
use camino::Utf8PathBuf;

/// Absolute path to the workspace root, resolved from this crate's manifest dir.
fn workspace_root() -> Utf8PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    // crates/core → workspace root is two levels up
    let root = std::path::Path::new(manifest).parent().unwrap().parent().unwrap();
    Utf8PathBuf::from_path_buf(root.to_owned()).unwrap()
}

fn fixture_dir(library: &str) -> Utf8PathBuf {
    workspace_root().join("fixtures").join(library)
}

fn run_fixture(library: &str) -> serde_json::Value {
    let dir = fixture_dir(library);
    let tsconfig = workspace_root().join("fixtures").join("tsconfig.json");

    // A fresh cache per run: the default directory persists across builds, so a stale entry could stand in for the
    // current extractor's output and let a regression through.
    let cache = tempfile::TempDir::new().expect("temp cache dir");
    let options = PipelineOptions {
        src_dirs: vec![dir],
        tsconfig_path: Some(tsconfig),
        cache_dir: Some(Utf8PathBuf::from_path_buf(cache.path().to_owned()).expect("UTF-8 temp path")),
        // The fixtures are baselines of the extractor on single files. Loading dependencies would also read whatever
        // packages the checkout has installed (csstype turns Mantine's `string` props into its literal unions), so
        // the snapshots would vary with `node_modules`.
        load_dependencies: false,
        ..Default::default()
    };

    let output = extract(&options);

    let canonical = serde_json::to_string(&output).expect("extraction output must serialize");
    let read: caliper_docgen_core::types::ExtractionOutput =
        serde_json::from_str(&canonical).expect("the canonical JSON must read back");
    assert_eq!(read, output, "the canonical JSON must read back into the same output");

    // Props inline, as each component serializes on its own, so a snapshot diff shows the prop that changed rather
    // than an id.
    serde_json::json!({
        "components": output.components,
        "enums": output.enums,
        "diagnostics": output.diagnostics,
        "stats": output.stats,
    })
}

/// Replace pnpm store versions (`.pnpm/@types+react@19.2.17/`) with `[VERSION]`, so dependency bumps don't rewrite
/// every snapshot that references a React builtin.
fn redact_pnpm_versions(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(".pnpm/") {
        let (before, after) = rest.split_at(start + ".pnpm/".len());
        out.push_str(before);
        let (segment, tail) = match after.find('/') {
            Some(end) => after.split_at(end),
            None => (after, ""),
        };
        // Version starts at the first `@` after index 0: skips a scope's leading `@`, and keeps peer suffixes
        // (`react-dom@19.3.0_react@19.3.0`) inside the redacted span.
        match segment[1..].find('@').map(|at| at + 1) {
            Some(at) => {
                out.push_str(&segment[..at]);
                out.push_str("@[VERSION]");
            }
            None => out.push_str(segment),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Replace all absolute paths in the JSON value with `[ROOT]`.
/// This makes snapshots portable across machines and directory layouts.
fn redact_paths(value: &mut serde_json::Value, workspace: &str) {
    match value {
        serde_json::Value::String(s) => {
            if s.starts_with('/') {
                // Pure path string: strip the workspace prefix and replace with [ROOT]/
                let trimmed = s.trim_start_matches(workspace).trim_start_matches('/').to_owned();
                *s = format!("[ROOT]/{trimmed}");
            } else if s.contains(workspace) {
                // Embedded path (e.g., diagnostic messages containing quoted absolute paths):
                // replace all occurrences so snapshots are stable across checkout locations.
                *s = s.replace(workspace, "[ROOT]");
            }
            if s.contains(".pnpm/") {
                *s = redact_pnpm_versions(s);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                redact_paths(v, workspace);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                redact_paths(v, workspace);
            }
        }
        _ => {}
    }
}

fn snapshot_fixture(library: &str) -> serde_json::Value {
    let mut value = run_fixture(library);
    let root = workspace_root();
    redact_paths(&mut value, root.as_str());
    // Zero out non-deterministic stats fields so snapshots don't flap.
    if let Some(stats) = value.get_mut("stats").and_then(|s| s.as_object_mut()) {
        stats.insert("durationMs".into(), serde_json::Value::Number(0.into()));
        stats.insert("dtsCacheHits".into(), serde_json::Value::Number(0.into()));
    }
    value
}

// ── Per-library snapshot tests ────────────────────────────────────────────────

#[test]
fn snapshot_shadcn() {
    let value = snapshot_fixture("shadcn");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_radix() {
    let value = snapshot_fixture("radix");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_mui() {
    let value = snapshot_fixture("mui");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_chakra() {
    let value = snapshot_fixture("chakra");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_mantine() {
    let value = snapshot_fixture("mantine");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_react_aria() {
    let value = snapshot_fixture("react-aria");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_panda() {
    let value = snapshot_fixture("panda");
    insta::assert_json_snapshot!(value);
}

#[test]
fn snapshot_rdt_compat() {
    let value = snapshot_fixture("rdt-compat");
    insta::assert_json_snapshot!(value);
}

// ── Redaction unit tests ──────────────────────────────────────────────────────

#[test]
fn pnpm_version_redaction_covers_scoped_peer_and_repeated_segments() {
    // Scoped: the leading `@` is the scope, not the version.
    assert_eq!(
        redact_pnpm_versions("[ROOT]/node_modules/.pnpm/@types+react@19.3.0/node_modules/@types/react/index.d.ts"),
        "[ROOT]/node_modules/.pnpm/@types+react@[VERSION]/node_modules/@types/react/index.d.ts"
    );
    // Peer suffix is part of the version span.
    assert_eq!(
        redact_pnpm_versions(
            "[ROOT]/node_modules/.pnpm/react-dom@19.3.0_react@19.3.0/node_modules/react-dom/index.d.ts"
        ),
        "[ROOT]/node_modules/.pnpm/react-dom@[VERSION]/node_modules/react-dom/index.d.ts"
    );
    // Every `.pnpm/` segment, not just the first.
    assert_eq!(
        redact_pnpm_versions(".pnpm/a@1.0.0/node_modules/.pnpm/b@2.0.0/node_modules/b.d.ts"),
        ".pnpm/a@[VERSION]/node_modules/.pnpm/b@[VERSION]/node_modules/b.d.ts"
    );
    assert_eq!(redact_pnpm_versions("[ROOT]/fixtures/radix/button.tsx"), "[ROOT]/fixtures/radix/button.tsx");
    assert_eq!(redact_pnpm_versions(".pnpm/@scope+pkg/node_modules/x.d.ts"), ".pnpm/@scope+pkg/node_modules/x.d.ts");
}
