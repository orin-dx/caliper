#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Tripwires for the layering rules in `ARCHITECTURE.md`. Each rule is a pure function over `(path, source)` so a
//! negative test can prove it fires. Allowlists carry a reason and fail when the entry goes stale.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct SourceFile {
    /// Path relative to `crates/core/src`, with `/` separators.
    rel: String,
    /// Production lines only: comments and everything from the first `#[cfg(test)]` on are dropped.
    code: Vec<String>,
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<SourceFile>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "tests.rs") {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push(SourceFile { rel, code: production_lines(&fs::read_to_string(&path).unwrap()) });
        }
    }
}

fn production_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]" {
            // `mod tests;` is one line; an inline module runs to the closing brace at column 0.
            if lines.next().is_some_and(|item| !item.trim_end().ends_with(';')) {
                for body in lines.by_ref() {
                    if body == "}" {
                        break;
                    }
                }
            }
        } else if !line.trim_start().starts_with("//") {
            out.push(line.to_owned());
        }
    }
    out
}

fn core_sources() -> Vec<SourceFile> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    collect(&root, &root, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn in_layer(rel: &str, layer: &str) -> bool {
    rel == format!("{layer}.rs") || rel.starts_with(&format!("{layer}/"))
}

fn mentions(file: &SourceFile, needle: &str) -> bool {
    file.code.iter().any(|l| l.contains(needle))
}

/// Files in `layers` that mention any of `forbidden`, minus allowlisted `(file, reason)` entries. An allowlist entry
/// that no longer matches is itself a violation.
fn forbidden_imports(files: &[SourceFile], layers: &[&str], forbidden: &[&str], allow: &[(&str, &str)]) -> Vec<String> {
    let mut violations = Vec::new();
    for file in files.iter().filter(|f| layers.iter().any(|l| in_layer(&f.rel, l))) {
        let hit = forbidden.iter().any(|n| mentions(file, n));
        let allowed = allow.iter().any(|(f, _)| *f == file.rel);
        if hit && !allowed {
            violations.push(format!("{} imports {forbidden:?}", file.rel));
        }
    }
    for (path, reason) in allow {
        let still_needed = files.iter().any(|f| f.rel == *path && forbidden.iter().any(|n| mentions(f, n)));
        if !still_needed {
            violations.push(format!("stale allowlist entry {path} ({reason})"));
        }
    }
    violations
}

fn constructor_outside(files: &[SourceFile], needle: &str, allow: &[(&str, &str)]) -> Vec<String> {
    let mut violations: Vec<String> = files
        .iter()
        .filter(|f| mentions(f, needle) && !allow.iter().any(|(p, _)| *p == f.rel))
        .map(|f| format!("{} calls {needle}", f.rel))
        .collect();
    for (path, reason) in allow {
        if !files.iter().any(|f| f.rel == *path && mentions(f, needle)) {
            violations.push(format!("stale allowlist entry {path} ({reason})"));
        }
    }
    violations
}

fn hash_maps_in(files: &[SourceFile], rel: &str) -> Vec<String> {
    files
        .iter()
        .filter(|f| f.rel == rel)
        .filter(|f| mentions(f, "HashMap"))
        .map(|f| format!("{} uses a HashMap in a JSON-output type; use BTreeMap", f.rel))
        .collect()
}

fn oxc_in_public_signatures(files: &[SourceFile]) -> Vec<String> {
    let mut violations = Vec::new();
    for file in files.iter().filter(|f| in_layer(&f.rel, "extractor")) {
        for line in &file.code {
            let t = line.trim_start();
            let public = t.starts_with("pub ") && !t.starts_with("pub(");
            if public && (t.contains("oxc_") || t.contains("Allocator") || t.contains("Program<")) {
                violations.push(format!("{}: public signature exposes an AST type: {t}", file.rel));
            }
        }
    }
    violations
}

fn panic_abort_manifests(manifests: &[(String, String)]) -> Vec<String> {
    manifests
        .iter()
        .filter(|(_, text)| text.lines().any(|l| l.replace(' ', "").starts_with("panic=\"abort\"")))
        .map(|(path, _)| format!("{path} sets panic = \"abort\"; ADR 0005 needs unwinding"))
        .collect()
}

fn file(rel: &str, text: &str) -> SourceFile {
    SourceFile { rel: rel.to_owned(), code: production_lines(text) }
}

#[test]
fn extractor_and_known_never_import_resolver_or_pipeline() {
    let violations = forbidden_imports(
        &core_sources(),
        &["extractor", "known"],
        &["crate::resolver", "crate::pipeline", "super::resolver", "super::pipeline"],
        &[],
    );
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn resolver_never_imports_pipeline_beyond_the_allowlist() {
    // Removing these is roadmap item 2.3 (ResolverOptions moves into types/).
    let violations = forbidden_imports(
        &core_sources(),
        &["resolver"],
        &["crate::pipeline"],
        &[
            ("resolver/extends.rs", "HtmlAttributeMode lives in pipeline until 2.3"),
            ("resolver/mod.rs", "PipelineOptions and HtmlAttributeMode live in pipeline until 2.3"),
        ],
    );
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn opaque_detail_is_constructed_only_where_the_reason_is_known() {
    let violations = constructor_outside(
        &core_sources(),
        "OpaqueDetail::new(",
        &[
            ("known.rs", "library shortcuts name their own opaque reason"),
            ("resolver/named.rs", "the depth guard names its own reason"),
            ("types/output.rs", "deserialization rebuilds the detail"),
        ],
    );
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn json_output_types_use_btreemap() {
    assert_eq!(hash_maps_in(&core_sources(), "types/output.rs"), Vec::<String>::new());
}

#[test]
fn no_ast_type_in_a_public_extractor_signature() {
    assert_eq!(oxc_in_public_signatures(&core_sources()), Vec::<String>::new());
}

#[test]
fn no_manifest_sets_panic_abort() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut manifests = vec![("Cargo.toml".to_owned(), fs::read_to_string(workspace.join("Cargo.toml")).unwrap())];
    for entry in fs::read_dir(workspace.join("crates")).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        if manifest.exists() {
            manifests.push((manifest.to_string_lossy().into_owned(), fs::read_to_string(&manifest).unwrap()));
        }
    }
    assert_eq!(panic_abort_manifests(&manifests), Vec::<String>::new());
}

#[test]
fn publishable_crates_have_publishable_versioned_path_dependencies() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    let workspace_members = metadata["workspace_members"].as_array().unwrap();
    let mut violations = Vec::new();
    for package in packages {
        if !workspace_members.contains(&package["id"]) || package["publish"] == serde_json::json!([]) {
            continue;
        }
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["path"].is_null() {
                continue;
            }
            let name = dependency["name"].as_str().unwrap();
            if dependency["req"] == "*" {
                violations.push(format!("{} depends on {name} without a version", package["name"]));
            }
            if packages
                .iter()
                .any(|candidate| candidate["name"] == *name && candidate["publish"] == serde_json::json!([]))
            {
                violations.push(format!("{} depends on unpublished {name}", package["name"]));
            }
        }
    }
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn rules_fire_on_violations() {
    let bad = [
        file("extractor/visit.rs", "use crate::resolver::Foo;\n"),
        file("resolver/chain.rs", "use crate::pipeline::PipelineOptions;\n"),
        file("types/output.rs", "use std::collections::HashMap;\n"),
        file("extractor/mod.rs", "pub fn parse(alloc: &Allocator) {}\n"),
        file("elsewhere.rs", "OpaqueDetail::new(x, y);\n"),
    ];
    assert_eq!(forbidden_imports(&bad, &["extractor"], &["crate::resolver"], &[]).len(), 1);
    assert_eq!(forbidden_imports(&bad, &["resolver"], &["crate::pipeline"], &[]).len(), 1);
    assert_eq!(hash_maps_in(&bad, "types/output.rs").len(), 1);
    assert_eq!(oxc_in_public_signatures(&bad).len(), 1);
    assert_eq!(constructor_outside(&bad, "OpaqueDetail::new(", &[]).len(), 1);
    assert_eq!(
        panic_abort_manifests(&[("Cargo.toml".into(), "[profile.release]\npanic = \"abort\"\n".into())]).len(),
        1
    );
}

#[test]
fn stale_allowlist_entries_fail() {
    let clean = [file("resolver/extends.rs", "use crate::types::Foo;\n")];
    let violations = forbidden_imports(&clean, &["resolver"], &["crate::pipeline"], &[("resolver/extends.rs", "old")]);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].starts_with("stale allowlist entry"));
}

#[test]
fn test_code_and_comments_are_ignored() {
    let f = file(
        "extractor/mod.rs",
        "// crate::resolver is forbidden\nfn a() {}\n#[cfg(test)]\nmod t {\n    use crate::resolver::X;\n}\n",
    );
    assert!(forbidden_imports(&[f], &["extractor"], &["crate::resolver"], &[]).is_empty());
}
