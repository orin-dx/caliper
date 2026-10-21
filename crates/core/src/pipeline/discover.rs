use camino::{Utf8Path, Utf8PathBuf};

use crate::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// Walk `src_dirs` and collect every `.ts` / `.tsx` file, excluding:
/// - Built-in patterns: `.stories.`, `.test.`, `.spec.`, `__snapshots__`, `node_modules`
/// - User-supplied extra exclude patterns
///
/// Returns discovered files alongside any diagnostics raised while walking
/// (e.g. a permission-denied subtree) — never dropped silently (AGENTS.md
/// non-negotiable #6).
pub(super) fn discover_files(
    src_dirs: &[Utf8PathBuf],
    extra_excludes: &[String],
) -> (Vec<Utf8PathBuf>, Vec<Diagnostic>) {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();

    for dir in src_dirs {
        // If the user explicitly points at a node_modules path, respect it.
        let dir_str = dir.as_str();
        let dir_is_in_node_modules = dir_str.contains("node_modules");

        let walker =
            ignore::WalkBuilder::new(dir.as_std_path()).hidden(false).git_ignore(!dir_is_in_node_modules).build();

        for result in walker {
            let entry = match result {
                Ok(entry) => entry,
                Err(err) => {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        message: format!("Error walking '{dir}': {err}"),
                        file: Some(dir.to_string()),
                        line: None,
                        column: None,
                        help: Some("Check file/directory permissions and for broken symlinks under this path.".into()),
                        code: DiagnosticCode::IoError,
                        affects: Vec::new(),
                    });
                    continue;
                }
            };
            let path = entry.path();
            if !is_typescript(path) {
                continue;
            }
            let relative = path.strip_prefix(dir.as_std_path()).unwrap_or(path);
            if excluded(relative, dir_is_in_node_modules, extra_excludes) {
                continue;
            }

            if let Ok(utf8) = Utf8PathBuf::from_path_buf(path.to_owned()) {
                // Canonicalize to an absolute path so parent.fileName in output is
                // stable regardless of invocation context (relative --src, absolute
                // --src, or cwd inside the src dir all previously produced different
                // strings for the same file). Fall back to the uncanonicalized path
                // if canonicalization fails (e.g. a dangling symlink) rather than
                // dropping the file.
                let canonical = utf8.canonicalize_utf8().unwrap_or(utf8);
                files.push(canonical);
            } else {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!("Skipping non-UTF8 path: {}", path.to_string_lossy()),
                    file: None,
                    line: None,
                    column: None,
                    help: Some("Rename the file to use valid UTF-8 characters in its path.".into()),
                    code: DiagnosticCode::IoError,
                    affects: Vec::new(),
                });
            }
        }
    }

    // Sort for deterministic ordering across OS/FS, then dedup: overlapping
    // src_dirs (e.g. ["./src", "./src/components"]) walk the same physical
    // file more than once, and since paths are canonicalized above, an exact
    // duplicate always sorts adjacent — this catches it here rather than
    // producing two separate component_mappings downstream.
    files.sort();
    files.dedup();
    (files, diagnostics)
}

fn is_typescript(path: &std::path::Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("ts" | "tsx"))
}

/// Excludes apply to the path inside the source directory: a project checked out under `my.test.repo/` or
/// `~/node_modules-backup/` must not lose every file. `node_modules` is skipped only when the source directory is not
/// itself inside one.
fn excluded(relative: &std::path::Path, dir_is_in_node_modules: bool, extra_excludes: &[String]) -> bool {
    let relative_str = relative.to_str().unwrap_or("");
    let file_name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let in_component = |name: &str| relative.components().any(|c| c.as_os_str() == name);
    file_name.contains(".stories.")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
        || in_component("__snapshots__")
        || (!dir_is_in_node_modules && in_component("node_modules"))
        || extra_excludes.iter().any(|p| relative_str.contains(p.as_str()))
}

/// Whether discovery would include `path`: TypeScript, under a source directory, and not excluded. Watch mode asks
/// this of every changed path so an edit to an excluded file (a story, a test) never adds a component.
pub(super) fn is_source_file(path: &Utf8Path, src_dirs: &[Utf8PathBuf], extra_excludes: &[String]) -> bool {
    if !is_typescript(path.as_std_path()) {
        return false;
    }
    src_dirs.iter().any(|dir| {
        let dir = dir.canonicalize_utf8().unwrap_or_else(|_| dir.clone());
        path.strip_prefix(&dir).is_ok_and(|relative| {
            !excluded(relative.as_std_path(), dir.as_str().contains("node_modules"), extra_excludes)
        })
    })
}

pub(super) fn should_skip(name: &str, exclude_prefixes: &[String]) -> bool {
    exclude_prefixes.iter().any(|p| name.starts_with(p.as_str()))
}
