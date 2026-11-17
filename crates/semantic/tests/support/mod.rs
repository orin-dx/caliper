//! Shared by the tests that copy a fixture project into a scratch directory.

use camino::{Utf8Path, Utf8PathBuf};

/// `from` copied to `to`, with a `modules` directory named `node_modules` (which the repository's ignore rules would not
/// keep).
pub fn copy_fixture(from: &Utf8Path, to: &Utf8Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_str().unwrap().to_string();
        let target = to.join(if name == "modules" { "node_modules" } else { &name });
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_fixture(&Utf8PathBuf::from_path_buf(entry.path()).unwrap(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}
