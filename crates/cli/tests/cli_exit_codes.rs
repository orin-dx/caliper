//! Process-level tests for clap-level argument-rejection exit codes.
//! These happen in `main()` before any command handler runs, so they can't
//! be exercised through `cmd_extract`/`cmd_inspect`/etc. directly — only a
//! real subprocess invocation proves clap itself rejects them with exit 2.
//! SPEC-CLI-001a AC-015/AC-020.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_caliper-docgen")).args(args).output().expect("failed to spawn the real binary")
}

// ── AC-015: an unrecognized `completions` shell value is rejected by clap
// during argument parsing, before cmd_completions ever runs — exit 2, same
// numeric value as AC-020's clap-level rejections but a different mechanism.

#[test]
fn completions_with_an_unrecognized_shell_exits_2() {
    let output = run(&["completions", "bogus-shell"]);
    assert_eq!(output.status.code(), Some(2), "expected exit 2, stderr: {}", String::from_utf8_lossy(&output.stderr));
}

// ── AC-020: a missing required argument, an unrecognized flag, or a bad
// value_enum value are all rejected by clap before any command handler runs.

#[test]
fn inspect_with_no_component_positional_exits_2() {
    let output = run(&["inspect"]);
    assert_eq!(output.status.code(), Some(2), "expected exit 2, stderr: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn extract_with_an_unrecognized_flag_exits_2() {
    let output = run(&["extract", "--this-flag-does-not-exist"]);
    assert_eq!(output.status.code(), Some(2), "expected exit 2, stderr: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn extract_with_a_bad_format_value_exits_2() {
    let output = run(&["extract", "--format", "bogus"]);
    assert_eq!(output.status.code(), Some(2), "expected exit 2, stderr: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn extract_with_a_bad_html_attributes_value_exits_2() {
    let output = run(&["extract", "--html-attributes", "bogus"]);
    assert_eq!(output.status.code(), Some(2), "expected exit 2, stderr: {}", String::from_utf8_lossy(&output.stderr));
}
