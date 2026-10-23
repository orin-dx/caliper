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
    let found = std::sync::Mutex::new((Vec::new(), Vec::new()));

    for dir in src_dirs {
        // If the user explicitly points at a node_modules path, respect it.
        let dir_is_in_node_modules = dir.as_str().contains("node_modules");

        // Walking from the canonical root makes every entry's path canonical except a symlink itself (the walker
        // doesn't follow symlinked directories), so only those pay for `canonicalize` below.
        let root = dir.canonicalize_utf8().unwrap_or_else(|_| dir.clone());
        let walker = ignore::WalkBuilder::new(root.as_std_path())
            .hidden(false)
            .git_ignore(!dir_is_in_node_modules)
            .build_parallel();
        walker.run(|| {
            let (found, root) = (&found, &root);
            Box::new(move |result| {
                let visited = visit(result, dir, root, dir_is_in_node_modules, extra_excludes);
                if let Some(visited) = visited {
                    let mut found = found.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    match visited {
                        Ok(file) => found.0.push(file),
                        Err(diagnostic) => found.1.push(diagnostic),
                    }
                }
                ignore::WalkState::Continue
            })
        });
    }

    let (mut files, mut diagnostics) = found.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Sort for deterministic ordering across OS/FS and walker threads, then dedup: overlapping src_dirs (e.g.
    // ["./src", "./src/components"]) walk the same physical file more than once, and since paths are canonical, an
    // exact duplicate always sorts adjacent.
    files.sort();
    files.dedup();
    diagnostics.sort_by(|a, b| a.message.cmp(&b.message));
    (files, diagnostics)
}

/// Every directory `discover_files` walks under `src_dirs` (canonical), and every file in them named `is_config`: a
/// file created in or deleted from one of these directories changes the directory's modification time.
pub(super) fn discover_dirs(
    src_dirs: &[Utf8PathBuf],
    extra_excludes: &[String],
    is_config: impl Fn(&str) -> bool,
) -> (Vec<Utf8PathBuf>, Vec<Utf8PathBuf>) {
    let (mut dirs, mut configs) = (Vec::new(), Vec::new());
    for dir in src_dirs {
        let dir_is_in_node_modules = dir.as_str().contains("node_modules");
        let root = dir.canonicalize_utf8().unwrap_or_else(|_| dir.clone());
        let walker =
            ignore::WalkBuilder::new(root.as_std_path()).hidden(false).git_ignore(!dir_is_in_node_modules).build();
        for entry in walker.flatten() {
            let Ok(path) = Utf8PathBuf::from_path_buf(entry.path().to_owned()) else { continue };
            let relative = path.strip_prefix(&root).unwrap_or(&path);
            if excluded(relative.as_std_path(), dir_is_in_node_modules, extra_excludes) {
                continue;
            }
            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                dirs.push(path);
            } else if path.file_name().is_some_and(&is_config) {
                configs.push(path);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    configs.sort();
    configs.dedup();
    (dirs, configs)
}

/// One walked entry: a source file to keep, a diagnostic, or `None` to skip it.
fn visit(
    result: Result<ignore::DirEntry, ignore::Error>,
    dir: &Utf8Path,
    root: &Utf8Path,
    dir_is_in_node_modules: bool,
    extra_excludes: &[String],
) -> Option<Result<Utf8PathBuf, Diagnostic>> {
    let entry = match result {
        Ok(entry) => entry,
        Err(err) => {
            return Some(Err(Diagnostic::new(
                DiagnosticSeverity::Warning,
                DiagnosticCode::IoError,
                format!("Error walking '{dir}': {err}"),
            )
            .with_file(dir.to_string())
            .with_help("Check file/directory permissions and for broken symlinks under this path.")));
        }
    };
    let path = entry.path();
    if !is_typescript(path) {
        return None;
    }
    let relative = path.strip_prefix(root.as_std_path()).unwrap_or(path);
    if excluded(relative, dir_is_in_node_modules, extra_excludes) {
        return None;
    }
    match Utf8PathBuf::from_path_buf(path.to_owned()) {
        // Absolute and canonical, so parent.fileName in output is stable regardless of invocation context (relative
        // --src, absolute --src, cwd inside the src dir). A dangling symlink keeps its own path rather than being
        // dropped.
        Ok(utf8) if entry.path_is_symlink() => Some(Ok(utf8.canonicalize_utf8().unwrap_or(utf8))),
        Ok(utf8) => Some(Ok(utf8)),
        Err(_) => Some(Err(Diagnostic::new(
            DiagnosticSeverity::Warning,
            DiagnosticCode::IoError,
            format!("Skipping non-UTF8 path: {}", path.to_string_lossy()),
        )
        .with_help("Rename the file to use valid UTF-8 characters in its path."))),
    }
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
