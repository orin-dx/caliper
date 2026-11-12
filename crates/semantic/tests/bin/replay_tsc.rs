#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! A fake `tsc --api` that answers from a recording of the real `7.1.0-beta`: every request is matched, by method and
//! exact params, against the `{method, params, result}` entries of the JSON file named by `REPLAY_FIXTURE`; a batch the
//! recording does not hold whole is answered request by request (see `batch_from_parts`). A request with no recorded
//! answer gets a JSON-RPC error, so a change to the calls the backend makes fails its test instead of
//! being silently answered. `REPLAY_SCENARIO=crash` exits at once, `hang` answers nothing, `invalid_json` answers every request after `initialize` with a
//! well-framed body that is not JSON.
//!
//! `REPLAY_LOG` names a file that gets every request, one JSON object per line, as the backend sent it. `REPLAY_HOLD`
//! names a file the fake waits for before it answers anything, so a test can queue requests while the checker is busy.
//!
//! Paths in the recording are written relative to the project: the project root (`--cwd`) is `<ROOT>`, and its
//! lower-cased form, which the checker uses as a file's `path` key, is `<root>`. The root must contain an upper-case
//! letter, or the two spellings collide.
//!
//! Refresh the recording with `SemanticRequest::record_to` against a real install (see `tests/fixtures/README.md`).
//! Blocking std I/O on purpose: it is a test fixture, and it keeps the binary free of the async dev-dependencies a
//! `[[bin]]` cannot use.

use std::io::{BufRead, Read, Write};

use serde_json::{Value, json};

/// The answer to a batch the recording does not hold as a whole, put together from the answers to each of its requests:
/// alone, or inside a recorded batch (a page of one starts where the page before it ended). The backend shares what a
/// snapshot was already told between components that are answered concurrently, so which requests one batch holds
/// depends on their order, while each request and its answer do not. The entry it returns has the recorded shape
/// (`result`); `None` when any request has no recorded answer.
fn batch_from_parts(recorded: &[Value], params: &Value) -> Option<Value> {
    let mut answered: Vec<(Value, Value)> = Vec::new();
    let mut offsets: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in recorded {
        let method = entry.get("method").and_then(Value::as_str);
        if method != Some("batchRequests") {
            if let (Some(method), Some(params), Some(result)) = (method, entry.get("params"), entry.get("result")) {
                answered.push((json!({ "method": method, "params": params }), result.clone()));
            }
            continue;
        }
        let (Some(requests), Some(responses)) = (
            entry.pointer("/params/requests").and_then(Value::as_array),
            entry.pointer("/result/responses").and_then(Value::as_array),
        ) else {
            continue;
        };
        let offset = offsets.entry(serde_json::to_string(requests).unwrap()).or_default();
        for (request, response) in requests.iter().skip(*offset).zip(responses) {
            answered.push((request.clone(), response.get("result").cloned().unwrap_or(Value::Null)));
        }
        *offset += responses.len();
    }
    let responses = params
        .get("requests")?
        .as_array()?
        .iter()
        .map(|request| {
            answered.iter().find(|(asked, _)| asked == request).map(|(_, result)| json!({ "result": result }))
        })
        .collect::<Option<Vec<Value>>>()?;
    Some(json!({ "result": { "responses": responses } }))
}

fn main() {
    match std::env::var("REPLAY_SCENARIO").as_deref() {
        Ok("crash") => std::process::exit(3),
        Ok("hang") => {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            return;
        }
        _ => {}
    }
    if let Ok(hold) = std::env::var("REPLAY_HOLD") {
        while !std::path::Path::new(&hold).exists() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let invalid_json = std::env::var("REPLAY_SCENARIO").as_deref() == Ok("invalid_json");
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
        if let Ok(log) = std::env::var("REPLAY_LOG") {
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
            writeln!(file, "{}", to_wire(serde_json::to_string(&request).unwrap())).unwrap();
        }
        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(serde_json::Value::Null);
        let answer = recorded
            .iter()
            .find(|entry| {
                entry.get("method").and_then(|m| m.as_str()) == Some(method) && entry.get("params") == Some(&params)
            })
            .cloned()
            .or_else(|| (method == "batchRequests").then(|| batch_from_parts(&recorded, &params)).flatten());
        if invalid_json && method != "initialize" {
            let bytes = b"{ this is not json";
            write!(stdout, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
            stdout.write_all(bytes).unwrap();
            stdout.flush().unwrap();
            continue;
        }
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
