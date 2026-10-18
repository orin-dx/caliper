//! LSP (Language Server Protocol) command scaffold for caliper-docgen.
//!
//! Provides IDE hover and component prop documentation in editors (VS Code, Neovim, Helix).

use std::io::{self, BufRead, Read, Write};

use miette::Result;

/// Cap on a single LSP frame's declared `Content-Length`, checked *before*
/// `vec![0u8; len]` allocates — `Content-Length` is client-controlled input
/// with no upper bound of its own.
const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

/// Reject an oversized frame before its receive buffer is allocated.
fn check_content_length(len: usize) -> Result<()> {
    if len > MAX_LSP_MESSAGE_BYTES {
        tracing::error!(
            "LSP frame declared Content-Length {len} bytes, exceeding the {MAX_LSP_MESSAGE_BYTES}-byte cap; closing connection"
        );
        return Err(miette::miette!(
            "LSP frame declared Content-Length {len} bytes, exceeding the {MAX_LSP_MESSAGE_BYTES}-byte cap"
        ));
    }
    Ok(())
}

/// Reads header lines up to the blank-line terminator and returns the parsed
/// `Content-Length`, if any.
///
/// `Err(())` means the stream ended before any header line was read at all —
/// a normal, expected way for a client to close the connection.
/// `Ok(None)` means a header block *was* read (the loop reached the blank
/// line) but it contained no usable `Content-Length` — missing header or an
/// unparsable value. That body's byte length is unknowable, so the caller
/// must not keep reading from the stream as if nothing happened: doing so
/// desyncs every subsequent frame, since the unconsumed body bytes get read
/// as the start of the *next* header block.
fn read_content_length<R: BufRead>(reader: &mut R) -> Result<Option<usize>, ()> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return Err(());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(header) = line.strip_prefix("Content-Length: ") {
            content_length = header.trim().parse().ok();
        }
    }

    Ok(content_length)
}

/// The `initialize` response's `result` payload — factored out so capability
/// advertisement is testable without driving the stdin/stdout loop.
fn initialize_result() -> serde_json::Value {
    serde_json::json!({
        "capabilities": {
            "textDocumentSync": 1,
            // No textDocument/hover handler exists in the method dispatch
            // below (it falls into the `_ => {}` catch-all) — advertising
            // `true` would make a client's hover request, which carries an
            // `id`, hang forever waiting for a response that never comes.
            "hoverProvider": false
        },
        "serverInfo": {
            "name": "caliper-docgen-lsp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

pub fn cmd_lsp() -> Result<()> {
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    loop {
        let content_length = match read_content_length(&mut stdin_lock) {
            Err(()) => return Ok(()), // EOF
            Ok(v) => v,
        };

        let Some(len) = content_length else {
            tracing::error!(
                "LSP frame's header block had no usable Content-Length; closing connection to avoid a desynced stream"
            );
            return Err(miette::miette!("received an LSP frame without a usable Content-Length header"));
        };

        check_content_length(len)?;

        let mut buf = vec![0u8; len];
        if stdin_lock.read_exact(&mut buf).is_err() {
            return Ok(());
        }

        let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&buf) else {
            continue;
        };

        if let Some(method) = msg["method"].as_str() {
            let id = msg.get("id");
            match method {
                "initialize" => {
                    if let Some(id) = id {
                        let response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": initialize_result()
                        });
                        send_response(&mut stdout_lock, &response);
                    }
                }
                "shutdown" => {
                    if let Some(id) = id {
                        let response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": null
                        });
                        send_response(&mut stdout_lock, &response);
                    }
                }
                "exit" => break,
                _ => {}
            }
        }
    }

    Ok(())
}

fn send_response<W: Write>(out: &mut W, response: &serde_json::Value) {
    if let Ok(json) = serde_json::to_string(response) {
        let header = format!("Content-Length: {}\r\n\r\n", json.len());
        if let Err(e) =
            out.write_all(header.as_bytes()).and_then(|()| out.write_all(json.as_bytes())).and_then(|()| out.flush())
        {
            // A broken client pipe otherwise leaves this response silently
            // undelivered with zero trace anywhere — stderr is the safe
            // channel here for the same reason `check_content_length` already
            // uses it: stdout is reserved exclusively for Content-Length-framed
            // messages.
            tracing::error!("Failed to write LSP response frame: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oversized_content_length_is_rejected() {
        let result = check_content_length(MAX_LSP_MESSAGE_BYTES + 1);
        assert!(result.is_err(), "expected a Content-Length past the cap to be rejected, got {:?}", result);
    }

    #[test]
    fn test_content_length_at_cap_is_accepted() {
        let result = check_content_length(MAX_LSP_MESSAGE_BYTES);
        assert!(result.is_ok(), "expected a Content-Length exactly at the cap to be accepted, got {:?}", result);
    }

    #[test]
    fn test_hover_not_advertised_without_a_handler() {
        let result = initialize_result();
        assert_eq!(
            result["capabilities"]["hoverProvider"],
            serde_json::json!(false),
            "hoverProvider must stay false until a textDocument/hover handler exists — a client hover \
             request carries an id and nothing replies to it today"
        );
    }

    #[test]
    fn test_headerless_frame_is_reported_as_malformed_not_eof() {
        // Blank line reached with no Content-Length header at all — the
        // pre-fix code left `content_length: None` and `continue`d straight
        // past the body that frame's sender still wrote, permanently
        // desyncing every header block read afterward.
        let mut reader = std::io::Cursor::new(b"X-Some-Other-Header: value\r\n\r\n".to_vec());
        assert_eq!(read_content_length(&mut reader), Ok(None));
    }

    #[test]
    fn test_unparsable_content_length_is_reported_as_malformed() {
        let mut reader = std::io::Cursor::new(b"Content-Length: not-a-number\r\n\r\n".to_vec());
        assert_eq!(read_content_length(&mut reader), Ok(None));
    }

    #[test]
    fn test_true_stream_eof_is_distinct_from_a_malformed_header_block() {
        // No bytes at all (client closed the connection) is a normal,
        // expected way to end the loop — must not be conflated with a
        // header block that was read but had no usable Content-Length.
        let mut reader = std::io::Cursor::new(Vec::new());
        assert_eq!(read_content_length(&mut reader), Err(()));
    }

    // ── SPEC-CLI-001c AC-005b: a connection that closes after one or more
    // complete header lines but BEFORE the blank-line terminator is reached
    // is currently treated identically to a clean EOF (Err(())), not as a
    // malformed header block (Ok(None)) — the blank line never arrives, so
    // read_line's next call returns 0 bytes while still inside the header
    // loop, same as AC-005's true-EOF case.

    #[test]
    fn test_connection_closed_mid_header_before_blank_line_is_treated_as_eof() {
        let mut reader = std::io::Cursor::new(b"Content-Length: 10\r\n".to_vec());
        assert_eq!(read_content_length(&mut reader), Err(()));
    }

    // ── SPEC-CLI-001c AC-005d: non-UTF8 bytes in the header block map to the
    // same Err(()) EOF-like outcome, since read_line's io::Error is folded
    // into unwrap_or(0) — indistinguishable from a clean disconnect.

    #[test]
    fn test_non_utf8_header_bytes_are_treated_as_eof() {
        let mut reader = std::io::Cursor::new(b"Content-Length: 10\xFF\r\n\r\n".to_vec());
        assert_eq!(read_content_length(&mut reader), Err(()));
    }

    // ── A broken client pipe (or any writer failure) during send_response
    // must not vanish silently — see the fix's doc comment. A failing writer
    // proves send_response doesn't panic; a capturing tracing subscriber
    // proves the failure actually reaches the log, not just "doesn't crash."

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "simulated broken pipe"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "simulated broken pipe"))
        }
    }

    #[derive(Clone, Default)]
    struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap_or_else(|p| p.into_inner()).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn send_response_write_failure_does_not_panic_and_is_logged() {
        let captured = CapturedLogs::default();
        let captured_for_writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || captured_for_writer.clone())
            .with_ansi(false)
            .without_time()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let mut writer = FailingWriter;
            send_response(&mut writer, &serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": null}));
        });

        let logged = String::from_utf8(captured.0.lock().unwrap_or_else(|p| p.into_inner()).clone()).unwrap();
        assert!(
            logged.contains("Failed to write LSP response frame"),
            "expected the write failure to be logged, got {logged:?}"
        );
        assert!(
            logged.contains("simulated broken pipe"),
            "expected the underlying io::Error to be included, got {logged:?}"
        );
    }

    #[test]
    fn send_response_succeeds_normally_with_a_working_writer() {
        // Regression guard: the fix must not change the success path's
        // behavior — a working writer still receives the full framed response.
        let mut buf: Vec<u8> = Vec::new();
        send_response(&mut buf, &serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": null}));
        let written = String::from_utf8(buf).unwrap();
        assert!(written.starts_with("Content-Length:"), "expected a framed response, got {written:?}");
        assert!(written.contains("\"result\":null"), "expected the real response body, got {written:?}");
    }
}
