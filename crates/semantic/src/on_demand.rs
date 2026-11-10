//! A warm checker that answers for the components a session asks about, one request at a time.
//!
//! The one-shot [`crate::resolve`] roots a single program at every component's file. A Storybook session asks about
//! one component per story, so this keeps one `tsc --api` process alive, starts it on the first request, and roots a
//! small program at just the requested files: the checker loads what those files import, not the project.
//!
//! The snapshot of that program is kept between requests (ADR 0026 S6). The next request brings it up to date with
//! `updateSnapshot`, which re-reads only the files the session announced as changed ([`SemanticProvider::changed`]) and
//! keeps the rest as parsed, re-rooting the program when the request names other files. The checker does not see an
//! edit it is not told about, so before an answer every file the program read is compared with the stamp it had when
//! read ([`crate::stamps`]): one that changed unannounced makes the answer come from a fresh snapshot, with a warning
//! that names it.
//!
//! The process runs on its own thread with its own runtime, so a caller on any thread (a NAPI worker, a rayon task)
//! never nests runtimes and can bound how long it waits. A failure to start or a crash yields an outcome with no
//! components and one diagnostic, never an error; after [`MAX_RESTARTS`] crashes the checker stays down.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use caliper_docgen_core::types::semantic::{SemanticOutcome, SemanticProvider, SemanticTarget, SourceChange};
use caliper_docgen_core::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use caliper_docgen_tsc_client::{Client, Supervisor};
use camino::Utf8PathBuf;
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::rpc::{CallError, Rpc};
use crate::stamps::{self, Stamp};
use crate::{SUPPORTED_VERSIONS, SemanticRequest, answer_on, group_by_file, root_files, supervisor_config};

/// How many files an unannounced-edit warning names before it counts the rest.
const UNTOLD_NAMED: usize = 5;

/// Crashes the checker may have before it stays down for the session.
const MAX_RESTARTS: u32 = 3;

/// Programs one `tsc` process builds before it is replaced. Its memory grows with every program it has loaded
/// (measured: antd reached 600 MiB after 50 single-component programs), and a fresh process costs 30 to 70 ms.
const MAX_REQUESTS_PER_PROCESS: u32 = 64;

/// Resident memory, in MiB, past which a `tsc` process is replaced between requests. Measured single-component programs
/// peak at 157 to 612 MiB (antd the highest); a process past this has loaded more than a story session needs.
const DEFAULT_MAX_RSS_MIB: u64 = 768;

/// How many requests pass between memory checks: each reads the process table, which costs a few milliseconds.
const RSS_CHECK_EVERY: u32 = 4;

/// How long dropping a checker waits, by default, for its worker to finish a request and stop `tsc`, before leaving the
/// worker to finish on its own. Closing a session must not wait for a 60 s request.
const DEFAULT_SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// How a warm [`OnDemand`] checker starts and is kept: where to find `tsc`, how long to wait on it, and when to replace
/// its process.
#[derive(Debug, Clone)]
pub struct OnDemandConfig {
    /// Where `tsc` finds `node_modules`; its `--cwd`, and where the search for `typescript` starts.
    pub project_root: Utf8PathBuf,
    /// A tsconfig whose compiler options the checker uses (read with `parseConfigFile`); with none, a default set.
    pub tsconfig: Option<Utf8PathBuf>,
    /// An explicit `tsc` binary, instead of `node_modules/typescript/bin/tsc` above `project_root`.
    pub tsc_path: Option<Utf8PathBuf>,
    /// Bound on each request to the checker, and on its start-up handshake.
    pub request_timeout: Duration,
    /// The `typescript` versions the checker may report; one that is not among them is a start-up failure.
    pub expected_versions: &'static [&'static str],
    /// Resident memory of the `tsc` process, in MiB, past which it is replaced after a request. A request that needs
    /// more finishes first: the bound is between requests, not inside one.
    pub max_rss_mib: u64,
    /// How long dropping the checker waits for a request in progress before it lets the worker finish alone.
    pub shutdown_wait: Duration,
    /// Environment for the `tsc` process; the tests use it to pick a replay scenario.
    pub extra_env: Vec<(String, String)>,
}

impl OnDemandConfig {
    /// A configuration for `project_root` with a 60 second request timeout, the supported versions, a 768 MiB memory
    /// bound, a 5 second shutdown wait, and no explicit tsconfig or `tsc` path.
    pub fn new(project_root: Utf8PathBuf) -> Self {
        Self {
            project_root,
            tsconfig: None,
            tsc_path: None,
            request_timeout: Duration::from_secs(60),
            expected_versions: SUPPORTED_VERSIONS,
            max_rss_mib: DEFAULT_MAX_RSS_MIB,
            shutdown_wait: DEFAULT_SHUTDOWN_WAIT,
            extra_env: Vec::new(),
        }
    }
}

/// The resident memory of a process in MiB: `/proc` where it exists, else `ps`. `None` when neither answers.
fn resident_mib(pid: u32) -> Option<u64> {
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

/// How often a warm checker's process was replaced on purpose, by cause (crashes are not counted).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Recycles {
    /// Replaced after building the most programs one process is allowed.
    pub worn: u32,
    /// Replaced after its resident memory passed [`OnDemandConfig::max_rss_mib`].
    pub bloated: u32,
}

/// How the snapshots a warm checker answered from were made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Snapshots {
    /// Built from the disk with `createSnapshot`: the first request of a process, one after a file changed that the
    /// checker was not told about, or one after a snapshot could not be listed or updated.
    pub created: u32,
    /// Brought up to date from the one before with `updateSnapshot`: announced edits re-read, the rest kept.
    pub updated: u32,
}

/// What the worker thread reports about the process, for the type that owns the thread to read.
#[derive(Default)]
struct Gauges {
    worn: AtomicU32,
    bloated: AtomicU32,
    created: AtomicU32,
    updated: AtomicU32,
    /// The process that answered last; 0 before the first answer.
    process: AtomicU32,
}

enum Job {
    /// Start the checker now instead of at the first request.
    Warm,
    /// Files changed on disk; queued before any request that may read them.
    Changed(SourceChange),
    Answer {
        targets: Vec<SemanticTarget>,
        reply: mpsc::SyncSender<SemanticOutcome>,
    },
}

/// A warm `tsc` that answers for the components it is asked about. Dropping it stops the process.
pub struct OnDemand {
    jobs: Option<UnboundedSender<Job>>,
    thread: Option<JoinHandle<()>>,
    /// Receives once when the worker has stopped `tsc` and is about to exit.
    stopped: std::sync::Mutex<Option<mpsc::Receiver<()>>>,
    shutdown_wait: Duration,
    gauges: Arc<Gauges>,
}

impl OnDemand {
    /// Starts the worker thread. The `tsc` process itself starts on the first request, or on [`OnDemand::warm_up`].
    pub fn new(config: OnDemandConfig) -> Self {
        let shutdown_wait = config.shutdown_wait;
        let (jobs, queue) = unbounded_channel();
        let (stopped_tx, stopped) = mpsc::channel();
        let gauges = Arc::new(Gauges::default());
        let worker_gauges = Arc::clone(&gauges);
        let thread = std::thread::Builder::new()
            .name("caliper-tsc".to_string())
            .spawn(move || {
                worker(config, queue, worker_gauges);
                stopped_tx.send(()).ok();
            })
            .ok();
        Self {
            jobs: thread.as_ref().map(|_| jobs),
            thread,
            stopped: std::sync::Mutex::new(Some(stopped)),
            shutdown_wait,
            gauges,
        }
    }

    /// How many times the process has been replaced so far, by cause. A replacement happens after the reply it
    /// follows, so read this after the next request to see the last one.
    pub fn recycles(&self) -> Recycles {
        Recycles {
            worn: self.gauges.worn.load(Ordering::Relaxed),
            bloated: self.gauges.bloated.load(Ordering::Relaxed),
        }
    }

    /// How many snapshots the answers so far came from, by how each was made.
    pub fn snapshots(&self) -> Snapshots {
        Snapshots {
            created: self.gauges.created.load(Ordering::Relaxed),
            updated: self.gauges.updated.load(Ordering::Relaxed),
        }
    }

    /// The operating system's id of the `tsc` process that answered the last request, `None` before the first one.
    /// Read after a reply, it is the process that built that answer.
    pub fn process_id(&self) -> Option<u32> {
        Some(self.gauges.process.load(Ordering::Relaxed)).filter(|pid| *pid != 0)
    }

    /// Starts `tsc` in the background so the first request does not pay for it.
    pub fn warm_up(&self) {
        if let Some(jobs) = &self.jobs {
            jobs.send(Job::Warm).ok();
        }
    }
}

impl SemanticProvider for OnDemand {
    fn request(&self, targets: Vec<SemanticTarget>) -> mpsc::Receiver<SemanticOutcome> {
        let (reply, receiver) = mpsc::sync_channel(1);
        match &self.jobs {
            Some(jobs) => {
                if let Err(refused) = jobs.send(Job::Answer { targets, reply })
                    && let Job::Answer { reply, .. } = refused.0
                {
                    reply.send(SemanticOutcome::unavailable("the checker thread has stopped")).ok();
                }
            }
            None => {
                reply.send(SemanticOutcome::unavailable("could not start the checker thread")).ok();
            }
        }
        receiver
    }

    fn changed(&self, change: SourceChange) {
        if let Some(jobs) = &self.jobs {
            jobs.send(Job::Changed(change)).ok();
        }
    }
}

impl Drop for OnDemand {
    fn drop(&mut self) {
        // Closing the queue ends the worker loop, which shuts `tsc` down before the thread exits. A worker still inside
        // a long request is left to finish it: `tsc` stops when it does, or with this process.
        self.jobs.take();
        let receiver = self.stopped.get_mut().ok().and_then(Option::take);
        let stopped = receiver.is_none_or(|receiver| receiver.recv_timeout(self.shutdown_wait).is_ok());
        if let Some(thread) = self.thread.take()
            && stopped
        {
            thread.join().ok();
        }
    }
}

fn worker(config: OnDemandConfig, mut queue: UnboundedReceiver<Job>, gauges: Arc<Gauges>) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            let reason = format!("could not start an async runtime: {error}");
            while let Some(job) = queue.blocking_recv() {
                if let Job::Answer { reply, .. } = job {
                    reply.send(SemanticOutcome::unavailable(reason.clone())).ok();
                }
            }
            return;
        }
    };
    runtime.block_on(async move {
        let mut checker = Checker { gauges, ..Checker::default() };
        while let Some(job) = queue.recv().await {
            match job {
                Job::Warm => {
                    checker.ensure(&config).await.ok();
                }
                Job::Changed(change) => checker.told(change).await,
                Job::Answer { targets, reply } => {
                    reply.send(checker.answer(&config, targets).await).ok();
                    // After the reply, so the caller never waits for the memory check or the replacement.
                    checker.maintain(&config).await;
                }
            }
        }
        if let Some(supervisor) = checker.supervisor.take() {
            supervisor.shutdown().await.ok();
        }
    });
}

#[derive(Default)]
struct Checker {
    supervisor: Option<Supervisor>,
    /// Compiler options read once from the tsconfig for the running process.
    options: Option<serde_json::Value>,
    restarts: u32,
    /// Requests the running process has answered.
    served: u32,
    /// Why the checker cannot be used for the rest of the session.
    down: Option<String>,
    gauges: Arc<Gauges>,
    /// The snapshot the last request was answered from, kept for the next.
    live: Option<Live>,
    /// Files announced as changed since `live` was brought up to date.
    told: FxHashSet<Utf8PathBuf>,
}

/// A snapshot kept between requests, and what its program read.
struct Live {
    rpc: Arc<Rpc>,
    /// The files its program is rooted at.
    roots: Vec<String>,
    /// When the files were compared with the disk, just before the snapshot was built or brought up to date.
    read_from: SystemTime,
    /// Every file the program read, as it was then; `None` until listed after the reply the snapshot built.
    stamps: Option<FxHashMap<Utf8PathBuf, Stamp>>,
}

/// The warning an answer carries when files the snapshot read changed and the checker was not told.
fn untold_warning(files: &[Utf8PathBuf]) -> Diagnostic {
    let mut named: Vec<String> = files.iter().take(UNTOLD_NAMED).map(ToString::to_string).collect();
    if files.len() > UNTOLD_NAMED {
        named.push(format!("{} more", files.len() - UNTOLD_NAMED));
    }
    Diagnostic::new(
        DiagnosticSeverity::Warning,
        DiagnosticCode::SemanticBackend,
        format!(
            "the TypeScript checker was not told that {} changed; it read every file again for this answer",
            named.join(", ")
        ),
    )
    .with_help(
        "Whatever writes the project's files must tell the session (`update_file` or `ensure_fresh`); an edit the \
         checker is not told about is answered from the text it read before.",
    )
}

impl Checker {
    async fn ensure(&mut self, config: &OnDemandConfig) -> Result<(), String> {
        if let Some(reason) = &self.down {
            return Err(reason.clone());
        }
        if self.supervisor.as_ref().is_some_and(|supervisor| !supervisor.is_alive()) {
            if let Some(dead) = self.supervisor.take() {
                dead.shutdown().await.ok();
            }
            self.options = None;
            self.live = None;
            self.told.clear();
            self.restarts += 1;
            if self.restarts > MAX_RESTARTS {
                return Err(self.stay_down("the checker kept crashing".to_string()));
            }
        }
        if self.supervisor.is_none() {
            let request = SemanticRequest {
                project_root: config.project_root.clone(),
                tsconfig: config.tsconfig.clone(),
                tsc_path: config.tsc_path.clone(),
                targets: Vec::new(),
                request_timeout: config.request_timeout,
                expected_versions: config.expected_versions,
                record_to: None,
                extra_env: config.extra_env.clone(),
                reverse_queries: false,
            };
            let started = supervisor_config(&request).map_err(|reason| self.stay_down(reason))?;
            let supervisor = match Supervisor::spawn(started).await {
                Ok(supervisor) => supervisor,
                Err(error) => return Err(self.stay_down(error.to_string())),
            };
            // Initialize the session and read the compiler options once; every request after builds only its program.
            let tsconfig = config.tsconfig.as_ref().map(|path| path.as_str());
            match Rpc::prepare(&supervisor.client(), config.request_timeout, tsconfig).await {
                Ok(options) => self.options = Some(options),
                Err(error) => {
                    supervisor.shutdown().await.ok();
                    return Err(self.stay_down(format!("the checker did not initialize: {error}")));
                }
            }
            self.supervisor = Some(supervisor);
        }
        Ok(())
    }

    fn stay_down(&mut self, reason: String) -> String {
        self.down = Some(reason.clone());
        reason
    }

    async fn answer(&mut self, config: &OnDemandConfig, targets: Vec<SemanticTarget>) -> SemanticOutcome {
        if let Err(reason) = self.ensure(config).await {
            return SemanticOutcome::unavailable(reason);
        }
        let Some(supervisor) = self.supervisor.as_ref() else {
            return SemanticOutcome::unavailable("the checker is not running");
        };
        let (client, process) = (supervisor.client(), supervisor.process_id());
        let mut request = SemanticRequest::new(config.project_root.clone(), targets);
        request.tsconfig = config.tsconfig.clone();
        request.tsc_path = config.tsc_path.clone();
        request.request_timeout = config.request_timeout;
        request.expected_versions = config.expected_versions;
        request.extra_env.clone_from(&config.extra_env);
        let roots = root_files(&request);
        if roots.is_empty() {
            return group_by_file(&request).0;
        }
        let answered = match self.snapshot_for(client, config.request_timeout, roots.clone()).await {
            Ok((rpc, warning)) => answer_on(&rpc, &request, &roots).await.map(|mut outcome| {
                outcome.diagnostics.extend(warning);
                outcome
            }),
            Err(error) => Err(error),
        };
        self.gauges.process.store(process.unwrap_or(0), Ordering::Relaxed);
        self.served += 1;
        match answered {
            Ok(outcome) => outcome,
            Err(error) => {
                self.forget().await;
                SemanticOutcome::unavailable(format!("the checker stopped answering: {error}"))
            }
        }
    }

    /// The snapshot to answer from, its program rooted at `roots`: the kept one brought up to date with the files the
    /// checker was told about, or a fresh one when none is kept, the kept one cannot be checked or updated, or a file
    /// it read changed with nobody telling the checker. The last comes with a warning that names the files.
    async fn snapshot_for(
        &mut self,
        client: Client,
        deadline: Duration,
        roots: Vec<String>,
    ) -> Result<(Arc<Rpc>, Option<Diagnostic>), CallError> {
        let read_from = SystemTime::now();
        let told = std::mem::take(&mut self.told);
        let mut warning = None;
        if let Some(live) = self.live.take() {
            if let Some(stamps) = &live.stamps {
                let drift = stamps::drift(stamps, &told);
                if drift.untold.is_empty() {
                    let reread: BTreeSet<Utf8PathBuf> = told.into_iter().chain(drift.unsure).collect();
                    if reread.is_empty() && live.roots == roots {
                        let rpc = live.rpc.clone();
                        self.live = Some(live);
                        return Ok((rpc, None));
                    }
                    let (changed, deleted): (Vec<String>, Vec<String>) = {
                        let (present, gone): (Vec<_>, Vec<_>) = reread.into_iter().partition(|file| file.exists());
                        (present.into_iter().map(String::from).collect(), gone.into_iter().map(String::from).collect())
                    };
                    let rerooted = (live.roots != roots).then_some(roots.as_slice());
                    match live.rpc.updated(&changed, &deleted, rerooted).await {
                        Ok(rpc) => {
                            live.rpc.release().await;
                            let rpc = Arc::new(rpc);
                            self.gauges.updated.fetch_add(1, Ordering::Relaxed);
                            self.live = Some(Live { rpc: rpc.clone(), roots, read_from, stamps: None });
                            return Ok((rpc, None));
                        }
                        Err(CallError::Rejected(message)) => {
                            tracing::debug!(%message, "the checker could not update its snapshot; building a fresh one");
                        }
                        Err(fatal) => return Err(fatal),
                    }
                } else {
                    tracing::warn!(files = ?drift.untold, "files changed that the checker was not told about");
                    warning = Some(untold_warning(&drift.untold));
                }
            }
            live.rpc.release().await;
        }
        let options = self.options.as_ref().ok_or_else(|| CallError::Fatal("the checker is not initialized".into()))?;
        let rpc = Arc::new(Rpc::open_with(client, deadline, options, &roots, false).await?);
        self.gauges.created.fetch_add(1, Ordering::Relaxed);
        self.live = Some(Live { rpc: rpc.clone(), roots, read_from, stamps: None });
        Ok((rpc, warning))
    }

    /// Files changed on disk: the kept snapshot reads them again at the next request, or, when anything may have
    /// changed, is dropped.
    async fn told(&mut self, change: SourceChange) {
        match change {
            SourceChange::Files(files) if self.live.is_some() => self.told.extend(files),
            SourceChange::Files(_) => {}
            SourceChange::Everything => self.forget().await,
        }
    }

    /// Drops the kept snapshot, so the next request builds one from the disk.
    async fn forget(&mut self) {
        self.told.clear();
        if let Some(live) = self.live.take() {
            live.rpc.release().await;
        }
    }

    /// Lists and stamps the files the kept snapshot's program read, once, after the reply it answered, so the first
    /// answer does not wait for it. A snapshot that cannot be listed cannot be checked, and is dropped.
    async fn stamp(&mut self) {
        let Some(live) = self.live.as_mut().filter(|live| live.stamps.is_none()) else { return };
        match live.rpc.source_file_names().await {
            Ok(files) => {
                live.stamps = Some(stamps::record(files.into_iter().map(Utf8PathBuf::from).collect(), live.read_from))
            }
            Err(error) => {
                tracing::debug!(%error, "could not list the snapshot's files; the next request builds a fresh one");
                self.forget().await;
            }
        }
    }

    /// Replaces the process when it has built enough programs or grown past the memory bound, and otherwise stamps
    /// the snapshot it keeps. A planned replacement, not a crash: the next request starts a fresh process.
    async fn maintain(&mut self, config: &OnDemandConfig) {
        let Some(supervisor) = self.supervisor.as_ref() else { return };
        let worn = self.served >= MAX_REQUESTS_PER_PROCESS;
        let resident = (!worn && self.served.is_multiple_of(RSS_CHECK_EVERY))
            .then(|| supervisor.process_id().and_then(resident_mib))
            .flatten();
        let bloated = resident.is_some_and(|mib| mib > config.max_rss_mib);
        if worn || bloated {
            let (count, cause) = if worn { (&self.gauges.worn, "worn") } else { (&self.gauges.bloated, "bloated") };
            count.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(cause, served = self.served, resident_mib = ?resident, "replacing the checker process");
            // The snapshot goes with the process; the next one reads every file from the disk.
            self.live = None;
            self.told.clear();
            if let Some(old) = self.supervisor.take() {
                old.shutdown().await.ok();
            }
            self.options = None;
            self.served = 0;
            return;
        }
        self.stamp().await;
    }
}

#[cfg(test)]
mod tests {
    use super::resident_mib;

    #[test]
    fn a_running_process_has_resident_memory() {
        let mib = resident_mib(std::process::id()).expect("this process is listed");
        assert!((1..16_384).contains(&mib), "{mib} MiB");
    }

    #[test]
    fn a_process_that_does_not_exist_has_none() {
        assert_eq!(resident_mib(u32::MAX - 1), None);
    }
}
