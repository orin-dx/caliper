//! The checker a run asks: the `tsrs` crates linked into this process, asked through `tsrs_api::Session` with the
//! methods and JSON of `tsc --api` ([`crate::rpc`]).
//!
//! The linked checker answers on threads of its own with a 512 MiB stack, the stack the `tsrs` binary serves on. A
//! query runs on the thread that calls it, and a type nested 2,000 deep overflows a default 2 MiB thread: a stack
//! overflow aborts the whole process and cannot be caught. Callers on any thread send it requests over a channel, one
//! per checker thread (a lane), and each thread answers its own one at a time, in the order they arrive; a run that asks
//! several programs at once sends each program's requests down a lane of its own ([`Client::lane`]). A panic inside the
//! checker comes back as an error to the request that caused it (`tsrs_api` catches it), and the session keeps
//! answering.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use camino::Utf8Path;
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::sync::oneshot;
use tsrs_api::{Handler, Response, Session, SessionOptions};
use tsrs_vfs::{FS, bundled, osvfs};

/// The stack of the thread the linked checker answers on: what the `tsrs` binary gives each of its threads.
const CHECKER_STACK_BYTES: usize = 512 << 20;

/// Why the checker did not answer a request.
#[derive(Debug)]
pub(crate) enum RequestError {
    /// The checker answered with an error (a panic inside it among them): this request failed, and the session keeps
    /// answering.
    Rejected(String),
    /// No answer came within the deadline.
    Timeout { method: String, after: Duration },
    /// The checker was stopped, or its session could not be created.
    Closed,
    /// The request could not be written as JSON, or the answer is not JSON.
    Malformed(String),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::Rejected(message) | RequestError::Malformed(message) => f.write_str(message),
            RequestError::Timeout { method, after } => write!(f, "request {method:?} timed out after {after:?}"),
            RequestError::Closed => f.write_str("session shut down"),
        }
    }
}

/// The linked checker: the threads that share its session, and the way to them.
pub(crate) struct Backend {
    client: Client,
    thread: JoinHandle<()>,
    threads: usize,
}

enum Job {
    Request(Asked),
    /// Close the session and end the thread; requests sent after it fail as from a stopped checker.
    Stop,
}

struct Asked {
    method: String,
    params: Vec<u8>,
    reply: oneshot::Sender<Result<String, String>>,
}

/// A handle that sends requests to a running checker: the sending ends of its queues, one per thread, and the one this
/// client sends to. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Client {
    lanes: Arc<[mpsc::Sender<Job>]>,
    lane: usize,
}

impl Backend {
    /// Spawns the checker thread, which creates its session there: the session's work, from the first request on,
    /// runs on that thread's stack. With `threads` above one it starts as many more, each answering a queue of its own
    /// (a lane, [`Client::lane`]) on the same session: requests about one program wait for each other (each program
    /// has one checker, which `tsrs_api` locks for the length of a request), so a run that sends the requests about
    /// each of its programs down a lane of its own has them answered at once. A thread that cannot be spawned is an
    /// error that says so.
    pub(crate) fn start(project_root: &Utf8Path, threads: usize) -> Result<Backend, String> {
        let (lanes, queues): (Vec<_>, Vec<_>) = (0..threads.max(1)).map(|_| mpsc::channel()).unzip();
        let threads = lanes.len();
        let cwd = project_root.to_string();
        let thread = std::thread::Builder::new()
            .name("caliper-checker".to_string())
            .stack_size(CHECKER_STACK_BYTES)
            .spawn(move || serve(cwd, queues))
            .map_err(|error| format!("could not start the checker thread: {error}"))?;
        Ok(Backend { client: Client { lanes: lanes.into(), lane: 0 }, thread, threads })
    }

    /// How many programs a run asks at once: one per thread.
    pub(crate) fn parallel_programs(&self) -> usize {
        self.threads
    }

    pub(crate) fn client(&self) -> Client {
        self.client.clone()
    }

    pub(crate) fn is_alive(&self) -> bool {
        !self.thread.is_finished()
    }

    /// Stops the checker. Its threads finish the request they are in, close the session and exit on their own: they
    /// are not waited for, so a hung request cannot hold the caller.
    pub(crate) async fn shutdown(self) {
        self.client.stop();
    }
}

impl Client {
    /// The client whose requests the `lane`th thread of the checker answers ([`Backend::start`]).
    pub(crate) fn lane(&self, lane: usize) -> Client {
        Client { lanes: self.lanes.clone(), lane }
    }

    /// One request, its answer the `result` as the JSON text it arrived as. The wait is bounded by `deadline`.
    pub(crate) async fn request_raw(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Box<RawValue>, RequestError> {
        let params = serde_json::to_vec(&params)
            .map_err(|error| RequestError::Malformed(format!("failed to serialize a request: {error}")))?;
        let (reply, answer) = oneshot::channel();
        let asked = Asked { method: method.to_string(), params, reply };
        let Some(jobs) = self.lanes.get(self.lane % self.lanes.len().max(1)) else {
            return Err(RequestError::Closed);
        };
        if jobs.send(Job::Request(asked)).is_err() {
            return Err(RequestError::Closed);
        }
        let answered = match tokio::time::timeout(deadline, answer).await {
            Ok(Ok(answered)) => answered,
            // The thread ended without answering: stopped, or its session could not be created.
            Ok(Err(_dropped)) => return Err(RequestError::Closed),
            Err(_elapsed) => return Err(RequestError::Timeout { method: method.to_string(), after: deadline }),
        };
        match answered {
            Ok(text) => RawValue::from_string(text)
                .map_err(|_invalid| RequestError::Malformed(format!("{method} answered with text that is not JSON"))),
            Err(message) => Err(RequestError::Rejected(message)),
        }
    }

    fn stop(&self) {
        // A thread that already ended has nothing to stop.
        for jobs in self.lanes.iter() {
            jobs.send(Job::Stop).ok();
        }
    }
}

/// The checker threads: one session, each thread answering the requests of its queue in turn until it is stopped or
/// every sender is gone. The first thread creates the session and starts the others; the last to leave closes it.
fn serve(cwd: String, queues: Vec<mpsc::Receiver<Job>>) {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let session = Session::new(SessionOptions::new(cwd, bundled::lib_path(), fs, false));
    let left = Arc::new(AtomicUsize::new(queues.len()));
    let mut queues = queues.into_iter();
    let Some(own) = queues.next() else { return session.close() };
    for (index, queue) in queues.enumerate() {
        let (session, still) = (session.clone(), left.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("caliper-checker-{}", index + 1))
            .stack_size(CHECKER_STACK_BYTES)
            .spawn(move || answer(&session, queue, &still));
        // Its queue is dropped with the closure, so its lane's requests fail as from a stopped checker.
        if spawned.is_err() {
            left.fetch_sub(1, Ordering::AcqRel);
        }
    }
    answer(&session, own, &left);
}

/// One checker thread: the requests of `queue` in turn, until it is stopped or every sender is gone. The last thread
/// of the session (`left`) closes it.
fn answer(session: &Session, queue: mpsc::Receiver<Job>, left: &AtomicUsize) {
    for job in queue {
        let asked = match job {
            Job::Request(asked) => asked,
            Job::Stop => break,
        };
        // A caller that stopped waiting (its deadline passed, its run ended) is not answered.
        if asked.reply.is_closed() {
            continue;
        }
        let answer = match session.handle_request(&asked.method, &asked.params) {
            Ok(Response::Json(text)) => Ok(text),
            Ok(Response::Binary(_)) => Err(format!("{} answered in binary, not JSON", asked.method)),
            Err(error) => Err(error.to_string()),
        };
        asked.reply.send(answer).ok();
    }
    if left.fetch_sub(1, Ordering::AcqRel) == 1 {
        session.close();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use serde_json::json;

    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn linked() -> Backend {
        Backend::start(Utf8Path::new("/"), 1).unwrap()
    }

    async fn ask(client: &Client, method: &str, params: Value) -> Result<Value, RequestError> {
        let answer = client.request_raw(method, params, Duration::from_secs(120)).await?;
        Ok(serde_json::from_str(answer.get()).unwrap())
    }

    /// A program of `file` holding `text`, held in the checker's memory: its snapshot and project.
    async fn program(client: &Client, file: &str, text: &str, resolution: u32) -> Result<Value, RequestError> {
        let options =
            json!({ "strict": true, "noEmit": true, "target": 9, "module": 99, "moduleResolution": resolution });
        let created = ask(
            client,
            "createSnapshot",
            json!({
                "fileSystem": { "kind": "layer", "files": { file: text } },
                "createPrograms": [{ "rootFiles": [file], "compilerOptions": options }],
            }),
        )
        .await?;
        Ok(json!({ "snapshot": created["snapshot"], "project": created["projects"][0]["id"] }))
    }

    /// `Deep` nested `depth` levels (`{ a: { a: ... string ... } }`), printed in full by the checker.
    async fn printed_deep(client: &Client, depth: usize) -> String {
        let file = "/virtual/deep.ts";
        let text = format!("export type Deep = {}string{};\n", "{ a: ".repeat(depth), " }".repeat(depth));
        let scope = program(client, file, &text, 100).await.unwrap();
        let with = |extra: Value| {
            let mut params = scope.clone();
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            params
        };
        let module = ask(client, "getSymbolOfSourceFile", with(json!({ "file": file }))).await.unwrap();
        let exports = ask(client, "getExportsOfModule", with(json!({ "symbol": module["reference"] }))).await.unwrap();
        let deep = exports.as_array().unwrap().iter().find(|export| export["name"] == "Deep").unwrap();
        let declared =
            ask(client, "getDeclaredTypeOfSymbol", with(json!({ "symbol": deep["reference"] }))).await.unwrap();
        let flags = crate::rpc::flag::PRINT_EXPANDED;
        let printed =
            ask(client, "typeToString", with(json!({ "type": declared["id"], "flags": flags }))).await.unwrap();
        printed.as_str().unwrap().to_string()
    }

    fn expected_deep(depth: usize) -> String {
        format!("{}string{}", "{ a: ".repeat(depth), "; }".repeat(depth))
    }

    #[test]
    fn a_panic_inside_the_checker_is_an_error_to_that_request_and_the_session_keeps_answering() {
        let runtime = runtime();
        let checker = linked();
        let client = checker.client();
        let file = "/virtual/panic.ts";
        let text = "import x from \"./other\";\nexport const z = x;\n";

        // A `moduleResolution` that names no kind reaches the resolver's panic, as it does in the Go checker.
        let panicked = runtime.block_on(program(&client, file, text, 12345));
        let after = runtime.block_on(program(&client, file, text, 100));

        match panicked {
            Err(RequestError::Rejected(message)) => {
                assert_eq!(
                    message,
                    "api: client error: unsupported moduleResolution value 12345 (not a ModuleResolutionKind)"
                );
            }
            other => panic!("expected the panic as an error, got {other:?}"),
        }
        let after = after.unwrap();
        assert!(after["snapshot"].as_u64().is_some() && after["project"].as_str().is_some(), "{after}");
        assert!(checker.is_alive());
        runtime.block_on(checker.shutdown());
    }

    /// A type nested 2,000 deep overflows a default 2 MiB thread and aborts the process; the checker thread's stack
    /// holds it.
    #[test]
    fn a_type_nested_two_thousand_deep_is_printed_in_full() {
        let runtime = runtime();
        let checker = linked();

        let printed = runtime.block_on(printed_deep(&checker.client(), 2_000));

        assert_eq!(printed, expected_deep(2_000));
        runtime.block_on(checker.shutdown());
    }

    /// Ten thousand deep takes about 3 s in a release build and far longer in a debug one:
    /// `cargo test --release -p caliper-docgen-semantic --lib checker -- --ignored`.
    #[test]
    #[ignore = "slow in a debug build; run in release"]
    fn a_type_nested_ten_thousand_deep_is_printed_in_full() {
        let runtime = runtime();
        let checker = linked();

        let printed = runtime.block_on(printed_deep(&checker.client(), 10_000));

        assert_eq!(printed, expected_deep(10_000));
        runtime.block_on(checker.shutdown());
    }

    #[test]
    fn a_request_past_its_deadline_times_out_and_the_next_is_answered() {
        let runtime = runtime();
        let checker = linked();
        let client = checker.client();
        let text = format!("export type Deep = {}string{};\n", "{ a: ".repeat(500), " }".repeat(500));
        let params = json!({
            "fileSystem": { "kind": "layer", "files": { "/virtual/slow.ts": text } },
            "createPrograms": [{ "rootFiles": ["/virtual/slow.ts"], "compilerOptions": { "noEmit": true } }],
        });

        let late = runtime.block_on(client.request_raw("createSnapshot", params, Duration::from_micros(1)));
        let next = runtime.block_on(program(&client, "/virtual/next.ts", "export const n = 1;\n", 100));

        match late {
            Err(RequestError::Timeout { method, after }) => {
                assert_eq!((method.as_str(), after), ("createSnapshot", Duration::from_micros(1)));
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(next.unwrap()["snapshot"].as_u64().is_some());
        runtime.block_on(checker.shutdown());
    }

    #[test]
    fn a_stopped_checker_answers_every_later_request_as_closed() {
        let runtime = runtime();
        let checker = linked();
        let client = checker.client();

        runtime.block_on(checker.shutdown());
        let after = runtime.block_on(client.request_raw("initialize", Value::Null, Duration::from_secs(5)));

        assert!(matches!(after, Err(RequestError::Closed)), "{after:?}");
    }
}
