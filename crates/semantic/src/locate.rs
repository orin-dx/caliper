//! Checking the checker binary an explicit `--tsc-path` (`tscPath`) names. Without one the linked checker answers,
//! so a project's own `typescript` or `tsrs` is never looked for.

use camino::{Utf8Path, Utf8PathBuf};

/// `path` when it is a file; otherwise the error says what to pass.
pub(crate) fn explicit_tsc(path: &Utf8Path) -> Result<Utf8PathBuf, String> {
    if path.is_file() {
        Ok(path.to_owned())
    } else {
        Err(format!(
            "no tsc binary at {path}; pass --tsc-path (tscPath) a `tsc` of typescript@7.1 or a `tsrs` binary, or leave \
             it out to use the built-in checker"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_path_that_does_not_exist_is_an_error_that_says_what_to_pass() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let missing = root.join("nope");

        assert_eq!(
            explicit_tsc(&missing),
            Err(format!(
                "no tsc binary at {missing}; pass --tsc-path (tscPath) a `tsc` of typescript@7.1 or a `tsrs` binary, or \
                 leave it out to use the built-in checker"
            ))
        );
    }

    #[test]
    fn an_explicit_path_to_a_file_is_taken_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let tsc = root.join("tsc");
        std::fs::write(&tsc, "").unwrap();

        assert_eq!(explicit_tsc(&tsc), Ok(tsc));
    }
}
