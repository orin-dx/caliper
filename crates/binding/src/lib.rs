use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use camino::Utf8Path;
use dashmap::DashMap;
use napi_derive::napi;

use caliper_docgen_core::pipeline::{
    extraction_output_to_json, incremental_update_to_json, PipelineOptions, WatchSession,
};

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
    #[napi(ts_type = "'react18' | 'react19'")]
    pub react_version: Option<String>,
    pub cross_package: Option<bool>,
    pub pandacss_outdir: Option<String>,
    pub variant_functions: Option<Vec<String>>,
    #[napi(ts_type = "'curated' | 'full' | 'none'")]
    pub html_attributes: Option<String>,
    pub tsconfig_path: Option<String>,
    pub extra_builtins: Option<Vec<String>>,
    pub vanilla_extract: Option<bool>,
    pub cache_dir: Option<String>,
    pub resolve_complex_types: Option<bool>,
    // Complex fields accepted as JSON strings to avoid napi object complexity:
    /// JSON: Record<string, string[]>
    pub extra_paths_json: Option<String>,
    /// JSON: Record<string, {kind: ...}>
    pub known_type_overrides_json: Option<String>,
}

// ─── TryFrom<JsExtractOptions> for PipelineOptions ────────────────────────────

impl TryFrom<JsExtractOptions> for PipelineOptions {
    /// Names the bad `reactVersion` or `htmlAttributes` value — a typo (or a caller bypassing the TS union, e.g. via
    /// `as any`) must not silently fall back to a default (AGENTS.md non-negotiable #6).
    type Error = String;

    fn try_from(js: JsExtractOptions) -> Result<Self, Self::Error> {
        use compact_str::CompactString;

        let mut opts =
            PipelineOptions { src_dirs: js.src_dirs.into_iter().map(Into::into).collect(), ..Default::default() };

        if let Some(exclude) = js.exclude {
            opts.exclude_patterns = exclude;
        }
        if let Some(v) = js.react_version.as_deref() {
            opts.react_version = caliper_docgen_core::react_types::parse_react_version(v)
                .map_err(|bad| format!("reactVersion is '{bad}', expected \"react18\" or \"react19\""))?;
        }
        if let Some(cross_package) = js.cross_package {
            opts.cross_package = cross_package;
        }
        if let Some(dir) = js.pandacss_outdir {
            opts.pandacss_outdir = Some(dir.into());
        }
        if let Some(fns) = js.variant_functions {
            opts.variant_functions = fns;
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
        if let Some(json) = js.extra_paths_json {
            opts.extra_paths = serde_json::from_str(&json).map_err(|e| format!("extraPaths is not valid JSON: {e}"))?;
        }
        if let Some(json) = js.known_type_overrides_json {
            opts.known_type_overrides =
                serde_json::from_str(&json).map_err(|e| format!("knownTypeOverrides is not valid JSON: {e}"))?;
        }
        if let Some(v) = js.vanilla_extract {
            opts.vanilla_extract = v;
        }
        if let Some(dir) = js.cache_dir {
            opts.cache_dir = Some(dir.into());
        }
        if let Some(v) = js.resolve_complex_types {
            opts.resolve_complex_types = v;
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
        let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;

        #[cfg(test)]
        if pipeline_options.src_dirs.iter().any(|d| d.as_str() == CREATE_SESSION_PANIC_TEST_SENTINEL) {
            panic!("simulated create_session panic (test-only sentinel)");
        }

        let id = next_session_id();
        let session = Arc::new(WatchSession::new(pipeline_options));
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

                    let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;
                    let s = Arc::new(WatchSession::new(pipeline_options));
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

/// Initialize a watch session with a full cold extraction.
/// Returns JSON string of ExtractionOutput.
/// Call this after create_session(), in configureServer.
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

                    let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;
                    let s = Arc::new(WatchSession::new(pipeline_options));
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

/// Release session state. Call in Vite's buildEnd hook.
#[napi]
pub fn close_session(session_id: u32) {
    let _ = caliper_docgen_core::panic_guard::contain_panic("close_session", move || {
        #[cfg(test)]
        if session_id == CLOSE_SESSION_PANIC_TEST_SENTINEL {
            panic!("simulated close_session panic (test-only sentinel)");
        }

        SESSIONS.remove(&session_id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_options() -> JsExtractOptions {
        JsExtractOptions {
            src_dirs: vec!["./src".into()],
            exclude: None,
            react_version: None,
            cross_package: None,
            pandacss_outdir: None,
            variant_functions: None,
            html_attributes: None,
            tsconfig_path: None,
            extra_builtins: None,
            vanilla_extract: None,
            cache_dir: None,
            resolve_complex_types: None,
            extra_paths_json: None,
            known_type_overrides_json: None,
        }
    }

    #[test]
    fn unset_fields_match_pipeline_options_defaults() {
        let opts = PipelineOptions::try_from(base_options()).expect("should convert");
        let defaults = PipelineOptions::default();

        assert_eq!(opts.cross_package, defaults.cross_package);
        assert_eq!(opts.variant_functions, defaults.variant_functions);
        assert_eq!(opts.html_attributes, defaults.html_attributes);
        assert_eq!(opts.vanilla_extract, defaults.vanilla_extract);
        assert_eq!(opts.resolve_complex_types, defaults.resolve_complex_types);
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
    fn malformed_extra_paths_json_is_a_hard_error_not_a_silent_empty_map() {
        let mut js = base_options();
        js.extra_paths_json = Some("not valid json".into());

        let err = PipelineOptions::try_from(js).expect_err("malformed JSON should not silently default");
        assert!(err.contains("extraPaths"), "error should name the bad field, got: {err}");
    }

    #[test]
    fn malformed_known_type_overrides_json_is_a_hard_error_not_a_silent_empty_map() {
        let mut js = base_options();
        js.known_type_overrides_json = Some("not valid json".into());

        let err = PipelineOptions::try_from(js).expect_err("malformed JSON should not silently default");
        assert!(err.contains("knownTypeOverrides"), "error should name the bad field, got: {err}");
    }

    #[test]
    fn valid_extra_paths_json_is_parsed_correctly() {
        let mut js = base_options();
        js.extra_paths_json = Some(r#"{"@myorg/ui": ["../packages/ui/src"]}"#.into());

        let opts = PipelineOptions::try_from(js).expect("should parse valid JSON");
        assert_eq!(opts.extra_paths.get("@myorg/ui").map(|v| v.len()), Some(1));
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
}
