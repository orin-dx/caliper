//! Finding the checker binary the project installed.

use camino::{Utf8Path, Utf8PathBuf};

/// The binaries searched for in each `node_modules`, in order of precedence: `typescript@7.1`'s `tsc`, then the `tsrs`
/// Node shim. An install that has both keeps using `typescript`, as before `tsrs` was supported; `--tsc-path` picks
/// the other.
const CANDIDATES: [[&str; 3]; 2] = [["typescript", "bin", "tsc"], ["tsrs", "bin", "tsrs"]];

/// `explicit` if given (it must exist), else the binary of the nearest `node_modules` at or above `project_root` that
/// holds one: `typescript/bin/tsc`, else `tsrs/bin/tsrs`. The version is checked later, when the process is spawned.
pub(crate) fn find_tsc(project_root: &Utf8Path, explicit: Option<&Utf8Path>) -> Result<Utf8PathBuf, String> {
    if let Some(path) = explicit {
        return if path.is_file() { Ok(path.to_owned()) } else { Err(format!("no tsc binary at {path}")) };
    }
    project_root
        .ancestors()
        .flat_map(|dir| {
            CANDIDATES.map(|parts| parts.iter().fold(dir.join("node_modules"), |path, part| path.join(part)))
        })
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "no node_modules/typescript/bin/tsc or node_modules/tsrs/bin/tsrs at or above {project_root}; install \
                 `typescript@beta` (7.1) or `tsrs`, or pass --tsc-path"
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
    fn the_tsrs_shim_is_found_when_typescript_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let bin = root.join("node_modules/tsrs/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("tsrs"), "").unwrap();

        assert_eq!(find_tsc(&root.join("packages/ui"), None), Ok(bin.join("tsrs")));
    }

    #[test]
    fn typescript_wins_over_tsrs_in_the_same_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        for (package, binary) in [("typescript", "tsc"), ("tsrs", "tsrs")] {
            let bin = root.join("node_modules").join(package).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join(binary), "").unwrap();
        }

        assert_eq!(find_tsc(&root, None), Ok(root.join("node_modules/typescript/bin/tsc")));
    }

    #[test]
    fn the_nearest_node_modules_wins_whichever_checker_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let far = root.join("node_modules/typescript/bin");
        let near = root.join("packages/ui/node_modules/tsrs/bin");
        for bin in [&far, &near] {
            std::fs::create_dir_all(bin).unwrap();
        }
        std::fs::write(far.join("tsc"), "").unwrap();
        std::fs::write(near.join("tsrs"), "").unwrap();

        assert_eq!(find_tsc(&root.join("packages/ui/src"), None), Ok(near.join("tsrs")));
    }

    #[test]
    fn a_project_without_typescript_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

        let error = find_tsc(&root, None).unwrap_err();

        assert!(error.contains("no node_modules/typescript/bin/tsc"), "{error}");
        assert!(error.contains("node_modules/tsrs/bin/tsrs"), "{error}");
    }

    #[test]
    fn an_explicit_path_that_does_not_exist_is_an_error_not_a_search() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

        assert_eq!(find_tsc(&root, Some(&root.join("nope"))), Err(format!("no tsc binary at {}", root.join("nope"))));
    }
}
