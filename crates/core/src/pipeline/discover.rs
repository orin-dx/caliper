use camino::{Utf8Path, Utf8PathBuf};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// Whether a path names a file or a directory. Exclude rules differ between the two (`legacy/` matches only a
/// directory), so [`excluded`] takes it with no default.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryKind {
    File,
    Dir,
}

impl EntryKind {
    fn of(entry: &ignore::DirEntry) -> Self {
        if entry.file_type().is_some_and(|kind| kind.is_dir()) { Self::Dir } else { Self::File }
    }
}

/// Walk `src_dirs` and collect every TypeScript file (`.ts`, `.tsx`, `.mts`, `.cts`), excluding:
/// - Built-in patterns: `.stories.`, `.test.`, `.spec.`, `__snapshots__`, `node_modules`
/// - User-supplied extra exclude patterns: gitignore-style globs on the path inside the source directory (see
///   [`compile_excludes`]); one that does not parse excludes nothing and is reported
///
/// Returns discovered files alongside any diagnostics raised while walking
/// (e.g. a permission-denied subtree) — never dropped silently (AGENTS.md
/// non-negotiable #6).
pub(super) fn discover_files(
    src_dirs: &[Utf8PathBuf],
    extra_excludes: &[String],
) -> (Vec<Utf8PathBuf>, Vec<Diagnostic>) {
    let (extra_excludes, invalid) = compile_excludes(extra_excludes);
    let extra_excludes = &extra_excludes;
    let found = std::sync::Mutex::new((Vec::new(), invalid_pattern_diagnostics(&invalid)));

    for dir in src_dirs {
        // If the user explicitly points at a node_modules path, respect it.
        let dir_is_in_node_modules = in_node_modules(dir);

        // Walking from the canonical root makes every entry's path canonical except a symlink itself (the walker
        // doesn't follow symlinked directories), so only those pay for `canonicalize` below.
        let root = crate::canonical_path::canonicalize(dir).unwrap_or_else(|_| dir.clone());
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
    let (extra_excludes, _) = compile_excludes(extra_excludes);
    let extra_excludes = &extra_excludes;
    let (mut dirs, mut configs) = (Vec::new(), Vec::new());
    for dir in src_dirs {
        let dir_is_in_node_modules = in_node_modules(dir);
        let root = crate::canonical_path::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        let walker =
            ignore::WalkBuilder::new(root.as_std_path()).hidden(false).git_ignore(!dir_is_in_node_modules).build();
        for entry in walker.flatten() {
            let Ok(path) = Utf8PathBuf::from_path_buf(entry.path().to_owned()) else { continue };
            let relative = path.strip_prefix(&root).unwrap_or(&path);
            let kind = EntryKind::of(&entry);
            match kind {
                EntryKind::Dir => {
                    if !excluded(relative.as_std_path(), kind, dir_is_in_node_modules, extra_excludes) {
                        dirs.push(path);
                    }
                }
                // A resolution config is not a source file, so the exclude list does not name it; it is watched
                // unless the directory holding it is excluded.
                EntryKind::File if path.file_name().is_some_and(&is_config) => {
                    let holder = relative.parent().unwrap_or(relative);
                    if !excluded(holder.as_std_path(), EntryKind::Dir, dir_is_in_node_modules, extra_excludes) {
                        configs.push(path);
                    }
                }
                EntryKind::File => {}
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
    extra_excludes: &Gitignore,
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
    let kind = EntryKind::of(&entry);
    if kind == EntryKind::Dir || !crate::extractor::is_typescript(path) {
        return None;
    }
    let relative = path.strip_prefix(root.as_std_path()).unwrap_or(path);
    if excluded(relative, kind, dir_is_in_node_modules, extra_excludes) {
        return None;
    }
    match Utf8PathBuf::from_path_buf(path.to_owned()) {
        // Absolute and canonical, so parent.fileName in output is stable regardless of invocation context (relative
        // --src, absolute --src, cwd inside the src dir). A dangling symlink keeps its own path rather than being
        // dropped.
        Ok(utf8) if entry.path_is_symlink() => Some(Ok(crate::canonical_path::canonicalize(&utf8).unwrap_or(utf8))),
        Ok(utf8) => Some(Ok(utf8)),
        Err(_) => Some(Err(Diagnostic::new(
            DiagnosticSeverity::Warning,
            DiagnosticCode::IoError,
            format!("Skipping non-UTF8 path: {}", path.to_string_lossy()),
        )
        .with_help("Rename the file to use valid UTF-8 characters in its path."))),
    }
}

/// Whether `dir` is a `node_modules` directory or inside one: a path component, not text, so `~/node_modules-backup/`
/// is not.
fn in_node_modules(dir: &Utf8Path) -> bool {
    dir.components().any(|component| component.as_str() == "node_modules")
}

/// Excludes apply to the path inside the source directory: a project checked out under `my.test.repo/` or
/// `~/node_modules-backup/` must not lose every file. `node_modules` is skipped only when the source directory is not
/// itself inside one.
///
/// The user's patterns follow gitignore: every directory on the way to the entry is tested as a directory, and one that
/// is excluded takes everything inside it, which a later `!` cannot bring back. That makes the answer for a file
/// depend only on its path, so a directory walk, a file walk and a check of one changed path agree. The built-in
/// `.stories.`, `.test.` and `.spec.` names apply to files only.
fn excluded(
    relative: &std::path::Path,
    kind: EntryKind,
    dir_is_in_node_modules: bool,
    extra_excludes: &Gitignore,
) -> bool {
    let file_name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let in_component = |name: &str| relative.components().any(|c| c.as_os_str() == name);
    let built_in_name = kind == EntryKind::File
        && (file_name.contains(".stories.") || file_name.contains(".test.") || file_name.contains(".spec."));
    built_in_name
        || in_component("__snapshots__")
        || (!dir_is_in_node_modules && in_component("node_modules"))
        || excluded_by_patterns(relative, kind, extra_excludes)
}

fn excluded_by_patterns(relative: &std::path::Path, kind: EntryKind, extra_excludes: &Gitignore) -> bool {
    let mut prefix = std::path::PathBuf::new();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        prefix.push(component);
        let is_dir = components.peek().is_some() || kind == EntryKind::Dir;
        if extra_excludes.matched(&prefix, is_dir).is_ignore() {
            return true;
        }
    }
    false
}

/// The user's `exclude` patterns as one matcher over paths relative to a source directory. Patterns are gitignore-style
/// globs: `*.tsx`, `legacy/` (a directory), `**/fixtures/**`, `src/Button.tsx` (anchored at the source directory, as it
/// has a slash). A bare name such as `Button` matches only a file or directory named exactly that. The patterns that do
/// not parse come back beside the matcher and match nothing.
pub(super) fn compile_excludes(patterns: &[String]) -> (Gitignore, Vec<(String, ignore::Error)>) {
    let mut builder = GitignoreBuilder::new("");
    let mut invalid: Vec<(String, ignore::Error)> = patterns
        .iter()
        .filter_map(|pattern| builder.add_line(None, pattern).err().map(|err| (pattern.clone(), err)))
        .collect();
    let matcher = builder.build().unwrap_or_else(|err| {
        invalid.push((patterns.join(", "), err));
        Gitignore::empty()
    });
    (matcher, invalid)
}

fn invalid_pattern_diagnostics(invalid: &[(String, ignore::Error)]) -> Vec<Diagnostic> {
    invalid
        .iter()
        .map(|(pattern, err)| {
            Diagnostic::new(
                DiagnosticSeverity::Warning,
                DiagnosticCode::IoError,
                format!("exclude pattern '{pattern}' is not a valid glob and excludes nothing: {err}"),
            )
            .with_help("Exclude patterns are gitignore-style globs, e.g. \"**/*.fixture.tsx\" or \"legacy/\".")
        })
        .collect()
}

/// Whether discovery would include `path`: TypeScript, under a source directory, and not excluded. Watch mode asks
/// this of every changed path so an edit to an excluded file (a story, a test) never adds a component. A path that no
/// longer exists was a file, and its parent directories are tested as [`excluded`] tests them during discovery.
pub(super) fn is_source_file(path: &Utf8Path, src_dirs: &[Utf8PathBuf], extra_excludes: &[String]) -> bool {
    if !crate::extractor::is_typescript(path.as_std_path()) || path.is_dir() {
        return false;
    }
    let (extra_excludes, _) = compile_excludes(extra_excludes);
    src_dirs.iter().any(|dir| {
        let dir = crate::canonical_path::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        path.strip_prefix(&dir).is_ok_and(|relative| {
            !excluded(relative.as_std_path(), EntryKind::File, in_node_modules(&dir), &extra_excludes)
        })
    })
}

pub(super) fn should_skip(name: &str, exclude_prefixes: &[String]) -> bool {
    exclude_prefixes.iter().any(|p| name.starts_with(p.as_str()))
}
