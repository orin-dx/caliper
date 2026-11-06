//! Canonical paths without the Windows verbatim prefix.
//!
//! On Windows, `fs::canonicalize` returns verbatim paths (`\\?\C:\a\b`, `\\?\UNC\srv\sh`). Every other path in the
//! pipeline and in the consumers that compare against it (the addon's default-export lookup, the Vite plugin's
//! docgen injection) is spelled `C:\a\b`, so a verbatim spelling never matches. Discovery and watch canonicalize
//! through [`canonicalize`] to report one spelling.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};

/// [`Utf8Path::canonicalize_utf8`] with the verbatim prefix removed.
pub fn canonicalize(path: &Utf8Path) -> io::Result<Utf8PathBuf> {
    path.canonicalize_utf8().map(|canonical| strip_verbatim_prefix(canonical.as_str()))
}

/// Rewrites `\\?\C:\a\b` as `C:\a\b` and `\\?\UNC\srv\sh` as `\\srv\sh`. Any other path, including verbatim forms
/// with no drive-letter or UNC equivalent (`\\?\Volume{..}`) and those only the verbatim spelling can open (see
/// [`needs_verbatim`]), is returned unchanged.
fn strip_verbatim_prefix(path: &str) -> Utf8PathBuf {
    if needs_verbatim(path) {
        return Utf8PathBuf::from(path);
    }
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

/// Whether the plain spelling of a verbatim `path` would not open it: the plain form is limited to `MAX_PATH`, and
/// Windows reads a trailing dot or space or a reserved device name in a component differently without the prefix.
fn needs_verbatim(path: &str) -> bool {
    const MAX_PATH: usize = 260;
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let reserved = |component: &str| {
        let stem = component.split('.').next().unwrap_or(component).to_ascii_uppercase();
        RESERVED.contains(&stem.as_str())
            || ["COM", "LPT"]
                .iter()
                .any(|device| stem.strip_prefix(device).is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9'])))
    };
    // The limit is on the plain spelling, which is shorter than the verbatim one by its prefix.
    let plain_len = match (path.strip_prefix(r"\\?\UNC\"), path.strip_prefix(r"\\?\")) {
        (Some(unc), _) => unc.len() + 2,
        (None, Some(drive)) => drive.len(),
        (None, None) => path.len(),
    };
    plain_len >= MAX_PATH
        || path.split('\\').skip(3).any(|component| {
            component.ends_with(['.', ' ']) && component != "." && component != ".." || reserved(component)
        })
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

    #[test]
    fn paths_only_the_verbatim_spelling_can_open_keep_it() {
        let long = format!(r"\\?\C:\{}", "a".repeat(260));
        for path in [
            long.as_str(),
            r"\\?\C:\a\nul",
            r"\\?\C:\a\CON.txt",
            r"\\?\C:\a\com1",
            r"\\?\C:\a\b.",
            r"\\?\C:\a\b \c",
            r"\\?\UNC\srv\sh\aux",
        ] {
            assert_eq!(strip_verbatim_prefix(path), Utf8PathBuf::from(path));
        }
    }

    #[test]
    fn the_limit_applies_to_the_plain_spelling_not_the_verbatim_one() {
        // A plain path of 259 characters still opens; the verbatim spelling of it is 263 long.
        let plain = format!(r"C:\{}", "a".repeat(256));
        assert_eq!(plain.len(), 259);
        assert_eq!(strip_verbatim_prefix(&format!(r"\\?\{plain}")), Utf8PathBuf::from(&plain));

        let too_long = format!(r"\\?\C:\{}", "a".repeat(257));
        assert_eq!(strip_verbatim_prefix(&too_long), Utf8PathBuf::from(&too_long));

        let unc = format!(r"\\srv\sh\{}", "a".repeat(250));
        assert_eq!(unc.len(), 259);
        assert_eq!(strip_verbatim_prefix(&format!(r"\\?\UNC\{}", &unc[2..])), Utf8PathBuf::from(&unc));
    }

    #[test]
    fn names_that_only_resemble_reserved_ones_are_stripped() {
        assert_eq!(
            strip_verbatim_prefix(r"\\?\C:\a\console\com10\lpt0\.hidden"),
            Utf8PathBuf::from(r"C:\a\console\com10\lpt0\.hidden")
        );
    }
}
