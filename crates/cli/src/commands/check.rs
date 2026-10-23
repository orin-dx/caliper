use miette::{IntoDiagnostic, Result};

use crate::config::{build_options, BuildOptionsArgs};
use crate::output::{print_diagnostics, print_summary};

/// Returns the process exit code (0 = clean, 1 = --strict and warnings found,
/// 2 = errors found) rather than calling `std::process::exit` directly — see
/// `cmd_extract`'s doc comment for why.
pub fn cmd_check(args: crate::CheckArgs, quiet: bool, config_path: Option<&str>) -> Result<i32> {
    let options = build_options(BuildOptionsArgs {
        src: &args.src,
        cache_dir: None,
        html_attributes: None,
        config_path,
        extra_builtins: &[],
        load_dependencies: args.load_dependencies,
        collapse_inherited_props: args.collapse_inherited_props.map(Into::into),
    })?;
    let output = caliper_docgen_core::pipeline::extract(&options);

    if args.json {
        println!("{}", serde_json::to_string(&output.diagnostics).into_diagnostic()?);
    } else if !quiet {
        print_summary(&output, quiet);
        print_diagnostics(&output.diagnostics);
    }

    Ok(output.exit_code(args.strict))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_mode_still_returns_the_error_exit_code() {
        let args = crate::CheckArgs {
            src: vec!["/nonexistent/does-not-exist".into()],
            strict: false,
            json: true,
            load_dependencies: false,
            collapse_inherited_props: None,
        };
        let code = cmd_check(args, true, None).expect("cmd_check itself should not error");
        assert_eq!(code, 2, "expected exit code 2 for a nonexistent src dir even in --json mode");
    }

    // ── SPEC-CLI-001a AC-003: check against a source directory that produces
    // zero diagnostics of any severity returns exit code 0.

    #[test]
    fn clean_run_returns_exit_code_zero() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
        std::fs::write(
            tmp.path().join("Widget.tsx"),
            "export function Widget(props: { label: string }) { return null; }\n",
        )
        .unwrap();
        let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let args = crate::CheckArgs {
            src: vec![dir.to_string()],
            strict: false,
            json: true,
            load_dependencies: false,
            collapse_inherited_props: None,
        };
        let code = cmd_check(args, true, None).expect("cmd_check should not error");
        assert_eq!(code, 0, "expected exit code 0 for a clean run");
    }

    fn warning_only_fixture() -> (tempfile::TempDir, camino::Utf8PathBuf) {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
        // A named type reference the resolver can't find anywhere produces a
        // Warning-severity "Cannot resolve type" diagnostic — no Error anywhere
        // in the fixture, isolating the Warning-only case AC-005/AC-006 need.
        std::fs::write(
            tmp.path().join("Widget.tsx"),
            "export function Widget(props: { x: SomeTypeThatIsNeverDeclaredAnywhere }) { return null; }\n",
        )
        .unwrap();
        let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        (tmp, dir)
    }

    // ── SPEC-CLI-001a AC-005: check --strict against a Warning-only source
    // directory (zero Error-severity diagnostics) returns exit code 1.

    #[test]
    fn strict_mode_with_warning_only_diagnostics_returns_exit_code_one() {
        let (_tmp, dir) = warning_only_fixture();
        let args = crate::CheckArgs {
            src: vec![dir.to_string()],
            strict: true,
            json: true,
            load_dependencies: false,
            collapse_inherited_props: None,
        };
        let code = cmd_check(args, true, None).expect("cmd_check should not error");
        assert_eq!(code, 1, "expected exit code 1 for --strict with warning-only diagnostics");
    }

    // ── SPEC-CLI-001a AC-006: the same Warning-only source directory without
    // --strict returns exit code 0.

    #[test]
    fn non_strict_mode_with_warning_only_diagnostics_returns_exit_code_zero() {
        let (_tmp, dir) = warning_only_fixture();
        let args = crate::CheckArgs {
            src: vec![dir.to_string()],
            strict: false,
            json: true,
            load_dependencies: false,
            collapse_inherited_props: None,
        };
        let code = cmd_check(args, true, None).expect("cmd_check should not error");
        assert_eq!(code, 0, "expected exit code 0 without --strict for warning-only diagnostics");
    }

    // ── SPEC-CLI-001a AC-004: an Error-severity diagnostic always outranks
    // --strict's Warning-only exit-1 path — even when both an Error AND a
    // Warning are present in the same diagnostic set, the result is exit 2,
    // not 1.

    #[test]
    fn error_takes_precedence_over_warning_even_with_strict_and_both_present() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
        // Warning: unresolvable type reference.
        std::fs::write(
            tmp.path().join("Widget.tsx"),
            "export function Widget(props: { x: SomeTypeThatIsNeverDeclaredAnywhere }) { return null; }\n",
        )
        .unwrap();
        // Error: unclosed interface body, a parse error.
        std::fs::write(tmp.path().join("Bad.tsx"), "export interface BrokenProps {\n    label: string;\n").unwrap();

        let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let args = crate::CheckArgs {
            src: vec![dir.to_string()],
            strict: true,
            json: true,
            load_dependencies: false,
            collapse_inherited_props: None,
        };
        let code = cmd_check(args, true, None).expect("cmd_check should not error");
        assert_eq!(code, 2, "expected exit code 2 — the Error must outrank the Warning even with --strict");
    }
}
