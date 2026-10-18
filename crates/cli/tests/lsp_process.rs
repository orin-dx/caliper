//! Process-level tests for the `lsp` subcommand: drive the real binary's
//! stdin/stdout, not just its pure helper functions. `src/commands/lsp.rs`'s
//! own unit tests only assert `check_content_length`/`read_content_length`
//! in isolation — a regression at the `cmd_lsp` call site itself (e.g.
//! deleting the `check_content_length(len)?` call, or replacing a `return
//! Err` with `continue`) would leave those tests green while the actual
//! process-level behavior broke. SPEC-CLI-001c AC-001/002/004/005.

use std::io::Write;
use std::process::{Command, Output, Stdio};

mod common;
use common::strip_ansi;

// Mirrors `crates/cli/src/commands/lsp.rs`'s private `MAX_LSP_MESSAGE_BYTES`.
const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

fn run_lsp(args: &[&str], stdin_bytes: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_caliper-docgen"))
        .arg("lsp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn the real lsp binary");
    child.stdin.take().unwrap().write_all(stdin_bytes).expect("write to lsp stdin");
    child.wait_with_output().expect("wait for lsp process")
}

#[test]
fn oversized_content_length_exits_1_with_empty_stdout() {
    let frame = format!("Content-Length: {}\r\n\r\n", MAX_LSP_MESSAGE_BYTES + 1);
    let output = run_lsp(&[], frame.as_bytes());
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1, got {:?}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "expected empty stdout, got {:?}", String::from_utf8_lossy(&output.stdout));
}

// ── SPEC-CLI-001c AC-002: a frame declaring Content-Length EXACTLY at the
// 16 MiB cap (not past it) is read and dispatched normally — the boundary
// check itself was already unit-tested (test_content_length_at_cap_is_
// accepted), but nothing proved a REAL 16 MiB body makes it through
// cmd_lsp's read_exact + JSON parse + dispatch end to end.

#[test]
fn exactly_16mib_content_length_is_read_and_dispatched_normally() {
    let prefix = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"pad":""#;
    let suffix = br#""}}"#;
    let pad_len = MAX_LSP_MESSAGE_BYTES - prefix.len() - suffix.len();
    let mut body = Vec::with_capacity(MAX_LSP_MESSAGE_BYTES);
    body.extend_from_slice(prefix);
    body.extend(std::iter::repeat_n(b'x', pad_len));
    body.extend_from_slice(suffix);
    assert_eq!(body.len(), MAX_LSP_MESSAGE_BYTES, "test fixture must be exactly at the cap");
    serde_json::from_slice::<serde_json::Value>(&body).expect("fixture must itself be valid JSON");

    let mut input = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    input.extend_from_slice(&body);
    let exit_body = serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "method": "exit"})).unwrap();
    input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", exit_body.len()).as_bytes());
    input.extend_from_slice(&exit_body);

    let output = run_lsp(&[], &input);
    assert_eq!(output.status.code(), Some(0), "expected exit 0, stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Content-Length:"), "expected a well-formed response frame, got {stdout:?}");
    assert!(stdout.contains("\"id\":1"), "expected the response to echo the request id, got {stdout:?}");
    assert!(stdout.contains("\"result\""), "expected a result object in the response, got {stdout:?}");
}

#[test]
fn headerless_frame_exits_1_with_empty_stdout() {
    let output = run_lsp(&[], b"X-Some-Other-Header: value\r\n\r\n");
    assert_eq!(output.status.code(), Some(1), "expected exit 1, got {:?}", output.status);
    assert!(output.stdout.is_empty(), "expected empty stdout, got {:?}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn well_formed_initialize_gets_a_response_frame_with_hover_false() {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let mut input = format!("Content-Length: {}\r\n\r\n", body_bytes.len()).into_bytes();
    input.extend_from_slice(&body_bytes);

    // Follow with an `exit` notification so the process terminates cleanly.
    let exit_body = serde_json::json!({"jsonrpc": "2.0", "method": "exit"});
    let exit_bytes = serde_json::to_vec(&exit_body).unwrap();
    input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", exit_bytes.len()).as_bytes());
    input.extend_from_slice(&exit_bytes);

    let output = run_lsp(&[], &input);
    assert_eq!(output.status.code(), Some(0), "expected exit 0, stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Content-Length:"), "expected a well-formed response frame, got {stdout:?}");
    assert!(stdout.contains("\"hoverProvider\":false"), "expected hoverProvider:false in the response, got {stdout:?}");
}

#[test]
fn verbose_mode_writes_tracing_output_to_stderr_only() {
    // A tracing::error! firing mid-session (here, via the oversized-Content-
    // Length path) must land on stderr only — LSP reserves stdout
    // exclusively for Content-Length-framed messages — and only when `-v` asked for it.
    let frame = format!("Content-Length: {}\r\n\r\n", MAX_LSP_MESSAGE_BYTES + 1);

    let verbose = run_lsp(&["-v"], frame.as_bytes());
    assert!(
        verbose.stdout.is_empty(),
        "stdout must contain no tracing output under -v, got {:?}",
        String::from_utf8_lossy(&verbose.stdout)
    );
    let stderr = strip_ansi(&String::from_utf8_lossy(&verbose.stderr));
    assert!(stderr.contains("ERROR caliper_docgen::commands::lsp: LSP frame declared Content-Length"), "{stderr}");

    let quiet = run_lsp(&[], frame.as_bytes());
    let stderr = strip_ansi(&String::from_utf8_lossy(&quiet.stderr));
    assert!(!stderr.contains("ERROR caliper_docgen"), "no tracing output without -v: {stderr}");
}

// ── SPEC-CLI-001c AC-005c: a client that declares a valid Content-Length
// then closes before delivering that many body bytes is treated as a clean
// disconnect — exit 0, no response frame.

#[test]
fn truncated_body_exits_0_with_empty_stdout() {
    let mut input = b"Content-Length: 100\r\n\r\n".to_vec();
    input.extend_from_slice(b"{\"short\": true}"); // far fewer than the declared 100 bytes
    let output = run_lsp(&[], &input);
    assert_eq!(output.status.code(), Some(0), "expected exit 0, got {:?}", output.status);
    assert!(output.stdout.is_empty(), "expected empty stdout, got {:?}", String::from_utf8_lossy(&output.stdout));
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

fn well_formed_initialize_then_exit() -> Vec<u8> {
    let init = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    let exit = serde_json::json!({"jsonrpc": "2.0", "method": "exit"});
    let mut out = framed(&serde_json::to_vec(&init).unwrap());
    out.extend_from_slice(&framed(&serde_json::to_vec(&exit).unwrap()));
    out
}

// ── SPEC-CLI-001c AC-006: a well-formed frame whose body isn't valid JSON
// produces no response for that frame, but doesn't desync the stream — a
// subsequent well-formed initialize still gets answered.

#[test]
fn malformed_json_body_produces_no_response_but_does_not_desync_the_stream() {
    let mut input = framed(b"this is not json");
    input.extend_from_slice(&well_formed_initialize_then_exit());

    let output = run_lsp(&[], &input);
    assert_eq!(output.status.code(), Some(0), "expected exit 0, stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Content-Length:"), "expected a well-formed response frame, got {stdout:?}");
    assert!(
        stdout.contains("\"hoverProvider\":false"),
        "expected the subsequent initialize to still be answered, got {stdout:?}"
    );
    // Exactly one response frame — the malformed frame must not also have
    // produced a (garbage) response of its own.
    assert_eq!(stdout.matches("Content-Length:").count(), 1, "expected exactly one response frame, got {stdout:?}");
}

// ── SPEC-CLI-001c AC-007: a well-formed frame whose JSON body has no string
// "method" member produces no response for that frame, but doesn't desync
// the stream — a subsequent well-formed initialize still gets answered.

#[test]
fn missing_method_field_produces_no_response_but_does_not_desync_the_stream() {
    let mut input = framed(&serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": 1})).unwrap());
    input.extend_from_slice(&well_formed_initialize_then_exit());

    let output = run_lsp(&[], &input);
    assert_eq!(output.status.code(), Some(0), "expected exit 0, stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Content-Length:"), "expected a well-formed response frame, got {stdout:?}");
    assert!(
        stdout.contains("\"hoverProvider\":false"),
        "expected the subsequent initialize to still be answered, got {stdout:?}"
    );
    assert_eq!(stdout.matches("Content-Length:").count(), 1, "expected exactly one response frame, got {stdout:?}");
}

fn exit_notification() -> Vec<u8> {
    framed(&serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "method": "exit"})).unwrap())
}

#[test]
fn closing_stdin_without_any_frame_exits_0_with_empty_stdout() {
    let output = run_lsp(&[], b"");
    assert_eq!(output.status.code(), Some(0), "expected exit 0, got {:?}", output.status);
    assert!(output.stdout.is_empty(), "expected empty stdout, got {:?}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn shutdown_is_answered_with_a_null_result_for_the_same_id() {
    let mut input =
        framed(&serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": "shutdown"})).unwrap());
    input.extend_from_slice(&exit_notification());

    let output = run_lsp(&[], &input);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let body = stdout.split("\r\n\r\n").nth(1).expect("a response frame with a body");
    let response: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(response, serde_json::json!({"jsonrpc": "2.0", "id": 7, "result": null}));
}

#[test]
fn frames_after_exit_are_never_answered() {
    let mut input = exit_notification();
    input.extend_from_slice(&well_formed_initialize_then_exit());

    let output = run_lsp(&[], &input);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "expected empty stdout, got {:?}", String::from_utf8_lossy(&output.stdout));
}
