use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use camino::Utf8Path;
use dashmap::DashMap;
use napi_derive::napi;

use caliper_docgen_core::pipeline::{
    extraction_output_to_json, incremental_update_to_json, PipelineOptions, WatchSession,
};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

// ─── Session store ────────────────────────────────────────────────────────────

static SESSIONS: LazyLock<DashMap<u32, Arc<WatchSession>>> = LazyLock::new(DashMap::new);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(0);

/// Src-dir value that trips a simulated panic inside `create_session`'s real
/// body — exists only to let tests exercise `panic_guard::contain_panic`'s
/// containment through the actual production entry point, the same pattern
/// `caliper_docgen_core::pipeline`'s `PARSE_PANIC_TEST_SENTINEL` uses.
#[cfg(test)]
const CREATE_SESSION_PANIC_TEST_SENTINEL: &str = "__CREATE_SESSION_PANIC_TEST__";

/// Src-dir value that trips a simulated panic inside `extract_all`'s real
/// body (at the `pipeline::extract` step, before serialization). SPEC-BINDING-001 AC6.
#[cfg(test)]
const EXTRACT_ALL_PANIC_TEST_SENTINEL: &str = "__EXTRACT_ALL_PANIC_TEST__";

/// Reserved `session_id` values used only by `#[cfg(test)]` sentinel checks
/// below, to trip a simulated panic at a SPECIFIC point in each entry
/// point's real body. Each targets a different point deliberately — see
/// SPEC-BINDING-001 AC3/AC7/AC8/AC10, which each require proving panic
/// containment at a *distinct* trigger site, not just "some panic somewhere
/// is caught":
///
/// - `AUTO_VIVIFY_PANIC_TEST_SENTINEL` fires during auto-vivified
///   `WatchSession` construction (the `None` branch of the SESSIONS.get
///   match in `initialize_session`/`extract_file_incremental`) — before the
///   session is registered at all.
/// - `INITIALIZE_SESSION_PANIC_TEST_SENTINEL` fires at `initialize_session`'s
///   actual core call (`session.initialize()`), reachable only for an
///   ALREADY-REGISTERED session — i.e. strictly after auto-vivification (or
///   `create_session`) has already completed. Distinct from
///   `AUTO_VIVIFY_PANIC_TEST_SENTINEL`'s trigger point.
/// - `CLOSE_SESSION_PANIC_TEST_SENTINEL` fires inside `close_session`'s body.
///
/// Chosen as the top of `u32`'s range: `next_session_id()`'s output is
/// `(pid & 0xFFFF) << 16 | (counter & 0xFFFF)`, so either value could
/// theoretically collide with a real generated ID under an astronomically
/// unlikely combination of process ID and call count — judged an acceptable
/// risk for test-only sentinels, consistent with the documented tradeoff on
/// `next_session_id`'s own 16-bit allocation.
#[cfg(test)]
const AUTO_VIVIFY_PANIC_TEST_SENTINEL: u32 = u32::MAX;
#[cfg(test)]
const INITIALIZE_SESSION_PANIC_TEST_SENTINEL: u32 = u32::MAX - 1;
#[cfg(test)]
const CLOSE_SESSION_PANIC_TEST_SENTINEL: u32 = u32::MAX - 3;

/// `extract_file_incremental`'s core-call sentinel is file_path-based
/// instead — its core call (`session.update_file(path)`) is keyed by path,
/// not session_id, so a distinguishing file_path is the natural trigger for
/// SPEC-BINDING-001 AC8, reached only for an already-registered session,
/// strictly after `AUTO_VIVIFY_PANIC_TEST_SENTINEL`'s trigger point.
#[cfg(test)]
const EXTRACT_FILE_INCREMENTAL_PANIC_TEST_SENTINEL: &str = "__EXTRACT_FILE_INCREMENTAL_PANIC_TEST__";

/// Pure bit-packing seam for `next_session_id()`, so SPEC-BINDING-001 AC4's
/// uniqueness claim (no duplicate for N <= 65,536 calls in one process) is
/// testable directly against synthetic counter values — `next_session_id()`
/// itself reads a real, process-global `AtomicU64` shared across every test
/// in this binary (including other tests' own `next_session_id()`/
/// `create_session()` calls), so driving 65,536 real calls through it would
/// risk wrapping mid-test on however much prior pollution happened to land
/// before this test ran, under parallel test execution.
fn compute_session_id(pid: u64, counter: u64) -> u32 {
    // Avoids session-ID collisions across concurrent Vite dev server instances.
    (((pid & 0xFFFF) << 16) | (counter & 0xFFFF)) as u32
}

fn next_session_id() -> u32 {
    let pid = std::process::id() as u64;
    let counter = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    compute_session_id(pid, counter)
}

// ─── JsExtractOptions ─────────────────────────────────────────────────────────

/// Options passed from TypeScript — flat struct for easy NAPI marshalling.
#[napi(object)]
pub struct JsExtractOptions {
    pub src_dirs: Vec<String>,
    pub exclude: Option<Vec<String>>,
    #[napi(ts_type = "'curated' | 'full' | 'none'")]
    pub html_attributes: Option<String>,
    pub tsconfig_path: Option<String>,
    pub extra_builtins: Option<Vec<String>>,
    pub cache_dir: Option<String>,
    /// Read and write the on-disk caches (default `true`). `false` reads and writes nothing under `cacheDir`; the
    /// output is the same.
    pub cache: Option<bool>,
    /// Load the dependency type declarations project types reference (default `true`).
    pub load_dependencies: Option<bool>,
    /// Which inherited sources of more than 30 props to list as one layer (default `'dependencies'`).
    #[napi(ts_type = "'dependencies' | 'all' | 'none'")]
    pub collapse_inherited_props: Option<String>,
    /// Session only (`createSession`, `initializeSession`, `extractFileIncremental`): parse and index every file, and
    /// resolve a component when `sessionDocgenInfo` asks for it (off by default; ADR 0016). `initializeSession` then
    /// returns an output with no components.
    pub lazy: Option<bool>,
    /// Session only: `'on-demand'` asks TypeScript's 7.1 checker for each component a request names, and answers
    /// with the structural props when the checker is slower than `semanticWaitMs` (experimental; ADR 0020). Needs
    /// `typescript@beta` installed or `tscPath`. `'off'` (the default) never starts it.
    #[napi(ts_type = "'off' | 'on-demand'")]
    pub semantic_types: Option<String>,
    /// Session only, with `semanticTypes: 'on-demand'`: the `tsc` binary to run, instead of the nearest
    /// `node_modules/typescript/bin/tsc`.
    pub tsc_path: Option<String>,
    /// Session only, with `semanticTypes: 'on-demand'`: how many milliseconds a request waits for the checker before
    /// it takes the structural answer (default 150).
    pub semantic_wait_ms: Option<u32>,
}

/// How long a request waits for the checker when `semanticWaitMs` is not set: about the first-story budget.
const DEFAULT_SEMANTIC_WAIT_MS: u32 = 150;

/// An empty session for `options`: lazy when they say so, with a checker attached when they ask for one.
fn new_session(options: JsExtractOptions) -> napi::Result<Arc<WatchSession>> {
    let lazy = options.lazy.unwrap_or(false);
    let semantic = match options.semantic_types.as_deref() {
        None | Some("off") => None,
        Some("on-demand") => Some((
            options.tsc_path.clone(),
            options.tsconfig_path.clone(),
            options.src_dirs.first().cloned(),
            options.semantic_wait_ms.unwrap_or(DEFAULT_SEMANTIC_WAIT_MS),
        )),
        Some(other) => {
            return Err(napi::Error::from_reason(format!("semanticTypes must be 'off' or 'on-demand', got '{other}'")))
        }
    };
    let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;
    let session = match lazy {
        true => WatchSession::lazy(pipeline_options),
        false => WatchSession::new(pipeline_options),
    };
    let session = match semantic {
        Some((tsc_path, tsconfig, first_src, wait_ms)) => {
            let checker = on_demand_checker(tsc_path, tsconfig, first_src);
            session.with_semantic(checker, std::time::Duration::from_millis(u64::from(wait_ms)))
        }
        None => session,
    };
    Ok(Arc::new(session))
}

/// A warm `tsc` for the project that holds `first_src`. It starts in the background, so a request that arrives before
/// it is up takes the structural answer rather than waiting for the process.
fn on_demand_checker(
    tsc_path: Option<String>,
    tsconfig: Option<String>,
    first_src: Option<String>,
) -> Arc<caliper_docgen_semantic::OnDemand> {
    let root = first_src
        .as_deref()
        .and_then(|dir| std::fs::canonicalize(dir).ok())
        .and_then(|dir| camino::Utf8PathBuf::from_path_buf(dir).ok())
        .or_else(|| std::env::current_dir().ok().and_then(|dir| camino::Utf8PathBuf::from_path_buf(dir).ok()))
        .unwrap_or_default();
    let mut config = caliper_docgen_semantic::OnDemandConfig::new(root.clone());
    config.tsconfig = tsconfig
        .map(camino::Utf8PathBuf::from)
        .or_else(|| root.ancestors().map(|dir| dir.join("tsconfig.json")).find(|file| file.is_file()));
    config.tsc_path = tsc_path.map(camino::Utf8PathBuf::from);
    let checker = caliper_docgen_semantic::OnDemand::new(config);
    checker.warm_up();
    Arc::new(checker)
}

// ─── TryFrom<JsExtractOptions> for PipelineOptions ────────────────────────────

impl TryFrom<JsExtractOptions> for PipelineOptions {
    /// Names the bad `htmlAttributes` value — a typo (or a caller bypassing the TS union, e.g. via
    /// `as any`) must not silently fall back to a default (AGENTS.md non-negotiable #6).
    type Error = String;

    fn try_from(js: JsExtractOptions) -> Result<Self, Self::Error> {
        use compact_str::CompactString;

        let mut opts =
            PipelineOptions { src_dirs: js.src_dirs.into_iter().map(Into::into).collect(), ..Default::default() };

        if let Some(exclude) = js.exclude {
            opts.exclude_patterns = exclude;
        }
        if let Some(mode) = js.html_attributes.as_deref() {
            opts.html_attributes = caliper_docgen_core::pipeline::HtmlAttributeMode::parse(mode)
                .map_err(|bad| format!("htmlAttributes is '{bad}', expected \"curated\", \"full\" or \"none\""))?;
        }
        if let Some(path) = js.tsconfig_path {
            opts.tsconfig_path = Some(path.into());
        }
        if let Some(names) = js.extra_builtins {
            opts.extra_builtins = names.into_iter().map(CompactString::from).collect();
        }
        if let Some(dir) = js.cache_dir {
            opts.cache_dir = Some(dir.into());
        }
        if let Some(cache) = js.cache {
            opts.use_cache = cache;
        }
        if let Some(load) = js.load_dependencies {
            opts.load_dependencies = load;
        }
        if let Some(mode) = js.collapse_inherited_props.as_deref() {
            opts.collapse_inherited_props = caliper_docgen_core::pipeline::CollapseInheritedProps::parse(mode)
                .map_err(|bad| {
                    format!("collapseInheritedProps is '{bad}', expected \"dependencies\", \"all\" or \"none\"")
                })?;
        }

        Ok(opts)
    }
}

// ─── Public NAPI functions ────────────────────────────────────────────────────

/// Cold extraction — returns JSON string of ExtractionOutput.
/// Use for: build-time extraction, CLI backing, one-off runs.
#[napi]
pub async fn extract_all(options: JsExtractOptions) -> napi::Result<String> {
    let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;
    tokio::task::spawn_blocking(move || {
        match caliper_docgen_core::panic_guard::contain_panic("extract_all", move || {
            #[cfg(test)]
            if pipeline_options.src_dirs.iter().any(|d| d.as_str() == EXTRACT_ALL_PANIC_TEST_SENTINEL) {
                panic!("simulated extract_all panic (test-only sentinel)");
            }

            let output = caliper_docgen_core::pipeline::extract(&pipeline_options);
            extraction_output_to_json(&output).map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}

/// Create a persistent watch session. Returns session ID.
/// Call this in Vite's configResolved hook.
#[napi]
pub fn create_session(options: JsExtractOptions) -> napi::Result<u32> {
    match caliper_docgen_core::panic_guard::contain_panic("create_session", move || {
        #[cfg(test)]
        if options.src_dirs.iter().any(|d| d == CREATE_SESSION_PANIC_TEST_SENTINEL) {
            panic!("simulated create_session panic (test-only sentinel)");
        }

        let session = new_session(options)?;
        let id = next_session_id();
        SESSIONS.insert(id, session);
        Ok(id)
    }) {
        Ok(result) => result,
        Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
    }
}

/// Incremental extraction for a single changed file.
/// Returns JSON string of IncrementalUpdate.
/// Use for: Vite HMR hotUpdate.
#[napi]
pub async fn extract_file_incremental(
    file_path: String,
    session_id: u32,
    options: JsExtractOptions,
) -> napi::Result<String> {
    tokio::task::spawn_blocking(move || {
        match caliper_docgen_core::panic_guard::contain_panic("extract_file_incremental", move || {
            let session = match SESSIONS.get(&session_id) {
                Some(s) => s.clone(),
                None => {
                    #[cfg(test)]
                    if session_id == AUTO_VIVIFY_PANIC_TEST_SENTINEL {
                        panic!("simulated extract_file_incremental auto-vivification panic (test-only sentinel)");
                    }

                    let s = new_session(options)?;
                    SESSIONS.insert(session_id, s.clone());
                    s
                }
            };

            #[cfg(test)]
            if file_path == EXTRACT_FILE_INCREMENTAL_PANIC_TEST_SENTINEL {
                panic!("simulated extract_file_incremental core-call panic (test-only sentinel)");
            }

            let path = Utf8Path::new(&file_path);
            let update = session.update_file(path);
            incremental_update_to_json(&update).map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}

/// The session's current output as a JSON string of ExtractionOutput, running the cold extraction first if the
/// session has none. Every change applied through `extract_file_incremental` is reflected.
#[napi]
pub async fn initialize_session(session_id: u32, options: JsExtractOptions) -> napi::Result<String> {
    tokio::task::spawn_blocking(move || {
        match caliper_docgen_core::panic_guard::contain_panic("initialize_session", move || {
            let session = match SESSIONS.get(&session_id) {
                Some(s) => s.clone(),
                None => {
                    #[cfg(test)]
                    if session_id == AUTO_VIVIFY_PANIC_TEST_SENTINEL {
                        panic!("simulated initialize_session auto-vivification panic (test-only sentinel)");
                    }

                    let s = new_session(options)?;
                    SESSIONS.insert(session_id, s.clone());
                    s
                }
            };

            #[cfg(test)]
            if session_id == INITIALIZE_SESSION_PANIC_TEST_SENTINEL {
                panic!("simulated initialize_session core-call panic (test-only sentinel)");
            }

            let output = session.initialize();
            caliper_docgen_core::pipeline::extraction_output_to_json(&output)
                .map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}

/// Like [`initialize_session`], but returns only what a host needs to find a component and judge whether its props are
/// complete: `{ components: { key: { filePath, displayName } }, diagnostics }`. The props stay in the session for
/// [`session_docgen_info`]. A host that documents one component at a time skips serializing and parsing every prop of
/// every component, which is tens of megabytes when dependencies are loaded.
#[napi]
pub async fn initialize_session_index(session_id: u32, options: JsExtractOptions) -> napi::Result<String> {
    tokio::task::spawn_blocking(move || {
        match caliper_docgen_core::panic_guard::contain_panic("initialize_session_index", move || {
            let session = match SESSIONS.get(&session_id) {
                Some(s) => s.clone(),
                None => {
                    let s = new_session(options)?;
                    SESSIONS.insert(session_id, s.clone());
                    s
                }
            };

            drop(session.initialize());
            let output = session
                .current_output()
                .ok_or_else(|| napi::Error::from_reason("the session has no output after initialize".to_owned()))?;
            // A lazy session's output has no components and no component diagnostics: it resolves none of them. Where
            // each is declared comes from parsing, and a component's diagnostics come with its `sessionComponentAnswers`.
            caliper_docgen_core::pipeline::session_index_to_json(&session.component_index(), &output)
                .map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}

/// The session's components with these keys as react-docgen-typescript documents them (`__docgenInfo`, which
/// Storybook converts to argTypes), as a JSON object keyed by component key. A key the session doesn't have is left
/// out; so is every key before the session is initialized.
#[napi]
pub fn session_docgen_info(session_id: u32, keys: Vec<String>) -> napi::Result<String> {
    match caliper_docgen_core::panic_guard::contain_panic("session_docgen_info", move || {
        let session = SESSIONS.get(&session_id).map(|session| session.clone());
        let docs: serde_json::Map<String, serde_json::Value> = match session {
            // A lazy session resolves each requested component now, and the ones it already resolved are answered
            // from its memo.
            Some(session) if session.is_lazy() => {
                let answers = session.components(&keys);
                keys.into_iter()
                    .zip(answers)
                    .filter_map(|(key, answer)| {
                        Some((key, caliper_docgen_core::pipeline::rdt::docgen_info(&answer?.entry)))
                    })
                    .collect()
            }
            session => {
                let output = session.and_then(|session| session.current_output());
                keys.into_iter()
                    .filter_map(|key| {
                        let entry = output.as_ref()?.components.get(&key)?;
                        Some((key, caliper_docgen_core::pipeline::rdt::docgen_info(entry)))
                    })
                    .collect()
            }
        };
        serde_json::to_string(&docs).map_err(|e| napi::Error::from_reason(e.to_string()))
    }) {
        Ok(result) => result,
        Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
    }
}

/// What a host needs to answer for one component: its documentation as `sessionDocgenInfo` returns it, and its own
/// diagnostics, each with `affects` set to the key. `{ key: { docgenInfo, diagnostics } }`; a key the session has no
/// component under is left out. A lazy session resolves each requested component now (in parallel), so a host learns
/// whether a component's props may be incomplete from the same call that returns them.
#[napi]
pub fn session_component_answers(session_id: u32, keys: Vec<String>) -> napi::Result<String> {
    match caliper_docgen_core::panic_guard::contain_panic("session_component_answers", move || {
        let session = SESSIONS.get(&session_id).map(|session| session.clone());
        let answers: serde_json::Map<String, serde_json::Value> = match session {
            Some(session) => {
                let answered = session.components(&keys);
                keys.into_iter()
                    .zip(answered)
                    .filter_map(|(key, answer)| {
                        let answer = answer?;
                        let answer = serde_json::json!({
                            "docgenInfo": caliper_docgen_core::pipeline::rdt::docgen_info(&answer.entry),
                            "diagnostics": answer.diagnostics,
                        });
                        Some((key, answer))
                    })
                    .collect()
            }
            None => serde_json::Map::new(),
        };
        serde_json::to_string(&answers).map_err(|e| napi::Error::from_reason(e.to_string()))
    }) {
        Ok(result) => result,
        Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
    }
}

/// Resolves `count` components spread evenly over the session's keys, without returning them: a lazy session then
/// holds the dependency declarations and resolver memos that components share, so the first request a host answers
/// does not load them. Runs off the calling thread and returns how many components it resolved. An eager session, an
/// unknown session and a count of 0 resolve nothing.
#[napi]
pub async fn session_warm_up(session_id: u32, count: u32) -> napi::Result<u32> {
    tokio::task::spawn_blocking(move || {
        match caliper_docgen_core::panic_guard::contain_panic("session_warm_up", move || {
            let Some(session) = SESSIONS.get(&session_id).map(|session| session.clone()) else { return 0 };
            if !session.is_lazy() || count == 0 {
                return 0;
            }
            let keys = session.component_keys();
            let wanted = (count as usize).min(keys.len());
            let spread: Vec<String> = (0..wanted).filter_map(|n| keys.get(n * keys.len() / wanted).cloned()).collect();
            session.components(&spread).into_iter().flatten().count() as u32
        }) {
            Ok(resolved) => Ok(resolved),
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}

/// Every component key `sessionDocgenInfo` answers, sorted. A lazy session resolves only imported-member aliases to
/// decide; the rest come from parsing alone. Empty for an unknown or uninitialized session.
#[napi]
pub fn session_component_keys(session_id: u32) -> Vec<String> {
    caliper_docgen_core::panic_guard::contain_panic("session_component_keys", move || {
        SESSIONS.get(&session_id).map(|session| session.component_keys()).unwrap_or_default()
    })
    .unwrap_or_default()
}

/// What a lazy session has indexed and resolved so far. `None` for an eager, unknown or uninitialized session.
#[napi(object)]
pub struct JsLazyStats {
    pub revision: u32,
    pub files: u32,
    /// Components found by parsing.
    pub components: u32,
    /// Components resolved since the last edit that could change them.
    pub resolved: u32,
    /// Dependency declaration files loaded for the names resolved components reached.
    pub dependency_files: u32,
}

#[napi]
pub fn session_stats(session_id: u32) -> Option<JsLazyStats> {
    caliper_docgen_core::panic_guard::contain_panic("session_stats", move || {
        let stats = SESSIONS.get(&session_id)?.lazy_stats()?;
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Some(JsLazyStats {
            revision: u32::try_from(stats.revision).unwrap_or(u32::MAX),
            files: count(stats.files),
            components: count(stats.components),
            resolved: count(stats.resolved),
            dependency_files: count(stats.dependency_files),
        })
    })
    .unwrap_or(None)
}

/// Bring the session up to date with the files on disk, for a host without a file watcher (a docgen provider in a
/// worker): stats every file the session read and applies what changed since, rebuilding when a tsconfig,
/// `package.json` or lockfile changed. Returns JSON string of IncrementalUpdate; the revision is unchanged when nothing
/// changed. Errors for an unknown session.
#[napi]
pub async fn ensure_fresh(session_id: u32) -> napi::Result<String> {
    tokio::task::spawn_blocking(move || {
        match caliper_docgen_core::panic_guard::contain_panic("ensure_fresh", move || {
            let session = SESSIONS
                .get(&session_id)
                .map(|s| s.clone())
                .ok_or_else(|| napi::Error::from_reason(format!("Unknown session {session_id}")))?;
            let update = session.ensure_fresh();
            incremental_update_to_json(&update).map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}

/// The tsconfigs, `package.json` files and lockfiles above the session's source directories: a watcher adds them, and
/// passes a change to one to `extractFileIncremental` like any other file, which rebuilds the session. Empty for an
/// unknown session.
#[napi]
pub fn resolution_config_files(session_id: u32) -> Vec<String> {
    caliper_docgen_core::panic_guard::contain_panic("resolution_config_files", move || {
        SESSIONS
            .get(&session_id)
            .map(|session| session.resolution_config_files().into_iter().map(String::from).collect())
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Whether a change to `file_path` rebuilds the session: a tsconfig, jsconfig, `package.json` or lockfile in or above
/// a source directory, outside `node_modules`. False for an unknown session.
#[napi]
pub fn is_resolution_config(session_id: u32, file_path: String) -> bool {
    caliper_docgen_core::panic_guard::contain_panic("is_resolution_config", move || {
        SESSIONS.get(&session_id).is_some_and(|session| session.is_resolution_config(Utf8Path::new(&file_path)))
    })
    .unwrap_or(false)
}

/// Release session state. Call in Vite's buildEnd hook.
#[napi]
pub fn close_session(session_id: u32) {
    caliper_docgen_core::panic_guard::contain_panic("close_session", move || {
        #[cfg(test)]
        if session_id == CLOSE_SESSION_PANIC_TEST_SENTINEL {
            panic!("simulated close_session panic (test-only sentinel)");
        }

        SESSIONS.remove(&session_id);
    })
    .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_options() -> JsExtractOptions {
        JsExtractOptions {
            src_dirs: vec!["./src".into()],
            exclude: None,
            html_attributes: None,
            tsconfig_path: None,
            extra_builtins: None,
            cache_dir: None,
            cache: None,
            load_dependencies: None,
            collapse_inherited_props: None,
            lazy: None,
            semantic_types: None,
            tsc_path: None,
            semantic_wait_ms: None,
        }
    }

    #[test]
    fn unset_fields_match_pipeline_options_defaults() {
        let opts = PipelineOptions::try_from(base_options()).expect("should convert");
        let defaults = PipelineOptions::default();

        assert_eq!(opts.html_attributes, defaults.html_attributes);
        assert_eq!(opts.extra_builtins, defaults.extra_builtins);
        assert_eq!(opts.cache_dir, defaults.cache_dir);
        assert!(opts.use_cache, "the cache is on unless asked off");
        let off =
            PipelineOptions::try_from(JsExtractOptions { cache: Some(false), ..base_options() }).expect("convert");
        assert!(!off.use_cache, "`cache: false` turns it off");
        assert_eq!(opts.load_dependencies, defaults.load_dependencies);
    }

    #[test]
    fn load_dependencies_maps_onto_pipeline_options() {
        let mut js = base_options();
        js.load_dependencies = Some(true);
        assert!(PipelineOptions::try_from(js).expect("should convert").load_dependencies);
    }

    #[test]
    fn collapse_inherited_props_names_map_onto_pipeline_options_and_a_bad_one_is_an_error() {
        use caliper_docgen_core::pipeline::CollapseInheritedProps;

        assert_eq!(
            PipelineOptions::try_from(base_options()).expect("should convert").collapse_inherited_props,
            CollapseInheritedProps::Dependencies
        );
        for (name, expected) in [
            ("dependencies", CollapseInheritedProps::Dependencies),
            ("all", CollapseInheritedProps::All),
            ("none", CollapseInheritedProps::None),
        ] {
            let mut js = base_options();
            js.collapse_inherited_props = Some(name.into());
            let opts = PipelineOptions::try_from(js).expect("should convert");
            assert_eq!(opts.collapse_inherited_props, expected, "{name}");
        }
        let mut js = base_options();
        js.collapse_inherited_props = Some("some".into());
        assert_eq!(
            PipelineOptions::try_from(js).expect_err("a typo must not fall back to the default"),
            "collapseInheritedProps is 'some', expected \"dependencies\", \"all\" or \"none\""
        );
    }

    #[test]
    fn html_attributes_names_map_onto_pipeline_options() {
        use caliper_docgen_core::pipeline::HtmlAttributeMode;

        for (name, expected) in [
            ("curated", HtmlAttributeMode::Curated),
            ("full", HtmlAttributeMode::Full),
            ("none", HtmlAttributeMode::None),
        ] {
            let mut js = base_options();
            js.html_attributes = Some(name.into());
            assert_eq!(PipelineOptions::try_from(js).expect("should convert").html_attributes, expected, "{name}");
        }
    }

    #[test]
    fn an_unrecognized_html_attributes_is_a_hard_error_naming_the_value_not_a_silent_default() {
        let mut js = base_options();
        js.html_attributes = Some("all".into());

        let err = PipelineOptions::try_from(js).expect_err("a typo must not silently fall back to curated");

        assert_eq!(err, "htmlAttributes is 'all', expected \"curated\", \"full\" or \"none\"");
    }

    #[test]
    fn a_panic_inside_create_session_surfaces_as_a_napi_error_not_a_crash() {
        let mut js = base_options();
        js.src_dirs = vec![CREATE_SESSION_PANIC_TEST_SENTINEL.into()];

        let err = create_session(js).expect_err("panic should be caught and converted to a napi::Error");
        assert!(err.reason.contains("simulated create_session panic"), "got {}", err.reason);
    }

    // ── SPEC-BINDING-001 AC10: panic during auto-vivified WatchSession
    // construction, for both initialize_session and extract_file_incremental
    // — session_id-based sentinel, fires only in the `None` branch (session
    // not yet registered).

    #[tokio::test]
    async fn a_panic_during_initialize_session_auto_vivification_surfaces_as_a_napi_error_not_a_crash() {
        let js = base_options();
        let err = initialize_session(AUTO_VIVIFY_PANIC_TEST_SENTINEL, js)
            .await
            .expect_err("panic during auto-vivification should be caught and converted to a napi::Error");
        assert!(err.reason.contains("simulated initialize_session auto-vivification panic"), "got {}", err.reason);
    }

    #[tokio::test]
    async fn a_panic_during_extract_file_incremental_auto_vivification_surfaces_as_a_napi_error_not_a_crash() {
        let js = base_options();
        let err = extract_file_incremental("Widget.tsx".to_string(), AUTO_VIVIFY_PANIC_TEST_SENTINEL, js)
            .await
            .expect_err("panic during auto-vivification should be caught and converted to a napi::Error");
        assert!(
            err.reason.contains("simulated extract_file_incremental auto-vivification panic"),
            "got {}",
            err.reason
        );
    }

    #[tokio::test]
    async fn ensure_fresh_errors_for_an_unknown_session_and_updates_a_registered_one() {
        let session_id = next_session_id();
        let err = ensure_fresh(session_id).await.expect_err("an unknown session has no options to extract with");
        assert_eq!(err.reason, format!("Unknown session {session_id}"));

        // `./src` holds this crate's Rust sources: an existing directory with no components.
        let options = PipelineOptions::try_from(base_options()).expect("valid options");
        SESSIONS.insert(session_id, Arc::new(WatchSession::new(options)));
        let first = ensure_fresh(session_id).await.expect("a registered session");
        let second = ensure_fresh(session_id).await.expect("a registered session");
        close_session(session_id);
        let without_duration = |update: &str| update.split_once(r#","durationMs""#).expect("durationMs").0.to_owned();
        let empty = r#"{"schemaVersion":2,"revision":1,"updatedComponents":[],"props":{},"propSets":{},"nameLists":{},"updatedKeys":[],"removedKeys":[],"affectedFiles":[],"diagnostics":[],"removedDiagnostics":[]"#;
        assert_eq!((without_duration(&first), without_duration(&second)), (empty.to_owned(), empty.to_owned()));
    }

    // ── SPEC-BINDING-001 AC3: close_session's panic is caught, and the call
    // completes normally (unit type) rather than aborting the process.

    #[test]
    fn a_panic_inside_close_session_does_not_abort_the_process() {
        // No return value to assert on — close_session returns () even on a
        // caught panic (the diagnostic is discarded, per non_goals). The
        // assertion IS that this line is reached at all.
        close_session(CLOSE_SESSION_PANIC_TEST_SENTINEL);
    }

    // ── SPEC-BINDING-001 AC6: extract_all's panic, at the pipeline::extract
    // step, is caught and surfaces as a napi::Error.

    #[tokio::test]
    async fn a_panic_inside_extract_all_surfaces_as_a_napi_error_not_a_crash() {
        let mut js = base_options();
        js.src_dirs = vec![EXTRACT_ALL_PANIC_TEST_SENTINEL.into()];

        let err = extract_all(js).await.expect_err("panic should be caught and converted to a napi::Error");
        assert!(err.reason.contains("simulated extract_all panic"), "got {}", err.reason);
    }

    // ── SPEC-BINDING-001 AC7/AC8: panic at the CORE CALL site (after
    // lookup-or-auto-vivify has already completed for a normally-registered
    // session) — distinct from AC10's construction-time trigger. The session
    // must already exist in SESSIONS before the sentinel call, proving these
    // fire on the post-auto-vivify path, not the auto-vivify path itself.

    #[tokio::test]
    async fn a_panic_at_initialize_sessions_core_call_surfaces_as_a_napi_error_not_a_crash() {
        let pipeline_options = PipelineOptions::try_from(base_options()).expect("valid options");
        SESSIONS.insert(INITIALIZE_SESSION_PANIC_TEST_SENTINEL, Arc::new(WatchSession::new(pipeline_options)));

        let js = base_options();
        let err = initialize_session(INITIALIZE_SESSION_PANIC_TEST_SENTINEL, js)
            .await
            .expect_err("panic at the core call should be caught and converted to a napi::Error");
        assert!(err.reason.contains("simulated initialize_session core-call panic"), "got {}", err.reason);
    }

    #[tokio::test]
    async fn a_panic_at_extract_file_incrementals_core_call_surfaces_as_a_napi_error_not_a_crash() {
        let session_id = next_session_id();
        let pipeline_options = PipelineOptions::try_from(base_options()).expect("valid options");
        SESSIONS.insert(session_id, Arc::new(WatchSession::new(pipeline_options)));

        let js = base_options();
        let err = extract_file_incremental(EXTRACT_FILE_INCREMENTAL_PANIC_TEST_SENTINEL.to_string(), session_id, js)
            .await
            .expect_err("panic at the core call should be caught and converted to a napi::Error");
        assert!(err.reason.contains("simulated extract_file_incremental core-call panic"), "got {}", err.reason);
    }

    // ── SPEC-BINDING-001 AC4: next_session_id() never produces a duplicate
    // across many calls in one process.

    #[test]
    fn next_session_id_never_duplicates_across_many_calls() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = next_session_id();
            assert!(seen.insert(id), "next_session_id() produced a duplicate: {id}");
        }
    }

    // ── SPEC-BINDING-001 AC4: the bit-packing formula itself guarantees no
    // duplicate for N <= 65,536 calls within one process — tested against the
    // full claimed bound via the pure seam, not the shared global counter
    // (see compute_session_id's doc comment for why).

    #[test]
    fn compute_session_id_never_duplicates_across_the_full_65536_counter_range() {
        let pid = 42u64;
        let mut seen = std::collections::HashSet::with_capacity(65_536);
        for counter in 0..65_536u64 {
            let id = compute_session_id(pid, counter);
            assert!(seen.insert(id), "compute_session_id produced a duplicate at counter={counter}: {id}");
        }
    }

    // ── SPEC-BINDING-001 AC9: an unrecognized session_id auto-vivifies
    // instead of failing; a second call with the same ID reuses the entry
    // rather than constructing another.

    #[tokio::test]
    async fn initialize_session_auto_vivifies_on_first_call_and_reuses_on_second() {
        let session_id = next_session_id();
        assert!(SESSIONS.get(&session_id).is_none(), "sanity check: session must not already exist");

        let first = initialize_session(session_id, base_options()).await;
        assert!(first.is_ok(), "expected Ok for an unrecognized session_id, got {first:?}");
        let first_session = SESSIONS.get(&session_id).expect("expected the session to now be registered").clone();

        // Reuse, not silent reconstruction: a bare "second call succeeds" proves
        // nothing — a freshly-constructed replacement WatchSession under the same
        // key would look identical from the outside. Arc::ptr_eq proves the SAME
        // underlying WatchSession object survived the second call rather than
        // being silently replaced.
        let second = initialize_session(session_id, base_options()).await;
        assert!(second.is_ok(), "expected Ok on reuse, got {second:?}");
        let second_session = SESSIONS.get(&session_id).expect("expected the session still registered");
        assert!(
            Arc::ptr_eq(&first_session, &second_session),
            "expected the second call to reuse the SAME WatchSession, not construct a new one under the same key"
        );
    }

    #[tokio::test]
    async fn extract_file_incremental_auto_vivifies_on_first_call_and_reuses_on_second() {
        let session_id = next_session_id();
        assert!(SESSIONS.get(&session_id).is_none(), "sanity check: session must not already exist");

        let first = extract_file_incremental("Widget.tsx".to_string(), session_id, base_options()).await;
        assert!(first.is_ok(), "expected Ok for an unrecognized session_id, got {first:?}");
        let first_session = SESSIONS.get(&session_id).expect("expected the session to now be registered").clone();

        let second = extract_file_incremental("Widget.tsx".to_string(), session_id, base_options()).await;
        assert!(second.is_ok(), "expected Ok on reuse, got {second:?}");
        let second_session = SESSIONS.get(&session_id).expect("expected the session still registered");
        assert!(
            Arc::ptr_eq(&first_session, &second_session),
            "expected the second call to reuse the SAME WatchSession, not construct a new one under the same key"
        );
    }

    /// A project directory holding one component, removed when dropped.
    struct Project(std::path::PathBuf);

    impl Project {
        fn with_button() -> Self {
            let dir = std::env::temp_dir().join(format!("caliper-binding-index-{}", next_session_id()));
            std::fs::create_dir_all(&dir).expect("create the project directory");
            std::fs::write(
                dir.join("Button.tsx"),
                "export interface ButtonProps { /** The label. */ label: string; size?: 'sm' | 'lg' }\n\
                 export function Button(props: ButtonProps) { return null }\n",
            )
            .expect("write the component");
            Self(dir)
        }

        fn options(&self) -> JsExtractOptions {
            JsExtractOptions { src_dirs: vec![self.0.to_string_lossy().into_owned()], ..base_options() }
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[tokio::test]
    async fn the_index_names_the_same_components_and_diagnostics_as_the_full_output() {
        let project = Project::with_button();
        let full = initialize_session(next_session_id(), project.options()).await.expect("full output");
        let full: serde_json::Value = serde_json::from_str(&full).expect("full output is JSON");
        let index = initialize_session_index(next_session_id(), project.options()).await.expect("index");
        let index: serde_json::Value = serde_json::from_str(&index).expect("index is JSON");

        let expected: serde_json::Map<String, serde_json::Value> = full["components"]
            .as_object()
            .expect("components")
            .iter()
            .map(|(key, entry)| {
                (key.clone(), serde_json::json!({ "filePath": entry["filePath"], "displayName": entry["displayName"] }))
            })
            .collect();
        assert_eq!(expected.keys().collect::<Vec<_>>(), ["Button"]);
        assert_eq!(index["components"], serde_json::Value::Object(expected));
        assert_eq!(index["diagnostics"], full["diagnostics"]);
    }

    #[tokio::test]
    async fn the_index_leaves_the_props_to_session_docgen_info() {
        let project = Project::with_button();
        let session_id = next_session_id();
        let index = initialize_session_index(session_id, project.options()).await.expect("index");
        assert!(!index.contains("\"props\""), "the index carries no props: {index}");

        let docs = session_docgen_info(session_id, vec!["Button".into()]).expect("docgen info");
        let docs: serde_json::Value = serde_json::from_str(&docs).expect("docgen info is JSON");
        let props = docs["Button"]["props"].as_object().expect("props");
        assert_eq!(props.keys().cloned().collect::<Vec<_>>(), ["label", "size"]);
    }

    // ── SPEC-BINDING-001 AC11: close_session on an unrecognized or
    // already-removed session_id returns normally, no error.

    #[test]
    fn close_session_on_an_unrecognized_or_already_closed_id_returns_normally() {
        let never_issued = next_session_id();
        close_session(never_issued);

        let pipeline_options = PipelineOptions::try_from(base_options()).expect("valid options");
        let valid_id = next_session_id();
        SESSIONS.insert(valid_id, Arc::new(WatchSession::new(pipeline_options)));
        close_session(valid_id);
        close_session(valid_id);
    }

    #[tokio::test]
    async fn a_lazy_session_documents_a_component_when_asked_exactly_as_an_eager_one_does() {
        let dir = std::env::temp_dir().join(format!("caliper-lazy-{}-{}", std::process::id(), next_session_id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("Button.tsx"),
            "export function Button(props: { /** Label. */ label: string; size?: 'sm' | 'lg' }) { return null; }\n\
             export function Card(props: { title: string }) { return null; }\n",
        )
        .expect("write fixture");
        let options = |lazy: Option<bool>| JsExtractOptions {
            src_dirs: vec![dir.to_string_lossy().into_owned()],
            cache_dir: Some(dir.join("cache").to_string_lossy().into_owned()),
            lazy,
            semantic_types: None,
            tsc_path: None,
            semantic_wait_ms: None,
            ..base_options()
        };

        let eager = create_session(options(None)).expect("eager session");
        let lazy = create_session(options(Some(true))).expect("lazy session");
        let eager_output = initialize_session(eager, options(None)).await.expect("eager initialize");
        let lazy_output = initialize_session(lazy, options(Some(true))).await.expect("lazy initialize");

        assert!(eager_output.contains(r#""Button""#), "an eager session resolves everything up front");
        assert!(!lazy_output.contains(r#""Button""#), "a lazy session resolves nothing up front: {lazy_output}");
        let stats = session_stats(lazy).expect("lazy stats");
        assert_eq!((stats.components, stats.resolved), (2, 0));
        assert!(session_stats(eager).is_none());
        assert_eq!(session_component_keys(lazy), ["Button", "Card"]);

        let keys = vec!["Button".to_owned(), "Missing".to_owned()];
        let from_eager = session_docgen_info(eager, keys.clone()).expect("eager docgen info");
        let from_lazy = session_docgen_info(lazy, keys).expect("lazy docgen info");
        assert_eq!(from_lazy, from_eager);
        assert!(from_lazy.contains(r#""label""#));
        assert_eq!(session_stats(lazy).expect("lazy stats").resolved, 1, "Card was never asked for");

        close_session(eager);
        close_session(lazy);
        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_lazy_session_indexes_where_components_are_and_answers_with_their_diagnostics() {
        let dir = std::env::temp_dir().join(format!("caliper-index-{}-{}", std::process::id(), next_session_id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("Button.tsx"),
            "export function Button(props: { label: string }) { return null; }\n\
             export function Card(props: { title: string }) { return null; }\n",
        )
        .expect("write fixture");
        let options = |lazy: Option<bool>| JsExtractOptions {
            src_dirs: vec![dir.to_string_lossy().into_owned()],
            cache_dir: Some(dir.join("cache").to_string_lossy().into_owned()),
            lazy,
            semantic_types: None,
            tsc_path: None,
            semantic_wait_ms: None,
            ..base_options()
        };

        let eager = create_session(options(None)).expect("eager session");
        let lazy = create_session(options(Some(true))).expect("lazy session");
        let from_eager = initialize_session_index(eager, options(None)).await.expect("eager index");
        let from_lazy = initialize_session_index(lazy, options(Some(true))).await.expect("lazy index");

        let components =
            |index: &str| serde_json::from_str::<serde_json::Value>(index).expect("json")["components"].clone();
        assert_eq!(components(&from_lazy), components(&from_eager));
        assert_eq!(components(&from_lazy)["Card"]["displayName"], "Card");
        assert_eq!(session_stats(lazy).expect("lazy stats").resolved, 0, "indexing resolves nothing");

        let keys = vec!["Card".to_owned(), "Missing".to_owned()];
        let answers = |id| {
            serde_json::from_str::<serde_json::Value>(&session_component_answers(id, keys.clone()).expect("answers"))
                .expect("json")
        };
        let (lazy_answers, eager_answers) = (answers(lazy), answers(eager));
        assert_eq!(lazy_answers, eager_answers);
        assert_eq!(lazy_answers["Card"]["docgenInfo"]["displayName"], "Card");
        assert_eq!(lazy_answers["Card"]["diagnostics"], serde_json::json!([]));
        assert!(lazy_answers.get("Missing").is_none());
        assert_eq!(session_stats(lazy).expect("lazy stats").resolved, 1, "Button was never asked for");

        close_session(eager);
        close_session(lazy);
        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn warming_up_resolves_a_spread_of_components_in_a_lazy_session_only() {
        let dir = std::env::temp_dir().join(format!("caliper-warm-{}-{}", std::process::id(), next_session_id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for name in ["A", "B", "C", "D"] {
            std::fs::write(
                dir.join(format!("{name}.tsx")),
                format!("export function {name}(props: {{ v: string }}) {{ return null; }}\n"),
            )
            .expect("write fixture");
        }
        let options = |lazy: Option<bool>| JsExtractOptions {
            src_dirs: vec![dir.to_string_lossy().into_owned()],
            cache_dir: Some(dir.join("cache").to_string_lossy().into_owned()),
            lazy,
            semantic_types: None,
            tsc_path: None,
            semantic_wait_ms: None,
            ..base_options()
        };
        let eager = create_session(options(None)).expect("eager session");
        let lazy = create_session(options(Some(true))).expect("lazy session");
        initialize_session_index(eager, options(None)).await.expect("eager index");
        initialize_session_index(lazy, options(Some(true))).await.expect("lazy index");

        assert_eq!(session_warm_up(lazy, 0).await.expect("no warm-up"), 0);
        assert_eq!(session_stats(lazy).expect("stats").resolved, 0);
        assert_eq!(session_warm_up(lazy, 2).await.expect("warm-up"), 2);
        assert_eq!(session_stats(lazy).expect("stats").resolved, 2);
        assert_eq!(session_warm_up(lazy, 99).await.expect("more than there are"), 4);
        assert_eq!(session_stats(lazy).expect("stats").resolved, 4);
        assert_eq!(session_warm_up(eager, 2).await.expect("eager"), 0);
        assert_eq!(session_warm_up(9_999_999, 2).await.expect("unknown"), 0);

        close_session(eager);
        close_session(lazy);
        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }
}
