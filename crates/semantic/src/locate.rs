//! Finding the `tsc` binary the project installed.

use camino::{Utf8Path, Utf8PathBuf};

/// `explicit` if given (it must exist), else the nearest `node_modules/typescript/bin/tsc` at or above `project_root`.
/// The version is checked later, when the process is spawned.
pub(crate) fn find_tsc(project_root: &Utf8Path, explicit: Option<&Utf8Path>) -> Result<Utf8PathBuf, String> {
    if let Some(path) = explicit {
        return if path.is_file() { Ok(path.to_owned()) } else { Err(format!("no tsc binary at {path}")) };
    }
    project_root
        .ancestors()
        .map(|dir| dir.join("node_modules").join("typescript").join("bin").join("tsc"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "no node_modules/typescript/bin/tsc at or above {project_root}; install `typescript@beta` (7.1) or pass \
                 --tsc-path"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_installed_typescript_above_the_project_wins() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let bin = root.join("node_modules/typescript/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("tsc"), "").unwrap();
        let project = root.join("packages/ui");
        std::fs::create_dir_all(&project).unwrap();

        assert_eq!(find_tsc(&project, None), Ok(bin.join("tsc")));
    }

    #[test]
    fn a_project_without_typescript_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

        let error = find_tsc(&root, None).unwrap_err();

        assert!(error.contains("no node_modules/typescript/bin/tsc"), "{error}");
    }

    #[test]
    fn an_explicit_path_that_does_not_exist_is_an_error_not_a_search() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

        assert_eq!(find_tsc(&root, Some(&root.join("nope"))), Err(format!("no tsc binary at {}", root.join("nope"))));
    }
}
