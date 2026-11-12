#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! The layering rules in `ARCHITECTURE.md` that a type, a visibility or a clippy lint cannot state: which module may
//! import which. What clippy can state is in `.clippy.toml` (`HashMap`, `OpaqueDetail::new`), and `extractor` is
//! `pub(crate)`, so no AST type reaches the public API without a re-export.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Path relative to `crates/core/src` (with `/` separators) and its production code: comment lines and everything from
/// the first `#[cfg(test)]` item on are dropped.
struct SourceFile {
    rel: String,
    code: String,
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<SourceFile>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "tests.rs") {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push(SourceFile { rel, code: production_code(&fs::read_to_string(&path).unwrap()) });
        }
    }
}

fn production_code(text: &str) -> String {
    let mut out = String::new();
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
            out.push_str(line);
            out.push('\n');
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

/// Whether `code` writes `module::` as the start of a path: `crate::module::X`, `super::module::X`, or `module::X` in a
/// grouped `use crate::{…}`. `oxc_resolver::` is not `resolver::`.
fn names_module(code: &str, module: &str) -> bool {
    code.match_indices(&format!("{module}::"))
        .any(|(at, _)| !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// The files in `layers`, other than `except`, that name any of the `forbidden` modules.
fn forbidden_imports(layers: &[&str], forbidden: &[&str], except: &[&str]) -> Vec<String> {
    core_sources()
        .iter()
        .filter(|file| layers.iter().any(|layer| in_layer(&file.rel, layer)) && !except.contains(&file.rel.as_str()))
        .filter_map(|file| {
            let hits: Vec<_> = forbidden.iter().filter(|module| names_module(&file.code, module)).collect();
            (!hits.is_empty()).then(|| format!("{} names {hits:?}", file.rel))
        })
        .collect()
}

#[test]
fn extractor_and_known_never_import_resolver_or_pipeline() {
    assert_eq!(forbidden_imports(&["extractor", "known"], &["resolver", "pipeline"], &[]), Vec::<String>::new());
}

#[test]
fn resolver_and_dependency_types_never_import_pipeline() {
    // The options both read live in `types/options.rs`; `pipeline` re-exports them.
    assert_eq!(forbidden_imports(&["resolver", "dependency_types"], &["pipeline"], &[]), Vec::<String>::new());
}

#[test]
fn types_never_import_resolver_or_pipeline() {
    assert_eq!(forbidden_imports(&["types"], &["resolver", "pipeline", "dependency_types"], &[]), Vec::<String>::new());
}

#[test]
fn the_resolver_names_dependency_types_only_for_the_layer_it_reads() {
    // `dependency_types` sits above the resolver: it loads declarations through the resolver's module resolution. The
    // one edge back is the `DependencyLayer` value `ResolutionContext` holds. Moving that type under `types/` would
    // remove it; that is more than a move, so it is recorded here.
    assert_eq!(forbidden_imports(&["resolver"], &["dependency_types"], &["resolver/mod.rs"]), Vec::<String>::new());
}

#[test]
fn no_manifest_sets_panic_abort() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut manifests = vec![workspace.join("Cargo.toml")];
    for entry in fs::read_dir(workspace.join("crates")).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        if manifest.exists() {
            manifests.push(manifest);
        }
    }
    let aborting: Vec<_> = manifests
        .iter()
        .filter(|path| {
            fs::read_to_string(path).unwrap().lines().any(|l| l.replace(' ', "").starts_with("panic=\"abort\""))
        })
        .collect();
    assert_eq!(aborting, Vec::<&PathBuf>::new(), "ADR 0005 needs unwinding");
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
