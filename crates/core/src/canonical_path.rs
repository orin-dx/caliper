//! Canonical paths without the Windows verbatim prefix.
//!
//! On Windows, `fs::canonicalize` returns verbatim paths (`\\?\C:\a\b`, `\\?\UNC\srv\sh`). Every other path in the
//! pipeline and in the consumers that compare against it (the addon's default-export lookup, the Vite plugin's
//! docgen injection) is spelled `C:\a\b`, so a verbatim spelling never matches. Discovery and watch canonicalize
//! through [`canonicalize`] to report one spelling.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};

/// [`Utf8Path::canonicalize_utf8`] with the verbatim prefix removed.
pub(crate) fn canonicalize(path: &Utf8Path) -> io::Result<Utf8PathBuf> {
    path.canonicalize_utf8().map(|canonical| strip_verbatim_prefix(canonical.as_str()))
}

/// Rewrites `\\?\C:\a\b` as `C:\a\b` and `\\?\UNC\srv\sh` as `\\srv\sh`. Any other path, including verbatim forms
/// with no drive-letter or UNC equivalent (`\\?\Volume{..}`), is returned unchanged.
fn strip_verbatim_prefix(path: &str) -> Utf8PathBuf {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return Utf8PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(drive) = path.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return Utf8PathBuf::from(drive);
        }
    }
    Utf8PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_drive_path_loses_its_prefix() {
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\a\b"), Utf8PathBuf::from(r"C:\a\b"));
    }

    #[test]
    fn verbatim_unc_path_becomes_a_plain_unc_path() {
        assert_eq!(strip_verbatim_prefix(r"\\?\UNC\srv\sh\a"), Utf8PathBuf::from(r"\\srv\sh\a"));
    }

    #[test]
    fn plain_paths_are_unchanged() {
        for path in [r"C:\a\b", r"\\srv\sh\a", "/usr/local/lib", "relative/dir", ""] {
            assert_eq!(strip_verbatim_prefix(path), Utf8PathBuf::from(path));
        }
    }

    #[test]
    fn verbatim_forms_without_a_plain_equivalent_are_unchanged() {
        for path in [r"\\?\Volume{0123}\a", r"\\?\GLOBALROOT\a", r"\\?\1:\a"] {
            assert_eq!(strip_verbatim_prefix(path), Utf8PathBuf::from(path));
        }
    }
}
