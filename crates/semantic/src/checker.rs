//! The checker a run asks, and the one place that picks which: the `tsrs` crates linked into this process, or a
//! `tsc --api` process when an explicit `tsc_path` names one (the Go `typescript@7.1` the tests and the parity gate
//! run, or a `tsrs` binary). Both answer the same methods with the same JSON, so [`crate::rpc`] does not know which.
//!
//! The linked checker answers on one thread of its own with a 512 MiB stack, the stack the `tsrs` binary serves on. A
//! query runs on the thread that calls it, and a type nested 2,000 deep overflows a default 2 MiB thread: a stack
//! overflow aborts the whole process and cannot be caught. Callers on any thread send it requests over a channel, and
//! it answers them one at a time, in the order they arrive. A panic inside the checker comes back as an error to the
//! request that caused it (`tsrs_api` catches it), and the session keeps answering.

use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use caliper_docgen_tsc_client::{MalformedFrameReason, Supervisor, SupervisorConfig, TscClientError};
use camino::Utf8Path;
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::sync::oneshot;
use tsrs_api::{Handler, Response, Session, SessionOptions};
use tsrs_vfs::{FS, bundled, osvfs};

use crate::locate;

/// The stack of the thread the linked checker answers on: what the `tsrs` binary gives each of its threads.
const CHECKER_STACK_BYTES: usize = 512 << 20;

/// The JSON-RPC code the `tsrs` and `tsc` processes answer a failed request with (`jsonrpc.CodeInternalError`); the
/// message is the error's text, as here.
const INTERNAL_ERROR: i64 = -32603;

/// How to start a checker: an explicit binary runs as a process, none links the bundled one.
pub(crate) struct Start<'a> {
    pub(crate) project_root: &'a Utf8Path,
    pub(crate) tsc_path: Option<&'a Utf8Path>,
    pub(crate) expected_versions: &'static [&'static str],
    pub(crate) timeout: Duration,
    pub(crate) extra_env: &'a [(String, String)],
}

/// A running checker.
pub(crate) enum Backend {
    /// A `tsc --api` process for an explicit binary.
    Process(Box<Supervisor>),
    /// The checker linked into this process, on its own thread.
    InProcess(InProcess),
}

impl Backend {
    /// The checker `start` asks for. An explicit path that does not exist, a binary that does not start or is not a
    /// supported version, or a checker thread that cannot be spawned, is an error that says so.
    pub(crate) async fn start(start: Start<'_>) -> Result<Backend, String> {
        let Some(tsc_path) = start.tsc_path else {
            return InProcess::start(start.project_root).map(Backend::InProcess);
        };
        let tsc_path = locate::explicit_tsc(tsc_path)?;
        let mut config = SupervisorConfig::new(tsc_path, start.project_root.to_owned(), start.expected_versions);
        config.handshake_timeout = start.timeout;
        config.extra_env = start.extra_env.to_vec();
        Supervisor::spawn(config)
            .await
            .map(|supervisor| Backend::Process(Box::new(supervisor)))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn client(&self) -> Client {
        match self {
            Backend::Process(supervisor) => Client::Process(supervisor.client()),
            Backend::InProcess(checker) => Client::InProcess(checker.client.clone()),
        }
    }

    pub(crate) fn is_alive(&self) -> bool {
        match self {
            Backend::Process(supervisor) => supervisor.is_alive(),
            Backend::InProcess(checker) => !checker.thread.is_finished(),
        }
    }

    /// The operating system's id of the checker process; `None` for the linked checker, which is this process.
    pub(crate) fn process_id(&self) -> Option<u32> {
        match self {
            Backend::Process(supervisor) => supervisor.process_id(),
            Backend::InProcess(_) => None,
        }
    }

    /// Stops the checker. A process is reaped. The linked checker's thread finishes the request it is in, closes its
    /// session and exits on its own: it is not waited for, so a hung request cannot hold the caller.
    pub(crate) async fn shutdown(self) {
        match self {
            Backend::Process(supervisor) => {
                if let Err(error) = (*supervisor).shutdown().await {
                    tracing::debug!(%error, "the checker process did not shut down cleanly");
                }
            }
            Backend::InProcess(checker) => checker.client.stop(),
        }
    }
}

/// A handle that sends requests to a running checker. Cheap to clone.
#[derive(Clone)]
pub(crate) enum Client {
    Process(caliper_docgen_tsc_client::Client),
    InProcess(Linked),
}

impl Client {
    /// One request, its answer the `result` as the JSON text it arrived as. The wait is bounded by `deadline`; an
    /// error is the one the process route would give for the same failure.
    pub(crate) async fn request_raw(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Box<RawValue>, TscClientError> {
        match self {
            Client::Process(client) => Ok(client.request_raw(method, params, deadline).await?.result),
            Client::InProcess(linked) => linked.request(method, params, deadline).await,
        }
    }
}

/// The linked checker: the thread that owns its session, and the way to it.
pub(crate) struct InProcess {
    client: Linked,
    thread: JoinHandle<()>,
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

/// The sending end of the linked checker's queue.
#[derive(Clone)]
pub(crate) struct Linked {
    jobs: mpsc::Sender<Job>,
}

impl InProcess {
    /// Spawns the checker thread, which creates its session there: the session's work, from the first request on,
    /// runs on that thread's stack.
    pub(crate) fn start(project_root: &Utf8Path) -> Result<InProcess, String> {
        let (jobs, queue) = mpsc::channel();
        let cwd = project_root.to_string();
        let thread = std::thread::Builder::new()
            .name("caliper-checker".to_string())
            .stack_size(CHECKER_STACK_BYTES)
            .spawn(move || serve(cwd, queue))
            .map_err(|error| format!("could not start the checker thread: {error}"))?;
        Ok(InProcess { client: Linked { jobs }, thread })
    }
}

impl Linked {
    async fn request(&self, method: &str, params: Value, deadline: Duration) -> Result<Box<RawValue>, TscClientError> {
        let params = serde_json::to_vec(&params).map_err(|error| TscClientError::Serialize(Arc::new(error)))?;
        let (reply, answer) = oneshot::channel();
        let asked = Asked { method: method.to_string(), params, reply };
        if self.jobs.send(Job::Request(asked)).is_err() {
            return Err(TscClientError::SessionClosed);
        }
        let answered = match tokio::time::timeout(deadline, answer).await {
            Ok(Ok(answered)) => answered,
            // The thread ended without answering: stopped, or its session could not be created.
            Ok(Err(_dropped)) => return Err(TscClientError::SessionClosed),
            Err(_elapsed) => return Err(TscClientError::Timeout { method: method.to_string(), after: deadline }),
        };
        match answered {
            Ok(text) => RawValue::from_string(text)
                .map_err(|_invalid| TscClientError::MalformedFrame { reason: MalformedFrameReason::InvalidJson }),
            Err(message) => Err(TscClientError::RpcError { code: INTERNAL_ERROR, message, data: None }),
        }
    }

    fn stop(&self) {
        // A thread that already ended has nothing to stop.
        self.jobs.send(Job::Stop).ok();
    }
}

/// The checker thread: one session, answering each request in turn until it is stopped or every sender is gone.
fn serve(cwd: String, queue: mpsc::Receiver<Job>) {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let session = Session::new(SessionOptions::new(cwd, bundled::lib_path(), fs, false));
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
    session.close();
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
        Backend::InProcess(InProcess::start(Utf8Path::new("/")).unwrap())
    }

    async fn ask(client: &Client, method: &str, params: Value) -> Result<Value, TscClientError> {
        let answer = client.request_raw(method, params, Duration::from_secs(120)).await?;
        Ok(serde_json::from_str(answer.get()).unwrap())
    }

    /// A program of `file` holding `text`, held in the checker's memory: its snapshot and project.
    async fn program(client: &Client, file: &str, text: &str, resolution: u32) -> Result<Value, TscClientError> {
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
    fn no_tsc_path_starts_the_linked_checker_and_an_explicit_one_is_checked_for_a_process() {
        let runtime = runtime();
        let start = |tsc_path: Option<&'static str>| Start {
            project_root: Utf8Path::new("/"),
            tsc_path: tsc_path.map(Utf8Path::new),
            expected_versions: &["7.1"],
            timeout: Duration::from_secs(5),
            extra_env: &[],
        };

        let linked = runtime.block_on(Backend::start(start(None))).unwrap();
        let missing = runtime.block_on(Backend::start(start(Some("/nowhere/tsc"))));

        assert!(matches!(linked, Backend::InProcess(_)));
        assert_eq!(linked.process_id(), None);
        assert_eq!(missing.err(), locate::explicit_tsc(Utf8Path::new("/nowhere/tsc")).err());
        runtime.block_on(linked.shutdown());
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
            Err(TscClientError::RpcError { code, message, data }) => {
                assert_eq!(code, INTERNAL_ERROR);
                assert_eq!(
                    message,
                    "api: client error: unsupported moduleResolution value 12345 (not a ModuleResolutionKind)"
                );
                assert_eq!(data, None);
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
            Err(TscClientError::Timeout { method, after }) => {
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

        assert!(matches!(after, Err(TscClientError::SessionClosed)), "{after:?}");
    }
}
