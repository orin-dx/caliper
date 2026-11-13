//! TypeScript 7.1 checker backend, on by default (ADR 0026; `--no-checker` turns it off).
//!
//! Asks TypeScript's checker for the props of each component the structural pass found, and hands
//! `caliper-docgen-core` a [`SemanticOutcome`] to merge (ADR 0006, ADR 0007). The checker is the `tsrs` crates, linked
//! in and run on a thread of their own; an explicit `tsc_path` runs that binary as a `tsc --api` process instead
//! (through `caliper-docgen-tsc-client`), which is how the tests and the parity gate run the Go checker. It is always
//! compiled in and enabled at run time. Any failure to start or finish (a missing or unpinned binary, a crash, a
//! timeout) yields an outcome with no components and one diagnostic, never an error: the structural result is
//! complete on its own.
//!
//! This slice answers function components by the exports the extractor recorded for them (a name, `default`, or the
//! property of an exported component a sub-component is attached to); a generic one whose first type parameter
//! declares a default is read at that default (`instantiate`). Class components, overloaded components and union props
//! types keep their structural result and say why.

#![deny(missing_docs)]

mod checker;
mod component;
mod convert;
mod declared;
mod instantiate;
mod kept;
mod locate;
mod on_demand;
mod printed;
mod rpc;
mod stamps;

pub use on_demand::{OnDemand, OnDemandConfig, Recycles, Snapshots};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use caliper_docgen_core::types::ExtractionOutput;
use caliper_docgen_core::types::semantic::{PropInterner, SemanticComponent, SemanticFallback, SemanticOutcome};
use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::FxHashMap;
use tokio::task::JoinSet;

use crate::checker::{Backend, Start};
use crate::component::Answer;
use crate::convert::Converter;
use crate::instantiate::Instantiation;
use crate::rpc::{CallError, REQUESTS, Rpc};

/// The checker binaries a `tsc_path` may name, the builds whose protocol this backend was traced against (the built-in
/// checker is pinned in `Cargo.toml` instead): the `typescript` 7.1 series, from `7.1.0-beta` on, and the `tsrs` builds
/// listed exactly (`<tsrs version>-ts<typescript version>`), each only while its TypeScript part is in the series above.
/// The protocol has no version handshake, so the package's `package.json` version and the initialize call that follows
/// the spawn are the whole check; a build that fails either takes the structural result with one warning.
pub const SUPPORTED_VERSIONS: &[&str] = &["7.1", "0.23.0-ts7.1.0-beta"];

/// How many files are asked about at once, shared among the programs. A program's checker answers requests one at a
/// time; this keeps it from idling between them. Four per program is enough: more only queue behind the checker, and
/// sixteen measured slower and 150 MiB larger on Chakra's components (docs/gate-measurements.md).
const FILES_IN_FLIGHT: usize = 8;

/// The most threads the whole-project run converts the checker's answers on: past four, the checker's own thread is
/// the bound (measured on Chakra's components, docs/gate-measurements.md).
const CONVERTER_THREADS: usize = 4;

/// How many programs a whole-project run asks at once, each on a thread of the linked checker: a program's requests are
/// answered one at a time, and the checker's answering is what bounds the run (docs/gate-measurements.md).
const CHECKER_PROGRAMS: usize = 2;

/// The fewest files a program answers for: a project smaller than this asks one program, which builds no second checker
/// for a handful of components.
const MIN_FILES_PER_PROGRAM: usize = 32;

/// A component the structural pass found.
pub use caliper_docgen_core::types::semantic::SemanticTarget as Target;

/// The components of `output`, in output order.
pub fn targets_of(output: &ExtractionOutput) -> Vec<Target> {
    output
        .components
        .iter()
        .map(|(key, entry)| Target {
            key: key.clone(),
            file_path: entry.file_path.clone(),
            display_name: entry.display_name.clone(),
            exports: entry.exports.clone(),
            described: entry
                .props
                .values()
                .filter(|prop| !prop.description.is_empty())
                .map(|prop| prop.name.clone())
                .collect(),
            // The whole-project merge leaves `@types/react`'s props to the inheritance layers.
            react_wanted: Some(Default::default()),
        })
        .collect()
}

/// One whole-project request to the checker: the components to answer for, and which checker to ask. Built with
/// [`SemanticRequest::new`] and answered by [`resolve`].
#[derive(Debug, Clone)]
pub struct SemanticRequest {
    /// Where the checker finds `node_modules`: its working directory.
    pub project_root: Utf8PathBuf,
    /// A tsconfig whose compiler options the checker uses (read with `parseConfigFile`); with none, a default set.
    pub tsconfig: Option<Utf8PathBuf>,
    /// A checker binary to run as a `tsc --api` process (`typescript@7.1`'s `tsc`, or `tsrs`) instead of the built-in
    /// checker.
    pub tsc_path: Option<Utf8PathBuf>,
    /// The components to ask about.
    pub targets: Vec<Target>,
    /// Bound on each request, and on a checker process's start-up handshake.
    pub request_timeout: Duration,
    /// The versions a checker binary's `package.json` may declare; one that is not among them is a start-up failure.
    /// The built-in checker is pinned and not checked.
    pub expected_versions: &'static [&'static str],
    /// Write every request and its answer here as JSON, to refresh the replay fixture the tests use.
    pub record_to: Option<Utf8PathBuf>,
    /// Environment for a checker process; the tests use it to pick a replay scenario.
    pub extra_env: Vec<(String, String)>,
    /// Ask the checker about a component's props, and about the members of each type, last to first. The answer must
    /// not change: no id the checker hands out, and so no order of queries, may show in it. The tests set this.
    pub reverse_queries: bool,
}

impl SemanticRequest {
    /// A request for `targets` under `project_root`, with a 60 second timeout, the supported versions, and no
    /// tsconfig or explicit `tsc` path.
    pub fn new(project_root: Utf8PathBuf, targets: Vec<Target>) -> Self {
        Self {
            project_root,
            tsconfig: None,
            tsc_path: None,
            targets,
            request_timeout: Duration::from_secs(60),
            expected_versions: SUPPORTED_VERSIONS,
            record_to: None,
            extra_env: Vec::new(),
            reverse_queries: false,
        }
    }
}

/// Resolves the request's components with the checker. Blocks; builds its own runtime, so it must not be called from
/// inside one. The runtime has up to four threads: the built-in checker answers two programs at once, each on a thread
/// of its own, and while it does, the answers it gave the other files in flight are read and converted on these.
pub fn resolve(request: SemanticRequest) -> SemanticOutcome {
    resolve_noting(request, None)
}

/// [`resolve`], with the answer kept in `cache_dir` for the next run over the same request: a run whose built-in
/// checker would read the disk exactly as the last one did gets the last one's answer without starting the checker.
/// Every question that checker asked of the file system (each file it read or looked for, each directory it listed,
/// the tsconfig and what it extends) is asked again first, and any different answer, a new file where it found none
/// included, discards the kept answer whole; so does another build of Caliper or of the pinned checker, and another
/// request. A request that names a checker binary (`tsc_path`) or records its calls is answered as [`resolve`] answers
/// it, and nothing is kept for it. The answer is the one [`resolve`] gives.
pub fn resolve_cached(request: SemanticRequest, cache_dir: &Utf8Path) -> SemanticOutcome {
    if request.tsc_path.is_some() || request.record_to.is_some() {
        return resolve(request);
    }
    let file = cache_dir.join(kept::FILE);
    let key = kept::request_key(&request);
    let started = std::time::Instant::now();
    if let Some(outcome) = kept::load(&file, key) {
        tracing::debug!(ms = started.elapsed().as_millis() as u64, "the checker's kept answer still holds");
        return outcome;
    }
    let reads = Arc::new(kept::Reads::default());
    let outcome = resolve_noting(request, Some(reads.clone()));
    kept::save(&file, key, &reads, &outcome);
    outcome
}

fn resolve_noting(request: SemanticRequest, reads: Option<Arc<kept::Reads>>) -> SemanticOutcome {
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get().min(CONVERTER_THREADS));
    let built = tokio::runtime::Builder::new_multi_thread().worker_threads(threads).enable_all().build();
    let runtime = match built {
        Ok(runtime) => runtime,
        Err(error) => return SemanticOutcome::unavailable(format!("could not start an async runtime: {error}")),
    };
    runtime.block_on(run_noting(request, None, reads))
}

/// The exported targets by file, in first-seen order, and an outcome that already holds a fallback for each target
/// the checker cannot be asked about.
fn group_by_file(request: &SemanticRequest) -> (SemanticOutcome, FxHashMap<&Utf8Path, Vec<&Target>>, Vec<&Utf8Path>) {
    let mut outcome = SemanticOutcome::default();
    let mut by_file: FxHashMap<&Utf8Path, Vec<&Target>> = FxHashMap::default();
    let mut order: Vec<&Utf8Path> = Vec::new();
    for target in &request.targets {
        if target.exports.is_empty() {
            outcome.fallbacks.push(fallback_of(target, "not exported by its module"));
            continue;
        }
        let group = by_file.entry(target.file_path.as_path()).or_default();
        if group.is_empty() {
            order.push(target.file_path.as_path());
        }
        group.push(target);
    }
    (outcome, by_file, order)
}

/// The checker `request` names: its `tsc_path` as a process, else the built-in one.
async fn start_checker(request: &SemanticRequest, threads: usize) -> Result<Backend, String> {
    start_noting(request, threads, None).await
}

/// [`start_checker`], the linked checker noting what it reads in `reads` ([`kept`]).
async fn start_noting(
    request: &SemanticRequest,
    threads: usize,
    reads: Option<Arc<kept::Reads>>,
) -> Result<Backend, String> {
    Backend::start(Start {
        project_root: &request.project_root,
        tsc_path: request.tsc_path.as_deref(),
        expected_versions: request.expected_versions,
        timeout: request.request_timeout,
        extra_env: &request.extra_env,
        threads,
        reads,
    })
    .await
}

/// Answers `request` with a checker started for it, asking `programs` programs at once, or by default as many as
/// [`CHECKER_PROGRAMS`] and the files allow ([`MIN_FILES_PER_PROGRAM`]).
#[cfg(test)]
async fn run(request: SemanticRequest, programs: Option<usize>) -> SemanticOutcome {
    run_noting(request, programs, None).await
}

/// [`run`], the linked checker noting what it reads in `reads` ([`kept`]).
async fn run_noting(
    request: SemanticRequest,
    programs: Option<usize>,
    reads: Option<Arc<kept::Reads>>,
) -> SemanticOutcome {
    if request.targets.iter().all(|target| target.exports.is_empty()) {
        return group_by_file(&request).0;
    }
    let files = group_by_file(&request).2.len();
    let programs = programs.unwrap_or_else(|| CHECKER_PROGRAMS.min(files.div_ceil(MIN_FILES_PER_PROGRAM))).max(1);
    let checker = match start_noting(&request, programs, reads).await {
        Ok(checker) => checker,
        Err(reason) => return SemanticOutcome::unavailable(reason),
    };
    let outcome = answer_with(&checker, &request).await;
    checker.shutdown().await;
    outcome
}

/// The files of the request's exported targets, in first-seen order: the roots of the program that answers it.
fn root_files(request: &SemanticRequest) -> Vec<String> {
    group_by_file(request).2.iter().map(|file| file.to_string()).collect()
}

/// Answers the request's targets with a checker that is already running: one snapshot of as many programs rooted at
/// their files as the checker answers at once ([`Backend::parallel_programs`]), released once they are answered.
async fn answer_with(checker: &Backend, request: &SemanticRequest) -> SemanticOutcome {
    let files = root_files(request);
    if files.is_empty() {
        return group_by_file(request).0;
    }
    let started = std::time::Instant::now();
    let tsconfig = request.tsconfig.as_ref().map(|path| path.as_str());
    let count = checker.parallel_programs();
    let record = request.record_to.is_some();
    let opened = Rpc::open_programs(checker.client(), request.request_timeout, tsconfig, &files, record, count).await;
    let programs: Vec<Arc<Rpc>> = match opened {
        Ok(programs) => programs.into_iter().map(Arc::new).collect(),
        Err(error) => return SemanticOutcome::unavailable(format!("the checker stopped answering: {error}")),
    };
    tracing::debug!(program_ms = started.elapsed().as_millis() as u64, programs = count, "semantic program built");
    let answered = answer_on(&programs, request, &files).await;
    if let Some(first) = programs.first() {
        first.release().await;
    }
    answered.unwrap_or_else(|error| SemanticOutcome::unavailable(format!("the checker stopped answering: {error}")))
}

/// Answers the request's targets on a snapshot whose programs are each rooted at `files` ([`root_files`]), one session
/// bound to each: a file is answered by one of them, the `n`th file by the `n % programs.len()`th. Each program has its
/// checker and its handles, so each has its own [`Converter`]; since every program holds the same files, each answers
/// a component as any other would, and the output does not depend on how the files are shared out. The snapshot is
/// left as it is, for the caller to keep or release.
async fn answer_on(
    programs: &[Arc<Rpc>],
    request: &SemanticRequest,
    files: &[String],
) -> Result<SemanticOutcome, CallError> {
    let (mut outcome, by_file, order) = group_by_file(request);
    let mut answered = session(programs, request, files, &by_file, &order).await?;
    outcome.components.append(&mut answered.components);
    outcome.fallbacks.append(&mut answered.fallbacks);
    Ok(outcome)
}

async fn session(
    programs: &[Arc<Rpc>],
    request: &SemanticRequest,
    files: &[String],
    by_file: &FxHashMap<&Utf8Path, Vec<&Target>>,
    order: &[&Utf8Path],
) -> Result<SemanticOutcome, CallError> {
    let started = std::time::Instant::now();
    let Some(rpc) = programs.first() else { return Ok(SemanticOutcome::default()) };
    let converters: Vec<Converter> =
        programs.iter().map(|program| Converter::new(program.clone(), request.reverse_queries)).collect();
    let interner = Arc::new(PropInterner::default());
    // The files in order; each worker takes the next one when it is done with its last, so a program whose files take
    // longer answers fewer of them.
    let queue: VecDeque<Vec<Target>> = order
        .iter()
        .map(|file| by_file.get(file).into_iter().flatten().map(|target| (*target).clone()).collect())
        .collect();
    let queue = Arc::new(Mutex::new(queue));

    let mut tasks = JoinSet::new();
    for worker in 0..FILES_IN_FLIGHT {
        let lane = worker % programs.len();
        let (Some(rpc), Some(converter)) = (programs.get(lane), converters.get(lane)) else { continue };
        let (rpc, converter, queue, interner) = (rpc.clone(), converter.clone(), queue.clone(), interner.clone());
        tasks.spawn(async move {
            let mut answered = (SemanticOutcome::default(), Vec::new());
            loop {
                let next = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop_front();
                let Some(targets) = next else { return Ok(answered) };
                let (mut outcome, mut generic) = answer_file(&rpc, &converter, &interner, targets).await?;
                answered.0.components.append(&mut outcome.components);
                answered.0.fallbacks.append(&mut outcome.fallbacks);
                answered.1.append(&mut generic);
            }
        });
    }

    let mut outcome = SemanticOutcome::default();
    let mut generic = Vec::new();
    let mut failure = None;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok((mut answered, mut instantiations))) => {
                outcome.components.append(&mut answered.components);
                outcome.fallbacks.append(&mut answered.fallbacks);
                generic.append(&mut instantiations);
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
    if !generic.is_empty() {
        let mut instantiated =
            instantiate::answer(rpc, files, &request.project_root, &interner, request.reverse_queries, generic).await?;
        outcome.components.append(&mut instantiated.components);
        outcome.fallbacks.append(&mut instantiated.fallbacks);
    }

    if let (Some(path), Some(recording)) = (&request.record_to, rpc.recording())
        && let Ok(json) = serde_json::to_string_pretty(&recording)
        && let Err(error) = std::fs::write(path, json)
    {
        tracing::warn!("could not write the recording to {path}: {error}");
    }
    tracing::debug!(
        requests = ?rpc.calls(),
        files = files.len(),
        type_memo = ?converters.iter().map(Converter::memo_stats).fold((0, 0), |sum, (hits, misses)| (sum.0 + hits, sum.1 + misses)),
        total_ms = started.elapsed().as_millis() as u64,
        "semantic backend finished"
    );
    outcome.components.sort_by(|a, b| a.key.cmp(&b.key));
    outcome.fallbacks.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(outcome)
}

fn fallback_of(target: &Target, reason: &str) -> SemanticFallback {
    SemanticFallback { key: target.key.clone(), reason: reason.to_string() }
}

/// A fatal error aborts the run; a call the checker rejected only sends that component to its structural result. The
/// generic components come back apart, to be answered from their instantiations once every file is done.
async fn answer_file(
    rpc: &Rpc,
    converter: &Converter,
    interner: &PropInterner,
    targets: Vec<Target>,
) -> Result<(SemanticOutcome, Vec<(Target, Instantiation)>), CallError> {
    let mut outcome = SemanticOutcome::default();
    let mut generic = Vec::new();
    let Some(first) = targets.first() else { return Ok((outcome, generic)) };
    let module = match module_exports(rpc, first.file_path.as_str()).await {
        Ok(Some(module)) => module,
        Ok(None) | Err(CallError::Rejected(_)) => {
            outcome
                .fallbacks
                .extend(targets.iter().map(|target| fallback_of(target, "not part of the checker's program")));
            return Ok((outcome, generic));
        }
        Err(fatal) => return Err(fatal),
    };

    for target in &targets {
        let requests = Arc::new(AtomicU64::new(0));
        let started = std::time::Instant::now();
        let answered =
            REQUESTS.scope(requests.clone(), component::props_of(rpc, converter, interner, &module, target)).await;
        tracing::debug!(
            component = %target.display_name,
            requests = requests.load(Ordering::Relaxed),
            ms = started.elapsed().as_millis() as u64,
            "semantic component answered"
        );
        match answered {
            Ok(Answer::Props(component::Props { props, composes })) => {
                outcome.components.push(SemanticComponent { key: target.key.clone(), props, composes });
            }
            Ok(Answer::Fallback(reason)) => outcome.fallbacks.push(fallback_of(target, &reason)),
            Ok(Answer::Instantiate(instantiation)) => generic.push((target.clone(), instantiation)),
            Err(CallError::Rejected(message)) => {
                outcome.fallbacks.push(fallback_of(target, &format!("the checker rejected a request: {message}")));
            }
            Err(fatal) => return Err(fatal),
        }
    }
    Ok((outcome, generic))
}

/// The module `file` is, as the checker reads it, with its exports; `None` when the file is not in its program.
async fn module_exports(rpc: &Rpc, file: &str) -> Result<Option<component::Module>, CallError> {
    let Some(symbol) = rpc.source_file_symbol(file).await? else { return Ok(None) };
    let exports = rpc.exports_of_module(&symbol.reference).await?;
    Ok(Some(component::Module { symbol, exports }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use caliper_docgen_core::pipeline::{PipelineOptions, extract};

    use super::*;

    /// A project of `files` components over shared types (a union, an interface base, an `Omit` in an intersection, a
    /// recursive alias) whose props all extend an interface the last file augments (`declare global`).
    fn shared_types_project(files: usize) -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::Builder::new().prefix("Programs").tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        std::fs::write(
            root.join("types.ts"),
            "export type Size = 'sm' | 'md' | 'lg';\n\
             export interface Base {\n  /** The id. */\n  id: string;\n  size?: Size;\n}\n\
             export type Tree = { label: string; children?: Tree[] };\n\
             export type Styled = Omit<Base, 'id'> & { tone?: 'a' | 'b'; onPick?: (value: Size) => void };\n\
             declare global { interface Augmented { fromTypes?: number } }\n",
        )
        .unwrap();
        for index in 0..files {
            let augments = if index + 1 == files {
                "declare global { interface Augmented { fromLastFile?: boolean } }\n"
            } else {
                ""
            };
            std::fs::write(
                root.join(format!("C{index}.tsx")),
                format!(
                    "import type {{ Base, Styled, Tree }} from './types';\n{augments}\
                     export interface C{index}Props extends Base, Augmented {{ own{index}: number; tree?: Tree; styled: Styled }}\n\
                     export function C{index}(props: C{index}Props) {{ return null; }}\n"
                ),
            )
            .unwrap();
        }
        (dir, root)
    }

    fn answered(root: &Utf8Path, programs: usize) -> SemanticOutcome {
        let output = extract(&PipelineOptions { src_dirs: vec![root.to_owned()], ..PipelineOptions::default() });
        let request = SemanticRequest::new(root.to_owned(), targets_of(&output));
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(run(request, Some(programs)))
    }

    /// Three programs on the built-in checker, each answering some of the files, give the answer one program gives:
    /// each holds every root file, so what one file declares (here a global augmentation in the last) reaches the
    /// components the other programs answer.
    #[test]
    fn several_programs_answer_as_one_does() {
        let (_dir, root) = shared_types_project(9);

        let one = answered(&root, 1);
        let three = answered(&root, 3);

        assert!(one.diagnostics.is_empty() && one.fallbacks.is_empty(), "{one:?}");
        assert_eq!(one.components.len(), 9);
        for component in &one.components {
            let names: Vec<&str> = component.props.iter().map(|prop| prop.name.as_str()).collect();
            assert!(names.contains(&"fromLastFile") && names.contains(&"fromTypes"), "{names:?}");
        }
        assert_eq!(one, three);
    }

    /// A symbol's type is kept by the symbol, not by what it is named or where it is declared: two instantiations of
    /// one generic interface list a property of the same name and declaration, each a symbol of its own with its own
    /// type, and the second component, answered after the first on the same program, is not handed the first's type.
    #[test]
    fn a_kept_symbol_type_is_the_type_of_that_symbol() {
        let (_dir, root) = shared_types_project(0);
        std::fs::write(
            root.join("Boxes.tsx"),
            "interface Box<T> { value: T; label?: string }\n\
             export function Text(props: Box<string>) { return null; }\n\
             export function Count(props: Box<number>) { return null; }\n\
             export function Again(props: Box<string>) { return null; }\n",
        )
        .unwrap();
        let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();

        let outcome = runtime.block_on(run(SemanticRequest::new(root.clone(), targets_of(&output)), Some(1)));

        let value = |key: &str| {
            let component = outcome.components.iter().find(|component| component.key == key).unwrap();
            component.props.iter().find(|prop| prop.name == "value").unwrap().prop_type.clone()
        };
        assert_eq!(value("Text"), caliper_docgen_core::types::PropType::String);
        assert_eq!(value("Count"), caliper_docgen_core::types::PropType::Number);
        assert_eq!(value("Again"), caliper_docgen_core::types::PropType::String);
    }

    /// Stands in for the built-in checker's session: it initializes and makes a snapshot of as many programs as asked,
    /// and panics on any question about them, counting how many it was asked.
    struct Panicking {
        asked: Arc<std::sync::atomic::AtomicU32>,
    }

    impl checker::Served for Panicking {
        fn handle(&self, method: &str, params: &[u8]) -> Result<String, checker::Failure> {
            match method {
                "initialize" | "release" => Ok("null".to_string()),
                "createSnapshot" => {
                    let params: serde_json::Value = serde_json::from_slice(params).unwrap();
                    let count = params["createPrograms"].as_array().map_or(1, Vec::len);
                    let projects: Vec<_> = (0..count).map(|n| serde_json::json!({ "id": format!("p{n}") })).collect();
                    Ok(serde_json::json!({ "snapshot": 1, "projects": projects }).to_string())
                }
                _ => {
                    self.asked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    panic!("checker invariant broken")
                }
            }
        }

        fn discard(&self, _method: &str, _answer: &str) {}

        fn close(&self) {}
    }

    /// A whole-project run on two lanes whose checker panics: the lane that met it fails its file, the other lane
    /// stops, and the run gives every component its structural answer with one diagnostic, never a partial answer.
    #[test]
    fn a_panic_on_one_lane_fails_the_whole_run_to_the_structural_answer() {
        let (_dir, root) = shared_types_project(6);
        let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
        let request = SemanticRequest::new(root.clone(), targets_of(&output));
        let asked = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let session = Panicking { asked: Arc::clone(&asked) };
        let checker = Backend::InProcess(checker::InProcess::spawn(2, move || session).unwrap());
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();

        let outcome = runtime.block_on(answer_with(&checker, &request));

        assert!(outcome.components.is_empty(), "{:?}", outcome.components);
        assert_eq!(outcome.diagnostics.len(), 1, "{:?}", outcome.diagnostics);
        let message = &outcome.diagnostics[0].message;
        // Whichever lane's error the run meets first: the panic, the other lane's refusal after it, or a request that
        // was still queued on a lane that stopped.
        let reason = message.strip_prefix("TypeScript checker backend unavailable: the checker stopped answering: ");
        assert!(
            reason.is_some_and(|reason| [
                "panic: checker invariant broken",
                "the built-in checker stopped: a panic unwound through its session",
                "session shut down",
            ]
            .iter()
            .any(|said| reason.contains(said))),
            "{message}"
        );
        // Each lane met the panic at most once: none was asked again after it.
        assert!((1..=2).contains(&asked.load(std::sync::atomic::Ordering::Relaxed)));
        assert!(!checker.is_alive());
        runtime.block_on(checker.shutdown());
    }

    /// A prop the converter keeps for later components is kept by what makes it, including which of its JSDoc was
    /// asked: a component whose structural pass already described `id` is not handed the description another asked for,
    /// and the one after it is not handed an empty one. The three are in one file, so they are answered in turn and the
    /// later ones find the first one's props kept.
    #[test]
    fn a_kept_prop_is_one_made_by_the_same_question() {
        let (_dir, root) = shared_types_project(0);
        let component = |name: &str| {
            format!(
                "export interface {name}Props extends Base {{ own: number }}\nexport function {name}(props: {name}Props) {{ return null; }}\n"
            )
        };
        let text = format!(
            "import type {{ Base }} from './types';\n{}{}{}",
            component("C0"),
            component("C1"),
            component("C2")
        );
        std::fs::write(root.join("All.tsx"), text).unwrap();
        let output = extract(&PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() });
        let mut targets = targets_of(&output);
        for target in &mut targets {
            target.described.clear();
        }
        if let Some(target) = targets.iter_mut().find(|target| target.key == "C1") {
            target.described.insert("id".to_string());
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();

        let outcome = runtime.block_on(run(SemanticRequest::new(root.clone(), targets), Some(1)));

        let description = |key: &str| {
            let component = outcome.components.iter().find(|component| component.key == key).unwrap();
            component.props.iter().find(|prop| prop.name == "id").unwrap().description.clone()
        };
        assert_eq!(
            (description("C0"), description("C1"), description("C2")),
            ("The id.".into(), String::new(), "The id.".into())
        );
    }
}
