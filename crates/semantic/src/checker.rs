//! The checker a run asks, and the one place that picks which: the `tsrs` crates linked into this process, or a
//! `tsc --api` process when an explicit `tsc_path` names one (the Go `typescript@7.1` the tests and the parity gate
//! run, or a `tsrs` binary). Both answer the same methods with the same JSON, so [`crate::rpc`] does not know which.
//!
//! The linked checker answers on one thread of its own with a 512 MiB stack, the stack the `tsrs` binary serves on. A
//! query runs on the thread that calls it, and a type nested 2,000 deep overflows a default 2 MiB thread: a stack
//! overflow aborts the whole process and cannot be caught. Callers on any thread send it requests over a channel, and
//! it answers them one at a time, in the order they arrive.
//!
//! It is guarded so that it does not take its host (Node, Storybook) down with it:
//!
//! - **Panics.** A panic inside the checker comes back as an error to the request that caused it, and the thread then
//!   closes its session and ends, as a crashed process would: a session a panic unwound through is not asked again, so
//!   no later answer comes from state it left half-built. The caller starts a fresh one ([`crate::on_demand`] does, up
//!   to its restart limit). A panic `tsrs` itself reports as the client's fault (an unknown `moduleResolution`) is an
//!   error like any other, and the session keeps answering.
//! - **Nesting.** `tsrs` parses on threads of its own with a 256 MiB stack, which a file nested a few hundred thousand
//!   brackets deep overflows (measured on macOS arm64: an object type 200,000 deep parses, 300,000 aborts). Every file
//!   the checker reads from the disk is measured first ([`nesting`]), and a request that read one nested deeper than
//!   [`MAX_NESTING`] fails with an error that names it, so its components keep their structural answer.
//! - **Memory.** [`Backend::memory_mib`] reads what the linked checker holds: the resident part of the address range
//!   `tsrs` allocates its arenas from, which leaves out the host's own memory. [`crate::on_demand`] bounds it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use caliper_docgen_tsc_client::{MalformedFrameReason, Supervisor, SupervisorConfig, TscClientError};
use camino::Utf8Path;
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::sync::oneshot;
use tsrs_api::{ErrorKind, Handler, Response, Session, SessionOptions};
use tsrs_vfs::{FS, bundled, osvfs, wrapvfs};

use crate::locate;

/// The stack of the thread the linked checker answers on: what the `tsrs` binary gives each of its threads.
const CHECKER_STACK_BYTES: usize = 512 << 20;

/// The deepest bracket nesting a file may have for the linked checker to read it: twice the 10,000 levels the deep-type
/// tests print in full, and a tenth of the 200,000 an object type was measured to parse at on `tsrs`'s 256 MiB parse
/// threads (300,000 overflows them, which aborts the process).
pub(crate) const MAX_NESTING: usize = 20_000;

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

    /// Whether the checker still answers. The linked checker stops once a panic has unwound through its session,
    /// before its thread has finished closing it.
    pub(crate) fn is_alive(&self) -> bool {
        match self {
            Backend::Process(supervisor) => supervisor.is_alive(),
            Backend::InProcess(checker) => !checker.ended.load(Ordering::Acquire) && !checker.thread.is_finished(),
        }
    }

    /// Whether this is the checker linked into this process.
    pub(crate) fn is_linked(&self) -> bool {
        matches!(self, Backend::InProcess(_))
    }

    /// The operating system's id of the checker process; `None` for the linked checker, which is this process.
    pub(crate) fn process_id(&self) -> Option<u32> {
        match self {
            Backend::Process(supervisor) => supervisor.process_id(),
            Backend::InProcess(_) => None,
        }
    }

    /// What the checker holds, in MiB: a process's resident memory, or, for the linked checker, the resident part of
    /// the address range the `tsrs` crates allocate their arenas from. That range is theirs alone, so the host's memory
    /// is not counted, and a closed session gives its programs' part of it back. `None` when it cannot be read.
    pub(crate) fn memory_mib(&self) -> Option<u64> {
        match self {
            Backend::Process(supervisor) => supervisor.process_id().and_then(resident_mib),
            Backend::InProcess(_) => arena_mib(),
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

    /// Stops the checker and, for the linked one, waits up to `wait` for its thread to close the session, so what the
    /// session held is given back before a fresh one is read. For a replacement between requests, when nothing is in
    /// progress.
    pub(crate) async fn shutdown_within(self, wait: Duration) {
        match self {
            Backend::Process(_) => self.shutdown().await,
            Backend::InProcess(checker) => {
                checker.client.stop();
                // The thread drops its end once the session is closed.
                checker.closed.recv_timeout(wait).ok();
            }
        }
    }
}

/// The resident memory of a process in MiB: `/proc` where it exists, else `ps`. `None` when neither answers.
pub(crate) fn resident_mib(pid: u32) -> Option<u64> {
    let kib = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(status) => status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()?,
        Err(_) => {
            let output = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().ok()?;
            String::from_utf8_lossy(&output.stdout).trim().parse::<u64>().ok()?
        }
    };
    Some(kib / 1024)
}

/// The resident MiB of the address range every `tsrs` arena is carved from (`tsrs_core::reserve`, one range per
/// process): the nodes, symbols and types of the programs the linked checker holds. Reading it asks the kernel which
/// of the range's pages are resident (`mincore`), about 6 ms on macOS arm64. 0 before the checker has allocated.
#[cfg(unix)]
pub(crate) fn arena_mib() -> Option<u64> {
    let resident = tsrs_core::memsplit::resident(tsrs_core::reserve::base().addr(), tsrs_core::reserve::RESERVE);
    u64::try_from(resident >> 20).ok()
}

/// Elsewhere `tsrs` keeps plain pointers, with no one range to read.
#[cfg(not(unix))]
pub(crate) fn arena_mib() -> Option<u64> {
    None
}

/// The deepest bracket nesting of `text`: `(`, `[` and `{` counted together and `<` apart, each closer taking a level
/// off (never below zero). A nested type or expression written with brackets is a level of the parser's recursion, so
/// this bounds how deep a file makes it recurse. Brackets in strings and comments count too, which can only raise the
/// number. Angles are counted apart because the `>` of every `=>` closes one.
pub(crate) fn nesting(text: &str) -> usize {
    let (mut brackets, mut angles, mut deepest) = (0usize, 0usize, 0usize);
    for byte in text.bytes() {
        match byte {
            b'(' | b'[' | b'{' => {
                brackets += 1;
                deepest = deepest.max(brackets);
            }
            b')' | b']' | b'}' => brackets = brackets.saturating_sub(1),
            b'<' => {
                angles += 1;
                deepest = deepest.max(angles);
            }
            b'>' => angles = angles.saturating_sub(1),
            _ => {}
        }
    }
    deepest
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
    /// Set by the thread, before it answers, when it will answer nothing more: a panic unwound through its session.
    ended: Arc<AtomicBool>,
    /// Disconnected once the thread has closed its session.
    closed: mpsc::Receiver<()>,
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
        let cwd = project_root.to_string();
        InProcess::spawn(move || TsrsSession::open(cwd))
    }

    /// Spawns the checker thread over the session `open` makes on it.
    fn spawn<S: Served>(open: impl FnOnce() -> S + Send + 'static) -> Result<InProcess, String> {
        let (jobs, queue) = mpsc::channel();
        let ended = Arc::new(AtomicBool::new(false));
        let thread_ended = Arc::clone(&ended);
        let (closing, closed) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("caliper-checker".to_string())
            .stack_size(CHECKER_STACK_BYTES)
            .spawn(move || {
                serve(open, &queue, &thread_ended);
                drop(closing);
            })
            .map_err(|error| format!("could not start the checker thread: {error}"))?;
        Ok(InProcess { client: Linked { jobs }, thread, ended, closed })
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

/// Why the checker did not answer a request.
struct Failure {
    message: String,
    /// A panic unwound through the session, which is not asked again.
    panicked: bool,
}

/// What the checker thread asks: the `tsrs` session, or a stand-in in the tests.
trait Served {
    /// Answers one request with its JSON text.
    fn handle(&self, method: &str, params: &[u8]) -> Result<String, Failure>;
    fn close(&self);
}

/// A `tsrs` session over the disk, where every file is measured before the session reads it.
struct TsrsSession {
    session: Arc<Session>,
    /// The files nested past [`MAX_NESTING`] the session was refused since the last request began, with their depth.
    refused: Arc<Mutex<Vec<(String, usize)>>>,
}

impl TsrsSession {
    fn open(cwd: String) -> TsrsSession {
        let refused: Arc<Mutex<Vec<(String, usize)>>> = Arc::default();
        let noted = Arc::clone(&refused);
        let measured = wrapvfs::wrap(
            osvfs::fs(),
            wrapvfs::Replacements {
                read_file: Some(Box::new(move |path: &str| {
                    let text = osvfs::fs().read_file(path)?;
                    let depth = nesting(&text);
                    if depth <= MAX_NESTING {
                        return Some(text);
                    }
                    noted.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((path.to_string(), depth));
                    None
                })),
                ..wrapvfs::Replacements::default()
            },
        );
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(measured));
        let session = Session::new(SessionOptions::new(cwd, bundled::lib_path(), fs, false));
        TsrsSession { session, refused }
    }

    fn take_refused(&self) -> Vec<(String, usize)> {
        std::mem::take(&mut *self.refused.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl Served for TsrsSession {
    fn handle(&self, method: &str, params: &[u8]) -> Result<String, Failure> {
        self.take_refused();
        let answered = self.session.handle_request(method, params);
        if let Some((file, depth)) = self.take_refused().into_iter().next() {
            // The session answered without the file, which it took for missing: not an answer to give.
            let message = format!(
                "{file} nests brackets {depth} deep, more than the {MAX_NESTING} the built-in checker reads without \
                 risking a stack overflow; its components keep their structural answer"
            );
            return Err(Failure { message, panicked: false });
        }
        match answered {
            Ok(Response::Json(text)) => Ok(text),
            Ok(Response::Binary(_)) => {
                Err(Failure { message: format!("{method} answered in binary, not JSON"), panicked: false })
            }
            Err(error) => {
                let message = error.to_string();
                // `tsrs_api` catches a panic in a handler and reports it as an internal error that says so.
                let panicked = error.kind == ErrorKind::Internal && message.starts_with("panic: ");
                Err(Failure { message, panicked })
            }
        }
    }

    fn close(&self) {
        self.session.close();
    }
}

/// The checker thread: one session, answering each request in turn until it is stopped, every sender is gone, or a
/// panic unwound through the session. A panic is caught here too, should one escape `tsrs`'s own catch.
fn serve<S: Served>(open: impl FnOnce() -> S, queue: &mpsc::Receiver<Job>, ended: &AtomicBool) {
    let session = open();
    for job in queue {
        let asked = match job {
            Job::Request(asked) => asked,
            Job::Stop => break,
        };
        // A caller that stopped waiting (its deadline passed, its run ended) is not answered.
        if asked.reply.is_closed() {
            continue;
        }
        let answered =
            catch_unwind(AssertUnwindSafe(|| session.handle(&asked.method, &asked.params))).unwrap_or_else(|panic| {
                let text = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("a panic with no message");
                Err(Failure { message: format!("panic: {text}"), panicked: true })
            });
        match answered {
            Ok(text) => {
                asked.reply.send(Ok(text)).ok();
            }
            Err(Failure { message, panicked }) => {
                if panicked {
                    tracing::warn!(method = asked.method, %message, "the built-in checker panicked; it is replaced");
                    // Before the reply, so the caller never sends its next request to a session that will not answer.
                    ended.store(true, Ordering::Release);
                    asked.reply.send(Err(message)).ok();
                    break;
                }
                asked.reply.send(Err(message)).ok();
            }
        }
    }
    // A session a panic left half-built may panic again while it closes; the thread ends either way.
    catch_unwind(AssertUnwindSafe(|| session.close())).ok();
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

    /// A program of `file` as it is on the disk.
    async fn program_on_disk(client: &Client, file: &Utf8Path) -> Result<Value, TscClientError> {
        let created = ask(
            client,
            "createSnapshot",
            json!({ "createPrograms": [{ "rootFiles": [file], "compilerOptions": { "strict": true, "noEmit": true } }] }),
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

    /// `tsrs` reports this panic as the client's fault, so the session is kept.
    #[test]
    fn a_panic_tsrs_reports_as_the_clients_fault_is_an_error_and_the_session_keeps_answering() {
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

    /// Stands in for a session: answers `ping`, panics on `boom` (as an escaped panic), and reports a panic `tsrs`
    /// caught on `caught`. Counts how often it was closed.
    struct Fake {
        closed: Arc<std::sync::atomic::AtomicU32>,
    }

    impl Served for Fake {
        fn handle(&self, method: &str, _params: &[u8]) -> Result<String, Failure> {
            match method {
                "ping" => Ok("\"pong\"".to_string()),
                "boom" => panic!("checker invariant broken"),
                _ => Err(Failure { message: "panic: caught by tsrs".to_string(), panicked: true }),
            }
        }

        fn close(&self) {
            self.closed.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn fake() -> (Backend, Arc<std::sync::atomic::AtomicU32>) {
        let closed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let session = Fake { closed: Arc::clone(&closed) };
        (Backend::InProcess(InProcess::spawn(move || session).unwrap()), closed)
    }

    /// A panic, escaped or caught by `tsrs`, is an error to its request; the session is closed and not asked again,
    /// and the checker says at once that it no longer answers, so its owner starts a fresh one.
    #[test]
    fn a_panic_ends_the_session_after_answering_its_request_with_the_error() {
        let runtime = runtime();
        for (method, said) in [("boom", "panic: checker invariant broken"), ("caught", "panic: caught by tsrs")] {
            let (checker, closed) = fake();
            let client = checker.client();

            let before = runtime.block_on(ask(&client, "ping", Value::Null));
            let panicked = runtime.block_on(ask(&client, method, Value::Null));
            let alive = checker.is_alive();
            let after = runtime.block_on(ask(&client, "ping", Value::Null));

            assert_eq!(before.unwrap(), json!("pong"));
            match panicked {
                Err(TscClientError::RpcError { message, .. }) => assert_eq!(message, said),
                other => panic!("expected the panic as an error, got {other:?}"),
            }
            assert!(!alive, "{method}: the checker must not be asked again");
            assert!(matches!(after, Err(TscClientError::SessionClosed)), "{method}: {after:?}");
            let waited = std::time::Instant::now();
            while closed.load(Ordering::Relaxed) == 0 && waited.elapsed() < Duration::from_secs(5) {
                std::thread::yield_now();
            }
            assert_eq!(closed.load(Ordering::Relaxed), 1, "{method}");
        }
    }

    /// After a panic, the checker its owner starts in its place answers.
    #[test]
    fn a_fresh_linked_checker_answers_after_one_that_panicked() {
        let runtime = runtime();
        let (panicked, _closed) = fake();
        drop(runtime.block_on(ask(&panicked.client(), "boom", Value::Null)));
        assert!(!panicked.is_alive());

        let fresh = linked();
        let scope = runtime.block_on(program(&fresh.client(), "/virtual/after.ts", "export const n = 1;\n", 100));

        assert!(scope.unwrap()["snapshot"].as_u64().is_some());
        assert!(fresh.is_alive());
        runtime.block_on(fresh.shutdown());
    }

    #[test]
    fn nesting_counts_brackets_together_and_angles_apart() {
        assert_eq!(nesting(""), 0);
        assert_eq!(nesting("type A = { a: [ (string) ] }"), 3);
        assert_eq!(nesting("type B = Array<Array<string>>"), 2);
        // A `=>` closes an angle, never a bracket; a comparison's `<` opens one; a closer with nothing open counts nothing.
        assert_eq!(nesting("const f = (a) => (b) => { return a < b }"), 1);
        assert_eq!(nesting("type F = Array<Array<() => void>> | ((a: [string]) => void)"), 3);
        assert_eq!(nesting(")))]]}}} {"), 1);
        assert_eq!(nesting(&format!("{}x{}", "{ a: ".repeat(30), " }".repeat(30))), 30);
    }

    /// A file nested deeper than the bound is not read: the request that read it fails and names it, and the session
    /// keeps answering. On the base this program was built, and the next request would print the type 20,001 deep.
    #[test]
    fn a_file_nested_past_the_bound_fails_its_request_with_its_name_and_the_session_keeps_answering() {
        let runtime = runtime();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap().canonicalize_utf8().unwrap();
        let (deep, shallow) = (root.join("deep.ts"), root.join("shallow.ts"));
        let depth = MAX_NESTING + 1;
        std::fs::write(&deep, format!("export type Deep = {}string{};\n", "{ a: ".repeat(depth), " }".repeat(depth)))
            .unwrap();
        std::fs::write(&shallow, "export const n = 1;\n").unwrap();
        let checker = Backend::InProcess(InProcess::start(&root).unwrap());
        let client = checker.client();

        let refused = runtime.block_on(program_on_disk(&client, &deep));
        let after = runtime.block_on(program_on_disk(&client, &shallow));

        match refused {
            Err(TscClientError::RpcError { message, .. }) => assert_eq!(
                message,
                format!(
                    "{deep} nests brackets {depth} deep, more than the {MAX_NESTING} the built-in checker reads \
                     without risking a stack overflow; its components keep their structural answer"
                )
            ),
            other => panic!("expected the file refused, got {other:?}"),
        }
        assert!(after.unwrap()["snapshot"].as_u64().is_some());
        assert!(checker.is_alive());
        runtime.block_on(checker.shutdown());
    }

    #[test]
    fn the_linked_checker_holds_arena_memory_once_it_has_built_a_program() {
        let runtime = runtime();
        let checker = linked();
        drop(runtime.block_on(program(&checker.client(), "/virtual/held.ts", "export const n = 1;\n", 100)).unwrap());

        let held = checker.memory_mib();

        assert!(held.is_some_and(|mib| (1..4096).contains(&mib)), "{held:?}");
        runtime.block_on(checker.shutdown());
    }

    #[test]
    fn a_running_process_has_resident_memory() {
        let mib = resident_mib(std::process::id()).expect("this process is listed");
        assert!((1..16_384).contains(&mib), "{mib} MiB");
    }

    #[test]
    fn a_process_that_does_not_exist_has_none() {
        assert_eq!(resident_mib(u32::MAX - 1), None);
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

    /// Ten thousand deep takes about 1 s in a release build and far longer in a debug one; the checker lane runs it in
    /// release (`just test-checker-lane`).
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
