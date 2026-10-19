//! A scenario-driven fake `tsc --api` process for the Lifecycle-gate test suite.
//! Scenario is picked via the `FAKE_TSC_SCENARIO` env var (env, not argv, matching
//! `crates/cli/src/config.rs`'s precedent for test/caller-controlled input):
//! `echo` (default), `never_respond`, `hang_after_n:<N>`, `exit_after:<N>:<code>`,
//! `crash_immediately:<code>`, `garbage_bytes`, `bad_content_length`,
//! `oversized_frame:<N>`, `truncated_body`, `reorder_ack`, `shutdown_slow:<ms>`, `never_read`,
//! `fatal_frame_then_alive`, `fatal_frame_once_then_echo`, `notify_then_echo`, `noisy_stderr`.
//!
//! Every scenario appends its PID as one line to `<cwd>/fake-tsc-spawns.log` at startup, where `<cwd>` is the value of
//! the `--cwd` argument the supervisor passes. Tests read that file to count spawns.
//!
//! Valid-frame scenarios are built on this crate's own `transport::{ContentLengthDecoder,
//! write_frame}`, so this fixture can't drift from what the real client parses;
//! bad-frame scenarios intentionally hand-write raw bytes instead.

use std::time::Duration;

use caliper_docgen_tsc_client::transport::{write_frame, ContentLengthDecoder, DEFAULT_MAX_FRAME_BYTES};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;
use tokio_util::codec::FramedRead;

type Requests = FramedRead<tokio::io::Stdin, ContentLengthDecoder>;

fn requests() -> Requests {
    FramedRead::new(tokio::io::stdin(), ContentLengthDecoder::new(DEFAULT_MAX_FRAME_BYTES))
}

#[tokio::main]
async fn main() {
    let prior_spawns = log_spawn();
    let scenario = std::env::var("FAKE_TSC_SCENARIO").unwrap_or_else(|_| "echo".to_string());
    let mut parts = scenario.splitn(3, ':');
    let name = parts.next().unwrap_or("echo");
    let arg1 = parts.next();
    let arg2 = parts.next();

    match name {
        "never_respond" => never_respond().await,
        "hang_after_n" => hang_after_n(arg1.and_then(|s| s.parse().ok()).unwrap_or(0)).await,
        "exit_after" => {
            exit_after(arg1.and_then(|s| s.parse().ok()).unwrap_or(0), arg2.and_then(|s| s.parse().ok()).unwrap_or(1))
                .await;
        }
        "crash_immediately" => std::process::exit(arg1.and_then(|s| s.parse().ok()).unwrap_or(1)),
        "garbage_bytes" => garbage_bytes().await,
        "bad_content_length" => bad_content_length().await,
        "oversized_frame" => oversized_frame(arg1.and_then(|s| s.parse().ok()).unwrap_or(usize::MAX)).await,
        "truncated_body" => truncated_body().await,
        "reorder_ack" => reorder_ack().await,
        "notify_then_echo" => notify_then_echo().await,
        "noisy_stderr" => noisy_stderr().await,
        "never_read" => tokio::time::sleep(Duration::from_secs(3600)).await,
        "fatal_frame_then_alive" => fatal_frame_then_alive().await,
        "fatal_frame_once_then_echo" if prior_spawns == 0 => fatal_frame_then_alive().await,
        "shutdown_slow" => shutdown_slow(arg1.and_then(|s| s.parse().ok()).unwrap_or(10_000)).await,
        _ => echo().await,
    }
}

/// Appends this process's PID to `<--cwd>/fake-tsc-spawns.log`. Returns the number of lines already there, so a
/// scenario can tell which spawn it is. Logging is skipped (returning 0) when the supervisor passed no `--cwd`.
fn log_spawn() -> usize {
    let mut args = std::env::args().skip_while(|arg| arg != "--cwd");
    let Some(cwd) = args.nth(1) else { return 0 };
    let path = std::path::Path::new(&cwd).join("fake-tsc-spawns.log");
    let prior = std::fs::read_to_string(&path).map_or(0, |contents| contents.lines().count());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = std::io::Write::write_all(&mut file, format!("{}\n", std::process::id()).as_bytes());
    }
    prior
}

fn request_id(request: &serde_json::Value) -> serde_json::Value {
    request.get("id").cloned().unwrap_or(serde_json::Value::Null)
}

async fn read_request(reader: &mut Requests) -> Option<serde_json::Value> {
    match reader.next().await {
        Some(Ok(body)) => serde_json::from_slice(&body).ok(),
        _ => None,
    }
}

async fn respond<W: tokio::io::AsyncWrite + Unpin>(writer: &mut W, id: &serde_json::Value, result: serde_json::Value) {
    let body = serde_json::json!({ "id": id, "result": result });
    if let Ok(bytes) = serde_json::to_vec(&body) {
        let _ = write_frame(writer, &bytes).await;
    }
}

async fn respond_error<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    id: &serde_json::Value,
    code: i64,
    message: &str,
) {
    let body = serde_json::json!({ "id": id, "error": { "code": code, "message": message } });
    if let Ok(bytes) = serde_json::to_vec(&body) {
        let _ = write_frame(writer, &bytes).await;
    }
}

/// Baseline: echoes `params` back as `result.echo`, or replies with an `error` object
/// when `params.fail` is `true` — used to test that one failing call doesn't affect a
/// concurrently in-flight sibling (partial project failure).
async fn echo() {
    let mut reader = requests();
    let mut stdout = tokio::io::stdout();
    while let Some(request) = read_request(&mut reader).await {
        let id = request_id(&request);
        let params = request.get("params").cloned().unwrap_or(serde_json::Value::Null);
        if params.get("fail").and_then(serde_json::Value::as_bool).unwrap_or(false) {
            respond_error(&mut stdout, &id, -32000, "requested failure").await;
        } else {
            respond(&mut stdout, &id, serde_json::json!({ "echo": params })).await;
        }
    }
}

/// Sends one server-to-client notification frame at startup, then behaves like `echo`.
async fn notify_then_echo() {
    let mut stdout = tokio::io::stdout();
    if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({ "method": "tsc/test", "params": { "n": 1 } })) {
        let _ = write_frame(&mut stdout, &bytes).await;
    }
    echo().await;
}

/// Writes well over a pipe buffer (64 KiB) to stderr before answering. Exits when a write fails, as a real process
/// does once the parent has closed its end of the pipe; blocks forever if the parent holds the pipe but never reads.
async fn noisy_stderr() {
    let line = [b'e'; 1023];
    let mut stderr = std::io::stderr();
    for _ in 0..2048 {
        let written =
            std::io::Write::write_all(&mut stderr, &line).and_then(|()| std::io::Write::write_all(&mut stderr, b"\n"));
        if written.is_err() {
            std::process::exit(3);
        }
    }
    echo().await;
}

/// Reads and discards requests forever, never responding — exercises the client's
/// per-request timeout.
async fn never_respond() {
    let mut reader = requests();
    while read_request(&mut reader).await.is_some() {}
}

/// Responds normally to the first `n` requests, then keeps reading (so the pipe never
/// backs up) without ever responding again.
async fn hang_after_n(n: usize) {
    let mut reader = requests();
    let mut stdout = tokio::io::stdout();
    let mut count = 0usize;
    while let Some(request) = read_request(&mut reader).await {
        count += 1;
        if count > n {
            continue;
        }
        respond(&mut stdout, &request_id(&request), serde_json::json!({ "ok": true })).await;
    }
}

/// Responds normally to the first `n` requests, then exits with `code` — exercises
/// crash detection mid-session (a request issued after the `n`th gets no response).
async fn exit_after(n: usize, code: i32) {
    let mut reader = requests();
    let mut stdout = tokio::io::stdout();
    let mut count = 0usize;
    while let Some(request) = read_request(&mut reader).await {
        count += 1;
        respond(&mut stdout, &request_id(&request), serde_json::json!({ "ok": true })).await;
        if count >= n {
            std::process::exit(code);
        }
    }
}

/// Ends with the blank-line header terminator so the reader reaches "a header block
/// was read, but it has no Content-Length" rather than hitting EOF mid-header.
async fn write_garbage_block() {
    let mut stdout = tokio::io::stdout();
    let _ = stdout.write_all(b"this is not a Content-Length frame at all\r\n\r\n").await;
    let _ = stdout.flush().await;
}

async fn garbage_bytes() {
    write_garbage_block().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Sends a fatal frame, then stays alive reading and discarding stdin — the child outlives its own bad frame, so
/// only the client's reader ending can reveal the session is over.
async fn fatal_frame_then_alive() {
    write_garbage_block().await;
    never_respond().await;
}

async fn bad_content_length() {
    let mut stdout = tokio::io::stdout();
    let _ = stdout.write_all(b"Content-Length: not-a-number\r\n\r\n").await;
    let _ = stdout.flush().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Declares a `Content-Length` far larger than the client's cap and never writes that
/// many bytes — the client must reject based on the declared length alone.
async fn oversized_frame(declared: usize) {
    let mut stdout = tokio::io::stdout();
    let header = format!("Content-Length: {declared}\r\n\r\n");
    let _ = stdout.write_all(header.as_bytes()).await;
    let _ = stdout.flush().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn truncated_body() {
    let mut stdout = tokio::io::stdout();
    let _ = stdout.write_all(b"Content-Length: 100\r\n\r\ntoo short").await;
    let _ = stdout.flush().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Reads two requests, answers the second one first — proves a response's `seq`
/// reflects wire-arrival order, not issue order (the stale-notification mechanism).
async fn reorder_ack() {
    let mut reader = requests();
    let mut stdout = tokio::io::stdout();
    let Some(first) = read_request(&mut reader).await else { return };
    let Some(second) = read_request(&mut reader).await else { return };
    respond(&mut stdout, &request_id(&second), serde_json::json!({ "order": "second" })).await;
    respond(&mut stdout, &request_id(&first), serde_json::json!({ "order": "first" })).await;
    while let Some(request) = read_request(&mut reader).await {
        respond(&mut stdout, &request_id(&request), serde_json::json!({ "ok": true })).await;
    }
}

/// Answers requests normally but ignores shutdown, including stdin EOF, for `delay_ms` before exiting —
/// forces the supervisor's shutdown grace period to elapse into a force-kill.
async fn shutdown_slow(delay_ms: u64) {
    let mut reader = requests();
    let mut stdout = tokio::io::stdout();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(delay_ms);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::select! {
            request = read_request(&mut reader) => {
                let Some(request) = request else {
                    tokio::time::sleep(remaining).await;
                    break;
                };
                respond(&mut stdout, &request_id(&request), serde_json::json!({ "ok": true })).await;
            }
            () = tokio::time::sleep(remaining) => break,
        }
    }
}
