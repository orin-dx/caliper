#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A fake `tsc --api` that answers from a recording of the real `7.1.0-beta`: every request is matched, by method and
//! exact params, against the `{method, params, result}` entries of the JSON file named by `REPLAY_FIXTURE`. A request
//! with no recorded answer gets a JSON-RPC error, so a change to the calls the backend makes fails its test instead of
//! being silently answered. `REPLAY_SCENARIO=crash` exits at once, `hang` answers nothing.
//!
//! Paths in the recording are written relative to the project: the project root (`--cwd`) is `<ROOT>`, and its
//! lower-cased form, which the checker uses as a file's `path` key, is `<root>`. The root must contain an upper-case
//! letter, or the two spellings collide.
//!
//! Refresh the recording with `SemanticRequest::record_to` against a real install (see `tests/fixtures/README.md`).
//! Blocking std I/O on purpose: it is a test fixture, and it keeps the binary free of the async dev-dependencies a
//! `[[bin]]` cannot use.

use std::io::{BufRead, Read, Write};

fn main() {
    match std::env::var("REPLAY_SCENARIO").as_deref() {
        Ok("crash") => std::process::exit(3),
        Ok("hang") => {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            return;
        }
        _ => {}
    }
    let root = std::env::args().skip_while(|arg| arg != "--cwd").nth(1).unwrap_or_default();
    let to_wire = |text: String| text.replace("<ROOT>", &root).replace("<root>", &root.to_lowercase());
    let from_wire = |text: String| text.replace(&root, "<ROOT>").replace(&root.to_lowercase(), "<root>");
    let fixture = std::env::var("REPLAY_FIXTURE").expect("REPLAY_FIXTURE names the recording");
    let recorded: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if stdin.read_line(&mut line).unwrap() == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("a Content-Length header")];
        stdin.read_exact(&mut body).unwrap();

        let request: serde_json::Value = serde_json::from_str(&from_wire(String::from_utf8(body).unwrap())).unwrap();
        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(serde_json::Value::Null);
        let answer = recorded.iter().find(|entry| {
            entry.get("method").and_then(|m| m.as_str()) == Some(method) && entry.get("params") == Some(&params)
        });
        let response = match answer {
            Some(entry) => serde_json::json!({ "id": id, "result": entry["result"] }),
            None => serde_json::json!({
                "id": id,
                "error": { "code": -32601, "message": format!("no recorded answer for {method} {params}") }
            }),
        };
        let bytes = to_wire(serde_json::to_string(&response).unwrap()).into_bytes();
        write!(stdout, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
        stdout.write_all(&bytes).unwrap();
        stdout.flush().unwrap();
    }
}
