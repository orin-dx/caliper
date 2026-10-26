//! A warm checker that answers for the components a session asks about, one request at a time.
//!
//! The one-shot [`crate::resolve`] roots a single program at every component's file. A Storybook session asks about
//! one component per story, so this keeps one `tsc --api` process alive, starts it on the first request, and roots a
//! small program at just the requested files: the checker loads what those files import, not the project.
//!
//! The process runs on its own thread with its own runtime, so a caller on any thread (a NAPI worker, a rayon task)
//! never nests runtimes and can bound how long it waits. A failure to start or a crash yields an outcome with no
//! components and one diagnostic, never an error; after [`MAX_RESTARTS`] crashes the checker stays down.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use caliper_docgen_core::types::semantic::{SemanticOutcome, SemanticProvider, SemanticTarget};
use caliper_docgen_tsc_client::Supervisor;
use camino::Utf8PathBuf;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::rpc::Rpc;
use crate::{answer_with, supervisor_config, SemanticRequest, PINNED_VERSIONS};

/// Crashes the checker may have before it stays down for the session.
const MAX_RESTARTS: u32 = 3;

/// Programs one `tsc` process builds before it is replaced. Its memory grows with every program it has loaded
/// (measured: antd reached 600 MiB after 50 single-component programs), and a fresh process costs 30 to 70 ms.
const MAX_REQUESTS_PER_PROCESS: u32 = 64;

#[derive(Debug, Clone)]
pub struct OnDemandConfig {
    /// Where `tsc` finds `node_modules`; its `--cwd`, and where the search for `typescript` starts.
    pub project_root: Utf8PathBuf,
    pub tsconfig: Option<Utf8PathBuf>,
    /// An explicit `tsc` binary, instead of `node_modules/typescript/bin/tsc` above `project_root`.
    pub tsc_path: Option<Utf8PathBuf>,
    /// Bound on each request to the checker, and on its start-up handshake.
    pub request_timeout: Duration,
    pub expected_versions: &'static [&'static str],
    /// Environment for the `tsc` process; the tests use it to pick a replay scenario.
    pub extra_env: Vec<(String, String)>,
}

impl OnDemandConfig {
    pub fn new(project_root: Utf8PathBuf) -> Self {
        Self {
            project_root,
            tsconfig: None,
            tsc_path: None,
            request_timeout: Duration::from_secs(60),
            expected_versions: PINNED_VERSIONS,
            extra_env: Vec::new(),
        }
    }
}

enum Job {
    /// Start the checker now instead of at the first request.
    Warm,
    Answer { targets: Vec<SemanticTarget>, reply: mpsc::SyncSender<SemanticOutcome> },
}

/// A warm `tsc` that answers for the components it is asked about. Dropping it stops the process.
pub struct OnDemand {
    jobs: Option<UnboundedSender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl OnDemand {
    /// Starts the worker thread. The `tsc` process itself starts on the first request, or on [`OnDemand::warm_up`].
    pub fn new(config: OnDemandConfig) -> Self {
        let (jobs, queue) = unbounded_channel();
        let thread = std::thread::Builder::new()
            .name("caliper-tsc".to_string())
            .spawn(move || worker(config, queue))
            .ok();
        Self { jobs: thread.as_ref().map(|_| jobs), thread }
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
                if let Err(refused) = jobs.send(Job::Answer { targets, reply }) {
                    if let Job::Answer { reply, .. } = refused.0 {
                        reply.send(SemanticOutcome::unavailable("the checker thread has stopped")).ok();
                    }
                }
            }
            None => {
                reply.send(SemanticOutcome::unavailable("could not start the checker thread")).ok();
            }
        }
        receiver
    }
}

impl Drop for OnDemand {
    fn drop(&mut self) {
        // Closing the queue ends the worker loop, which shuts `tsc` down before the thread exits.
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

fn worker(config: OnDemandConfig, mut queue: UnboundedReceiver<Job>) {
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
        let mut checker = Checker::default();
        while let Some(job) = queue.recv().await {
            match job {
                Job::Warm => {
                    checker.ensure(&config).await.ok();
                }
                Job::Answer { targets, reply } => {
                    reply.send(checker.answer(&config, targets).await).ok();
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
                prepared_options: None,
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
        let mut request = SemanticRequest::new(config.project_root.clone(), targets);
        request.tsconfig = config.tsconfig.clone();
        request.tsc_path = config.tsc_path.clone();
        request.request_timeout = config.request_timeout;
        request.expected_versions = config.expected_versions;
        request.extra_env.clone_from(&config.extra_env);
        request.prepared_options.clone_from(&self.options);
        let outcome = answer_with(supervisor, &request).await;
        self.served += 1;
        if self.served >= MAX_REQUESTS_PER_PROCESS {
            // A planned replacement, not a crash: the next request starts a fresh process.
            if let Some(old) = self.supervisor.take() {
                old.shutdown().await.ok();
            }
            self.options = None;
            self.served = 0;
        }
        outcome
    }
}
