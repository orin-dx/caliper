use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};

use crate::client::{Client, TaggedResponse};
use crate::error::TscClientError;

const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10); // measured 33-72ms; generous headroom
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const MAX_RESTARTS: u32 = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const BACKOFF_STEPS: [Duration; 3] =
    [Duration::from_millis(200), Duration::from_millis(400), Duration::from_millis(800)];
const BACKOFF_MAX: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// Path to the `tsc` binary, e.g. `node_modules/typescript/bin/tsc` — resolving
    /// that path is the caller's job, not this crate's.
    pub tsc_path: Utf8PathBuf,
    /// Passed as `tsc --api --async --cwd <cwd>`.
    pub cwd: Utf8PathBuf,
    /// `tsc_path`'s adjacent `package.json` "version" must match one of these, checked before every spawn. An entry
    /// is an exact version (`7.1.0-beta`) or a `major.minor` series (`7.1`, which takes `7.1.0`, `7.1.0-rc.1` and
    /// `7.1.4`, not `7.10.0`). The protocol has no version handshake (ADR 0006), so this and the initialize call
    /// that follows the spawn are the whole check — the caller decides which builds it has traced the protocol on.
    pub expected_versions: &'static [&'static str],
    pub handshake_timeout: Duration,
    pub max_frame_bytes: usize,
    /// Extra environment variables for the spawned process. Empty for real use; tests
    /// use this to pick a `fake-tsc` scenario via `FAKE_TSC_SCENARIO`.
    pub extra_env: Vec<(String, String)>,
}

impl SupervisorConfig {
    pub fn new(tsc_path: Utf8PathBuf, cwd: Utf8PathBuf, expected_versions: &'static [&'static str]) -> Self {
        Self {
            tsc_path,
            cwd,
            expected_versions,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            max_frame_bytes: crate::transport::DEFAULT_MAX_FRAME_BYTES,
            extra_env: Vec::new(),
        }
    }
}

fn check_version(tsc_path: &Utf8Path, expected: &'static [&'static str]) -> Result<(), TscClientError> {
    let reason_at = |reason: String| TscClientError::VersionCheckFailed { path: tsc_path.to_owned(), reason };

    let Some(package_json) =
        tsc_path.parent().and_then(|bin_dir| bin_dir.parent()).map(|pkg_dir| pkg_dir.join("package.json"))
    else {
        return Err(reason_at("tsc_path has no grandparent directory to find package.json in".to_string()));
    };

    let contents = std::fs::read_to_string(&package_json)
        .map_err(|source| reason_at(format!("reading {package_json}: {source}")))?;
    let value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| reason_at(format!("parsing {package_json}: {source}")))?;
    let Some(found) = value.get("version").and_then(|v| v.as_str()) else {
        return Err(reason_at(format!("{package_json} has no \"version\" string")));
    };

    if expected.iter().any(|entry| version_matches(found, entry)) {
        Ok(())
    } else {
        Err(TscClientError::UnsupportedVersion { path: tsc_path.to_owned(), found: found.to_string(), expected })
    }
}

/// Whether `found` is `entry` or a release of the `entry` series: `7.1` takes `7.1.0-beta` and `7.1.3`, not `7.10.0`.
fn version_matches(found: &str, entry: &str) -> bool {
    found == entry || found.strip_prefix(entry).is_some_and(|rest| rest.starts_with(['.', '-']))
}

fn spawn_stderr_drain(stderr: tokio::process::ChildStderr) {
    // Must be continuously drained: an unread stderr pipe fills its OS buffer and can
    // deadlock the child. Logged at debug, not warn — tsc's own diagnostic chatter
    // isn't inherently an error, but it's valuable when a crash needs explaining.
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "tsc_client::stderr", "{line}");
        }
    });
}

/// Spawns one `tsc --api` process and wires up its transport. Split out from
/// `Supervisor::spawn`/`restart` so a restart can replace the process/client pair
/// without losing the restart-attempt bookkeeping a fresh `Supervisor` would
/// otherwise reset.
async fn spawn_process(config: &SupervisorConfig) -> Result<(Child, Client), TscClientError> {
    check_version(&config.tsc_path, config.expected_versions)?;

    let mut command = Command::new(config.tsc_path.as_std_path());
    command.arg("--api").arg("--async").arg("--cwd").arg(config.cwd.as_std_path());
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.envs(config.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    // Best-effort fallback if `Supervisor::shutdown` is never called — this alone
    // doesn't guarantee reaping (Drop can't await), only `.shutdown()` does.
    command.kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|source| TscClientError::SpawnFailed { path: config.tsc_path.clone(), source: Arc::new(source) })?;

    let Some(stdin) = child.stdin.take() else { unreachable!("stdin is None despite Stdio::piped()") };
    let Some(stdout) = child.stdout.take() else { unreachable!("stdout is None despite Stdio::piped()") };
    let Some(stderr) = child.stderr.take() else { unreachable!("stderr is None despite Stdio::piped()") };

    spawn_stderr_drain(stderr);
    let client = Client::connect(stdin, stdout, config.max_frame_bytes);

    Ok((child, client))
}

/// Owns one `tsc --api` child process across its lifetime: spawn, version check,
/// crash-triggered restart (bounded, to avoid a restart storm), and shutdown/reaping.
pub struct Supervisor {
    child: Child,
    client: Client,
    config: SupervisorConfig,
    restart_attempts: Vec<Instant>,
    /// Set once a restart-storm cap is hit. Terminal — every later call short-circuits
    /// without touching the process again.
    failed: bool,
}

impl Supervisor {
    pub async fn spawn(config: SupervisorConfig) -> Result<Self, TscClientError> {
        let (child, client) = spawn_process(&config).await?;
        Ok(Self { child, client, config, restart_attempts: Vec::new(), failed: false })
    }

    /// The current session's low-level handle. Cheap to clone. A clone obtained before a
    /// supervisor-driven restart fails immediately with the reason its session closed with
    /// (`Crashed`, `Write`, `Read`, `MalformedFrame` or `OversizedFrame`) — call this again to get the live one. A
    /// clone held across [`Supervisor::shutdown`] fails with `SessionClosed`.
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// The operating system's id of the running `tsc` process, `None` once it has exited.
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    /// True while the current session is open and the supervisor hasn't permanently failed. A crash between calls
    /// to this method and the next `request()` is still possible — this is a snapshot, not a guarantee.
    pub fn is_alive(&self) -> bool {
        self.client.closed().is_none() && !self.failed
    }

    /// Sends a request through the current session. A session that ended before the call is restarted first
    /// (budget permitting); one that ends during the call is restarted and the call retried once. At most one
    /// restart per call, so a request lost to a crash is never silently replayed more than once. A `Timeout` on a
    /// live session restarts nothing.
    ///
    /// Worst case is one restart plus one retry: `deadline` applies to each attempt separately, and a restart adds up
    /// to 800ms of backoff plus the spawn.
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
        deadline: Duration,
    ) -> Result<TaggedResponse, TscClientError> {
        if self.failed {
            return Err(self.storm());
        }

        let restarted_first = self.client.closed().is_some();
        if restarted_first {
            self.restart().await?;
        }

        let result = self.client.request(method, params.clone(), deadline).await;
        if result.is_err() && !restarted_first && self.client.closed().is_some() {
            self.restart().await?;
            return self.client.request(method, params, deadline).await;
        }
        result
    }

    fn storm(&self) -> TscClientError {
        TscClientError::RestartStormDetected { attempts: self.restart_attempts.len() as u32, window: RESTART_WINDOW }
    }

    async fn restart(&mut self) -> Result<(), TscClientError> {
        let now = Instant::now();
        self.restart_attempts.retain(|attempt| now.duration_since(*attempt) < RESTART_WINDOW);

        if self.restart_attempts.len() as u32 >= MAX_RESTARTS {
            self.failed = true;
            return Err(self.storm());
        }

        let backoff = BACKOFF_STEPS.get(self.restart_attempts.len()).copied().unwrap_or(BACKOFF_MAX);
        // Counted before the spawn and the backoff: a spawn failure or a caller cancelling this call mid-restart still
        // burns one attempt, by design.
        self.restart_attempts.push(now);
        tokio::time::sleep(backoff).await;

        // Best-effort cleanup of the crashed process before respawning — it may
        // already be gone, so failures here are expected and ignored.
        self.child.start_kill().ok();
        self.child.wait().await.ok();

        let (child, client) = spawn_process(&self.config).await?;
        self.child = child;
        self.client = client;
        Ok(())
    }

    /// Ends the session and guarantees the child is reaped, never left as a zombie: close the session (fails
    /// in-flight requests with `SessionClosed` and makes the child see stdin EOF), race a grace period against a
    /// clean exit, force-kill past it, then unconditionally wait so the OS process table entry is always cleared.
    /// Returns the reaped exit status.
    pub async fn shutdown(mut self) -> Result<ExitStatus, TscClientError> {
        self.client.close();

        match tokio::time::timeout(SHUTDOWN_GRACE, self.child.wait()).await {
            Ok(waited) => waited.map_err(|source| TscClientError::Reap(Arc::new(source))),
            Err(_elapsed) => {
                self.child.start_kill().ok();
                // Unconditional — never left unreaped.
                self.child.wait().await.map_err(|source| TscClientError::Reap(Arc::new(source)))
            }
        }
    }
}
