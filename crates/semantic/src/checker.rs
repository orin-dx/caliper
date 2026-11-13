//! The checker a run asks: the `tsrs` crates linked into this process, asked through `tsrs_api::Session` with the
//! methods and JSON of `tsc --api` ([`crate::rpc`]).
//!
//! The linked checker answers on threads of its own with a 512 MiB stack, the stack the `tsrs` binary serves on. A
//! query runs on the thread that calls it, and a type nested 2,000 deep overflows a default 2 MiB thread: a stack
//! overflow aborts the whole process and cannot be caught. Callers on any thread send it requests over a channel, one
//! per checker thread (a lane), and each thread answers its own one at a time, in the order they arrive; a run that asks
//! several programs at once sends each program's requests down a lane of its own ([`Client::lane`]). Every lane asks
//! the same session.
//!
//! It is guarded so that it does not take its host (Node, Storybook) down with it:
//!
//! - **Panics.** A panic inside the checker comes back as an error to the request that caused it, and every lane then
//!   stops: a request any lane takes afterwards fails, and the last lane to leave closes the session, as a crashed
//!   process would end. A session a panic unwound through is not asked again, so no later answer comes from state it
//!   left half-built. The caller starts a fresh one ([`crate::on_demand`] does, up to its restart limit; a
//!   whole-project run fails on the first lane's error and answers structurally). A panic `tsrs` itself reports as the
//!   client's fault (an unknown `moduleResolution`) is an error like any other, and the session keeps answering.
//! - **Nesting.** `tsrs` parses on threads of its own with a 256 MiB stack, which a file nested a few hundred thousand
//!   brackets deep overflows (measured on macOS arm64: an object type 200,000 deep parses, 300,000 aborts). Every file
//!   the checker reads from the disk is measured first ([`nesting`]), and a request that read one nested deeper than
//!   [`MAX_NESTING`] fails with an error that names it, so its components keep their structural answer. Requests that
//!   build snapshots, which are the ones that read files, run one at a time across the lanes, so a refusal is the
//!   request's that met it; any other request that sees a refusal fails too.
//! - **Memory.** [`Backend::memory_mib`] reads what the linked checker holds: the resident part of the address range
//!   `tsrs` allocates its arenas from, which leaves out the host's own memory. [`crate::on_demand`] bounds it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use camino::Utf8Path;
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::sync::oneshot;
use tsrs_api::{ErrorKind, Handler, Response, Session, SessionOptions};
use tsrs_vfs::{FS, bundled, osvfs, wrapvfs};

/// The stack of the thread the linked checker answers on: what the `tsrs` binary gives each of its threads.
const CHECKER_STACK_BYTES: usize = 512 << 20;

/// The deepest bracket nesting a file may have for the linked checker to read it: twice the 10,000 levels the deep-type
/// tests print in full, and a tenth of the 200,000 an object type was measured to parse at on `tsrs`'s 256 MiB parse
/// threads (300,000 overflows them, which aborts the process).
pub(crate) const MAX_NESTING: usize = 20_000;

/// Why the checker did not answer a request.
#[derive(Debug)]
pub(crate) enum RequestError {
    /// The checker answered with an error (a panic inside it among them): this request failed.
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

/// The linked checker: the first of the threads that share its session, and the way to them.
pub(crate) struct Backend {
    client: Client,
    thread: JoinHandle<()>,
    threads: usize,
    /// Set by a thread, before it answers, when no thread will answer anything more: a panic unwound through the
    /// session.
    ended: Arc<AtomicBool>,
    /// Disconnected once every thread has left and the last has closed the session.
    closed: mpsc::Receiver<()>,
}

impl Backend {
    /// Spawns the checker thread, which creates its session there: the session's work, from the first request on,
    /// runs on that thread's stack. With `threads` above one it starts as many more, each answering a queue of its own
    /// (a lane, [`Client::lane`]) on the same session: requests about one program wait for each other (each program
    /// has one checker, which `tsrs_api` locks for the length of a request), so a run that sends the requests about
    /// each of its programs down a lane of its own has them answered at once. A thread that cannot be spawned is an
    /// error that says so.
    pub(crate) fn start(project_root: &Utf8Path, threads: usize) -> Result<Backend, String> {
        let cwd = project_root.to_string();
        Backend::spawn(threads, move || TsrsSession::open(cwd))
    }

    /// Spawns the checker threads over the session `open` makes on the first.
    pub(crate) fn spawn<S: Served + Send + Sync + 'static>(
        threads: usize,
        open: impl FnOnce() -> S + Send + 'static,
    ) -> Result<Backend, String> {
        let (lanes, queues): (Vec<_>, Vec<_>) = (0..threads.max(1)).map(|_| mpsc::channel()).unzip();
        let threads = lanes.len();
        let ended = Arc::new(AtomicBool::new(false));
        let thread_ended = Arc::clone(&ended);
        let (closing, closed) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("caliper-checker".to_string())
            .stack_size(CHECKER_STACK_BYTES)
            .spawn(move || serve(open, queues, thread_ended, closing))
            .map_err(|error| format!("could not start the checker thread: {error}"))?;
        Ok(Backend { client: Client { lanes: lanes.into(), lane: 0 }, thread, threads, ended, closed })
    }

    /// How many programs a run asks at once: one per thread.
    pub(crate) fn parallel_programs(&self) -> usize {
        self.threads
    }

    pub(crate) fn client(&self) -> Client {
        self.client.clone()
    }

    /// Whether the checker still answers. It stops once a panic has unwound through its session, before its thread
    /// has finished closing it.
    pub(crate) fn is_alive(&self) -> bool {
        !self.ended.load(Ordering::Acquire) && !self.thread.is_finished()
    }

    /// What the checker holds, in MiB: the resident part of the address range the `tsrs` crates allocate their arenas
    /// from. That range is theirs alone, so the host's memory is not counted, and a closed session gives its programs'
    /// part of it back. `None` when it cannot be read.
    pub(crate) fn memory_mib(&self) -> Option<u64> {
        arena_mib()
    }

    /// Stops the checker. Its threads finish the request they are in, close the session and exit on their own: they
    /// are not waited for, so a hung request cannot hold the caller.
    pub(crate) async fn shutdown(self) {
        self.client.stop();
    }

    /// Stops the checker and waits up to `wait` for its threads to close the session, so what the session held is
    /// given back before a fresh one is read. For a replacement between requests, when nothing is in progress.
    pub(crate) async fn shutdown_within(self, wait: Duration) {
        self.client.stop();
        // The threads drop their ends once the session is closed.
        self.closed.recv_timeout(wait).ok();
    }
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

enum Job {
    Request(Asked),
    /// Close the session and end the thread; requests sent after it fail as from a stopped checker.
    Stop,
}

struct Asked {
    method: &'static str,
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

impl Client {
    /// The client whose requests the `lane`th thread of the checker answers ([`Backend::start`]).
    pub(crate) fn lane(&self, lane: usize) -> Client {
        Client { lanes: self.lanes.clone(), lane }
    }

    /// One request, its parameters the JSON text `params`, its answer the `result` as the JSON text it arrived as. The
    /// wait is bounded by `deadline`.
    pub(crate) async fn request_raw(
        &self,
        method: &'static str,
        params: Vec<u8>,
        deadline: Duration,
    ) -> Result<Box<RawValue>, RequestError> {
        let (reply, answer) = oneshot::channel();
        let asked = Asked { method, params, reply };
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

/// Why the checker did not answer a request.
pub(crate) struct Failure {
    pub(crate) message: String,
    /// A panic unwound through the session, which is not asked again.
    pub(crate) panicked: bool,
}

/// What the checker thread asks: the `tsrs` session, or a stand-in in the tests.
pub(crate) trait Served {
    /// Answers one request with its JSON text.
    fn handle(&self, method: &str, params: &[u8]) -> Result<String, Failure>;
    /// Releases the snapshot an answer of `method` made, when that answer will not reach the caller, who could then
    /// never release it.
    fn discard(&self, method: &str, answer: &str);
    fn close(&self);
}

/// The snapshot an answer of `method` made: `createSnapshot` and `updateSnapshot` each make one, which lives until it
/// is released.
fn snapshot_made(method: &str, answer: &str) -> Option<u64> {
    if !matches!(method, "createSnapshot" | "updateSnapshot") {
        return None;
    }
    serde_json::from_str::<Value>(answer).ok()?.get("snapshot")?.as_u64()
}

/// A `tsrs` session over the disk, where every file is measured before the session reads it.
struct TsrsSession {
    session: Arc<Session>,
    /// The files nested past [`MAX_NESTING`] the session was refused, with their depth (see [`TsrsSession::answer`]).
    refused: Arc<Mutex<Vec<(String, usize)>>>,
    /// Held alone by a request that builds a snapshot, and shared by every other, across the lanes.
    building: RwLock<()>,
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
        TsrsSession { session, refused, building: RwLock::new(()) }
    }

    fn refused(&self) -> std::sync::MutexGuard<'_, Vec<(String, usize)>> {
        self.refused.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Answers one request and the refusals it met. A request that builds a snapshot (the ones that read files) has
    /// the session to itself among the lanes, so a refusal noted meanwhile is its own; it clears the list before and
    /// takes it after. Any other request runs beside the others and only looks: a refusal one of them met (a file read
    /// late) fails every request that sees it, until the next snapshot is built.
    fn answer(&self, method: &str, params: &[u8]) -> (tsrs_api::ApiResult<Response>, Option<(String, usize)>) {
        if matches!(method, "createSnapshot" | "updateSnapshot") {
            let _alone = self.building.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.refused().clear();
            let answered = self.session.handle_request(method, params);
            let refused = std::mem::take(&mut *self.refused()).into_iter().next();
            (answered, refused)
        } else {
            let _beside = self.building.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let answered = self.session.handle_request(method, params);
            let refused = self.refused().first().cloned();
            (answered, refused)
        }
    }
}

impl Served for TsrsSession {
    fn handle(&self, method: &str, params: &[u8]) -> Result<String, Failure> {
        let (answered, refused) = self.answer(method, params);
        if let Some((file, depth)) = refused {
            // The session answered without the file, which it took for missing: not an answer to give, and a snapshot
            // it made for it would be held with no one to release it.
            if let Ok(Response::Json(text)) = &answered {
                self.discard(method, text);
            }
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

    fn discard(&self, method: &str, answer: &str) {
        if let Some(snapshot) = snapshot_made(method, answer) {
            let params = serde_json::json!({ "snapshot": snapshot }).to_string();
            if let Err(error) = self.session.handle_request("release", params.as_bytes()) {
                tracing::debug!(snapshot, %error, "could not release the snapshot of an answer nobody receives");
            }
        }
    }

    fn close(&self) {
        self.session.close();
    }
}

/// The checker threads: one session, each thread answering the requests of its queue (its lane) in turn. The first
/// thread creates the session and starts the others.
fn serve<S: Served + Send + Sync + 'static>(
    open: impl FnOnce() -> S,
    queues: Vec<mpsc::Receiver<Job>>,
    ended: Arc<AtomicBool>,
    closing: mpsc::Sender<()>,
) {
    let session = Arc::new(open());
    let left = Arc::new(AtomicUsize::new(queues.len()));
    let mut queues = queues.into_iter();
    let Some(own) = queues.next() else {
        catch_unwind(AssertUnwindSafe(|| session.close())).ok();
        return;
    };
    for (index, queue) in queues.enumerate() {
        let lane =
            Lane { session: session.clone(), ended: ended.clone(), left: left.clone(), _closing: closing.clone() };
        let spawned = std::thread::Builder::new()
            .name(format!("caliper-checker-{}", index + 1))
            .stack_size(CHECKER_STACK_BYTES)
            .spawn(move || lane.answer(&queue));
        // Its queue is dropped with the closure, so its lane's requests fail as from a stopped checker.
        if spawned.is_err() {
            left.fetch_sub(1, Ordering::AcqRel);
        }
    }
    Lane { session, ended, left, _closing: closing }.answer(&own);
}

/// What one checker thread shares with the others.
struct Lane<S> {
    session: Arc<S>,
    /// Set, before its reply, by the thread whose request a panic unwound through: no thread answers after it.
    ended: Arc<AtomicBool>,
    /// The threads that have not left; the last to leave closes the session.
    left: Arc<AtomicUsize>,
    /// Dropped as the thread leaves: [`Backend::closed`] disconnects once every thread has.
    _closing: mpsc::Sender<()>,
}

impl<S: Served> Lane<S> {
    /// The requests of `queue` in turn, until the thread is stopped, every sender is gone, or a panic unwound through
    /// the session on any thread. A panic is caught here too, should one escape `tsrs`'s own catch.
    fn answer(self, queue: &mpsc::Receiver<Job>) {
        for job in queue {
            let asked = match job {
                Job::Request(asked) => asked,
                Job::Stop => break,
            };
            if self.ended.load(Ordering::Acquire) {
                let message = "the built-in checker stopped: a panic unwound through its session".to_string();
                asked.reply.send(Err(message)).ok();
                break;
            }
            // A caller that stopped waiting (its deadline passed, its run ended) is not answered.
            if asked.reply.is_closed() {
                continue;
            }
            let session = &self.session;
            let answered = catch_unwind(AssertUnwindSafe(|| session.handle(asked.method, &asked.params)))
                .unwrap_or_else(|panic| {
                    let text = panic
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic.downcast_ref::<&str>().copied())
                        .unwrap_or("a panic with no message");
                    Err(Failure { message: format!("panic: {text}"), panicked: true })
                });
            match answered {
                Ok(text) => {
                    // A caller that stopped waiting meanwhile (its deadline passed) never learns of a snapshot this
                    // made.
                    if let Err(Ok(unread)) = asked.reply.send(Ok(text)) {
                        session.discard(asked.method, &unread);
                    }
                }
                Err(Failure { message, panicked }) => {
                    if panicked {
                        tracing::warn!(method = asked.method, %message, "the built-in checker panicked; it is replaced");
                        // Before the reply, so the caller never sends its next request to a session that will not
                        // answer, and before any other lane takes its next.
                        self.ended.store(true, Ordering::Release);
                        asked.reply.send(Err(message)).ok();
                        break;
                    }
                    asked.reply.send(Err(message)).ok();
                }
            }
        }
        if self.left.fetch_sub(1, Ordering::AcqRel) == 1 {
            // A session a panic left half-built may panic again while it closes; the thread ends either way.
            catch_unwind(AssertUnwindSafe(|| self.session.close())).ok();
        }
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

    async fn ask(client: &Client, method: &'static str, params: Value) -> Result<Value, RequestError> {
        let answer = client.request_raw(method, serde_json::to_vec(&params).unwrap(), Duration::from_secs(120)).await?;
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

    /// A program of `file` as it is on the disk.
    async fn program_on_disk(client: &Client, file: &Utf8Path) -> Result<Value, RequestError> {
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

        fn discard(&self, _method: &str, _answer: &str) {}

        fn close(&self) {
            self.closed.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn fake() -> (Backend, Arc<std::sync::atomic::AtomicU32>) {
        let closed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let session = Fake { closed: Arc::clone(&closed) };
        (Backend::spawn(1, move || session).unwrap(), closed)
    }

    /// Each discarded answer's method and the snapshot it made.
    type Discarded = Arc<Mutex<Vec<(String, Option<u64>)>>>;

    /// Stands in for a session whose snapshots take longer than the caller waits; records what it was told to discard.
    struct Slow {
        discarded: Discarded,
    }

    impl Served for Slow {
        fn handle(&self, method: &str, _params: &[u8]) -> Result<String, Failure> {
            std::thread::sleep(Duration::from_millis(300));
            Ok(match method {
                "createSnapshot" => r#"{"snapshot":7,"projects":[]}"#.to_string(),
                _ => r#"{"snapshot":8}"#.to_string(),
            })
        }

        fn discard(&self, method: &str, answer: &str) {
            self.discarded.lock().unwrap().push((method.to_string(), snapshot_made(method, answer)));
        }

        fn close(&self) {}
    }

    /// A snapshot made for a caller that stopped waiting is released: the caller never learns its id. An answer that
    /// made none (any other method) has nothing to release.
    #[test]
    fn a_snapshot_made_after_its_caller_stopped_waiting_is_released() {
        let runtime = runtime();
        let discarded = Arc::default();
        let session = Slow { discarded: Arc::clone(&discarded) };
        let checker = Backend::spawn(1, move || session).unwrap();
        let client = checker.client();

        // The second waits until the first is done (300 ms) and is taken, then stops waiting before its own is.
        for (method, wait) in [("createSnapshot", 100), ("getTypeOfSymbol", 350)] {
            let late = runtime.block_on(client.request_raw(method, b"null".to_vec(), Duration::from_millis(wait)));
            assert!(matches!(late, Err(RequestError::Timeout { .. })), "{late:?}");
        }
        let waited = std::time::Instant::now();
        while discarded.lock().unwrap().len() < 2 && waited.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            *discarded.lock().unwrap(),
            [("createSnapshot".to_string(), Some(7)), ("getTypeOfSymbol".to_string(), None)]
        );
        runtime.block_on(checker.shutdown());
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
                Err(RequestError::Rejected(message)) => assert_eq!(message, said),
                other => panic!("expected the panic as an error, got {other:?}"),
            }
            assert!(!alive, "{method}: the checker must not be asked again");
            assert!(matches!(after, Err(RequestError::Closed)), "{method}: {after:?}");
            let waited = std::time::Instant::now();
            while closed.load(Ordering::Relaxed) == 0 && waited.elapsed() < Duration::from_secs(5) {
                std::thread::yield_now();
            }
            assert_eq!(closed.load(Ordering::Relaxed), 1, "{method}");
        }
    }

    /// Every lane shares the one session, so a panic on one stops them all: the request the other lane takes next fails,
    /// the session is closed once, and the checker says it no longer answers.
    #[test]
    fn a_panic_on_one_lane_stops_every_lane() {
        let runtime = runtime();
        let closed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let session = Fake { closed: Arc::clone(&closed) };
        let checker = Backend::spawn(2, move || session).unwrap();
        let (first, second) = (checker.client().lane(0), checker.client().lane(1));

        let before = runtime.block_on(ask(&second, "ping", Value::Null));
        let panicked = runtime.block_on(ask(&first, "boom", Value::Null));
        let after = runtime.block_on(ask(&second, "ping", Value::Null));

        assert_eq!(before.unwrap(), json!("pong"));
        assert!(matches!(panicked, Err(RequestError::Rejected(_))), "{panicked:?}");
        match after {
            Err(RequestError::Rejected(message)) => {
                assert_eq!(message, "the built-in checker stopped: a panic unwound through its session");
            }
            other => panic!("expected the other lane stopped, got {other:?}"),
        }
        assert!(!checker.is_alive());
        assert!(matches!(runtime.block_on(ask(&second, "ping", Value::Null)), Err(RequestError::Closed)));
        runtime.block_on(checker.shutdown_within(Duration::from_secs(5)));
        assert_eq!(closed.load(Ordering::Relaxed), 1, "the last lane to leave closes the session, once");
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

    /// A snapshot a refused request made is released before the error goes back, since its id never reaches the caller,
    /// who could not release it: a refused `createSnapshot` or `updateSnapshot` leaves nothing behind. Snapshot ids are
    /// numbered in order, so the refused ones lie between the snapshots that were answered; releasing one of them again
    /// must fail as releasing an unknown snapshot does.
    #[test]
    fn a_snapshot_made_by_a_refused_request_is_released_and_the_ones_answered_are_kept() {
        let runtime = runtime();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap().canonicalize_utf8().unwrap();
        let depth = MAX_NESTING + 1;
        std::fs::write(
            root.join("deep.ts"),
            format!("export type Deep = {}string{};\n", "{ a: ".repeat(depth), " }".repeat(depth)),
        )
        .unwrap();
        let (importer, shallow) = (root.join("importer.ts"), root.join("shallow.ts"));
        std::fs::write(&importer, "export const n = 1;\n").unwrap();
        std::fs::write(&shallow, "export const m = 2;\n").unwrap();
        // Two lanes, as a whole-project run has: the refused requests go down the second while the first builds a
        // snapshot of its own, which must neither take the refusal nor be released.
        let checker = Backend::start(&root, 2).unwrap();
        let (client, other) = (checker.client(), checker.client().lane(1));
        let snapshot = |scope: &Value| scope["snapshot"].as_u64().unwrap();

        let kept = runtime.block_on(program_on_disk(&client, &importer)).unwrap();
        // The importer now reads the deep file: the update that is told so is refused.
        std::fs::write(&importer, "import type { Deep } from './deep';\nexport type D = Deep;\n").unwrap();
        let changes = json!({ "fileNotifications": { "changed": [importer], "deleted": [] }, "ensurePrograms": true });
        let updated = runtime.block_on(ask(
            &other,
            "updateSnapshot",
            json!({ "snapshot": kept["snapshot"], "changes": changes }),
        ));
        let (created, beside) = runtime
            .block_on(async { tokio::join!(program_on_disk(&other, &importer), program_on_disk(&client, &shallow)) });
        let after = runtime.block_on(program_on_disk(&client, &shallow)).unwrap();

        assert!(matches!(updated, Err(RequestError::Rejected(_))), "{updated:?}");
        assert!(matches!(created, Err(RequestError::Rejected(_))), "{created:?}");
        let beside = beside.unwrap();
        assert_eq!(snapshot(&after), snapshot(&kept) + 4, "three snapshots were made in between");
        let refused = (snapshot(&kept) + 1..snapshot(&after)).filter(|id| *id != snapshot(&beside));
        for refused in refused {
            let again = runtime.block_on(ask(&client, "release", json!({ "snapshot": refused })));
            assert!(again.is_err(), "snapshot {refused} of a refused request is still held");
        }
        // What the caller was answered it still holds, and releases itself.
        for answered in [&kept, &beside, &after] {
            let released = runtime.block_on(ask(&client, "release", json!({ "snapshot": answered["snapshot"] })));
            assert!(released.is_ok(), "{released:?}");
        }
        runtime.block_on(checker.shutdown());
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
        let checker = Backend::start(&root, 1).unwrap();
        let client = checker.client();

        let refused = runtime.block_on(program_on_disk(&client, &deep));
        let after = runtime.block_on(program_on_disk(&client, &shallow));

        match refused {
            Err(RequestError::Rejected(message)) => assert_eq!(
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

        let late = runtime.block_on(client.request_raw(
            "createSnapshot",
            serde_json::to_vec(&params).unwrap(),
            Duration::from_micros(1),
        ));
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
        let after = runtime.block_on(client.request_raw("initialize", b"null".to_vec(), Duration::from_secs(5)));

        assert!(matches!(after, Err(RequestError::Closed)), "{after:?}");
    }
}
