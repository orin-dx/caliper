//! Experimental TypeScript 7.1 checker backend (`--experimental-semantic-types`).
//!
//! Spawns `tsc --api` (through `caliper-docgen-tsc-client`), asks it for the props of each component the structural
//! pass found, and hands `caliper-docgen-core` a [`SemanticOutcome`] to merge (ADR 0006, ADR 0007). It is always
//! compiled in and enabled at run time. Any failure to start or finish (no `tsc`, an unpinned version, a crash, a
//! timeout) yields an outcome with no components and one diagnostic, never an error: the structural result is
//! complete on its own.
//!
//! This slice answers function components exported by name or as `default`. Class components, overloaded or
//! generic components and union props types keep their structural result and say why.

mod component;
mod convert;
mod locate;
mod rpc;

use std::sync::Arc;
use std::time::Duration;

use caliper_docgen_core::types::semantic::{PropInterner, SemanticComponent, SemanticFallback, SemanticOutcome};
use caliper_docgen_core::types::ExtractionOutput;
use caliper_docgen_tsc_client::{Supervisor, SupervisorConfig};
use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::component::Answer;
use crate::convert::Converter;
use crate::rpc::{CallError, Rpc};

/// The `typescript` package versions whose protocol this backend was traced against. The protocol has no version
/// handshake, so the package's `package.json` version is the whole check.
pub const PINNED_VERSIONS: &[&str] = &["7.1.0-beta"];

/// How many files are asked about at once. `tsc` answers requests one at a time; this keeps it from idling on round
/// trips.
const FILES_IN_FLIGHT: usize = 16;

/// A component the structural pass found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub file_path: Utf8PathBuf,
    pub display_name: String,
    pub exported: bool,
    /// Props the structural pass already has a description for. The checker is not asked for those: it is 65% of
    /// the requests on Mantine, and the merge keeps the structural description when the checker's is empty.
    pub described: FxHashSet<String>,
}

/// The components of `output`, in output order.
pub fn targets_of(output: &ExtractionOutput) -> Vec<Target> {
    output
        .components
        .values()
        .map(|entry| Target {
            file_path: entry.file_path.clone(),
            display_name: entry.display_name.clone(),
            exported: entry.exported,
            described: entry
                .props
                .values()
                .filter(|prop| !prop.description.is_empty())
                .map(|prop| prop.name.clone())
                .collect(),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct SemanticRequest {
    /// Where `tsc` finds `node_modules`; its `--cwd`, and where the search for `typescript` starts.
    pub project_root: Utf8PathBuf,
    pub tsconfig: Option<Utf8PathBuf>,
    /// An explicit `tsc` binary, instead of `node_modules/typescript/bin/tsc` above `project_root`.
    pub tsc_path: Option<Utf8PathBuf>,
    pub targets: Vec<Target>,
    /// Bound on each request, and on the `tsc` start-up handshake.
    pub request_timeout: Duration,
    pub expected_versions: &'static [&'static str],
    /// Write every request and its answer here as JSON, to refresh the replay fixture the tests use.
    pub record_to: Option<Utf8PathBuf>,
    /// Environment for the `tsc` process; the tests use it to pick a replay scenario.
    pub extra_env: Vec<(String, String)>,
}

impl SemanticRequest {
    pub fn new(project_root: Utf8PathBuf, targets: Vec<Target>) -> Self {
        Self {
            project_root,
            tsconfig: None,
            tsc_path: None,
            targets,
            request_timeout: Duration::from_secs(60),
            expected_versions: PINNED_VERSIONS,
            record_to: None,
            extra_env: Vec::new(),
        }
    }
}

/// Resolves the request's components with the checker. Blocks; builds its own single-threaded runtime, so it must
/// not be called from inside one.
pub fn resolve(request: SemanticRequest) -> SemanticOutcome {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return SemanticOutcome::unavailable(format!("could not start an async runtime: {error}")),
    };
    runtime.block_on(run(request))
}

async fn run(request: SemanticRequest) -> SemanticOutcome {
    let mut outcome = SemanticOutcome::default();
    let mut by_file: FxHashMap<&Utf8Path, Vec<&Target>> = FxHashMap::default();
    let mut order: Vec<&Utf8Path> = Vec::new();
    for target in &request.targets {
        if !target.exported {
            outcome.fallbacks.push(fallback_of(target, "not exported by its module"));
            continue;
        }
        let group = by_file.entry(target.file_path.as_path()).or_default();
        if group.is_empty() {
            order.push(target.file_path.as_path());
        }
        group.push(target);
    }
    if order.is_empty() {
        return outcome;
    }

    let tsc_path = match locate::find_tsc(&request.project_root, request.tsc_path.as_deref()) {
        Ok(path) => path,
        Err(reason) => return SemanticOutcome::unavailable(reason),
    };
    let mut config = SupervisorConfig::new(tsc_path, request.project_root.clone(), request.expected_versions);
    config.handshake_timeout = request.request_timeout;
    config.extra_env.clone_from(&request.extra_env);
    let supervisor = match Supervisor::spawn(config).await {
        Ok(supervisor) => supervisor,
        Err(error) => return SemanticOutcome::unavailable(error.to_string()),
    };

    let files: Vec<String> = order.iter().map(|file| file.to_string()).collect();
    let session = session(&supervisor, &request, &files, &by_file, &order).await;
    supervisor.shutdown().await.ok();

    match session {
        Ok(mut answered) => {
            outcome.components.append(&mut answered.components);
            outcome.fallbacks.append(&mut answered.fallbacks);
            outcome
        }
        Err(error) => SemanticOutcome::unavailable(format!("the checker stopped answering: {error}")),
    }
}

async fn session(
    supervisor: &Supervisor,
    request: &SemanticRequest,
    files: &[String],
    by_file: &FxHashMap<&Utf8Path, Vec<&Target>>,
    order: &[&Utf8Path],
) -> Result<SemanticOutcome, CallError> {
    let started = std::time::Instant::now();
    let rpc = Arc::new(
        Rpc::open(
            supervisor.client(),
            request.request_timeout,
            request.tsconfig.as_ref().map(|path| path.as_str()),
            files,
            request.record_to.is_some(),
        )
        .await?,
    );
    let opened = started.elapsed();
    let converter = Converter::new(rpc.clone());
    let limit = Arc::new(Semaphore::new(FILES_IN_FLIGHT));
    let interner = Arc::new(PropInterner::default());

    let mut tasks = JoinSet::new();
    for file in order {
        let targets: Vec<Target> = by_file.get(file).into_iter().flatten().map(|target| (*target).clone()).collect();
        let (rpc, converter, limit, interner) = (rpc.clone(), converter.clone(), limit.clone(), interner.clone());
        tasks.spawn(async move {
            let _permit = limit.acquire().await;
            answer_file(&rpc, &converter, &interner, targets).await
        });
    }

    let mut outcome = SemanticOutcome::default();
    let mut failure = None;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(mut answered)) => {
                outcome.components.append(&mut answered.components);
                outcome.fallbacks.append(&mut answered.fallbacks);
            }
            Ok(Err(error)) => {
                failure = Some(error);
                tasks.abort_all();
            }
            // A panic inside a worker must not silently leave its components structural.
            Err(joined) if joined.is_panic() => {
                failure = Some(CallError::Fatal("a worker panicked".to_string()));
                tasks.abort_all();
            }
            // A cancelled task is one this loop aborted after a failure.
            Err(_cancelled) => {}
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }

    if let (Some(path), Some(recording)) = (&request.record_to, rpc.recording()) {
        if let Ok(json) = serde_json::to_string_pretty(&recording) {
            if let Err(error) = std::fs::write(path, json) {
                tracing::warn!("could not write the recording to {path}: {error}");
            }
        }
    }
    tracing::debug!(
        requests = ?rpc.calls(),
        files = files.len(),
        program_ms = opened.as_millis() as u64,
        total_ms = started.elapsed().as_millis() as u64,
        "semantic backend finished"
    );
    rpc.release().await;
    outcome.components.sort_by(|a, b| (&a.file_path, &a.display_name).cmp(&(&b.file_path, &b.display_name)));
    outcome.fallbacks.sort_by(|a, b| (&a.file_path, &a.display_name).cmp(&(&b.file_path, &b.display_name)));
    Ok(outcome)
}

fn fallback_of(target: &Target, reason: &str) -> SemanticFallback {
    SemanticFallback {
        file_path: target.file_path.clone(),
        display_name: target.display_name.clone(),
        reason: reason.to_string(),
    }
}

/// A fatal error aborts the run; a call the checker rejected only sends that component to its structural result.
async fn answer_file(
    rpc: &Rpc,
    converter: &Converter,
    interner: &PropInterner,
    targets: Vec<Target>,
) -> Result<SemanticOutcome, CallError> {
    let mut outcome = SemanticOutcome::default();
    let Some(first) = targets.first() else { return Ok(outcome) };
    let exports = match module_exports(rpc, first.file_path.as_str()).await {
        Ok(Some(exports)) => exports,
        Ok(None) | Err(CallError::Rejected(_)) => {
            outcome
                .fallbacks
                .extend(targets.iter().map(|target| fallback_of(target, "not part of the checker's program")));
            return Ok(outcome);
        }
        Err(fatal) => return Err(fatal),
    };

    for target in &targets {
        match component::props_of(rpc, converter, interner, &exports, target).await {
            Ok(Answer::Props(props)) => outcome.components.push(SemanticComponent {
                file_path: target.file_path.clone(),
                display_name: target.display_name.clone(),
                props,
            }),
            Ok(Answer::Fallback(reason)) => outcome.fallbacks.push(fallback_of(target, &reason)),
            Err(CallError::Rejected(message)) => {
                outcome.fallbacks.push(fallback_of(target, &format!("the checker rejected a request: {message}")));
            }
            Err(fatal) => return Err(fatal),
        }
    }
    Ok(outcome)
}

async fn module_exports(rpc: &Rpc, file: &str) -> Result<Option<Vec<rpc::SymbolInfo>>, CallError> {
    let Some(module) = rpc.source_file_symbol(file).await? else { return Ok(None) };
    Ok(Some(rpc.exports_of_module(&module.reference).await?))
}
