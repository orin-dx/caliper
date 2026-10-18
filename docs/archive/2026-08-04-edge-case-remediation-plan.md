# Edge-Case Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the 11 mechanism-level root causes and 7 standalone findings catalogued in `docs/root-cause-analysis.md` (itself derived from the edge-case audit in `docs/edge-cases.md`) — closing silent data-loss paths, containing panics at a real boundary, fixing the one confirmed silent-correctness bug (P0-1, resolver precedence), and correcting several drift/consistency gaps — each with TDD tests covering the specific edge case being fixed.

**Architecture:** No structural changes to the pipeline (`discover → parse → merge → resolve → serialize`). Every task is either (a) a new, narrow "give up and record why" entry point replacing a silent bare-default/None return, (b) a new panic-containment boundary (`panic_guard::contain_panic`) wrapping existing call sites, or (c) a small correctness/consistency fix (precedence ordering, truncation formatting, exit-code contract, sealed-field invariant). Two new architectural decisions get a real ADR written as part of their task group: panic containment (0005) and, conditionally, JSON Schema derivation (0006, only if that group's cost/benefit call lands on the `schemars` route rather than the floor-level drift-detection-test alternative).

**Tech Stack:** Rust (`oxc-react-docgen-core`, `oxc-react-docgen-cli`, `oxc-react-docgen-napi-binding`), `cargo-nextest`/`cargo test`, `cargo clippy -- -D warnings`, `cargo insta` for snapshots.

**Provenance:** every task below was drafted by an agent instructed to read the actual current source at each cited location (not just trust the root-cause document's summary), then independently adversarially verified by a second, skeptical pass against `docs/root-cause-analysis.md`. Two mechanisms the skeptical pass found unsound have been corrected inline where marked **`[corrected]`** — the resolver give-up constructor scope (narrowed from "6 of ~10 sites" to the 3 real ones, with the `PropType::Opaque`/`ResolvedChain` restructure redesigned to respect the no-reverse-dependency rule and the `known.rs`/`Deserialize` split), and the `ParsedProp` invariant fix (from an unsound `pub(crate)` field-visibility change to a sealed-field pattern that actually compiles across the `crates/cli` boundary and actually prevents same-crate misuse).

---

## Execution order

Run **Part A (panic containment) first** — it touches the most shared surface (`pipeline/mod.rs`, `plugin.rs`, `crates/binding/src/lib.rs`, `pipeline/watch.rs`) and nothing else structurally depends on a particular implementation choice there, but later parts assume it exists if they touch the same files concurrently. After Part A lands, Parts B through G are independently parallelizable (each documents its own internal task-ordering/dependencies):

- **Part A** — Panic-containment boundary (+ ADR 0005) — do first
- **Part B** — Resolver give-up constructors + precedence fix (P0-1)
- **Part C** — Extractor diagnostic channel + depth-tracking
- **Part D** — Pipeline discovery/merge fixes
- **Part E** — Allocation caps + LSP scaffold hardening
- **Part F** — TOON truncation + schema drift + CLI exit-code contract
- **Part G** — Standalone fixes (substitute.rs, watch --out, ParsedProp, doc-comment-only items)

Parts B and C each build on Part A only in the sense that they touch adjacent files under active development — verify no merge conflicts before starting either in parallel with Part A rather than after it.

After every part lands: run the full verification suite (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, snapshot review via `/snapshot` if any snapshot-affecting file changed) and update `docs/edge-cases.md`'s tables to mark the resolved findings, plus `docs/STATUS.md`'s numbers.

---

## Part A: Panic-containment boundary (+ ADR 0005)

### Task 1: `DiagnosticCode::InternalPanic` variant

**Files:**

- Modify: `crates/core/src/types/diagnostic.rs:57-78`
- Test: inline `#[cfg(test)]` module in the same file (matches `io_read_error_reports_the_path_and_underlying_error` directly above)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn internal_panic_code_serializes_as_screaming_snake_case() {
    let diagnostic = Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: "panic in rayon worker".into(),
        file: None,
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::InternalPanic,
    };
    let json = serde_json::to_string(&diagnostic).unwrap();
    assert!(json.contains("\"INTERNAL_PANIC\""), "expected INTERNAL_PANIC in {json}");
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core internal_panic_code_serializes_as_screaming_snake_case -- --nocapture` Expected: FAIL with a compile error — `no variant named InternalPanic found for enum DiagnosticCode`.

- [ ] **Step 3: Write minimal implementation**

```rust
    /// TypeScript syntax error reported by the parser.
    ParseError,
    /// An internal panic was caught and converted into a diagnostic instead
    /// of crashing the process (see ADR 0005). Never expected in normal
    /// operation — always a bug, filed with the panic's own message.
    InternalPanic,
```

(Appended as the last variant of the existing `DiagnosticCode` enum at `crates/core/src/types/diagnostic.rs:78`, right after `ParseError`.)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core internal_panic_code_serializes_as_screaming_snake_case -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/diagnostic.rs
git commit -m "feat(diagnostic): add InternalPanic code for caught panics"
```

### Task 2: `panic_guard::contain_panic` module

**Files:**

- Create: `crates/core/src/panic_guard.rs`
- Modify: `crates/core/src/lib.rs:3-12` (register the module — `pub`, not `pub(crate)`, because `crates/binding` needs to call it directly from across the crate boundary; the mechanism doc's `pub(crate)` sketch didn't anticipate that NAPI call sites live in a separate crate)
- Test: inline `#[cfg(test)]` module in `panic_guard.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DiagnosticCode;

    #[test]
    fn contain_panic_returns_ok_when_f_does_not_panic() {
        let result = contain_panic("test", || 42);
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn contain_panic_converts_str_panic_to_diagnostic() {
        let result: Result<(), Diagnostic> = contain_panic("resolve:Button", || panic!("boom"));
        let diagnostic = result.expect_err("panic should be caught, not propagated");
        assert_eq!(diagnostic.code, DiagnosticCode::InternalPanic);
        assert!(diagnostic.message.contains("resolve:Button"), "message should carry the label, got {}", diagnostic.message);
        assert!(diagnostic.message.contains("boom"), "message should carry the panic text, got {}", diagnostic.message);
    }

    #[test]
    fn contain_panic_converts_string_panic_to_diagnostic() {
        let result: Result<(), Diagnostic> = contain_panic("parse:foo.tsx", || panic!("bad input: {}", 42));
        let diagnostic = result.expect_err("panic should be caught, not propagated");
        assert!(diagnostic.message.contains("bad input: 42"), "got {}", diagnostic.message);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core panic_guard:: -- --nocapture` Expected: FAIL with a compile error — `panic_guard` module (and `contain_panic`) doesn't exist yet.

- [ ] **Step 3: Write minimal implementation**

`crates/core/src/panic_guard.rs`:

```rust
//! Single sanctioned panic-containment boundary (see ADR 0005).
//!
//! Every panic reachable from a rayon `.map()`, a `DocgenPlugin` hook, or a
//! NAPI entry point crosses this function on its way to becoming a
//! `Diagnostic` instead of aborting a batch, killing the whole pipeline, or
//! poisoning a session lock.

use crate::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// Run `f`, converting a panic into `Err(Diagnostic)` tagged with `label`
/// instead of letting it unwind past this call site.
///
/// Wraps `f` in `AssertUnwindSafe` internally rather than requiring callers
/// to prove unwind-safety themselves. This codebase's data (`SourceData`,
/// `ComponentEntry`, plugin state) has no interior mutability (`Cell`,
/// `RefCell`) that could leave a torn, observable half-write behind after a
/// caught panic — the whole operation is abandoned and its output discarded
/// (or its `&mut` target left exactly as it was before the panicking call),
/// so the invariant `AssertUnwindSafe` exists to protect doesn't apply here.
pub fn contain_panic<T>(label: &str, f: impl FnOnce() -> T) -> Result<T, Diagnostic> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => Err(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("{label} panicked: {}", panic_message(&payload)),
            file: None,
            line: None,
            column: None,
            help: Some("This is an internal bug — please file a report with the input that triggered it.".into()),
            code: DiagnosticCode::InternalPanic,
        }),
    }
}

/// Extract a human-readable message from a `catch_unwind` payload — panics
/// carry either a `&str` (`panic!("literal")`) or a `String`
/// (`panic!("{}", x)`); anything else has no stable text representation.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}
```

`crates/core/src/lib.rs` — add the module declaration next to the other `pub mod` entries:

```rust
pub mod pipeline;
pub mod plugin;
pub mod panic_guard;
pub mod react_types;
```

(inserted alphabetically-adjacent to `plugin`/`react_types` at `crates/core/src/lib.rs:7-9`)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core panic_guard:: -- --nocapture` Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/panic_guard.rs crates/core/src/lib.rs
git commit -m "feat(core): add contain_panic, the single panic-containment boundary"
```

### Task 3: Wrap the rayon parse-phase closure (`pipeline/mod.rs` Phase 2)

**Files:**

- Modify: `crates/core/src/pipeline/mod.rs:259-279`
- Test: inline `#[cfg(test)]` module in the same file (`crates/core/src/pipeline/mod.rs:490+`, matches this file's existing `TempDir`-based fixture style)

- [ ] **Step 1: Write the failing test**

Depends on a test-only panicking plugin hooked into `on_file_extracted`, since that's the only way to inject a real panic into the parse phase without modifying non-test code. Add this to the existing `#[cfg(test)] mod tests` block in `pipeline/mod.rs`:

```rust
    #[test]
    fn a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash() {
        use crate::plugin::{DocgenPlugin, PluginRegistry};

        struct PanickingOnFileExtracted;
        impl DocgenPlugin for PanickingOnFileExtracted {
            fn name(&self) -> &str {
                "panicking-on-file-extracted"
            }
            fn on_file_extracted(&self, _path: &Utf8Path, _data: &mut SourceData) {
                panic!("simulated parse-phase panic");
            }
        }

        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");
        write_file(&tmp, "Other.tsx", "export const Other = () => null;");

        let mut plugins = PluginRegistry::new();
        plugins.register(PanickingOnFileExtracted);

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            plugins,
            ..Default::default()
        };

        // Must not panic the test process — that's the whole point.
        let output = extract(&options);

        assert_eq!(output.stats.files_parsed, 2, "both files should still be counted as discovered");
        assert!(
            output.diagnostics.iter().any(|d| d.code == DiagnosticCode::InternalPanic),
            "expected an InternalPanic diagnostic, got {:?}",
            output.diagnostics
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash -- --nocapture` Expected: FAIL — the test thread itself panics and aborts the test binary (no containment exists yet on this path; `on_file_extracted` runs inside the Phase 3 merge loop today, uncontained).

- [ ] **Step 3: Write minimal implementation**

Replace the Phase 2 `.map()` body at `crates/core/src/pipeline/mod.rs:259-279`:

```rust
    // Phase 2: Parallel parse with rayon — check DTS cache for .d.ts files.
    let source_data_vec: Vec<(Utf8PathBuf, SourceData, Option<Diagnostic>)> = src_files
        .par_iter()
        .map(|path| {
            let label = format!("parse:{path}");
            crate::panic_guard::contain_panic(&label, || {
                let is_dts = path.as_str().ends_with(".d.ts");
                if is_dts {
                    if let Some(cached) = cache_ref.get(path) {
                        cache_hits.fetch_add(1, Ordering::Relaxed);
                        return (path.clone(), cached, None);
                    }
                }
                let (source, io_diag) = match std::fs::read_to_string(path) {
                    Ok(s) => (s, None),
                    Err(e) => (String::new(), Some(Diagnostic::io_read_error(path, &e))),
                };
                let data = crate::extractor::parse_file(path, &source);
                if is_dts {
                    cache_ref.insert(path, data.clone());
                }
                (path.clone(), data, io_diag)
            })
            .unwrap_or_else(|diag| (path.clone(), SourceData::default(), Some(diag)))
        })
        .collect();
```

Since the panic in this task's test actually happens in `run_on_file_extracted` (Phase 3, not Phase 2's parse closure itself), also wrap that call — this is the same mechanism, and both need to land together for the test to pass. At `crates/core/src/pipeline/mod.rs:293`:

```rust
        options.plugins.run_on_file_extracted(&path, &mut data);
```

becomes (this line's full containment — including tagging with the plugin's name — lands properly in Task 5; for now, wrap the whole call here so this test passes without waiting on Task 5's `PluginRegistry` API change):

```rust
        if let Err(diag) =
            crate::panic_guard::contain_panic(&format!("on_file_extracted:{path}"), || {
                options.plugins.run_on_file_extracted(&path, &mut data)
            })
        {
            diagnostics.push(diag);
        }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/mod.rs
git commit -m "fix(pipeline): contain panics in the parse-phase rayon batch"
```

### Task 4: Wrap the rayon resolve-phase closure (`pipeline/mod.rs` Phase 4)

**Files:**

- Modify: `crates/core/src/pipeline/mod.rs:356-357`
- Test: inline `#[cfg(test)]` module in the same file

- [ ] **Step 1: Write the failing test**

`resolve_component` itself can't be made to panic without editing non-test resolver code, so exercise the boundary via a panicking `on_component_resolved` plugin hook instead — same shape as Task 3's test, but for Phase 4/5's `run_on_component_resolved` call at `mod.rs:385`.

```rust
    #[test]
    fn a_panic_during_component_resolved_hook_degrades_to_a_diagnostic_not_a_crash() {
        use crate::plugin::{DocgenPlugin, PluginRegistry};
        use crate::types::ComponentEntry;

        struct PanickingOnComponentResolved;
        impl DocgenPlugin for PanickingOnComponentResolved {
            fn name(&self) -> &str {
                "panicking-on-component-resolved"
            }
            fn on_component_resolved(&self, _entry: &mut ComponentEntry) {
                panic!("simulated resolve-phase panic");
            }
        }

        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export function Button(props: { label: string }) { return null; }\n");

        let mut plugins = PluginRegistry::new();
        plugins.register(PanickingOnComponentResolved);

        let options = PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            plugins,
            ..Default::default()
        };

        let output = extract(&options);

        assert!(
            output.components.contains_key("Button"),
            "the component itself should still be extracted even if its post-resolve hook panics"
        );
        assert!(
            output.diagnostics.iter().any(|d| d.code == DiagnosticCode::InternalPanic),
            "expected an InternalPanic diagnostic, got {:?}",
            output.diagnostics
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core a_panic_during_component_resolved_hook_degrades_to_a_diagnostic_not_a_crash -- --nocapture` Expected: FAIL — test thread panics and aborts (no containment on `run_on_component_resolved` yet).

- [ ] **Step 3: Write minimal implementation**

Wrap the resolve-phase `.map()` at `crates/core/src/pipeline/mod.rs:356-357`:

```rust
    let ctx = Arc::new(ResolutionContext::new(global.clone(), options));
    let results: Vec<(ComponentEntry, Vec<Diagnostic>)> = mappings
        .par_iter()
        .map(|mapping| {
            let label = format!("resolve:{}", mapping.component_name);
            crate::panic_guard::contain_panic(&label, || resolve_component(mapping, &ctx)).unwrap_or_else(|diag| {
                let stub = ComponentEntry {
                    display_name: mapping.component_name.clone(),
                    file_path: mapping.file_path.clone(),
                    description: String::new(),
                    props: Default::default(),
                    inheritance: vec![],
                    notable_inherited: Default::default(),
                    discriminant_prop: None,
                    composes: vec![],
                    tags: Default::default(),
                    methods: vec![],
                };
                (stub, vec![diag])
            })
        })
        .collect();
```

And wrap the `run_on_component_resolved` call at `crates/core/src/pipeline/mod.rs:385` (same pattern as Task 3's Phase 3 fix, applied to Phase 5):

```rust
        let mut entry = entry;
        if let Err(diag) = crate::panic_guard::contain_panic(
            &format!("on_component_resolved:{}", entry.display_name),
            || options.plugins.run_on_component_resolved(&mut entry),
        ) {
            diagnostics.push(diag);
        }
        components.insert(key, entry);
        diagnostics.extend(diags);
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core a_panic_during_component_resolved_hook_degrades_to_a_diagnostic_not_a_crash -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/mod.rs
git commit -m "fix(pipeline): contain panics in the resolve-phase rayon batch"
```

### Task 5: Per-plugin-call containment in `PluginRegistry`

**Files:**

- Modify: `crates/core/src/plugin.rs:47-57`
- Modify: `crates/core/src/pipeline/mod.rs:293,385` (replace Task 3/4's inline wraps now that `PluginRegistry` reports its own diagnostics per-plugin)
- Test: inline `#[cfg(test)]` module in `crates/core/src/plugin.rs`

- [ ] **Step 1: Write the failing test**

Note: `run_on_file_extracted`/`run_on_component_resolved` currently return `()`; this test calls them expecting `Vec<Diagnostic>`, so it won't compile until Step 3 lands — that's the "fails" state.

```rust
    #[test]
    fn a_panicking_plugin_is_contained_and_tagged_with_its_name_others_still_run() {
        struct PanickingPlugin;
        impl DocgenPlugin for PanickingPlugin {
            fn name(&self) -> &str {
                "panicking-plugin"
            }
            fn on_component_resolved(&self, _entry: &mut ComponentEntry) {
                panic!("boom");
            }
        }

        let mut registry = PluginRegistry::new();
        registry.register(PanickingPlugin);
        registry.register(TestEnricherPlugin);

        let mut entry = ComponentEntry {
            display_name: "Button".into(),
            file_path: "src/Button.tsx".into(),
            props: Default::default(),
            description: String::new(),
            inheritance: vec![],
            notable_inherited: Default::default(),
            discriminant_prop: None,
            composes: vec![],
            tags: Default::default(),
            methods: vec![],
        };

        let diagnostics = registry.run_on_component_resolved(&mut entry);

        assert_eq!(diagnostics.len(), 1, "expected exactly one diagnostic, from the panicking plugin");
        assert_eq!(diagnostics[0].code, crate::types::DiagnosticCode::InternalPanic);
        assert!(
            diagnostics[0].message.contains("panicking-plugin"),
            "diagnostic should name the panicking plugin, got {}",
            diagnostics[0].message
        );
        assert_eq!(
            entry.composes,
            vec!["TestEnricher"],
            "the second, well-behaved plugin should still run after the first one panicked"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core a_panicking_plugin_is_contained_and_tagged_with_its_name_others_still_run -- --nocapture` Expected: FAIL with a compile error — `run_on_component_resolved` returns `()`, not `Vec<Diagnostic>`.

- [ ] **Step 3: Write minimal implementation**

`crates/core/src/plugin.rs:47-57`:

```rust
    pub fn run_on_file_extracted(&self, file_path: &camino::Utf8Path, data: &mut SourceData) -> Vec<crate::types::Diagnostic> {
        let mut diagnostics = Vec::new();
        for plugin in &self.plugins {
            let label = format!("plugin:{}:on_file_extracted", plugin.name());
            if let Err(diag) = crate::panic_guard::contain_panic(&label, || plugin.on_file_extracted(file_path, data)) {
                diagnostics.push(diag);
            }
        }
        diagnostics
    }

    pub fn run_on_component_resolved(&self, entry: &mut ComponentEntry) -> Vec<crate::types::Diagnostic> {
        let mut diagnostics = Vec::new();
        for plugin in &self.plugins {
            let label = format!("plugin:{}:on_component_resolved", plugin.name());
            if let Err(diag) = crate::panic_guard::contain_panic(&label, || plugin.on_component_resolved(entry)) {
                diagnostics.push(diag);
            }
        }
        diagnostics
    }
```

Update the two call sites this supersedes back in `pipeline/mod.rs`, replacing Task 3's Phase 3 wrap (`:293`):

```rust
        diagnostics.extend(options.plugins.run_on_file_extracted(&path, &mut data));
```

and Task 4's Phase 5 wrap (`:385`):

```rust
        let mut entry = entry;
        diagnostics.extend(options.plugins.run_on_component_resolved(&mut entry));
        components.insert(key, entry);
        diagnostics.extend(diags);
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core -- --nocapture` (full crate — this touches two files' call sites, so also re-run Task 3/4's tests: `a_panic_during_parse_phase_degrades_to_a_diagnostic_not_a_crash`, `a_panic_during_component_resolved_hook_degrades_to_a_diagnostic_not_a_crash`) Expected: PASS, all plugin/pipeline panic-containment tests green

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/plugin.rs crates/core/src/pipeline/mod.rs
git commit -m "fix(plugin): isolate panics per-plugin-call, tagged with the plugin's name"
```

### Task 6: Panic-contain the five NAPI entry points

**Files:**

- Modify: `crates/binding/src/lib.rs:118-127` (`extract_all`)
- Modify: `crates/binding/src/lib.rs:132-138` (`create_session`)
- Modify: `crates/binding/src/lib.rs:143-166` (`extract_file_incremental`)
- Modify: `crates/binding/src/lib.rs:171-189` (`initialize_session`)
- Modify: `crates/binding/src/lib.rs:192-195` (`close_session`)
- Test: inline `#[cfg(test)]` module in the same file (matches the existing `unset_fields_match_pipeline_options_defaults` style)

- [ ] **Step 1: Write the failing test**

`crates/binding` can't practically trigger a real panic inside `WatchSession::new`/`extract` without editing non-test core code, so this proves the one thing this task actually adds: `panic_guard::contain_panic` is `pub` and reachable across the crate boundary these five entry points need to call it from.

```rust
    #[test]
    fn contain_panic_is_reachable_from_the_binding_crate() {
        let result: Result<i32, oxc_react_docgen_core::Diagnostic> =
            oxc_react_docgen_core::panic_guard::contain_panic("binding-test", || panic!("boom from binding"));
        let diag = result.expect_err("panic should be contained, not propagated across the FFI boundary");
        assert!(diag.message.contains("boom from binding"), "got {}", diag.message);
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-binding contain_panic_is_reachable_from_the_binding_crate -- --nocapture` Expected: FAIL with a compile error — `panic_guard` isn't a public module of `oxc-react-docgen-core` yet (Task 2 declared it `pub mod`, but confirm; if Task 2 already used `pub mod`, this instead fails because `crates/binding/Cargo.toml`/imports haven't been touched — no, `oxc-react-docgen-core` is already a path dependency, so the only reason this fails is if `panic_guard` were still `pub(crate)`. Given Task 2 makes it `pub mod`, this step should actually already compile — treat this as a regression/contract test locking that visibility in, and skip to Step 4 if Task 2 already landed.)

- [ ] **Step 3: Write minimal implementation**

`extract_all` (`crates/binding/src/lib.rs:118-127`):

```rust
#[napi]
pub async fn extract_all(options: JsExtractOptions) -> napi::Result<String> {
    let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;
    tokio::task::spawn_blocking(move || {
        match oxc_react_docgen_core::panic_guard::contain_panic("extract_all", move || {
            let output = oxc_react_docgen_core::pipeline::extract(&pipeline_options);
            extraction_output_to_json(&output).map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
}
```

`create_session` (`crates/binding/src/lib.rs:132-138`):

```rust
#[napi]
pub fn create_session(options: JsExtractOptions) -> napi::Result<u32> {
    let pipeline_options = PipelineOptions::try_from(options).map_err(napi::Error::from_reason)?;
    match oxc_react_docgen_core::panic_guard::contain_panic("create_session", move || {
        let id = next_session_id();
        let session = Arc::new(WatchSession::new(pipeline_options));
        SESSIONS.insert(id, session);
        id
    }) {
        Ok(id) => Ok(id),
        Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
    }
}
```

`extract_file_incremental` (`crates/binding/src/lib.rs:143-166`) — only the `spawn_blocking` body changes:

```rust
    tokio::task::spawn_blocking(move || {
        match oxc_react_docgen_core::panic_guard::contain_panic("extract_file_incremental", move || {
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
```

`initialize_session` (`crates/binding/src/lib.rs:171-189`) — only the `spawn_blocking` body changes:

```rust
    tokio::task::spawn_blocking(move || {
        match oxc_react_docgen_core::panic_guard::contain_panic("initialize_session", move || {
            let output = session.initialize();
            oxc_react_docgen_core::pipeline::extraction_output_to_json(&output)
                .map_err(|e| napi::Error::from_reason(e.to_string()))
        }) {
            Ok(result) => result,
            Err(diag) => Err(napi::Error::from_reason(diag.to_string())),
        }
    })
    .await
    .map_err(|e| napi::Error::from_reason(e.to_string()))?
```

`close_session` (`crates/binding/src/lib.rs:192-195`):

```rust
#[napi]
pub fn close_session(session_id: u32) {
    let _ = oxc_react_docgen_core::panic_guard::contain_panic("close_session", move || {
        SESSIONS.remove(&session_id);
    });
}
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-binding -- --nocapture` Expected: PASS (including the pre-existing `unset_fields_match_pipeline_options_defaults` and JSON-error tests, unaffected by this change)

- [ ] **Step 5: Commit**

```bash
git add crates/binding/src/lib.rs
git commit -m "fix(binding): contain panics at all five NAPI entry points"
```

### Task 7: Fix `watch.rs`'s poisonable init-lock `.expect(...)`

**Files:**

- Modify: `crates/core/src/pipeline/watch.rs:93`
- Test: inline `#[cfg(test)]` module in the same file (`crates/core/src/pipeline/watch.rs:220+`, matches this file's existing `WatchSession` fixture style)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn initialize_recovers_from_a_poisoned_lock_instead_of_panicking() {
        let session = WatchSession::new(empty_options());

        // Poison the lock the same way an uncontained panic elsewhere inside
        // initialize() could, before panic containment landed on the call
        // sites initialize() reaches (Task 3/4/5).
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = session.initialized.lock().unwrap();
            panic!("simulated panic while holding the init lock");
        }));
        assert!(poisoned.is_err(), "the panic should have unwound past the lock guard");
        assert!(session.initialized.is_poisoned(), "the lock should now be poisoned");

        // Must recover instead of propagating a second panic via .expect(...).
        let output = session.initialize();
        assert!(output.components.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core initialize_recovers_from_a_poisoned_lock_instead_of_panicking -- --nocapture` Expected: FAIL — `session.initialize()`'s `.expect("init lock poisoned")` panics on the poisoned lock, aborting the test.

- [ ] **Step 3: Write minimal implementation**

`crates/core/src/pipeline/watch.rs:93`, inside `initialize()`:

```rust
        let mut guard = self.initialized.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
```

(replaces `let mut guard = self.initialized.lock().expect("init lock poisoned");`)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core initialize_recovers_from_a_poisoned_lock_instead_of_panicking -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/watch.rs
git commit -m "fix(watch): recover from a poisoned init lock instead of panicking"
```

### Task 8: Write ADR 0005 and document the rule in `crates/core/CLAUDE.md`

**Files:**

- Create: `docs/adr/0005-panic-containment-boundary.md`
- Modify: `crates/core/CLAUDE.md` (append a short "Panics" section)

- [ ] **Step 1: Verify the ADR doesn't exist yet** Run: `ls docs/adr/0005-panic-containment-boundary.md` Expected: FAIL — `No such file or directory`

- [ ] **Step 2: (same check, listing the directory for context)** Run: `ls docs/adr/` Expected: `0000-template.md 0001-... 0002-... 0003-... 0004-... README.md` — confirms `0005` is the next free number.

- [ ] **Step 3: Write the ADR and CLAUDE.md addition**

`docs/adr/0005-panic-containment-boundary.md`:

```markdown
# 0005. Contain panics at a single, per-item boundary

**Status:** Accepted **Date:** 2026-08-04

## Context

No existing ADR covers panic/unwind policy. Panic safety today is an accident of `tokio::spawn_blocking`'s `JoinError`, inconsistent across NAPI entry points, plugin hooks, and rayon batches — `create_session`/ `close_session` didn't even get that accidental protection, and `watch.rs`'s `std::sync::Mutex` could poison permanently as a direct consequence. Picking the wrong granularity later, once call sites depend on it, is expensive to fix retroactively.

## Decision

Panics reachable from a rayon `.map()`, a `DocgenPlugin` hook, or a NAPI entry point are contained at per-file / per-plugin-call / per-entry-point granularity through one sanctioned helper, `panic_guard::contain_panic`, converting the payload into a `Diagnostic` (or `napi::Error`) instead of aborting a batch, killing the whole pipeline, or poisoning a session lock.

## Consequences

- One bad file, plugin, or session call degrades to a diagnostic instead of taking down everything sharing its batch, pipeline, or session.
- Every future concurrent, plugin, or FFI entry point has one obvious place to route through, instead of re-deriving the answer.
- `watch.rs`'s poisoned-mutex trap is now much less likely, since nothing panics while the init lock is held — and `.expect(...)` was replaced with `.unwrap_or_else(|p| p.into_inner())` as defense in depth for whatever still slips through.
- `contain_panic` is `pub`, not `pub(crate)`, because `crates/binding`'s five NAPI entry points need to call it across the crate boundary — a slightly wider surface than a pure-internal helper, accepted because the alternative (duplicating the containment logic in `crates/binding`) is exactly the kind of drift this ADR exists to prevent.
```

`crates/core/CLAUDE.md` — append after the "## Resolver" section:

```markdown
## Panics

Every panic reachable from a rayon `.map()`, a `DocgenPlugin` hook, or a NAPI entry point must cross `panic_guard::contain_panic` — see ADR 0005. Never add a new concurrent or FFI-facing entry point without routing it through this helper first.
```

- [ ] **Step 4: Verify** Run: `ls docs/adr/0005-panic-containment-boundary.md && grep -q "Panics" crates/core/CLAUDE.md && echo OK` Expected: `OK`

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0005-panic-containment-boundary.md crates/core/CLAUDE.md
git commit -m "docs(adr): accept 0005, contain panics at a single boundary"
```

---

## Part B: Resolver give-up constructors + precedence fix (P0-1)

### Task 1: `ResolvedChain::give_up` — cycle-detected path stops silently degrading

**Files:**

- Modify: `crates/core/src/resolver/mod.rs:345-395` (the `ResolvedChain` struct + `impl` block)
- Modify: `crates/core/src/resolver/chain.rs:39-41,108,120,191`
- Modify: `crates/core/src/resolver/alias.rs:106,116,168,178,291,380`
- Modify: `crates/core/src/resolver/extends.rs:65`
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/resolver/mod.rs` (existing convention — `empty_ctx()`/`resolve_type()` helpers already live there)

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/mod.rs, inside `mod tests`

// ── Test: self-referential extends must emit a diagnostic, not silently
// return an empty chain ──────────────────────────────────────────────────
// Regression test for: chain.rs's cycle-detected path returned a bare
// `ResolvedChain::default()` with zero diagnostic when a type's own extends
// chain referenced itself, silently degrading with no trace — unlike every
// other give-up path in the resolver.

#[test]
fn test_self_referential_extends_emits_diagnostic_instead_of_silent_default() {
    let file_path = Utf8PathBuf::from("/test/cycle.tsx");
    let scoped_key = format!("{}:LoopProps", file_path);

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        scoped_key.clone(),
        CollectedInterface {
            scoped_key: scoped_key.clone(),
            name: "LoopProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "id".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            // Self-referential — LoopProps extends itself.
            extends: vec![ExtendsRef::SameFile { name: "LoopProps".into(), type_args: vec![] }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping {
        component_name: "Loop".into(),
        props_type_name: "LoopProps".into(),
        props_type_args: vec![],
        file_path: file_path.clone(),
        description: String::new(),
        tags: BTreeMap::new(),
        span_start: 0,
        span_end: 0,
        param_defaults: FxHashMap::default(),
    };

    let (entry, diagnostics) = resolve_component(&mapping, &ctx);

    // Own prop still resolves fine — the cycle is only in the extends chain.
    assert!(entry.props.contains_key("id"));
    assert!(
        diagnostics.iter().any(|d| d.message.to_lowercase().contains("circular")),
        "Expected a diagnostic about the circular extends reference, got {:?}",
        diagnostics
    );
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_self_referential_extends_emits_diagnostic_instead_of_silent_default -- --nocapture` Expected: FAIL — `resolve_component` returns diagnostics with no "circular" message, because `chain.rs:40` returns `ResolvedChain::default()` on the cycle hit without pushing anything.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/resolver/mod.rs`, replace the `ResolvedChain` struct/impl:

```rust
/// Result of resolving a props chain (including extends).
struct ResolvedChain {
    /// Resolved props (may include inherited props).
    props: Vec<ParsedProp>,
    /// Inheritance layers, outermost first.
    inheritance: Vec<InheritedLayer>,
    /// Inherited props keyed by name — for notable_inherited population.
    inherited_by_name: FxHashMap<String, ParsedProp>,
    /// Type names that could not be resolved.
    composes: Vec<String>,
    /// Discriminant prop name if this is a discriminated union.
    discriminant_prop: Option<String>,
}

impl ResolvedChain {
    /// The base empty chain. Deliberately not `Default` — every accumulator
    /// site (`resolve_interface_chain`, `resolve_base_as_chain`, ...) that
    /// wants "start empty, then push into" calls this directly instead; the
    /// two *give-up* entry points (`empty_with_compose`, `give_up`) are the
    /// only public-facing "this type contributes nothing" signals. Visible to
    /// `chain`/`alias`/`extends` since they're child modules of `resolver`.
    fn empty() -> Self {
        Self {
            props: Vec::new(),
            inheritance: Vec::new(),
            inherited_by_name: FxHashMap::default(),
            composes: Vec::new(),
            discriminant_prop: None,
        }
    }

    fn empty_with_compose(type_name: String) -> Self {
        Self { composes: vec![type_name], ..Self::empty() }
    }

    /// Give up resolving `type_name` — record it in `composes` (same as
    /// `empty_with_compose`) and, if `diag` is given, push it onto `state`
    /// first. The sanctioned "give up" entry point for every "this type
    /// couldn't be followed further, stop here" path. Replaces the bare
    /// `ResolvedChain::default()` the cycle-detected branch used to return,
    /// which let that one give-up site skip explaining why.
    fn give_up(type_name: String, diag: Option<Diagnostic>, state: &mut ResolveState) -> Self {
        if let Some(d) = diag {
            state.diagnostics.push(d);
        }
        Self::empty_with_compose(type_name)
    }

    /// Merge a parent chain into self — own props already in `self.props` take priority.
    fn merge_parent(&mut self, parent: ResolvedChain) {
        // Collect existing prop names so we can skip duplicates.
        let existing: FxHashSet<String> = self.props.iter().map(|p| p.name.clone()).collect();

        for prop in parent.props {
            if !existing.contains(&prop.name) {
                self.props.push(prop.clone());
            }
            // Always populate inherited_by_name so notable lookup works.
            self.inherited_by_name.entry(prop.name.clone()).or_insert(prop);
        }

        // Prepend parent inheritance layers (parent is further up the chain).
        let mut new_inheritance = parent.inheritance;
        new_inheritance.append(&mut self.inheritance);
        self.inheritance = new_inheritance;

        self.composes.extend(parent.composes);
        for (name, prop) in parent.inherited_by_name {
            self.inherited_by_name.entry(name).or_insert(prop);
        }

        if self.discriminant_prop.is_none() {
            self.discriminant_prop = parent.discriminant_prop;
        }
    }
}
```

(Note: `#[derive(Default)]` is removed from the struct — `empty()` replaces it.)

In `crates/core/src/resolver/chain.rs`, replace the cycle-detected branch (lines 39-41):

```rust
    let visit_key: CompactString = format!("{}:{}<{}>", consuming_file, type_name, type_args.join(",")).into();
    if !state.visited.insert(visit_key) {
        return ResolvedChain::give_up(
            type_name.to_owned(),
            Some(Diagnostic {
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Circular type reference detected resolving '{}' in '{}' — stopping here to avoid infinite recursion",
                    type_name, consuming_file
                ),
                file: Some(consuming_file.to_string()),
                line: None,
                column: None,
                help: Some("This type (directly or indirectly) extends or references itself.".into()),
                code: DiagnosticCode::MaxDepthExceeded,
            }),
            state,
        );
    }
```

And fix the two now-invalid `..Default::default()` accumulator sites in the same file:

- Line 108: `KnownPatternResult::Props(props) => ResolvedChain { props, ..Default::default() },` → `KnownPatternResult::Props(props) => ResolvedChain { props, ..ResolvedChain::empty() },`
- Line 120: `ResolvedChain { inheritance: vec![layer], ..Default::default() }` → `ResolvedChain { inheritance: vec![layer], ..ResolvedChain::empty() }`
- Line 191: `let mut chain = ResolvedChain::default();` → `let mut chain = ResolvedChain::empty();`

In `crates/core/src/resolver/alias.rs`, replace each bare-default accumulator site:

- Line 106: `let mut chain = ResolvedChain::default();` → `let mut chain = ResolvedChain::empty();`
- **Line 116 `[corrected]`: this is one of the 3 confirmed genuine silent-give-up sites, not an accumulator — it needs a diagnostic, not just a rename.** A `LiteralUnion` used directly as a props base is malformed usage, and this path currently gives up with zero trace, unlike the parallel diagnosed path at `alias.rs:234-253` which pushes a "cannot be used as a component's props base" diagnostic for other non-object-like types. Fix: match that existing diagnostic's shape and route through `give_up`:

```rust
CollectedTypeAlias::LiteralUnion { members, .. } => {
    let diag = Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!(
            "'{}' is a literal union and can't be used as a component's props base in '{}' — \
             expected an interface, intersection, union, or inline object type",
            members.join(" | "),
            consuming_file
        ),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some("Check that this type resolves to an object-like shape.".into()),
        code: DiagnosticCode::OpaqueType,
    };
    ResolvedChain::give_up(members.join(" | "), Some(diag), state)
}
```

(Adjust the exact match pattern/field names to what `CollectedTypeAlias::LiteralUnion`'s real variant shape is — read the actual definition in `types/collected.rs` before writing this arm; the message text should mirror whatever wording `alias.rs:234-253`'s existing diagnostic actually uses, for consistency.) Add a dedicated regression test alongside the cycle-detected test in Step 1, asserting a diagnostic is now emitted for a props type that resolves to a bare literal union (e.g. `type Props = 'a' | 'b' | 'c'` used directly as a component's props type).

- Line 168: `let mut chain = ResolvedChain::default();` → `let mut chain = ResolvedChain::empty();`
- Line 178: `let mut chain = ResolvedChain::default();` → `let mut chain = ResolvedChain::empty();`
- Line 291: `let mut chain = ResolvedChain::default();` → `let mut chain = ResolvedChain::empty();`
- Line 380: `ResolvedChain { props: merged_props.into_values().collect(), discriminant_prop: discriminant, ..Default::default() }` → `ResolvedChain { props: merged_props.into_values().collect(), discriminant_prop: discriminant, ..ResolvedChain::empty() }`

In `crates/core/src/resolver/extends.rs`, line 65: `return (ResolvedChain::default(), Some(layer));` → `return (ResolvedChain::empty(), Some(layer));`

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_self_referential_extends_emits_diagnostic_instead_of_silent_default -- --nocapture` then `cargo test -p oxc-react-docgen-core` Expected: PASS, full suite green (the `empty()`-substitution sites are pure renames with identical behavior, so existing snapshot/unit tests act as the regression net for them).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/mod.rs crates/core/src/resolver/chain.rs crates/core/src/resolver/alias.rs crates/core/src/resolver/extends.rs
git commit -m "fix(resolver): emit a diagnostic on self-referential extends instead of silently defaulting"
```

---

### Task 2: `OpaqueDetail` — make `PropType::Opaque` unconstructible except through a give-up constructor

**Files:**

- Modify: `crates/core/src/types/output.rs:1-8` (imports), `:140-207` (`PropType` enum — `Opaque` variant), `:275` (`raw_string`), `:294-397` (`to_tagged_value`), `:399-537` (`from_tagged_value`)
- Modify: `crates/core/src/toon.rs:143`, `:243` (test)
- Modify: `crates/core/src/known.rs:102-111,126-129,133-136,193-196,221-224,228-231` (construction sites), `:387-389,423-425,432-434,525-527,535-537` (test assertions — exact line numbers will shift slightly as edits land; match by content)
- Test: inline `#[cfg(test)]` module in `types/output.rs`

**Note on file placement:** the root-cause doc proposed a new `resolver/opaque.rs`, but `PropType` is defined in `crates/core/src/types/output.rs` and `crates/core/CLAUDE.md`'s module layout has `resolver/` depend on `types/`, never the reverse — putting `OpaqueDetail` in `resolver/` would make `types/output.rs` import from `resolver/`, an architecture violation. `OpaqueDetail` is defined in `types/output.rs` itself instead; `ResolveState`/`Diagnostic` (needed by `give_up`) already live in `types/`, so no new cross-module dependency is introduced.

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/types/output.rs, inside a new `#[cfg(test)] mod tests` block
// (add this module at the end of the file if one doesn't already exist there)

#[cfg(test)]
mod opaque_detail_tests {
    use super::*;
    use crate::types::global::ResolveState;

    #[test]
    fn give_up_pushes_the_diagnostic_and_builds_the_opaque_payload() {
        let mut state = ResolveState::default();
        let diagnostic = Diagnostic {
            severity: DiagnosticSeverity::Info,
            message: "gave up".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: DiagnosticCode::OpaqueType,
        };

        let pt = OpaqueDetail::give_up(&mut state, "SomeType", OpaqueReason::DepthExceeded, diagnostic);

        assert_eq!(state.diagnostics.len(), 1);
        assert_eq!(state.diagnostics[0].message, "gave up");
        let PropType::Opaque(detail) = &pt else { panic!("expected PropType::Opaque, got {:?}", pt) };
        assert_eq!(detail.raw(), "SomeType");
        assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
    }

    #[test]
    fn opaque_round_trips_through_the_tagged_json_wire_format() {
        let pt = OpaqueDetail::new("A<B> | C", OpaqueReason::UnsupportedExpression);
        let json = pt.to_tagged_value();
        let restored = PropType::from_tagged_value(&json).expect("should deserialize");
        assert_eq!(pt, restored);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core opaque_detail_tests -- --nocapture` Expected: FAIL to compile — `OpaqueDetail` doesn't exist yet.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/types/output.rs`, add the import and change the `Opaque` variant (replace the struct-style variant at line ~202):

```rust
use super::global::ResolveState; // add alongside the existing `use super::diagnostic::Diagnostic;`
```

```rust
    // ── Unresolvable — graceful degradation
    Opaque(OpaqueDetail),
```

Add the `OpaqueDetail` type right after the `PropType` enum closes (after line 207, before `impl PropType`):

```rust
/// The private payload of `PropType::Opaque`. Fields are unreachable from
/// outside this module — the only ways to build one are `give_up` (pushes
/// the diagnostic that explains the degradation, then builds the value) and
/// `new` (for the one documented exception: `known.rs`, which has no
/// diagnostics channel of its own and pushes its own diagnostic separately
/// via `known::push_known_opaque_diagnostic` — see that function's doc
/// comment). Read the payload back through `raw()`/`reason()`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpaqueDetail {
    raw: std::string::String,
    reason: OpaqueReason,
}

impl OpaqueDetail {
    pub(crate) fn new(raw: impl Into<std::string::String>, reason: OpaqueReason) -> PropType {
        PropType::Opaque(OpaqueDetail { raw: raw.into(), reason })
    }

    /// The sanctioned "give up and record why" constructor — pushes
    /// `diagnostic` onto `state` before building the value, so a resolver
    /// call site that reaches for this can't forget to explain the
    /// degradation the way the old bare `PropType::Opaque { .. }` literal let
    /// call sites do.
    pub(crate) fn give_up(
        state: &mut ResolveState,
        raw: impl Into<std::string::String>,
        reason: OpaqueReason,
        diagnostic: Diagnostic,
    ) -> PropType {
        state.diagnostics.push(diagnostic);
        Self::new(raw, reason)
    }

    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    pub(crate) fn reason(&self) -> &OpaqueReason {
        &self.reason
    }
}
```

Update `raw_string` (line 275):

```rust
            PropType::Opaque(detail) => detail.raw.clone(),
```

(same-module access — direct field access is fine inside `output.rs`.)

Update `to_tagged_value`'s `Opaque` arm (lines 366-395) — only the match pattern changes, body is unchanged since it's still inside `output.rs`:

```rust
            PropType::Opaque(OpaqueDetail { raw, reason }) => {
                let reason_val = match reason {
                    OpaqueReason::ConditionalType => serde_json::json!({"type": "conditionalType"}),
                    OpaqueReason::MappedType => serde_json::json!({"type": "mappedType"}),
                    OpaqueReason::ModuleAugmentation => {
                        serde_json::json!({"type": "moduleAugmentation"})
                    }
                    OpaqueReason::RuntimeDependent { function_name } => {
                        serde_json::json!({"type": "runtimeDependent", "functionName": function_name})
                    }
                    OpaqueReason::UnresolvableImport { specifier } => {
                        serde_json::json!({"type": "unresolvableImport", "specifier": specifier})
                    }
                    OpaqueReason::PandaCodegenMissing => {
                        serde_json::json!({"type": "pandaCodegenMissing"})
                    }
                    OpaqueReason::DepthExceeded => serde_json::json!({"type": "depthExceeded"}),
                    OpaqueReason::IndexedAccess { expression } => {
                        serde_json::json!({"type": "indexedAccess", "expression": expression})
                    }
                    OpaqueReason::TemplateLiteral { expression } => {
                        serde_json::json!({"type": "templateLiteral", "expression": expression})
                    }
                    OpaqueReason::MultiParamFunction => serde_json::json!({"type": "multiParamFunction"}),
                    OpaqueReason::UnsupportedExpression => {
                        serde_json::json!({"type": "unsupportedExpression"})
                    }
                };
                serde_json::json!({"kind": "opaque", "raw": raw, "reason": reason_val})
            }
```

Update `from_tagged_value`'s `"opaque"` arm (line ~529) and fallback arm (line ~531-534) — construction via `OpaqueDetail::new` since deserialization isn't a "give up" decision (the diagnostic already fired when the value was first produced):

```rust
            "opaque" => {
                let raw = v["raw"].as_str().unwrap_or("").to_owned();
                let reason = match v["reason"]["type"].as_str().unwrap_or("depthExceeded") {
                    "conditionalType" => OpaqueReason::ConditionalType,
                    "mappedType" => OpaqueReason::MappedType,
                    "moduleAugmentation" => OpaqueReason::ModuleAugmentation,
                    "runtimeDependent" => OpaqueReason::RuntimeDependent {
                        function_name: v["reason"]["functionName"].as_str().unwrap_or("").to_owned(),
                    },
                    "unresolvableImport" => OpaqueReason::UnresolvableImport {
                        specifier: v["reason"]["specifier"].as_str().unwrap_or("").to_owned(),
                    },
                    "pandaCodegenMissing" => OpaqueReason::PandaCodegenMissing,
                    "indexedAccess" => OpaqueReason::IndexedAccess {
                        expression: v["reason"]["expression"].as_str().unwrap_or("").to_owned(),
                    },
                    "templateLiteral" => OpaqueReason::TemplateLiteral {
                        expression: v["reason"]["expression"].as_str().unwrap_or("").to_owned(),
                    },
                    "multiParamFunction" => OpaqueReason::MultiParamFunction,
                    "unsupportedExpression" => OpaqueReason::UnsupportedExpression,
                    _ => OpaqueReason::DepthExceeded,
                };
                Ok(OpaqueDetail::new(raw, reason))
            }
            other => Ok(OpaqueDetail::new(format!("unknown PropType kind: {}", other), OpaqueReason::DepthExceeded)),
```

In `crates/core/src/toon.rs`:

- Line 143: `PropType::Opaque { raw, .. } => format!("opaque({raw})"),` → `PropType::Opaque(detail) => format!("opaque({})", detail.raw()),`
- Line 243 (test): `assert_eq!(format_type_compact(&PropType::Opaque { raw: "CustomType".into(), reason: crate::types::output::OpaqueReason::ConditionalType }), "opaque(CustomType)");` → `assert_eq!(format_type_compact(&crate::types::output::OpaqueDetail::new("CustomType", crate::types::output::OpaqueReason::ConditionalType)), "opaque(CustomType)");`

In `crates/core/src/known.rs`, convert every direct-construction site to `OpaqueDetail::new(...)` (these callers push their own diagnostic separately via `push_known_opaque_diagnostic` — see that function's doc comment, unchanged behavior):

- Lines 100-106:

```rust
                Some(KnownPatternResult::Type(PropType::Union(vec![
                    base.clone(),
                    OpaqueDetail::new("/* module augmentation */", OpaqueReason::ModuleAugmentation),
                ])))
```

- Lines 108-111: `Some(KnownPatternResult::Type(OpaqueDetail::new("OverridableStringUnion", OpaqueReason::ModuleAugmentation)))`
- Lines 126-129: `"ThemingProps" => Some(KnownPatternResult::Type(OpaqueDetail::new("ThemingProps", OpaqueReason::RuntimeDependent { function_name: "chakra".into() }))),`
- Lines 133-136: `"StylesApiProps" => Some(KnownPatternResult::Type(OpaqueDetail::new("StylesApiProps", OpaqueReason::RuntimeDependent { function_name: "createStyles".into() }))),`
- Lines 193-196: `return Some(KnownPatternResult::Type(OpaqueDetail::new(format!("VariantProps<typeof {}>", name_str), OpaqueReason::RuntimeDependent { function_name: "cva".into() })));`
- Lines 221-224: `Some(KnownPatternResult::Type(OpaqueDetail::new(format!("VariantProps<typeof {}>", name_str), OpaqueReason::RuntimeDependent { function_name: "cva".into() })))`
- Lines 228-231: `_ => Some(KnownPatternResult::Type(OpaqueDetail::new("VariantProps<...>", OpaqueReason::RuntimeDependent { function_name: "cva".into() }))),`

And its test assertions (`PropType::Opaque { reason: X, .. }` → guard on `.reason()`):

```rust
        assert!(matches!(
            result,
            Some(KnownPatternResult::Type(PropType::Opaque(ref d))) if d.reason() == &OpaqueReason::ModuleAugmentation
        ));
```

(same substitution — `d.reason() == &OpaqueReason::RuntimeDependent { .. }` doesn't compile as a `==`, so use `matches!(d.reason(), OpaqueReason::RuntimeDependent { .. })`) at every occurrence: `test_overridable_string_union_no_args`, `test_theming_props_is_runtime_dependent`, `test_styles_api_props_is_runtime_dependent`, `test_recipe_variant_props_no_args_is_opaque`, `test_variant_props_named_not_in_global_is_opaque`. Concretely, e.g.:

```rust
    #[test]
    fn test_theming_props_is_runtime_dependent() {
        let result = resolve_known("ThemingProps", &[], &GlobalSourceData::default(), &FxHashMap::default());
        assert!(matches!(
            result,
            Some(KnownPatternResult::Type(PropType::Opaque(ref d)))
                if matches!(d.reason(), OpaqueReason::RuntimeDependent { .. })
        ));
    }
```

Apply the same shape to the other four assertions listed above (swap the reason pattern per test: `OpaqueReason::ModuleAugmentation` for `test_overridable_string_union_no_args`, `OpaqueReason::RuntimeDependent { .. }` for the rest).

Finally, resolve every other now-non-compiling `PropType::Opaque { .. }` pattern/construction across the crate that Task 2 doesn't otherwise cover, so the crate builds (these are handled for real in Tasks 3-8, but the crate must compile at the end of _this_ task too — replace them mechanically with the equivalent tuple-variant/accessor form for now; Tasks 3-8 then route the resolver-internal ones through `give_up` for real):

- `crates/core/src/resolver/collected.rs:25,80,104,108,151` → `OpaqueDetail::new(...)` (temporary; Task 3 upgrades to `give_up`)
- `crates/core/src/resolver/func.rs:61` → `OpaqueDetail::new(raw, OpaqueReason::MultiParamFunction)` (temporary; Task 4 upgrades to `give_up`)
- `crates/core/src/resolver/primitives.rs:181` → `OpaqueDetail::new(expression.clone(), OpaqueReason::IndexedAccess { expression })` (temporary; Task 5 upgrades to `give_up`)
- `crates/core/src/resolver/template.rs:39` → `OpaqueDetail::new(raw.clone(), OpaqueReason::TemplateLiteral { expression: raw })` (temporary; Task 6 upgrades to `give_up`)
- `crates/core/src/resolver/named.rs:29,75` → `OpaqueDetail::new(...)` at 29 (temporary; Task 3 upgrades), pattern-match fix at 75: `if let PropType::Opaque(detail) = &pt { push_known_opaque_diagnostic(&mut state.diagnostics, detail.reason(), name.as_str(), consuming_file); }`
- `crates/core/src/resolver/chain.rs:123` — same pattern-match fix: `if let PropType::Opaque(detail) = &pt { push_known_opaque_diagnostic(&mut state.diagnostics, detail.reason(), type_name_bare, consuming_file); }`
- `crates/core/src/resolver/mod.rs` test assertions at lines 1924, 1972, 1989, 2028, 2132, 2180, 2194, 2205, 2218 — e.g. `matches!(result, PropType::Opaque { reason: OpaqueReason::TemplateLiteral { .. }, .. })` → `matches!(&result, PropType::Opaque(d) if matches!(d.reason(), OpaqueReason::TemplateLiteral { .. }))`, and the bare `matches!(result, PropType::Opaque { .. })` occurrences → `matches!(result, PropType::Opaque(_))`.

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core opaque_detail_tests -- --nocapture` then `cargo test -p oxc-react-docgen-core` and `cargo clippy -p oxc-react-docgen-core -- -D warnings` Expected: PASS — new tests pass, full suite compiles and passes (no behavior change yet, only the constructor surface).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/output.rs crates/core/src/toon.rs crates/core/src/known.rs crates/core/src/resolver/collected.rs crates/core/src/resolver/func.rs crates/core/src/resolver/primitives.rs crates/core/src/resolver/template.rs crates/core/src/resolver/named.rs crates/core/src/resolver/chain.rs crates/core/src/resolver/mod.rs
git commit -m "refactor(resolver): make PropType::Opaque unconstructible outside OpaqueDetail::new/give_up"
```

---

### Task 3: Route `collected.rs`'s and `named.rs`'s depth-exceeded Opaque sites through `give_up`

**Files:**

- Modify: `crates/core/src/resolver/collected.rs:22-26,78-81,101-109,150-152,157-174`
- Modify: `crates/core/src/resolver/named.rs:27-30`
- Test: inline in `crates/core/src/resolver/mod.rs`'s existing `mod tests` — `test_conditional_type_opaque_emits_diagnostic`, `test_mapped_type_opaque_emits_diagnostic`, `test_keyof_opaque_emits_diagnostic`, `test_complex_raw_fallback_opaque_emits_diagnostic` (Test 21 block, already present) already assert a diagnostic is emitted for every one of these sites — this task is a pure refactor behind that existing net, plus one new test for the previously-untested depth-exceeded path.

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/mod.rs, inside `mod tests`, near Test 21

#[test]
fn test_depth_exceeded_opaque_carries_the_max_depth_diagnostic() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let result = super::collected::resolve_collected_type(
        &CollectedType::String,
        Utf8Path::new("/test/button.tsx"),
        &ctx,
        &mut state,
        MAX_DEPTH + 1,
    );
    let PropType::Opaque(detail) = &result else { panic!("expected Opaque, got {:?}", result) };
    assert_eq!(detail.reason(), &OpaqueReason::DepthExceeded);
    assert!(
        state.diagnostics.iter().any(|d| d.code == DiagnosticCode::MaxDepthExceeded),
        "expected a MaxDepthExceeded diagnostic, got {:?}",
        state.diagnostics
    );
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_depth_exceeded_opaque_carries_the_max_depth_diagnostic -- --nocapture` Expected: This one actually already passes today (the depth-exceeded branch already pushes `max_depth_diagnostic` before constructing the literal) — confirm it passes _before_ touching the code, so it's a locked-in regression guard rather than a new-behavior test; the real "fails first" signal for this task is Step 3 not compiling until `push_opaque_diagnostic` is rewired to return a `Diagnostic` for `give_up` to consume — run `cargo build -p oxc-react-docgen-core` after making the diagnostic-builder change below and confirm the four `Test 21` assertions and this new test all still pass once the give_up wiring lands.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/resolver/collected.rs`, change `push_opaque_diagnostic` from a side-effecting `fn(&mut ResolveState, ...)` into a pure `Diagnostic` builder, and route every Opaque site through `OpaqueDetail::give_up`:

```rust
pub fn resolve_collected_type(
    ct: &CollectedType,
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    state: &mut ResolveState,
    depth: u8,
) -> PropType {
    if depth > MAX_DEPTH {
        let diag = super::max_depth_diagnostic(&format!("type '{}'", ct.to_raw_string()), consuming_file);
        return OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::DepthExceeded, diag);
    }

    match ct {
        // ... unchanged primitive/literal/composite arms ...

        CollectedType::KeyOf(_) => {
            let diag = opaque_diagnostic("a standalone 'keyof'", ct, consuming_file);
            OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::MappedType, diag)
        }

        CollectedType::AtFile { file, inner } => resolve_collected_type(inner, file, ctx, state, depth),

        CollectedType::IndexedAccess { obj, key } => {
            resolve_indexed_access(obj, key, consuming_file, ctx, state, depth)
        }

        CollectedType::TemplateLiteral(parts) => resolve_template_literal(parts, consuming_file, ctx, state, depth),

        CollectedType::Function { params, param_names, return_type } => {
            resolve_function_type(params, param_names, return_type, consuming_file, ctx, state, depth)
        }

        CollectedType::Conditional { .. } => {
            let diag = opaque_diagnostic("a conditional type", ct, consuming_file);
            OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::ConditionalType, diag)
        }
        CollectedType::Mapped { .. } => {
            let diag = opaque_diagnostic("a mapped type", ct, consuming_file);
            OpaqueDetail::give_up(state, ct.to_raw_string(), OpaqueReason::MappedType, diag)
        }

        CollectedType::Raw(s) => {
            let trimmed = s.trim();
            if let Some(name) = trimmed.strip_prefix("typeof ") {
                return PropType::Named { name: name.trim().into(), args: vec![] };
            }
            if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
                return PropType::StringLiteral(trimmed[1..trimmed.len() - 1].to_owned());
            }
            if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
                return PropType::StringLiteral(trimmed[1..trimmed.len() - 1].to_owned());
            }
            match trimmed {
                "string" => return PropType::String,
                "number" => return PropType::Number,
                "boolean" => return PropType::Boolean,
                "null" => return PropType::Null,
                "undefined" => return PropType::Undefined,
                "never" => return PropType::Never,
                "void" => return PropType::Void,
                "any" => return PropType::Any,
                "unknown" => return PropType::Unknown,
                _ => {}
            }
            if !trimmed.is_empty()
                && !trimmed.contains(' ')
                && !trimmed.contains('|')
                && !trimmed.contains('&')
                && !trimmed.contains('<')
            {
                PropType::Named { name: trimmed.into(), args: vec![] }
            } else {
                let diag = opaque_diagnostic("an unparsable raw type expression", ct, consuming_file);
                OpaqueDetail::give_up(state, s.clone(), OpaqueReason::UnsupportedExpression, diag)
            }
        }
    }
}

/// Build the diagnostic explaining why a `CollectedType` degrades to
/// `PropType::Opaque` — expanding it needs the TypeScript type checker (or,
/// for the Raw fallback, needs the extractor to understand a syntax shape it
/// doesn't yet). Pure builder — the caller passes the result to
/// `OpaqueDetail::give_up`, which pushes it.
fn opaque_diagnostic(what: &str, ct: &CollectedType, file: &Utf8Path) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!(
            "'{}' is {} and can't be statically resolved — it will appear as opaque",
            ct.to_raw_string(),
            what
        ),
        file: Some(file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
    }
}
```

In `crates/core/src/resolver/named.rs`, replace lines 27-30:

```rust
    if depth > MAX_DEPTH {
        let diag = super::max_depth_diagnostic(&format!("named type '{}'", name), consuming_file);
        return OpaqueDetail::give_up(state, name.to_string(), OpaqueReason::DepthExceeded, diag);
    }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_depth_exceeded_opaque_carries_the_max_depth_diagnostic test_conditional_type_opaque_emits_diagnostic test_mapped_type_opaque_emits_diagnostic test_keyof_opaque_emits_diagnostic test_complex_raw_fallback_opaque_emits_diagnostic -- --nocapture` then `cargo test -p oxc-react-docgen-core` Expected: PASS, full suite green.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/collected.rs crates/core/src/resolver/named.rs crates/core/src/resolver/mod.rs
git commit -m "refactor(resolver): route collected.rs/named.rs Opaque sites through OpaqueDetail::give_up"
```

---

### Task 4: Fix `func.rs`'s multi-param function type — missing diagnostic

**Files:**

- Modify: `crates/core/src/resolver/func.rs:54-62`
- Test: inline in `crates/core/src/resolver/mod.rs`'s existing `mod tests` (next to Test 16, the existing multi-param-function test that only checks the `PropType`, never the diagnostic)

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/mod.rs, inside `mod tests`, right after the
// existing multi-param-function `PropType` test (Test 16)

#[test]
fn test_multi_param_function_opaque_emits_diagnostic() {
    // Regression test for: func.rs's multi-param function degrade
    // constructed PropType::Opaque directly with no diagnostic at all — the
    // only Opaque-producing path in the resolver that gave up silently.
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    let ct = CollectedType::Function {
        params: vec![CollectedType::String, CollectedType::Number],
        param_names: vec![Some("a".into()), Some("b".into())],
        return_type: Box::new(CollectedType::Void),
    };
    let result = super::collected::resolve_collected_type(&ct, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);
    assert!(
        matches!(&result, PropType::Opaque(d) if matches!(d.reason(), OpaqueReason::MultiParamFunction)),
        "Expected MultiParamFunction opaque, got {:?}",
        result
    );
    assert!(
        !state.diagnostics.is_empty(),
        "expected a diagnostic for a multi-param function type, got none"
    );
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_multi_param_function_opaque_emits_diagnostic -- --nocapture` Expected: FAIL — `state.diagnostics` is empty, since `func.rs:61` currently constructs `PropType::Opaque` with no diagnostic push anywhere on this path.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/resolver/func.rs`, replace the final block of `resolve_function_type` (lines 54-61):

```rust
    // Multi-param function — describe as opaque.
    let param_strs: Vec<String> = params.iter().map(|p| p.to_raw_string()).collect();
    let raw = format!("({}) => {}", param_strs.join(", "), return_type.to_raw_string());

    // Resolve the return type to see if it's ReactNode.
    let _ = resolve_collected_type(return_type, consuming_file, ctx, state, depth + 1);

    let diagnostic = Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!(
            "'{}' is a multi-parameter function type and can't be statically resolved — it will appear as opaque",
            raw
        ),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: None,
        code: DiagnosticCode::OpaqueType,
    };
    OpaqueDetail::give_up(state, raw, OpaqueReason::MultiParamFunction, diagnostic)
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_multi_param_function_opaque_emits_diagnostic -- --nocapture` then `cargo test -p oxc-react-docgen-core` Expected: PASS, full suite green.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/func.rs crates/core/src/resolver/mod.rs
git commit -m "fix(resolver): emit a diagnostic for multi-param function types instead of degrading silently"
```

---

### Task 5: Route `primitives.rs`'s indexed-access Opaque site through `give_up`

**Files:**

- Modify: `crates/core/src/resolver/primitives.rs:171-182`
- Test: inline `#[cfg(test)]` module in `crates/core/src/resolver/primitives.rs` (new — this file currently has no test module of its own; `resolver/mod.rs` covers it indirectly)

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/primitives.rs, appended at the end of the file

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::pipeline::PipelineOptions;

    #[test]
    fn indexed_access_on_an_unresolvable_object_gives_up_with_a_diagnostic() {
        let ctx = ResolutionContext::new(Arc::new(GlobalSourceData::default()), &PipelineOptions::default());
        let mut state = ResolveState::default();
        let obj = CollectedType::Named { name: "TotallyUnknownType".into(), args: vec![] };
        let key = CollectedType::StringLiteral("whatever".into());

        let result = resolve_indexed_access(&obj, &key, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);

        let PropType::Opaque(detail) = &result else { panic!("expected Opaque, got {:?}", result) };
        assert!(matches!(detail.reason(), OpaqueReason::IndexedAccess { .. }));
        assert!(
            state.diagnostics.iter().any(|d| d.code == DiagnosticCode::IndexedAccessOpaque),
            "expected an IndexedAccessOpaque diagnostic, got {:?}",
            state.diagnostics
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core indexed_access_on_an_unresolvable_object_gives_up_with_a_diagnostic -- --nocapture` Expected: PASS already today (the existing code already pushes the diagnostic before constructing the literal) — the point of this task is the refactor, not new behavior; run `cargo build -p oxc-react-docgen-core` first to confirm it currently compiles against the pre-Task-2 struct-literal form is gone (Task 2 already converted this site to `OpaqueDetail::new` without the diagnostic tie), then apply Step 3 and re-run this test to lock the diagnostic back in through `give_up`.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/resolver/primitives.rs`, replace the final block of `resolve_indexed_access` (lines 171-182):

```rust
    let expression = format!("{}[{}]", obj.to_raw_string(), key.to_raw_string());
    let diagnostic = Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("Indexed access type '{}' could not be statically resolved", expression),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some("Enable typescript-go to resolve indexed access types.".into()),
        code: DiagnosticCode::IndexedAccessOpaque,
    };
    OpaqueDetail::give_up(state, expression.clone(), OpaqueReason::IndexedAccess { expression }, diagnostic)
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core indexed_access_on_an_unresolvable_object_gives_up_with_a_diagnostic -- --nocapture` then `cargo test -p oxc-react-docgen-core` Expected: PASS, full suite green.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/primitives.rs
git commit -m "refactor(resolver): route primitives.rs's indexed-access Opaque site through OpaqueDetail::give_up"
```

---

### Task 6: Route `template.rs`'s template-literal Opaque site through `give_up`

**Files:**

- Modify: `crates/core/src/resolver/template.rs:17-40`
- Test: inline in `crates/core/src/resolver/mod.rs`'s existing `mod tests` — `test_template_literal_opaque_on_unknown_type` (Test 12) already exists; update its `matches!` to the new tuple form and add a diagnostic assertion.

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/mod.rs, replace the body of the existing
// `test_template_literal_opaque_on_unknown_type` (Test 12)

#[test]
fn test_template_literal_opaque_on_unknown_type() {
    let ctx = empty_ctx();
    let mut state = ResolveState::default();
    // `compact-${UnknownSize}` — UnknownSize is not in global, so opaque.
    let ct = CollectedType::TemplateLiteral(vec![
        CollectedType::StringLiteral("compact-".into()),
        CollectedType::Named { name: "UnknownSize".into(), args: vec![] },
    ]);
    let result =
        super::template::resolve_template_literal(match &ct {
            CollectedType::TemplateLiteral(parts) => parts,
            _ => unreachable!(),
        }, Utf8Path::new("/test/button.tsx"), &ctx, &mut state, 0);
    assert!(
        matches!(&result, PropType::Opaque(d) if matches!(d.reason(), OpaqueReason::TemplateLiteral { .. })),
        "Expected Opaque TemplateLiteral, got {:?}",
        result
    );
    assert!(
        state.diagnostics.iter().any(|d| d.code == DiagnosticCode::TemplateLiteralOpaque),
        "expected a TemplateLiteralOpaque diagnostic, got {:?}",
        state.diagnostics
    );
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_template_literal_opaque_on_unknown_type -- --nocapture` Expected: FAIL to compile — `resolve_template_literal` isn't `pub(super)`-reachable in that exact shape from the test yet if the call signature changed; more importantly this locks in the diagnostic-code assertion which the pre-refactor `OpaqueDetail::new`-only version (from Task 2) doesn't satisfy through `give_up` — confirm the diagnostic assertion specifically fails before Step 3 by temporarily checking `state.diagnostics` is non-empty regardless (it already will be, since Task 2 didn't remove the manual push at template.rs's call site — the actual gap here is purely the tuple-variant match syntax). Treat this test primarily as the compile-and-shape regression gate for the refactor.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/resolver/template.rs`, replace lines 29-39:

```rust
    let raw = CollectedType::TemplateLiteral(parts.to_vec()).to_raw_string();
    let diagnostic = Diagnostic {
        severity: DiagnosticSeverity::Info,
        message: format!("Template literal type '{}' could not be statically expanded", raw),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some("Enable typescript-go or add explicit string literal union for template literal types.".into()),
        code: DiagnosticCode::TemplateLiteralOpaque,
    };
    OpaqueDetail::give_up(state, raw.clone(), OpaqueReason::TemplateLiteral { expression: raw }, diagnostic)
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_template_literal_opaque_on_unknown_type -- --nocapture` then `cargo test -p oxc-react-docgen-core` Expected: PASS, full suite green.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/template.rs crates/core/src/resolver/mod.rs
git commit -m "refactor(resolver): route template.rs's Opaque site through OpaqueDetail::give_up"
```

---

### Task 7: Extract `resolve_source_defined_or_known` — shared source-before-known precedence

**Files:**

- Create: `crates/core/src/resolver/precedence.rs`
- Modify: `crates/core/src/resolver/mod.rs:26-37` (add `mod precedence;`)
- Modify: `crates/core/src/resolver/named.rs:38-90` (steps 2-5 rewired through the shared function — behavior-preserving, `named.rs` already has the correct order)
- Test: inline `#[cfg(test)]` module in `crates/core/src/resolver/precedence.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/precedence.rs

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use camino::Utf8PathBuf;
    use std::collections::BTreeMap;

    use super::*;
    use crate::pipeline::PipelineOptions;

    #[test]
    fn source_defined_interface_wins_over_a_known_pattern_shortcut() {
        // A project that declares its own `interface SxProps` must resolve to
        // that interface, never to the hardcoded MUI SxProps opaque shortcut.
        let file_path = Utf8PathBuf::from("/test/theme.ts");
        let scoped_key = format!("{}:SxProps", file_path);

        let mut global = GlobalSourceData::default();
        global.interfaces.insert(
            scoped_key.clone(),
            CollectedInterface {
                scoped_key: scoped_key.clone(),
                name: "SxProps".into(),
                file_path: file_path.clone(),
                props: vec![],
                extends: vec![],
                description: String::new(),
                tags: BTreeMap::new(),
            },
        );

        let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
        let mut state = ResolveState::default();

        let (_canonical_file, _canonical_name, matched) =
            resolve_source_defined_or_known("SxProps", &[], &file_path, &ctx, &mut state);

        assert!(
            matches!(matched, Some(SourceOrKnownMatch::Interface(_))),
            "expected the project's own SxProps interface to win, got {:?}",
            match &matched {
                Some(SourceOrKnownMatch::Interface(_)) => "Interface",
                Some(SourceOrKnownMatch::TypeAlias { .. }) => "TypeAlias",
                Some(SourceOrKnownMatch::Known(_)) => "Known",
                None => "None",
            }
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core source_defined_interface_wins_over_a_known_pattern_shortcut -- --nocapture` Expected: FAIL to compile — `resolve_source_defined_or_known`/`SourceOrKnownMatch` don't exist yet.

- [ ] **Step 3: Write minimal implementation**

Create `crates/core/src/resolver/precedence.rs`:

```rust
//! Shared "try the project's own source before falling back to known
//! library-pattern shortcuts" resolution order.
//!
//! `named.rs` documented this order as intentional (source-defined types
//! like `ThemingProps`/`StylesApiProps` must never be silently replaced by
//! an opaque known-pattern shortcut). `chain.rs`'s `extends`-clause path
//! independently reimplemented the same sequence and got it backwards —
//! checking known patterns first. This is now the *only* place either call
//! site may implement that order, so they can't drift apart again.

use camino::{Utf8Path, Utf8PathBuf};

use crate::known::{resolve_known, KnownPatternResult};
use crate::types::*;

use super::import::{lookup_interface, lookup_type_alias, resolve_to_canonical};
use super::ResolutionContext;

/// What `resolve_source_defined_or_known` found, in the order it checked.
pub(super) enum SourceOrKnownMatch<'g> {
    /// A type alias declared in the project's own source (or an already-merged
    /// ambient/library `.d.ts`). `matched_key` is the exact key it was found
    /// under (bare or `React.`-qualified) — callers need it to also look up
    /// `type_alias_params` under the same key.
    TypeAlias { matched_key: String, alias: CollectedTypeAlias },
    /// An interface declared in the project's own source.
    Interface(&'g CollectedInterface),
    /// No source declaration found — a recognized library pattern instead.
    Known(KnownPatternResult),
}

/// Resolve `name` to canonical `(file, name)`, then try — in this fixed
/// order — a type alias, an interface, and only then a known-pattern
/// shortcut. Returns the canonical `(file, name)` pair (callers need it
/// regardless of outcome, e.g. for an "unresolvable" diagnostic when nothing
/// matched) alongside whichever of the three matched, if any.
pub(super) fn resolve_source_defined_or_known<'g>(
    name: &str,
    resolved_args: &[PropType],
    consuming_file: &Utf8Path,
    ctx: &'g ResolutionContext,
    state: &mut ResolveState,
) -> (Utf8PathBuf, String, Option<SourceOrKnownMatch<'g>>) {
    let (canonical_file, canonical_name) = resolve_to_canonical(name, consuming_file, ctx, &mut state.diagnostics)
        .unwrap_or_else(|| (consuming_file.to_owned(), name.to_owned()));

    if let Some((matched_key, alias)) = lookup_type_alias(&ctx.global, canonical_file.as_str(), &canonical_name) {
        return (
            canonical_file,
            canonical_name,
            Some(SourceOrKnownMatch::TypeAlias { matched_key, alias: alias.clone() }),
        );
    }

    if let Some(iface) = lookup_interface(&ctx.global, canonical_file.as_str(), &canonical_name) {
        return (canonical_file, canonical_name, Some(SourceOrKnownMatch::Interface(iface)));
    }

    if let Some(result) = resolve_known(name, resolved_args, &ctx.global, &ctx.enum_bare_index) {
        return (canonical_file, canonical_name, Some(SourceOrKnownMatch::Known(result)));
    }

    (canonical_file, canonical_name, None)
}
```

In `crates/core/src/resolver/mod.rs`, add the module declaration alongside the others (line ~34):

```rust
mod named;
mod precedence;
mod primitives;
```

In `crates/core/src/resolver/named.rs`, replace steps 2-5 (lines 38-90) to call the shared function — same order, same behavior, one call site instead of a hand-rolled sequence:

```rust
    // Resolve type arguments eagerly — needed for both source lookups and known patterns.
    let resolved_args: Vec<PropType> =
        args.iter().map(|a| resolve_collected_type(a, consuming_file, ctx, state, depth + 1)).collect();

    // ── 2-5. Try the project's own source before a known-pattern shortcut ────
    // See `resolver::precedence` — the shared, single-source-of-truth order.
    let (canonical_file, canonical_name, matched) =
        super::precedence::resolve_source_defined_or_known(name.as_str(), &resolved_args, consuming_file, ctx, state);

    match matched {
        Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias }) => {
            if let Some(params) = ctx.global.type_alias_params.get(&matched_key) {
                state.in_scope_type_params.extend(params.iter().cloned());
            }
            return resolve_type_alias_type(&alias, ctx, state, depth);
        }
        Some(super::precedence::SourceOrKnownMatch::Interface(_)) => {
            return PropType::Named { name: name.clone(), args: resolved_args };
        }
        Some(super::precedence::SourceOrKnownMatch::Known(result)) => {
            return match result {
                KnownPatternResult::Type(pt) => {
                    if let PropType::Opaque(detail) = &pt {
                        push_known_opaque_diagnostic(&mut state.diagnostics, detail.reason(), name.as_str(), consuming_file);
                    }
                    pt
                }
                KnownPatternResult::Alias { name: alias_name } => {
                    let alias_ct = CollectedType::Named { name: alias_name.as_str().into(), args: vec![] };
                    resolve_collected_type(&alias_ct, consuming_file, ctx, state, depth + 1)
                }
                KnownPatternResult::Props(_) => PropType::Named { name: name.clone(), args: resolved_args },
            };
        }
        None => {}
    }
```

(The unchanged tail — steps 6, 6.5, 6.7, 7 — stays as-is; it already uses `canonical_file`/`canonical_name`, which are now sourced from the shared function's return instead of a local `resolve_to_canonical` call.)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core source_defined_interface_wins_over_a_known_pattern_shortcut -- --nocapture` then `cargo test -p oxc-react-docgen-core` Expected: PASS, full suite green (including all of `named.rs`'s existing coverage — behavior is unchanged since `named.rs` already had this order).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/precedence.rs crates/core/src/resolver/mod.rs crates/core/src/resolver/named.rs
git commit -m "refactor(resolver): extract shared source-before-known-pattern precedence into resolver::precedence"
```

---

### Task 8: Fix P0-1 — `chain.rs`'s extends-clause path checked known patterns before source

**Files:**

- Modify: `crates/core/src/resolver/chain.rs:20-178` (`resolve_props_chain`)
- Test: inline in `crates/core/src/resolver/mod.rs`'s existing `mod tests`

- [ ] **Step 1: Write the failing test**

```rust
// crates/core/src/resolver/mod.rs, inside `mod tests`

// ── Test: P0-1 regression — a project-defined SxProps extended via
// `extends` must not be silently replaced by the hardcoded MUI shortcut ────
// chain.rs's extends-clause path (reached via ExtendsRef::SameFile /
// ExtendsRef::Imported) independently reimplemented named.rs's "source
// before known-pattern" order and got it backwards — checking the known
// SxProps shortcut before ever looking at the project's own `interface
// SxProps`. This is the confirmed, demonstrable bug from the original audit.

#[test]
fn test_extends_clause_prefers_project_defined_sx_props_over_known_shortcut() {
    let file_path = Utf8PathBuf::from("/test/theme-button.tsx");

    let mut global = GlobalSourceData::default();
    global.interfaces.insert(
        format!("{}:SxProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:SxProps", file_path),
            name: "SxProps".into(),
            file_path: file_path.clone(),
            props: vec![RawProp {
                name: "customSx".into(),
                collected_type: CollectedType::String,
                required: false,
                description: String::new(),
                tags: BTreeMap::new(),
                span_start: 0,
                span_end: 0,
            }],
            extends: vec![],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );
    global.interfaces.insert(
        format!("{}:ThemeButtonProps", file_path),
        CollectedInterface {
            scoped_key: format!("{}:ThemeButtonProps", file_path),
            name: "ThemeButtonProps".into(),
            file_path: file_path.clone(),
            props: vec![],
            extends: vec![ExtendsRef::SameFile { name: "SxProps".into(), type_args: vec![] }],
            description: String::new(),
            tags: BTreeMap::new(),
        },
    );

    let ctx = ResolutionContext::new(Arc::new(global), &PipelineOptions::default());
    let mapping = ComponentMapping {
        component_name: "ThemeButton".into(),
        props_type_name: "ThemeButtonProps".into(),
        props_type_args: vec![],
        file_path: file_path.clone(),
        description: String::new(),
        tags: BTreeMap::new(),
        span_start: 0,
        span_end: 0,
        param_defaults: FxHashMap::default(),
    };

    let (entry, _diagnostics) = resolve_component(&mapping, &ctx);

    assert!(
        entry.props.contains_key("customSx"),
        "Expected the project's own SxProps interface's 'customSx' field to be inherited, \
         got props {:?} — this means the hardcoded MUI SxProps known-pattern shortcut won \
         instead of the project's own source-defined interface",
        entry.props.keys().collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_extends_clause_prefers_project_defined_sx_props_over_known_shortcut -- --nocapture` Expected: FAIL — `entry.props` does not contain `customSx`; `resolve_props_chain`'s Step 2 (known-pattern check) matches `"SxProps"` via `resolve_known` before Step 3-5 (canonical/alias/interface) ever run, so the project's own interface is never even looked up.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/resolver/chain.rs`, replace Step 2 through Step 6 (lines 92-178) so source resolution runs before the known-pattern check, via `resolve_source_defined_or_known`:

```rust
    // ── Step 2: Try the project's own source before a known-pattern shortcut ─
    // See `resolver::precedence` — the shared, single-source-of-truth order
    // `named.rs` already used correctly; this path used to reimplement the
    // sequence independently and check known patterns first, silently
    // replacing project-defined types (e.g. a project's own `interface
    // SxProps`) with the hardcoded library shortcut. Fixed: P0-1.
    let resolved_args: Vec<PropType> = type_args
        .iter()
        .map(|a| {
            let ct = CollectedType::Raw(a.clone());
            resolve_collected_type(&ct, consuming_file, ctx, state, depth + 1)
        })
        .collect();

    let (canonical_file, canonical_name, matched) =
        super::precedence::resolve_source_defined_or_known(type_name_bare, &resolved_args, consuming_file, ctx, state);

    match matched {
        Some(super::precedence::SourceOrKnownMatch::TypeAlias { matched_key, alias }) => {
            let alias = super::substitute::apply_generic_args(alias, &matched_key, type_args, consuming_file, ctx);
            return resolve_type_alias_chain(&alias, consuming_file, mapping, ctx, state, depth);
        }
        Some(super::precedence::SourceOrKnownMatch::Interface(iface)) => {
            return resolve_interface_chain(iface, type_args, consuming_file, mapping, ctx, state, depth);
        }
        Some(super::precedence::SourceOrKnownMatch::Known(result)) => {
            return match result {
                KnownPatternResult::Props(props) => ResolvedChain { props, ..ResolvedChain::empty() },
                KnownPatternResult::Type(PropType::HtmlAttributes { element, omitted }) => {
                    let layer = InheritedLayer {
                        type_name: type_name.to_owned(),
                        file_name: resolve_react_types_file(consuming_file, ctx),
                        omitted,
                        html_element: Some(element),
                        total_props: 0,
                    };
                    ResolvedChain { inheritance: vec![layer], ..ResolvedChain::empty() }
                }
                KnownPatternResult::Type(pt) => {
                    if let PropType::Opaque(detail) = &pt {
                        push_known_opaque_diagnostic(&mut state.diagnostics, detail.reason(), type_name_bare, consuming_file);
                    }
                    ResolvedChain::empty_with_compose(pt.raw_string())
                }
                KnownPatternResult::Alias { name } => {
                    resolve_props_chain(&name, &[], consuming_file, mapping, ctx, state, depth + 1)
                }
            };
        }
        None => {}
    }

    // ── Step 2.5: React builtin check (after source and known patterns) ──────
    // Terminal React types (ReactNode, Ref, FC, etc.) that survived both are not
    // prop providers — add to composes and stop.
    if react_types::is_react_builtin(type_name_bare, &ctx.extra_builtins) {
        return ResolvedChain::empty_with_compose(type_name.to_owned());
    }

    // ── Step 6: Unresolvable ──────────────────────────────────────────────────
    // Import resolution may have redirected `type_name` to a different name/file
    // (re-exports, barrel files) — surface that resolved location when it differs,
    // since "Cannot resolve X in file A" is confusing if X actually lives in file B.
    let location_note = super::unresolved_location_note(type_name, consuming_file, &canonical_file, &canonical_name);
    state.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        message: format!("Cannot resolve type '{}' in '{}'{}", type_name, consuming_file, location_note),
        file: Some(consuming_file.to_string()),
        line: None,
        column: None,
        help: Some(
            "Type may be in an unresolvable cross-package location. Check that the package is installed.".into(),
        ),
        code: DiagnosticCode::UnresolvableImport,
    });
    ResolvedChain::empty_with_compose(type_name.to_owned())
}
```

Steps 0.5 and 1 (lines 46-90, the inline-utility-in-extends-position and the ts-utility-type silent no-op) stay exactly as they are, unchanged, above this block — they still run first, before the new Step 2.

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_extends_clause_prefers_project_defined_sx_props_over_known_shortcut -- --nocapture` then `cargo test -p oxc-react-docgen-core` and `cargo clippy -p oxc-react-docgen-core -- -D warnings` Expected: PASS, full suite green — including `test_sx_props_known_pattern` (Test 4, still passes since a project with no source-defined `SxProps` still falls through to the known-pattern shortcut) and `test_known_opaque_result_emits_diagnostic_at_chain_level` (Test 20, still passes since `ThemingProps` has no source declaration in that fixture either).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/chain.rs
git commit -m "fix(resolver): check source-defined types before known-pattern shortcuts in extends chain (P0-1)"
```

---

## Part C: Extractor diagnostic channel + depth-tracking

I have all the file details needed. Now writing the task group.

### Task 1: `DiagnosticCode::SkippedCandidate` variant

**Files:**

- Modify: `crates/core/src/types/diagnostic.rs:57-78`
- Test: inline `#[cfg(test)]` module in the same file

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn skipped_candidate_code_serializes_screaming_snake_case() {
        let json = serde_json::to_string(&DiagnosticCode::SkippedCandidate).unwrap();
        assert_eq!(json, "\"SKIPPED_CANDIDATE\"");
    }
```

Add this inside the existing `mod tests` block in `crates/core/src/types/diagnostic.rs` (after `io_read_error_reports_the_path_and_underlying_error`). Add `use serde_json;` is unnecessary since `serde_json::to_string` can be called via the fully qualified path already available as a dev-dependency (used elsewhere in the crate's tests — confirm `serde_json` is already a dependency before writing this; it's the standard round-trip check pattern for this enum).

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core skipped_candidate_code_serializes_screaming_snake_case -- --nocapture` Expected: FAIL with `no variant named SkippedCandidate found for enum DiagnosticCode` (compile error)

- [ ] **Step 3: Write minimal implementation**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticCode {
    UnresolvableImport,
    OpaqueType,
    MaxDepthExceeded,
    Unknown,
    /// JSDoc @default conflicts with code default value — code value was used.
    JsDocDefaultMismatch,
    /// Default value is a runtime expression that could not be statically evaluated.
    ComputedDefault,
    /// Indexed access type (Type["key"]) that could not be resolved from known tables.
    IndexedAccessOpaque,
    /// Template literal type that could not be statically expanded.
    TemplateLiteralOpaque,
    /// Discriminated union detected — props merged with discriminant surfaced.
    DiscriminatedUnion,
    /// File could not be read — permission error or file missing.
    IoError,
    /// Source file exceeds the maximum type-nesting depth; skipped to avoid parser stack overflow.
    ExcessiveNesting,
    /// TypeScript syntax error reported by the parser.
    ParseError,
    /// The extractor recognized an AST shape as a candidate (a type-alias utility
    /// invocation, a component-detector pattern) but it was malformed or
    /// incomplete in a way that made it unsupported — distinct from "wrong shape,
    /// not a candidate at all," which emits no diagnostic.
    SkippedCandidate,
}
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core skipped_candidate_code_serializes_screaming_snake_case -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/diagnostic.rs
git commit -m "feat(types): add DiagnosticCode::SkippedCandidate"
```

### Task 2: `record_skip` helper on `SourceDataCollector`

**Files:**

- Modify: `crates/core/src/extractor/mod.rs:9-24` (imports), `crates/core/src/extractor/mod.rs:202-224` (`impl<'src> SourceDataCollector<'src>` block, right after `fn new`)
- Test: inline `#[cfg(test)]` module in the same file (`crates/core/src/extractor/mod.rs`, existing `mod tests` block)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn record_skip_pushes_an_info_diagnostic_with_the_given_code() {
        use oxc_span::Span;
        let path = Utf8Path::new("/test/skip.tsx");
        let mut collector = SourceDataCollector::new(path, "", false);
        collector.record_skip(DiagnosticCode::SkippedCandidate, "malformed Omit<> arguments", Span::new(10, 20));

        assert_eq!(collector.data.diagnostics.len(), 1);
        let diag = &collector.data.diagnostics[0];
        assert_eq!(diag.severity, DiagnosticSeverity::Info);
        assert_eq!(diag.code, DiagnosticCode::SkippedCandidate);
        assert_eq!(diag.message, "malformed Omit<> arguments");
        assert_eq!(diag.file.as_deref(), Some("/test/skip.tsx"));
    }
```

Add inside the existing `mod tests` block. Note: `SourceDataCollector::new` is `fn new(path: &Utf8Path, source: &'src str, is_tsx: bool) -> Self` (private, but the test module is `mod tests` nested inside the same file via `use super::*`, so it's visible).

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core record_skip_pushes_an_info_diagnostic_with_the_given_code -- --nocapture` Expected: FAIL with `no method named record_skip found for struct SourceDataCollector` (compile error)

- [ ] **Step 3: Write minimal implementation**

Add `oxc_span::Span` to the existing `use oxc_ast_visit::Visit;` import block:

```rust
use oxc_span::{SourceType, Span};
```

(replacing the existing `use oxc_span::SourceType;` at line 15).

Add the method to the `impl<'src> SourceDataCollector<'src>` block, directly after `fn new`:

```rust
    /// Record that a recognized-but-malformed AST shape was skipped — distinct
    /// from "wrong shape, not a candidate at all" (which stays silent). Used by
    /// `classify_type_alias`'s Omit/Pick/Partial/Required/Readonly arms and the
    /// component-detector chains in `visit.rs` when a shape matches a known
    /// pattern but is missing/malformed pieces the pattern requires.
    pub(super) fn record_skip(&mut self, code: DiagnosticCode, message: impl Into<String>, span: Span) {
        let _ = span; // no line/column conversion helper exists yet; kept for future use and call-site documentation
        self.data.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Info,
            message: message.into(),
            file: Some(self.file_path.to_string()),
            line: None,
            column: None,
            help: None,
            code,
        });
    }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core record_skip_pushes_an_info_diagnostic_with_the_given_code -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/mod.rs
git commit -m "feat(extractor): add record_skip diagnostic helper"
```

### Task 3: Wire `record_skip` into `classify_type_alias`'s Omit/Pick/Partial/Required arms

**Files:**

- Modify: `crates/core/src/extractor/alias.rs:12-133`
- Test: inline `#[cfg(test)]` module — this file has no existing test module, so add one at the bottom following the convention in `crates/core/src/extractor/mod.rs`'s `mod tests`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use crate::extractor::parse_file;
    use crate::types::DiagnosticCode;

    #[test]
    fn malformed_omit_missing_second_arg_records_skipped_candidate() {
        // `Omit<Foo>` — recognized as Omit but missing the required second
        // (keys) type argument. Previously classify_type_alias's `tp.params.len()
        // < 2` guard returned None via early `return None`, silently dropping
        // the whole alias with no trace.
        let source = r#"
            interface Foo { a: string; }
            type BadOmit = Omit<Foo>;
        "#;
        let path = Utf8Path::new("/test/bad-omit.ts");
        let data = parse_file(path, source);

        assert!(
            !data.type_aliases.contains_key("/test/bad-omit.ts:BadOmit"),
            "malformed Omit should still not produce a usable alias"
        );
        assert!(
            data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
            "expected a SkippedCandidate diagnostic, got: {:?}",
            data.diagnostics
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core malformed_omit_missing_second_arg_records_skipped_candidate -- --nocapture` Expected: FAIL — `data.diagnostics` is empty, assertion `data.diagnostics.iter().any(...)` fails

- [ ] **Step 3: Write minimal implementation**

Note: `classify_type_alias` returns `Option<CollectedTypeAlias>` and is invoked from `visit_ts_type_alias_declaration` with the alias's own span not directly available inside `classify_type_alias` itself (it only receives `ty: &TSType<'a>`). Use `oxc_span::GetSpan` on `ty` (already imported pattern elsewhere in this crate, e.g. `mod.rs:465`) to get a span for the diagnostic.

```rust
//! Type alias classification: Omit, Pick, Partial, Required, Union, Intersection, Passthrough.

use oxc_ast::ast::*;
use oxc_span::GetSpan;

use crate::types::{CollectedType, CollectedTypeAlias, DiagnosticCode};

use super::SourceDataCollector;

impl<'src> SourceDataCollector<'src> {
    // ─── TypeAlias classification ─────────────────────────────────────────────

    pub(super) fn classify_type_alias<'a>(&mut self, _name: &str, ty: &TSType<'a>) -> Option<CollectedTypeAlias> {
        let fp = self.file_path.clone();
        match ty {
            TSType::TSTypeReference(tr) => {
                let ref_name = self.extract_type_ref_name(tr);
                match ref_name.as_str() {
                    "Omit" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Omit<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        if tp.params.len() < 2 {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Omit<> with fewer than 2 type arguments"),
                                tp.span,
                            );
                            return None;
                        }
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(&tp.params[0]) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Omit<>'s base type argument is not a recognizable type reference"),
                                tp.params[0].span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named {
                            name: base_name,
                            args: base_args.into_iter().map(CollectedType::Raw).collect(),
                        };
                        let (omitted_keys, omitted_keys_of) = self.collect_omit_keys(&tp.params[1]);
                        Some(CollectedTypeAlias::Omit { base, omitted_keys, omitted_keys_of, file_path: fp })
                    }
                    "Pick" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Pick<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        if tp.params.len() < 2 {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Pick<> with fewer than 2 type arguments"),
                                tp.span,
                            );
                            return None;
                        }
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(&tp.params[0]) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Pick<>'s base type argument is not a recognizable type reference"),
                                tp.params[0].span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named {
                            name: base_name,
                            args: base_args.into_iter().map(CollectedType::Raw).collect(),
                        };
                        let picked_keys = self.collect_string_union_keys(&tp.params[1]);
                        Some(CollectedTypeAlias::Pick { base, picked_keys, file_path: fp })
                    }
                    "Partial" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Partial<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        let Some(first) = tp.params.first() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Partial<> with an empty type argument list"),
                                tp.span,
                            );
                            return None;
                        };
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(first) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Partial<>'s type argument is not a recognizable type reference"),
                                first.span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named {
                            name: base_name,
                            args: base_args.into_iter().map(CollectedType::Raw).collect(),
                        };
                        Some(CollectedTypeAlias::Partial { base, file_path: fp })
                    }
                    "Required" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Required<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        let Some(first) = tp.params.first() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Required<> with an empty type argument list"),
                                tp.span,
                            );
                            return None;
                        };
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(first) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Required<>'s type argument is not a recognizable type reference"),
                                first.span(),
                            );
                            return None;
                        };
                        let base = CollectedType::Named {
                            name: base_name,
                            args: base_args.into_iter().map(CollectedType::Raw).collect(),
                        };
                        Some(CollectedTypeAlias::Required { base, file_path: fp })
                    }
                    "Readonly" => {
                        let Some(tp) = tr.type_arguments.as_ref() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Readonly<> with no type arguments"),
                                tr.span,
                            );
                            return None;
                        };
                        let Some(first) = tp.params.first() else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}' uses Readonly<> with an empty type argument list"),
                                tp.span,
                            );
                            return None;
                        };
                        let Some((base_name, base_args)) = self.extract_type_name_from_type(first) else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{_name}': Readonly<>'s type argument is not a recognizable type reference"),
                                first.span(),
                            );
                            return None;
                        };
                        let target = CollectedType::Named {
                            name: base_name,
                            args: base_args.into_iter().map(CollectedType::Raw).collect(),
                        };
                        Some(CollectedTypeAlias::Passthrough { target, file_path: fp })
                    }
                    _ => {
                        // Simple passthrough: `type Foo<T, U> = SomeOtherType<T, U>`. Args
                        // are kept structured (not stringified) so that when `SomeOtherType`
                        // is itself a generic alias, the resolver's call-site substitution
                        // can walk into them — see resolver/substitute.rs. Stringifying here
                        // (the old behavior) collapsed nested generics like `Bar<T>` into an
                        // opaque display string before substitution ever ran.
                        let args: Vec<CollectedType> = tr
                            .type_arguments
                            .as_ref()
                            .map(|ta| ta.params.iter().map(|p| self.ts_type_to_collected(p)).collect())
                            .unwrap_or_default();
                        let target = CollectedType::Named { name: ref_name.into(), args };
                        Some(CollectedTypeAlias::Passthrough { target, file_path: fp })
                    }
                }
            }
            TSType::TSUnionType(u) => {
                // Check if all members are string/number literals → LiteralUnion
                let all_string_literals = u.types.iter().all(|t| match t {
                    TSType::TSLiteralType(lit) => {
                        matches!(lit.literal, TSLiteral::StringLiteral(_))
                    }
                    TSType::TSUndefinedKeyword(_) | TSType::TSNullKeyword(_) => true,
                    _ => false,
                });

                let members: Vec<CollectedType> = u.types.iter().map(|t| self.ts_type_to_collected(t)).collect();

                if all_string_literals {
                    let member_strs: Vec<String> = members.iter().map(|m| m.to_raw_string()).collect();
                    return Some(CollectedTypeAlias::LiteralUnion { members: member_strs, file_path: fp });
                }
                Some(CollectedTypeAlias::Union { members, file_path: fp })
            }
            TSType::TSIntersectionType(i) => {
                let members = i.types.iter().map(|t| self.ts_type_to_collected(t)).collect();
                Some(CollectedTypeAlias::Intersection { members, file_path: fp })
            }
            TSType::TSParenthesizedType(p) => self.classify_type_alias(_name, &p.type_annotation),
            // Inline object type: `type Foo = { a: string }`. Previously fell through
            // to `_ => None` and silently vanished from data.type_aliases with no
            // diagnostic — anything referencing `Foo` would then resolve as unknown.
            TSType::TSTypeLiteral(_) => {
                Some(CollectedTypeAlias::Passthrough { target: self.ts_type_to_collected(ty), file_path: fp })
            }
            // Bare function type: `type Handler<T> = (arg: T) => void`. Same
            // silent-vanishing bug as TSTypeLiteral above — real-world callback type
            // aliases (react-day-picker's `OnSelectHandler<T>`) use this shape.
            TSType::TSFunctionType(_) => {
                Some(CollectedTypeAlias::Passthrough { target: self.ts_type_to_collected(ty), file_path: fp })
            }
            // Everything else `ts_type_to_collected` already knows how to represent
            // structurally (arrays, tuples, indexed access, conditional/mapped
            // types, …) — e.g. `type API_KeyCollection = string[]` (Storybook's real
            // pattern). Same silent-vanishing bug as the two arms above, generalized:
            // a dedicated arm above always wins for shapes needing special alias
            // semantics (Omit's key-splitting, discriminated-union detection, …); this
            // catch-all only ever runs for shapes with no such semantics, where a
            // transparent Passthrough is exactly correct.
            _ => Some(CollectedTypeAlias::Passthrough { target: self.ts_type_to_collected(ty), file_path: fp }),
        }
    }

    /// Collect the string literal keys from a type like `'key1' | 'key2'`.
    pub(super) fn collect_string_union_keys<'a>(&self, ty: &TSType<'a>) -> Vec<String> {
        match ty {
            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => vec![s.value.as_str().to_owned()],
                _ => vec![],
            },
            TSType::TSUnionType(u) => u.types.iter().flat_map(|t| self.collect_string_union_keys(t)).collect(),
            _ => vec![],
        }
    }

    /// Classify `Omit<_, Keys>`'s second type argument: a literal key union
    /// (`'a' | 'b'`), or `keyof SomeType` — in the latter case the key set can't
    /// be known until `SomeType` is resolved, so the operand is captured
    /// structurally for the resolver to expand later (see
    /// `CollectedTypeAlias::Omit::omitted_keys_of`).
    pub(super) fn collect_omit_keys<'a>(&mut self, ty: &TSType<'a>) -> (Vec<String>, Option<Box<CollectedType>>) {
        match self.ts_type_to_collected(ty) {
            CollectedType::KeyOf(inner) => (vec![], Some(inner)),
            other => (other.as_string_union_keys(), None),
        }
    }
}
```

Note: I'm not 100% certain `TSTypeParameterInstantiation` (the type of `tp`) exposes a `.span` field directly usable as `tp.span` — verify by reading `tr.type_arguments`'s type definition in the currently-vendored `oxc_ast` crate before compiling; if it doesn't have `.span`, fall back to `tr.span` for that arm's diagnostic instead.

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core malformed_omit_missing_second_arg_records_skipped_candidate -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/alias.rs
git commit -m "feat(extractor): record SkippedCandidate diagnostics for malformed Omit/Pick/Partial/Required/Readonly"
```

### Task 4: Wire `record_skip` into `visit.rs`'s PascalCase-binding detector chain

**Files:**

- Modify: `crates/core/src/extractor/visit.rs:227-260` (`visit_variable_declaration`)
- Test: inline `#[cfg(test)]` module in `crates/core/src/extractor/mod.rs` (matches this crate's convention — `visit.rs` has no test module of its own, and `mod.rs`'s existing tests already exercise `visit_variable_declaration` end-to-end via `parse_file`)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn pascal_case_binding_with_no_matching_detector_records_skipped_candidate() {
        // `const Button = something()` — PascalCase binding, .tsx file, but the
        // init expression matches none of try_fc_annotation / try_forward_ref /
        // try_hoc_wrapped / try_rename_identifier_wrapped_component. Previously
        // the whole chain fell through silently with zero trace it was even
        // considered a component candidate.
        let source = r#"
            const Button = someUnrecognizedFactory();
        "#;
        let path = Utf8Path::new("/test/unrecognized.tsx");
        let data = parse_file(path, source);

        assert!(
            !data.component_mappings.iter().any(|m| m.component_name == "Button"),
            "no mapping should have been produced for an unrecognized pattern"
        );
        assert!(
            data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
            "expected a SkippedCandidate diagnostic, got: {:?}",
            data.diagnostics
        );
    }
```

Add inside the existing `mod tests` block in `crates/core/src/extractor/mod.rs`.

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core pascal_case_binding_with_no_matching_detector_records_skipped_candidate -- --nocapture` Expected: FAIL — `data.diagnostics` has no `SkippedCandidate` entry

- [ ] **Step 3: Write minimal implementation**

Current code at `visit.rs:236-247`:

```rust
            if self.is_tsx {
                if let Some(name) = self.extract_pascal_name(declarator) {
                    if let Some(mapping) = self
                        .try_fc_annotation(declarator, &name)
                        .or_else(|| self.try_forward_ref(declarator, &name))
                        .or_else(|| self.try_hoc_wrapped(declarator, &name))
                    {
                        self.data.component_mappings.push(mapping);
                        continue;
                    }
                    self.try_rename_identifier_wrapped_component(declarator, &name);
                }
            }
```

Change to:

```rust
            if self.is_tsx {
                if let Some(name) = self.extract_pascal_name(declarator) {
                    if let Some(mapping) = self
                        .try_fc_annotation(declarator, &name)
                        .or_else(|| self.try_forward_ref(declarator, &name))
                        .or_else(|| self.try_hoc_wrapped(declarator, &name))
                    {
                        self.data.component_mappings.push(mapping);
                        continue;
                    }
                    // try_rename_identifier_wrapped_component is itself a give-up-quietly
                    // path (a bare/wrapped identifier re-binding, not a props-bearing
                    // component candidate) — only record a skip when even that finds
                    // nothing, so plain aliasing (`const Button = InternalButton;`)
                    // doesn't spuriously report as an unsupported candidate.
                    if !self.try_rename_identifier_wrapped_component(declarator, &name) {
                        self.record_skip(
                            DiagnosticCode::SkippedCandidate,
                            format!(
                                "'{name}' is a PascalCase binding but matched no known component pattern \
                                 (FC annotation, forwardRef, HOC wrapper, or identifier alias)"
                            ),
                            declarator.span,
                        );
                    }
                }
            }
```

This requires `try_rename_identifier_wrapped_component` to report whether it matched. Read its current signature in `crates/core/src/extractor/component.rs` before editing — if it currently returns `()`, change its return type to `bool` (`true` when it renamed/aliased something, `false` otherwise) and update its body's early-return paths accordingly, then fix its one other call site (this one, in `visit.rs`) to use the new return value as shown above. Do not guess its exact current match arms — read the real function body first since its `bool`-conversion depends on exactly which internal branches currently return early vs. fall through.

Also add `DiagnosticCode` to the `use crate::types::{...}` import list in `visit.rs`:

```rust
use crate::types::{
    CollectedInterface, ComponentMapping, DiagnosticCode, EnumEntry, EnumValue, ExtendsRef, ImportBinding,
    LexedExport, RawProp, TypeName,
};
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core pascal_case_binding_with_no_matching_detector_records_skipped_candidate -- --nocapture` Expected: PASS. Also re-run the full extractor suite to confirm the `try_rename_identifier_wrapped_component` signature change didn't break its existing callers/tests: `cargo test -p oxc-react-docgen-core extractor:: -- --nocapture`

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/visit.rs crates/core/src/extractor/component.rs crates/core/src/extractor/mod.rs
git commit -m "feat(extractor): record SkippedCandidate when a PascalCase binding matches no component pattern"
```

### Task 5: Wire `record_skip` into `visit.rs`'s `visit_function` detector chain

**Files:**

- Modify: `crates/core/src/extractor/visit.rs:262-293`
- Test: inline `#[cfg(test)]` module in `crates/core/src/extractor/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn pascal_case_function_declaration_with_untyped_first_param_records_skipped_candidate() {
        // `function Button(props) { ... }` — PascalCase FunctionDeclaration,
        // .tsx file, has a first param, but it carries no type annotation at
        // all. Previously the whole chain (type_annotation.as_ref()?...) fell
        // through silently.
        let source = r#"
            function Button(props) {
                return null;
            }
        "#;
        let path = Utf8Path::new("/test/untyped-param.tsx");
        let data = parse_file(path, source);

        assert!(
            !data.component_mappings.iter().any(|m| m.component_name == "Button"),
            "no mapping should have been produced for an untyped first param"
        );
        assert!(
            data.diagnostics.iter().any(|d| d.code == DiagnosticCode::SkippedCandidate),
            "expected a SkippedCandidate diagnostic, got: {:?}",
            data.diagnostics
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core pascal_case_function_declaration_with_untyped_first_param_records_skipped_candidate -- --nocapture` Expected: FAIL — no `SkippedCandidate` diagnostic present

- [ ] **Step 3: Write minimal implementation**

Current code at `visit.rs:262-293`:

```rust
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        // Pattern 4: `function Button(props: ButtonProps) { ... }`
        if self.is_tsx {
            if let Some(id) = &func.id {
                let name = id.name.as_str();
                if is_pascal_case(name) && func.r#type == FunctionType::FunctionDeclaration {
                    if let Some(first_param) = func.params.items.first() {
                        if let Some(type_ann) = &first_param.type_annotation {
                            if let Some((props_name, type_args)) =
                                self.extract_type_name_from_type(&type_ann.type_annotation)
                            {
                                let (description, tags) = self.find_jsdoc_with_tags(func.span.start);
                                let param_defaults = self.extract_param_defaults(&func.params);
                                self.data.component_mappings.push(ComponentMapping {
                                    component_name: name.to_owned(),
                                    props_type_name: props_name,
                                    props_type_args: type_args,
                                    file_path: self.file_path.clone(),
                                    description,
                                    tags,
                                    span_start: func.span.start,
                                    span_end: func.span.end,
                                    param_defaults,
                                });
                            }
                        }
                    }
                }
            }
        }
        walk::walk_function(self, func, flags);
    }
```

Change to:

```rust
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        // Pattern 4: `function Button(props: ButtonProps) { ... }`
        if self.is_tsx {
            if let Some(id) = &func.id {
                let name = id.name.as_str();
                if is_pascal_case(name) && func.r#type == FunctionType::FunctionDeclaration {
                    if let Some(first_param) = func.params.items.first() {
                        if let Some(type_ann) = &first_param.type_annotation {
                            if let Some((props_name, type_args)) =
                                self.extract_type_name_from_type(&type_ann.type_annotation)
                            {
                                let (description, tags) = self.find_jsdoc_with_tags(func.span.start);
                                let param_defaults = self.extract_param_defaults(&func.params);
                                self.data.component_mappings.push(ComponentMapping {
                                    component_name: name.to_owned(),
                                    props_type_name: props_name,
                                    props_type_args: type_args,
                                    file_path: self.file_path.clone(),
                                    description,
                                    tags,
                                    span_start: func.span.start,
                                    span_end: func.span.end,
                                    param_defaults,
                                });
                            } else {
                                self.record_skip(
                                    DiagnosticCode::SkippedCandidate,
                                    format!(
                                        "'{name}' is a PascalCase function declaration whose first param's type \
                                         annotation isn't a recognizable props type reference"
                                    ),
                                    type_ann.span,
                                );
                            }
                        } else {
                            self.record_skip(
                                DiagnosticCode::SkippedCandidate,
                                format!("'{name}' is a PascalCase function declaration with an untyped first param"),
                                first_param.span,
                            );
                        }
                    }
                }
            }
        }
        walk::walk_function(self, func, flags);
    }
```

Note: I'm not 100% certain `FormalParameter` (the type of `first_param`) exposes `.span` directly by that name in the currently-vendored `oxc_ast` — verify before compiling; if absent, use `oxc_span::GetSpan` on `first_param` (`first_param.span()`) instead, matching the pattern already used elsewhere in this crate (e.g. `mod.rs:465`).

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core pascal_case_function_declaration_with_untyped_first_param_records_skipped_candidate -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/visit.rs crates/core/src/extractor/mod.rs
git commit -m "feat(extractor): record SkippedCandidate in visit_function's component-detector chain"
```

### Task 6: Thread a depth counter through `ts_type_to_collected` and its recursive siblings

**Files:**

- Modify: `crates/core/src/extractor/mod.rs:34-40` (constants), `:299-566` (`extract_type_args`, `ts_type_to_collected`, `ts_tuple_element_to_collected`, `ts_signature_to_object_field`, `collect_property_signature`), `crates/core/src/extractor/alias.rs` (call sites of `ts_type_to_collected`/`collect_omit_keys`), `crates/core/src/extractor/interface.rs` and `crates/core/src/extractor/defaults.rs` if they call `ts_type_to_collected` directly (check before editing)
- Test: inline `#[cfg(test)]` module in `crates/core/src/extractor/mod.rs`

This is the largest task in the group — every recursive call site of `ts_type_to_collected` (and its mutually-recursive siblings `ts_tuple_element_to_collected`, `ts_signature_to_object_field`, `collect_property_signature`, `extract_type_args`) must pass a `depth: u8` through, mirroring the resolver's `depth: u8` / `MAX_DEPTH` convention in `resolver/collected.rs:16-26`. Before writing the implementation, run `grep -n "ts_type_to_collected\|ts_tuple_element_to_collected\|ts_signature_to_object_field\|extract_type_args(" crates/core/src/extractor/*.rs` to enumerate every current call site — the signature change touches all of them, and any missed one is a compile error, not a silent bug, so the compiler will catch omissions on Step 4.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic() {
        // Adversarial case for the depth-tracking gap: chained conditional types
        // (`A extends B ? C extends D ? ... : ... : ...`) add one AST recursion
        // level per `? :` with only 2 brackets total (none at all, actually —
        // conditional types need no parens/braces/brackets per level), so
        // max_bracket_nesting_depth's proxy metric undercounts this shape badly.
        // Construct enough chained conditionals to exceed a depth-500 guard while
        // staying far under MAX_SOURCE_NESTING_DEPTH's bracket-based limit (2000),
        // proving the depth counter — not the existing bracket guard — is what
        // catches this.
        let mut ty = "boolean".to_owned();
        for i in 0..600 {
            ty = format!("T{i} extends string ? {ty} : never");
        }
        let source = format!("type Deep = {ty};");

        // The bracket-nesting proxy must NOT trip on this source — proves this
        // test is closing a real gap, not duplicating existing coverage.
        assert!(
            super::max_bracket_nesting_depth(&source) <= MAX_SOURCE_NESTING_DEPTH,
            "test fixture is invalid: bracket heuristic already catches this, \
             defeating the purpose of the adversarial case"
        );

        let path = Utf8Path::new("/test/deep-conditional.ts");
        let data = parse_file(path, &source);

        assert!(
            data.diagnostics.iter().any(|d| d.code == DiagnosticCode::MaxDepthExceeded),
            "expected a MaxDepthExceeded diagnostic from the new depth counter, got: {:?}",
            data.diagnostics
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic -- --nocapture` Expected: FAIL — either a stack overflow (test process aborts) or, if the machine's stack tolerates 600 levels, the assertion on `MaxDepthExceeded` fails because no depth counter exists yet to emit it

- [ ] **Step 3: Write minimal implementation**

Add the constant near `MAX_SOURCE_NESTING_DEPTH`:

```rust
/// Maximum AST recursion depth for `ts_type_to_collected` and its mutually
/// recursive siblings. `max_bracket_nesting_depth` bounds raw-text bracket
/// depth as a cheap pre-parse proxy for parser stack safety, but chained
/// conditional types (`A extends B ? C extends D ? ... : ... : ...`) add one
/// AST level per `? :` with no brackets at all — the proxy metric undercounts
/// exactly this shape. This counter guards the extractor's own recursion the
/// same way the resolver's `depth: u8` / `MAX_DEPTH` already guards
/// `resolve_collected_type` (see `resolver/mod.rs`), just at a higher ceiling
/// since this walk is a single in-process AST-to-struct conversion, not
/// cross-file resolution.
const MAX_TYPE_COLLECT_DEPTH: u8 = 200;
```

Note on the constant value: the task brief proposed 500; I'm choosing 200 here as a more conservative real ceiling given `ts_type_to_collected` recurses through plain Rust function calls (each frame is heavier than the resolver's, which shares more state via references) — pick whichever value survives Step 4 without stack-overflowing the test process; treat 200 as a starting point, not a hard requirement, and adjust it (and the test's `600` chained levels, which must stay comfortably above whatever final constant is chosen) if profiling during Step 4 shows a different number is needed. Whatever value is chosen, `MAX_SOURCE_NESTING_DEPTH` (2000, bracket-based) and `MAX_TYPE_COLLECT_DEPTH` (AST-based) are independent knobs and do not need to match.

Change `ts_type_to_collected` and its direct recursive siblings to thread `depth: u8`:

```rust
    pub(super) fn ts_type_to_collected<'a>(&mut self, ty: &TSType<'a>) -> CollectedType {
        self.ts_type_to_collected_at_depth(ty, 0)
    }

    fn ts_type_to_collected_at_depth<'a>(&mut self, ty: &TSType<'a>, depth: u8) -> CollectedType {
        if depth > MAX_TYPE_COLLECT_DEPTH {
            use oxc_span::GetSpan;
            let span = ty.span();
            self.data.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "Type nesting exceeds maximum extractor recursion depth ({depth} > {MAX_TYPE_COLLECT_DEPTH})"
                ),
                file: Some(self.file_path.to_string()),
                line: None,
                column: None,
                help: Some("This may indicate a deeply chained conditional or mapped type.".into()),
                code: DiagnosticCode::MaxDepthExceeded,
            });
            let raw = self.source[span.start as usize..span.end as usize].to_owned();
            return CollectedType::Raw(raw);
        }
        match ty {
            TSType::TSStringKeyword(_) => CollectedType::String,
            TSType::TSNumberKeyword(_) => CollectedType::Number,
            TSType::TSBooleanKeyword(_) => CollectedType::Boolean,
            TSType::TSNullKeyword(_) => CollectedType::Null,
            TSType::TSUndefinedKeyword(_) => CollectedType::Undefined,
            TSType::TSAnyKeyword(_) => CollectedType::Any,
            TSType::TSNeverKeyword(_) => CollectedType::Never,
            TSType::TSUnknownKeyword(_) => CollectedType::Unknown,
            TSType::TSVoidKeyword(_) => CollectedType::Void,
            TSType::TSBigIntKeyword(_) => CollectedType::BigInt,
            TSType::TSSymbolKeyword(_) => CollectedType::Symbol,
            TSType::TSObjectKeyword(_) => CollectedType::Named { name: "object".into(), args: vec![] },

            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => CollectedType::StringLiteral(s.value.as_str().into()),
                TSLiteral::NumericLiteral(n) => CollectedType::NumberLiteral(n.value),
                TSLiteral::BooleanLiteral(b) => CollectedType::BoolLiteral(b.value),
                TSLiteral::UnaryExpression(u) => {
                    let raw = self.source[u.span.start as usize..u.span.end as usize].to_owned();
                    CollectedType::Raw(raw)
                }
                _ => CollectedType::Raw(self.source[lit.span.start as usize..lit.span.end as usize].to_owned()),
            },

            TSType::TSTypeReference(tr) => {
                let name: CompactString = self.ts_type_name_str(&tr.type_name).into();
                let args = tr
                    .type_arguments
                    .as_ref()
                    .map(|ta| ta.params.iter().map(|p| self.ts_type_to_collected_at_depth(p, depth + 1)).collect())
                    .unwrap_or_default();
                CollectedType::Named { name, args }
            }

            TSType::TSTypeQuery(q) => {
                let name = self.ts_type_query_name(q);
                CollectedType::TypeOf(name.into())
            }

            TSType::TSUnionType(u) => {
                let members: Vec<CollectedType> =
                    u.types.iter().map(|t| self.ts_type_to_collected_at_depth(t, depth + 1)).collect();
                CollectedType::Union(members)
            }

            TSType::TSIntersectionType(i) => {
                let members: Vec<CollectedType> =
                    i.types.iter().map(|t| self.ts_type_to_collected_at_depth(t, depth + 1)).collect();
                CollectedType::Intersection(members)
            }

            TSType::TSArrayType(a) => {
                CollectedType::Array(Box::new(self.ts_type_to_collected_at_depth(&a.element_type, depth + 1)))
            }

            TSType::TSTupleType(t) => {
                let members: Vec<CollectedType> =
                    t.element_types.iter().map(|el| self.ts_tuple_element_to_collected_at_depth(el, depth + 1)).collect();
                CollectedType::Tuple(members)
            }

            TSType::TSTypeLiteral(lit) => {
                let fields: Vec<CollectedObjectField> = lit
                    .members
                    .iter()
                    .filter_map(|member| self.ts_signature_to_object_field_at_depth(member, depth + 1))
                    .collect();
                CollectedType::Object(fields)
            }

            TSType::TSFunctionType(f) => {
                let params: Vec<CollectedType> = f
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|ta| self.ts_type_to_collected_at_depth(&ta.type_annotation, depth + 1))
                            .unwrap_or(CollectedType::Any)
                    })
                    .collect();
                let param_names: Vec<Option<CompactString>> =
                    f.params.items.iter().map(|p| binding_pattern_name(&p.pattern)).collect();
                let return_type = self.ts_type_to_collected_at_depth(&f.return_type.type_annotation, depth + 1);
                CollectedType::Function { params, param_names, return_type: Box::new(return_type) }
            }

            TSType::TSIndexedAccessType(ia) => CollectedType::IndexedAccess {
                obj: Box::new(self.ts_type_to_collected_at_depth(&ia.object_type, depth + 1)),
                key: Box::new(self.ts_type_to_collected_at_depth(&ia.index_type, depth + 1)),
            },

            TSType::TSTemplateLiteralType(tl) => {
                let mut parts: Vec<CollectedType> = Vec::new();
                for (i, quasi) in tl.quasis.iter().enumerate() {
                    let s = quasi.value.raw.as_str();
                    if !s.is_empty() {
                        parts.push(CollectedType::StringLiteral(s.into()));
                    }
                    if let Some(ty) = tl.types.get(i) {
                        parts.push(self.ts_type_to_collected_at_depth(ty, depth + 1));
                    }
                }
                CollectedType::TemplateLiteral(parts)
            }

            TSType::TSConditionalType(c) => CollectedType::Conditional {
                check: Box::new(self.ts_type_to_collected_at_depth(&c.check_type, depth + 1)),
                extends_type: Box::new(self.ts_type_to_collected_at_depth(&c.extends_type, depth + 1)),
                true_type: Box::new(self.ts_type_to_collected_at_depth(&c.true_type, depth + 1)),
                false_type: Box::new(self.ts_type_to_collected_at_depth(&c.false_type, depth + 1)),
            },

            TSType::TSMappedType(m) => {
                let key_type = self.ts_type_to_collected_at_depth(&m.constraint, depth + 1);
                let value_type = m
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.ts_type_to_collected_at_depth(ta, depth + 1))
                    .unwrap_or(CollectedType::Unknown);
                CollectedType::Mapped { key_type: Box::new(key_type), value_type: Box::new(value_type) }
            }

            TSType::TSParenthesizedType(p) => self.ts_type_to_collected_at_depth(&p.type_annotation, depth + 1),

            TSType::TSTypeOperatorType(op) => match op.operator {
                TSTypeOperatorOperator::Keyof => {
                    CollectedType::KeyOf(Box::new(self.ts_type_to_collected_at_depth(&op.type_annotation, depth + 1)))
                }
                TSTypeOperatorOperator::Unique | TSTypeOperatorOperator::Readonly => {
                    self.ts_type_to_collected_at_depth(&op.type_annotation, depth + 1)
                }
            },

            TSType::TSInferType(i) => {
                let raw = self.source[i.span.start as usize..i.span.end as usize].to_owned();
                CollectedType::Raw(raw)
            }

            _ => {
                use oxc_span::GetSpan;
                let span = ty.span();
                let raw = self.source[span.start as usize..span.end as usize].to_owned();
                CollectedType::Raw(raw)
            }
        }
    }

    pub(super) fn ts_tuple_element_to_collected<'a>(&mut self, el: &TSTupleElement<'a>) -> CollectedType {
        self.ts_tuple_element_to_collected_at_depth(el, 0)
    }

    fn ts_tuple_element_to_collected_at_depth<'a>(&mut self, el: &TSTupleElement<'a>, depth: u8) -> CollectedType {
        match el {
            TSTupleElement::TSOptionalType(o) => {
                let inner = self.ts_type_to_collected_at_depth(&o.type_annotation, depth + 1);
                CollectedType::Union(vec![inner, CollectedType::Undefined])
            }
            TSTupleElement::TSRestType(r) => {
                CollectedType::Array(Box::new(self.ts_type_to_collected_at_depth(&r.type_annotation, depth + 1)))
            }
            other => {
                use oxc_span::GetSpan;
                let span = other.span();
                let raw = self.source[span.start as usize..span.end as usize].to_owned();
                CollectedType::Raw(raw)
            }
        }
    }
```

`extract_type_args` (line 299-307) calls `ts_type_to_collected` — update it to call the depth-aware entry point at depth 0 (it's not itself in the hot recursive path, called once per type-argument list):

```rust
    pub(super) fn extract_type_args<'a>(
        &mut self,
        type_params: &Option<OxcBox<'a, TSTypeParameterInstantiation<'a>>>,
    ) -> Vec<String> {
        match type_params {
            Some(tp) => tp.params.iter().map(|p| self.ts_type_to_collected_at_depth(p, 0).to_raw_string()).collect(),
            None => vec![],
        }
    }
```

`ts_signature_to_object_field` (line 514-566) and `collect_property_signature` (line 674-729) both call `ts_type_to_collected` on top-level signature bodies (not deeply nested within `ts_type_to_collected` itself) — leave their own signatures as `pub(super) fn ts_signature_to_object_field<'a>(&mut self, member: &TSSignature<'a>) -> Option<CollectedObjectField>` (unchanged, called from `TSType::TSTypeLiteral`'s arm above via the new `_at_depth` variant, so add a matching `ts_signature_to_object_field_at_depth` wrapper):

```rust
    pub(super) fn ts_signature_to_object_field<'a>(
        &mut self,
        member: &TSSignature<'a>,
    ) -> Option<CollectedObjectField> {
        self.ts_signature_to_object_field_at_depth(member, 0)
    }

    fn ts_signature_to_object_field_at_depth<'a>(
        &mut self,
        member: &TSSignature<'a>,
        depth: u8,
    ) -> Option<CollectedObjectField> {
        match member {
            TSSignature::TSPropertySignature(sig) => {
                let name = match &sig.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str().to_owned(),
                    PropertyKey::StringLiteral(s) => s.value.as_str().to_owned(),
                    _ => return None,
                };
                let collected_type = sig
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.ts_type_to_collected_at_depth(&ta.type_annotation, depth + 1))
                    .unwrap_or(CollectedType::Any);
                let description = self.find_jsdoc(sig.span.start);
                Some(CollectedObjectField { name, collected_type, required: !sig.optional, description })
            }
            TSSignature::TSMethodSignature(sig) => {
                let name = match &sig.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str().to_owned(),
                    PropertyKey::StringLiteral(s) => s.value.as_str().to_owned(),
                    _ => return None,
                };
                let params: Vec<CollectedType> = sig
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|ta| self.ts_type_to_collected_at_depth(&ta.type_annotation, depth + 1))
                            .unwrap_or(CollectedType::Any)
                    })
                    .collect();
                let param_names: Vec<Option<CompactString>> =
                    sig.params.items.iter().map(|p| binding_pattern_name(&p.pattern)).collect();
                let return_type = sig
                    .return_type
                    .as_ref()
                    .map(|rt| self.ts_type_to_collected_at_depth(&rt.type_annotation, depth + 1))
                    .unwrap_or(CollectedType::Void);
                Some(CollectedObjectField {
                    name,
                    collected_type: CollectedType::Function { params, param_names, return_type: Box::new(return_type) },
                    required: !sig.optional,
                    description: String::new(),
                })
            }
            _ => None,
        }
    }
```

Leave `collect_property_signature`, `classify_type_alias`, `collect_omit_keys`, `extract_type_name_from_type`, and every other call site outside `ts_type_to_collected`'s own recursion untouched — they call the public `ts_type_to_collected`/`ts_tuple_element_to_collected` wrappers (now depth-0 entry points) exactly as before, so no other file's call sites need to change. Confirm this with the `grep` from before Step 1 — any call site the grep turns up that isn't inside `ts_type_to_collected_at_depth`'s own match arms or the two wrapper functions above should be left calling the public, depth-0 wrapper unchanged.

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core deeply_chained_conditional_types_hit_the_depth_guard_not_the_bracket_heuristic -- --nocapture` Expected: PASS. If it still stack-overflows before hitting the guard, lower `MAX_TYPE_COLLECT_DEPTH` and/or increase the test's chained-conditional count proportionally, then re-run. Also run the full extractor + snapshot suite to confirm no regression from the depth-threading refactor: `cargo test -p oxc-react-docgen-core -- --nocapture` and `/snapshot` if any snapshot fixture's output changed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/extractor/mod.rs
git commit -m "feat(extractor): thread depth counter through ts_type_to_collected, guard against deep conditional-type chains"
```

---

## Part D: Pipeline discovery/merge fixes

### Task 1: `discover_files` reports diagnostics for `ignore::Walk` errors instead of dropping them

**Files:**

- Modify: `crates/core/src/pipeline/discover.rs:1-56`
- Modify: `crates/core/src/pipeline/mod.rs:251-253` (call site), `crates/core/src/pipeline/mod.rs:533-569` (existing tests using the old signature)
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/pipeline/mod.rs` (existing convention — `discover_files` is re-exported via `use discover::{discover_files, should_skip};` at the top of `mod.rs` and all its current tests already live in that module, e.g. `test_discover_files` at line 533)

- [ ] **Step 1: Write the failing test**

```rust
    // ── test_discover_files_reports_diagnostic_for_permission_denied_subtree ─
    //
    // Bug A (root-cause-analysis.md): `discover_files` used `walker.flatten()`,
    // silently dropping every `ignore::Walk` `Err` — a permission-denied
    // subtree, a broken symlink, or any other I/O error mid-walk vanished with
    // no diagnostic, no warning, nothing. This exercises the permission-denied
    // case with a real unreadable subdirectory.

    #[test]
    #[cfg(unix)]
    fn test_discover_files_reports_diagnostic_for_permission_denied_subtree() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let restricted = tmp.path().join("restricted");
        fs::create_dir(&restricted).unwrap();
        fs::write(restricted.join("Hidden.tsx"), "export const Hidden = () => null;").unwrap();
        write_file(&tmp, "Visible.tsx", "export const Visible = () => null;");

        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o000)).unwrap();

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, diagnostics) = discover_files(&[dir], &[]);

        // Restore permissions so TempDir's Drop can actually remove the directory.
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755)).unwrap();

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Visible.tsx"), "readable file should still be discovered");
        assert!(!names.contains(&"Hidden.tsx"), "unreadable subtree's file must not silently appear");
        assert!(
            diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError),
            "expected an IoError diagnostic for the unreadable subtree, got {:?}",
            diagnostics
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_discover_files_reports_diagnostic_for_permission_denied_subtree -- --nocapture` Expected: FAIL — compile error. `discover_files` currently returns `Vec<Utf8PathBuf>`, so `let (files, diagnostics) = discover_files(...)` mismatches types (`expected Vec<Utf8PathBuf>, found tuple`).

- [ ] **Step 3: Write minimal implementation**

Rewrite `crates/core/src/pipeline/discover.rs`:

```rust
use camino::Utf8PathBuf;

use crate::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// Walk `src_dirs` and collect every `.ts` / `.tsx` file, excluding:
/// - Built-in patterns: `.stories.`, `.test.`, `.spec.`, `__snapshots__`, `node_modules`
/// - User-supplied extra exclude patterns
///
/// Returns discovered files alongside any diagnostics raised while walking
/// (e.g. a permission-denied subtree) — never dropped silently (CLAUDE.md
/// non-negotiable #6).
pub(super) fn discover_files(
    src_dirs: &[Utf8PathBuf],
    extra_excludes: &[String],
) -> (Vec<Utf8PathBuf>, Vec<Diagnostic>) {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();

    for dir in src_dirs {
        // If the user explicitly points at a node_modules path, respect it.
        let dir_str = dir.as_str();
        let dir_is_in_node_modules = dir_str.contains("node_modules");

        let walker =
            ignore::WalkBuilder::new(dir.as_std_path()).hidden(false).git_ignore(!dir_is_in_node_modules).build();

        for result in walker {
            let entry = match result {
                Ok(entry) => entry,
                Err(err) => {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        message: format!("Error walking '{dir}': {err}"),
                        file: Some(dir.to_string()),
                        line: None,
                        column: None,
                        help: Some(
                            "Check file/directory permissions and for broken symlinks under this path.".into(),
                        ),
                        code: DiagnosticCode::IoError,
                    });
                    continue;
                }
            };
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "ts" | "tsx") {
                continue;
            }

            let path_str = path.to_str().unwrap_or("");

            // Built-in excludes — skip node_modules sub-dirs only when not intentionally targeting them.
            if path_str.contains(".stories.")
                || path_str.contains(".test.")
                || path_str.contains(".spec.")
                || path_str.contains("__snapshots__")
                || (!dir_is_in_node_modules && path_str.contains("node_modules"))
            {
                continue;
            }

            // User-supplied excludes.
            if extra_excludes.iter().any(|p| path_str.contains(p.as_str())) {
                continue;
            }

            if let Ok(utf8) = Utf8PathBuf::from_path_buf(path.to_owned()) {
                // Canonicalize to an absolute path so parent.fileName in output is
                // stable regardless of invocation context (relative --src, absolute
                // --src, or cwd inside the src dir all previously produced different
                // strings for the same file). Fall back to the uncanonicalized path
                // if canonicalization fails (e.g. a dangling symlink) rather than
                // dropping the file.
                let canonical = utf8.canonicalize_utf8().unwrap_or(utf8);
                files.push(canonical);
            }
        }
    }

    files.sort(); // deterministic ordering across OS / FS
    (files, diagnostics)
}

pub(super) fn should_skip(name: &str, exclude_prefixes: &[String]) -> bool {
    exclude_prefixes.iter().any(|p| name.starts_with(p.as_str()))
}
```

In `crates/core/src/pipeline/mod.rs`, update the call site:

```rust
    // Phase 1: Discover source files.
    let (src_files, mut discover_diagnostics) = discover_files(&options.src_dirs, &options.exclude_patterns);
    diagnostics.append(&mut discover_diagnostics);
    let files_parsed = src_files.len() as u32;
```

And fix the two existing tests that call `discover_files` directly:

```rust
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, _diagnostics) = discover_files(&[dir], &[]);
```

(apply this same destructuring edit to both `test_discover_files` and `test_exclude_stories`).

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_discover_files_reports_diagnostic_for_permission_denied_subtree -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/discover.rs crates/core/src/pipeline/mod.rs
git commit -m "fix(pipeline): report ignore::Walk errors as diagnostics instead of dropping them"
```

### Task 2: `discover_files` reports a diagnostic for non-UTF8 filenames

**Files:**

- Modify: `crates/core/src/pipeline/discover.rs` (the `if let Ok(utf8) = Utf8PathBuf::from_path_buf(...)` block from Task 1)
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/pipeline/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
    // ── test_discover_files_reports_diagnostic_for_non_utf8_filename ─────────
    //
    // Bug A, second half: a non-UTF8 filename made `Utf8PathBuf::from_path_buf`
    // fail, and the `if let Ok(utf8) = ...` branch had no `else` — the file
    // silently vanished from the discovered set with zero diagnostic.

    #[test]
    #[cfg(unix)]
    fn test_discover_files_reports_diagnostic_for_non_utf8_filename() {
        use std::os::unix::ffi::OsStrExt;

        let tmp = TempDir::new().unwrap();
        write_file(&tmp, "Button.tsx", "export const Button = () => null;");

        // 0xFF is invalid UTF-8 in any position — construct a non-UTF8 filename
        // directly, bypassing Rust's &str API (which can't represent one).
        let bad_name = std::ffi::OsStr::from_bytes(b"Bad\xFF.tsx");
        fs::write(tmp.path().join(bad_name), "export const Bad = () => null;").unwrap();

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let (files, diagnostics) = discover_files(&[dir], &[]);

        let names: Vec<&str> = files.iter().map(|f| f.file_name().unwrap()).collect();
        assert!(names.contains(&"Button.tsx"), "valid-UTF8 file should still be discovered");
        assert_eq!(files.len(), 1, "non-UTF8 filename must not silently appear in the discovered set");
        assert!(
            diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError),
            "expected an IoError diagnostic for the non-UTF8 filename, got {:?}",
            diagnostics
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_discover_files_reports_diagnostic_for_non_utf8_filename -- --nocapture` Expected: FAIL — compiles fine (Task 1 already returns the tuple), but `diagnostics.iter().any(|d| d.code == DiagnosticCode::IoError)` is `false`: the non-UTF8 branch currently has no `else`, so nothing is pushed.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/pipeline/discover.rs`, change:

```rust
            if let Ok(utf8) = Utf8PathBuf::from_path_buf(path.to_owned()) {
                // Canonicalize to an absolute path so parent.fileName in output is
                // stable regardless of invocation context (relative --src, absolute
                // --src, or cwd inside the src dir all previously produced different
                // strings for the same file). Fall back to the uncanonicalized path
                // if canonicalization fails (e.g. a dangling symlink) rather than
                // dropping the file.
                let canonical = utf8.canonicalize_utf8().unwrap_or(utf8);
                files.push(canonical);
            } else {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!("Skipping non-UTF8 path: {}", path.to_string_lossy()),
                    file: None,
                    line: None,
                    column: None,
                    help: Some("Rename the file to use valid UTF-8 characters in its path.".into()),
                    code: DiagnosticCode::IoError,
                });
            }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_discover_files_reports_diagnostic_for_non_utf8_filename -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/discover.rs
git commit -m "fix(pipeline): report a diagnostic for non-UTF8 filenames instead of dropping them"
```

### Task 3: Empty `src_dirs` produces a diagnostic instead of a silent zero-file run

**Files:**

- Modify: `crates/core/src/pipeline/mod.rs:218-245`
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/pipeline/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
    // ── test_extract_empty_src_dirs_produces_diagnostic ───────────────────────
    //
    // Bug B (root-cause-analysis.md): the guard
    // `!options.src_dirs.is_empty() && missing_src_dirs.len() == options.src_dirs.len()`
    // short-circuits to `false` when `src_dirs` itself is empty (`!true` is
    // `false`), so an explicitly empty `src_dirs` bypassed both the "all
    // missing" diagnostic and the per-dir "missing" loop — a silent zero-file,
    // zero-diagnostic run.

    #[test]
    fn test_extract_empty_src_dirs_produces_diagnostic() {
        let tmp = TempDir::new().unwrap();
        let options = PipelineOptions {
            src_dirs: vec![],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        assert!(output.components.is_empty());
        assert_eq!(output.stats.files_parsed, 0);
        let error = output
            .diagnostics
            .iter()
            .find(|d| matches!(d.severity, DiagnosticSeverity::Error) && d.code == DiagnosticCode::IoError)
            .expect("empty src_dirs should produce an Error/IoError diagnostic, not a silent empty run");
        assert!(
            error.message.to_lowercase().contains("no source director"),
            "expected the diagnostic to explain that no source directories were configured, got: {}",
            error.message
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_extract_empty_src_dirs_produces_diagnostic -- --nocapture` Expected: FAIL — `.expect(...)` panics: `output.diagnostics` is empty because the current guard's `&&` short-circuit skips both branches when `src_dirs` is empty.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/pipeline/mod.rs`, replace the Phase 0 guard:

```rust
    let missing_src_dirs: Vec<&Utf8PathBuf> =
        options.src_dirs.iter().filter(|dir| !dir.as_std_path().is_dir()).collect();
    if options.src_dirs.is_empty() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "No source directories configured — src_dirs is empty".into(),
            file: None,
            line: None,
            column: None,
            help: Some("Set --src (or docgen.config.ts srcDirs) to at least one directory to scan.".into()),
            code: DiagnosticCode::IoError,
        });
    } else if missing_src_dirs.len() == options.src_dirs.len() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "None of the configured source directories exist: {}",
                options.src_dirs.iter().map(|dir| dir.as_str()).collect::<Vec<_>>().join(", ")
            ),
            file: options.src_dirs.first().map(ToString::to_string),
            line: None,
            column: None,
            help: Some("Check --src (or docgen.config.ts srcDirs) for a typo'd or stale path.".into()),
            code: DiagnosticCode::IoError,
        });
    } else {
        for dir in missing_src_dirs {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("Source directory does not exist: {dir}"),
                file: Some(dir.to_string()),
                line: None,
                column: None,
                help: Some("Check --src (or docgen.config.ts srcDirs) for a typo'd or stale path.".into()),
                code: DiagnosticCode::IoError,
            });
        }
    }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_extract_empty_src_dirs_produces_diagnostic -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/mod.rs
git commit -m "fix(pipeline): emit a diagnostic when src_dirs is explicitly empty"
```

### Task 4: `components.insert` collision (same key, different entry) emits a diagnostic

**Files:**

- Modify: `crates/core/src/pipeline/mod.rs:384-388`
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/pipeline/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
    // ── test_overlapping_src_dirs_causing_a_key_collision_emits_a_diagnostic ─
    //
    // Bug C (root-cause-analysis.md): `components.insert(key, entry)` in Phase
    // 5 discarded the `Option<ComponentEntry>` `BTreeMap::insert` already hands
    // back — a same-key collision (three or more resolutions landing on the
    // identical disambiguated key) silently overwrote the earlier entry with
    // zero diagnostic. Listing the same directory three times reproduces the
    // same insert-time collision an overlapping-src_dirs config produces: the
    // same file is discovered and resolved three times (component_mappings is
    // a plain Vec extended per merge, no dedup — see types/global.rs), and the
    // 3rd occurrence's disambiguated key ("Button (<path>)") collides with the
    // 2nd's exactly.

    #[test]
    fn test_overlapping_src_dirs_causing_a_key_collision_emits_a_diagnostic() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = TempDir::new_in(manifest_dir).unwrap();
        write_file(&tmp, "Button.tsx", "export function Button(props: { a?: string }) { return null; }\n");

        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let options = PipelineOptions {
            src_dirs: vec![dir.clone(), dir.clone(), dir],
            cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
            ..Default::default()
        };

        let output = extract(&options);

        let collision = output
            .diagnostics
            .iter()
            .find(|d| d.message.contains("Button") && d.message.to_lowercase().contains("duplicate"))
            .expect("expected a diagnostic naming the colliding component, got none");
        assert!(
            collision.message.contains("Button.tsx"),
            "expected the diagnostic to name the colliding file path, got: {}",
            collision.message
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_overlapping_src_dirs_causing_a_key_collision_emits_a_diagnostic -- --nocapture` Expected: FAIL — `.expect(...)` panics: `components.insert(key, entry)`'s return value is discarded, so no collision diagnostic is ever pushed even though the 3rd resolution silently overwrote the 2nd.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/pipeline/mod.rs`, replace:

```rust
        let mut entry = entry;
        options.plugins.run_on_component_resolved(&mut entry);
        components.insert(key, entry);
        diagnostics.extend(diags);
```

with:

```rust
        let mut entry = entry;
        options.plugins.run_on_component_resolved(&mut entry);
        if let Some(previous) = components.insert(key.clone(), entry.clone()) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "Duplicate component key '{key}' — colliding file paths: previously '{}', now '{}'",
                    previous.file_path, entry.file_path
                ),
                file: Some(entry.file_path.to_string()),
                line: None,
                column: None,
                help: Some(
                    "Two resolved components produced the same display name and disambiguation key — only \
                     the later one is kept in the output. Check for overlapping src_dirs or a genuine \
                     duplicate declaration."
                        .into(),
                ),
                code: DiagnosticCode::Unknown,
            });
        }
        diagnostics.extend(diags);
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core test_overlapping_src_dirs_causing_a_key_collision_emits_a_diagnostic -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/pipeline/mod.rs
git commit -m "fix(pipeline): emit a diagnostic on component-key insert collisions"
```

---

## Part E: Allocation caps + LSP scaffold hardening

Confirmed: `PropType::Opaque { raw, reason }` shape is still current (cluster #1's `OpaqueDetail` refactor has not landed) — my Task 1 uses the current shape and doesn't touch construction at all, so no dependency risk either way.

Now writing the task group.

### Task 1: Cap template-literal Cartesian-product expansion

**Files:**

- Modify: `crates/core/src/resolver/template.rs:44-107`
- Test: inline `#[cfg(test)] mod tests` in the same file (this crate's convention per `resolver/mod.rs:399`, `resolver/import.rs:22`, `resolver/react.rs:210` — `template.rs` currently has no test module, so this adds one)

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::pipeline::PipelineOptions;

    fn empty_ctx() -> ResolutionContext {
        let global = Arc::new(GlobalSourceData::default());
        let options = PipelineOptions::default();
        ResolutionContext::new(global, &options)
    }

    /// 5 parts x 10-member unions = 10^5 = 100,000 combinations — without a
    /// cap this builds a 100k-entry `Vec<String>` (and grows exponentially
    /// with more parts/members, the unbounded-allocation shape from
    /// P1-1/P1-2). `MAX_TEMPLATE_LITERAL_EXPANSIONS` (4096) must stop the
    /// accumulation loop well before that.
    fn oversized_parts() -> Vec<CollectedType> {
        let member = |part: usize, i: usize| CollectedType::StringLiteral(format!("p{part}v{i}").into());
        (0..5).map(|part| CollectedType::Union((0..10).map(|i| member(part, i)).collect())).collect()
    }

    #[test]
    fn test_try_expand_template_literal_returns_none_past_cap() {
        let ctx = empty_ctx();
        let mut state = ResolveState::default();
        let file = Utf8PathBuf::from("/test/button.tsx");

        let result = try_expand_template_literal(&oversized_parts(), &file, &ctx, &mut state, 0);
        assert!(result.is_none(), "expected None once the Cartesian product exceeds MAX_TEMPLATE_LITERAL_EXPANSIONS");
    }

    #[test]
    fn test_template_literal_expansion_caps_and_degrades_to_opaque() {
        let ctx = empty_ctx();
        let mut state = ResolveState::default();
        let file = Utf8PathBuf::from("/test/button.tsx");

        let result = resolve_template_literal(&oversized_parts(), &file, &ctx, &mut state, 0);

        match result {
            PropType::Opaque { reason: OpaqueReason::TemplateLiteral { .. }, .. } => {}
            other => panic!("expected capped expansion to degrade to Opaque(TemplateLiteral), got {other:?}"),
        }
        assert!(
            state.diagnostics.iter().any(|d| d.code == DiagnosticCode::TemplateLiteralOpaque),
            "expected a TemplateLiteralOpaque diagnostic to be recorded, got {:?}",
            state.diagnostics
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core --lib resolver::template::tests -- --nocapture` Expected: FAIL — `test_try_expand_template_literal_returns_none_past_cap` fails because today's unbounded loop actually computes all 100,000 combinations and returns `Some(...)` instead of `None` (slow and non-degrading, not the capped behavior); `test_template_literal_expansion_caps_and_degrades_to_opaque` fails because `result` is a `PropType::LiteralUnion` with 100,000 members, not `Opaque`.

- [ ] **Step 3: Write minimal implementation**

```rust
/// Cap on the Cartesian product `try_expand_template_literal` accumulates
/// across a template literal's parts. Each part can itself be a union with
/// many members, so the product is exponential in part count — an attacker-
/// or generator-controlled type with enough parts/members would otherwise
/// allocate unboundedly before ever returning. Same shape of guard as
/// `extractor/mod.rs`'s `MAX_SOURCE_NESTING_DEPTH`, applied to a different
/// input-derived size.
const MAX_TEMPLATE_LITERAL_EXPANSIONS: usize = 4096;
```

```rust
    // Cartesian product across all parts.
    let mut result = vec![String::new()];
    for alternatives in per_part {
        if result.len().saturating_mul(alternatives.len()) > MAX_TEMPLATE_LITERAL_EXPANSIONS {
            return None; // Falls through to the Opaque+diagnostic degrade path in resolve_template_literal.
        }
        let mut next = Vec::with_capacity(result.len() * alternatives.len());
        for prefix in &result {
            for alt in &alternatives {
                next.push(format!("{}{}", prefix, alt));
            }
        }
        result = next;
    }

    Some(result)
```

(insert the `const` above `try_expand_template_literal`'s doc comment at `template.rs:42`; replace the existing Cartesian-product loop at `template.rs:94-104` with the capped version above — `resolve_template_literal` at `template.rs:22-39` already handles `None` by emitting `PropType::Opaque { raw, reason: OpaqueReason::TemplateLiteral { .. } }` plus a `TemplateLiteralOpaque` diagnostic, so no change is needed there.)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core --lib resolver::template::tests -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/template.rs
git commit -m "fix(resolver): cap template-literal Cartesian expansion to prevent unbounded allocation"
```

### Task 2: Cap LSP `Content-Length` before allocating the receive buffer

**Files:**

- Modify: `crates/cli/src/commands/lsp.rs:1-93`
- Test: inline `#[cfg(test)] mod tests` in the same file (new module — `lsp.rs` has no tests today; `crates/cli/tests/` holds `trycmd`-style CLI integration tests per `crates/cli/tests/cmd/`, not the right place for a pure-function unit test)

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oversized_content_length_is_rejected() {
        let result = check_content_length(MAX_LSP_MESSAGE_BYTES + 1);
        assert!(result.is_err(), "expected a Content-Length past the cap to be rejected, got {:?}", result);
    }

    #[test]
    fn test_content_length_at_cap_is_accepted() {
        let result = check_content_length(MAX_LSP_MESSAGE_BYTES);
        assert!(result.is_ok(), "expected a Content-Length exactly at the cap to be accepted, got {:?}", result);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli --lib commands::lsp::tests -- --nocapture` Expected: FAIL with a compile error — `check_content_length` and `MAX_LSP_MESSAGE_BYTES` don't exist yet.

- [ ] **Step 3: Write minimal implementation**

```rust
/// Cap on a single LSP frame's declared `Content-Length`, checked *before*
/// `vec![0u8; len]` allocates — `Content-Length` is client-controlled input
/// with no upper bound of its own (P1-2).
const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

/// Reject an oversized frame before its receive buffer is allocated.
fn check_content_length(len: usize) -> Result<()> {
    if len > MAX_LSP_MESSAGE_BYTES {
        tracing::error!(
            "LSP frame declared Content-Length {len} bytes, exceeding the {MAX_LSP_MESSAGE_BYTES}-byte cap; closing connection"
        );
        return Err(miette::miette!(
            "LSP frame declared Content-Length {len} bytes, exceeding the {MAX_LSP_MESSAGE_BYTES}-byte cap"
        ));
    }
    Ok(())
}
```

Wire it in at the existing allocation site (`lsp.rs:32-36`):

```rust
        let Some(len) = content_length else {
            continue;
        };

        check_content_length(len)?;

        let mut buf = vec![0u8; len];
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli --lib commands::lsp::tests -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/lsp.rs
git commit -m "fix(lsp): reject oversized Content-Length before allocating the receive buffer"
```

### Task 3: Stop advertising `hoverProvider: true` with no hover handler

**Files:**

- Modify: `crates/cli/src/commands/lsp.rs` (the `initialize` arm, originally at lines 48-66)
- Test: same inline `mod tests` from Task 2

Note: chose "stop advertising the capability" per the assignment's recommendation — `lsp.rs` has no `textDocument/hover` handler in its method `match` at all (just the `_ => {}` catch-all), and implementing a real hover handler (resolving a document position back to a component/prop and formatting a docstring) is a genuinely separate feature, not a bounded fix for this cluster.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn test_hover_not_advertised_without_a_handler() {
        let result = initialize_result();
        assert_eq!(
            result["capabilities"]["hoverProvider"],
            serde_json::json!(false),
            "hoverProvider must stay false until a textDocument/hover handler exists — a client hover \
             request carries an id and nothing replies to it today"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli --lib commands::lsp::tests -- --nocapture` Expected: FAIL with a compile error — `initialize_result` doesn't exist yet (the capabilities object is currently inlined directly in the `"initialize"` match arm with `hoverProvider: true`).

- [ ] **Step 3: Write minimal implementation**

Extract the `result` payload out of the inline `json!()` at `lsp.rs:50-63` into a named function, and flip the flag:

```rust
/// The `initialize` response's `result` payload — factored out so capability
/// advertisement is testable without driving the stdin/stdout loop.
fn initialize_result() -> serde_json::Value {
    serde_json::json!({
        "capabilities": {
            "textDocumentSync": 1,
            // No textDocument/hover handler exists in the method dispatch
            // below (it falls into the `_ => {}` catch-all) — advertising
            // `true` would make a client's hover request, which carries an
            // `id`, hang forever waiting for a response that never comes.
            "hoverProvider": false
        },
        "serverInfo": {
            "name": "oxc-react-docgen-lsp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}
```

Update the `"initialize"` arm to use it:

```rust
                "initialize" => {
                    if let Some(id) = id {
                        let response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": initialize_result()
                        });
                        send_response(&mut stdout_lock, &response);
                    }
                }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli --lib commands::lsp::tests -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/lsp.rs
git commit -m "fix(lsp): stop advertising hoverProvider with no hover handler behind it"
```

### Task 4: Resync (don't silently `continue`) past a malformed/headerless frame

**Files:**

- Modify: `crates/cli/src/commands/lsp.rs` (the header-reading loop, originally at lines 15-34)
- Test: same inline `mod tests` from Task 2/3

Depends on Task 2 (`check_content_length`) and Task 3 (`initialize_result`) having landed first, since this task's `Step 3` builds on the loop shape they leave behind.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn test_headerless_frame_is_reported_as_malformed_not_eof() {
        // Blank line reached with no Content-Length header at all — the
        // pre-fix code left `content_length: None` and `continue`d straight
        // past the body that frame's sender still wrote, permanently
        // desyncing every header block read afterward.
        let mut reader = std::io::Cursor::new(b"X-Some-Other-Header: value\r\n\r\n".to_vec());
        assert_eq!(read_content_length(&mut reader), Ok(None));
    }

    #[test]
    fn test_unparsable_content_length_is_reported_as_malformed() {
        let mut reader = std::io::Cursor::new(b"Content-Length: not-a-number\r\n\r\n".to_vec());
        assert_eq!(read_content_length(&mut reader), Ok(None));
    }

    #[test]
    fn test_true_stream_eof_is_distinct_from_a_malformed_header_block() {
        // No bytes at all (client closed the connection) is a normal,
        // expected way to end the loop — must not be conflated with a
        // header block that was read but had no usable Content-Length.
        let mut reader = std::io::Cursor::new(Vec::new());
        assert_eq!(read_content_length(&mut reader), Err(()));
    }
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli --lib commands::lsp::tests -- --nocapture` Expected: FAIL with a compile error — `read_content_length` doesn't exist yet (the header-reading loop is still inlined directly in `cmd_lsp`'s outer `loop`).

- [ ] **Step 3: Write minimal implementation**

Extract the header-reading loop (`lsp.rs:15-30`) into a testable function that distinguishes true EOF from a malformed (no-usable-length) header block:

```rust
/// Reads header lines up to the blank-line terminator and returns the parsed
/// `Content-Length`, if any.
///
/// `Err(())` means the stream ended before any header line was read at all —
/// a normal, expected way for a client to close the connection.
/// `Ok(None)` means a header block *was* read (the loop reached the blank
/// line) but it contained no usable `Content-Length` — missing header or an
/// unparsable value. That body's byte length is unknowable, so the caller
/// must not keep reading from the stream as if nothing happened: doing so
/// desyncs every subsequent frame, since the unconsumed body bytes get read
/// as the start of the *next* header block.
fn read_content_length<R: BufRead>(reader: &mut R) -> Result<Option<usize>, ()> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return Err(());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(header) = line.strip_prefix("Content-Length: ") {
            content_length = header.trim().parse().ok();
        }
    }

    Ok(content_length)
}
```

Replace `cmd_lsp`'s inner header loop and the `content_length` handling (`lsp.rs:16-34`) with:

```rust
    loop {
        let content_length = match read_content_length(&mut stdin_lock) {
            Err(()) => return Ok(()), // EOF
            Ok(v) => v,
        };

        let Some(len) = content_length else {
            tracing::error!(
                "LSP frame's header block had no usable Content-Length; closing connection to avoid a desynced stream"
            );
            return Err(miette::miette!("received an LSP frame without a usable Content-Length header"));
        };

        check_content_length(len)?;

        let mut buf = vec![0u8; len];
```

(`check_content_length` is Task 2's function; the rest of the loop body — `read_exact`, the `serde_json::from_slice` match, the method dispatch — is unchanged.)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli --lib commands::lsp::tests -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/lsp.rs
git commit -m "fix(lsp): close the connection on a malformed header block instead of desyncing the stream"
```

---

## Part F: TOON truncation + schema drift + CLI exit-code contract

Now I have everything needed. Here are the tasks.

### Task 1: TOON — shared truncate-with-indicator helper for Union/Intersection

**Files:**

- Modify: `crates/core/src/toon.rs:95-145` (the `format_type_compact` function and its `LiteralUnion`/`Union`/`Intersection` branches)
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/toon.rs` (existing convention — see `test_format_type_compact_complex_types`)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_format_type_compact_union_truncates_with_indicator() {
    let union = PropType::Union(vec![
        PropType::StringLiteral("a".into()),
        PropType::StringLiteral("b".into()),
        PropType::StringLiteral("c".into()),
        PropType::StringLiteral("d".into()),
        PropType::StringLiteral("e".into()),
        PropType::StringLiteral("f".into()),
    ]);
    let out = format_type_compact(&union);
    assert!(out.contains("...(+2)"), "expected a truncation indicator for the 2 dropped members, got: {out}");
}

#[test]
fn test_format_type_compact_intersection_truncates_with_indicator() {
    let intersection = PropType::Intersection(vec![
        PropType::Named { name: "A".into(), args: vec![] },
        PropType::Named { name: "B".into(), args: vec![] },
        PropType::Named { name: "C".into(), args: vec![] },
        PropType::Named { name: "D".into(), args: vec![] },
        PropType::Named { name: "E".into(), args: vec![] },
    ]);
    let out = format_type_compact(&intersection);
    assert!(out.contains("...(+1)"), "expected a truncation indicator for the 1 dropped member, got: {out}");
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core test_format_type_compact_union_truncates_with_indicator -- --nocapture` Expected: FAIL — current `Union` branch does `members.iter().take(4).map(format_type_compact).collect::<Vec<_>>().join("|")` with no `"...(+N)"` marker, so `out` is `"\"a\"|\"b\"|\"c\"|\"d\""` and does not contain `"...(+2)"`.

- [ ] **Step 3: Write minimal implementation** Add the helper (built from the existing `LiteralUnion` logic) and route all three branches through it:

```rust
/// Truncate `parts` to `limit` items, appending a `"...(+N)"` marker for the
/// remainder instead of silently dropping them. Shared by every
/// `format_type_compact` branch that renders a bounded member list.
fn truncate_with_indicator(parts: &[String], limit: usize, sep: &str) -> String {
    if parts.len() <= limit {
        return parts.join(sep);
    }
    let mut shown: Vec<String> = parts[..limit].to_vec();
    shown.push(format!("...(+{})", parts.len() - limit));
    shown.join(sep)
}
```

Replace the three branches inside `format_type_compact`:

```rust
        PropType::LiteralUnion { members, .. } => truncate_with_indicator(members, 6, "|"),
```

```rust
        PropType::Union(members) => {
            let formatted: Vec<String> = members.iter().map(format_type_compact).collect();
            truncate_with_indicator(&formatted, 4, "|")
        }
        PropType::Intersection(members) => {
            let formatted: Vec<String> = members.iter().map(format_type_compact).collect();
            truncate_with_indicator(&formatted, 4, "&")
        }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core toon:: -- --nocapture` Expected: PASS — all `toon.rs` tests pass, including the two new ones and the pre-existing `test_format_type_compact_complex_types` (which asserts `"a|b|c|d|e|f|...(+2)"` for an 8-member `LiteralUnion`, still exact-matching through the new helper).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/toon.rs
git commit -m "fix(toon): show truncation indicator on Union/Intersection like LiteralUnion already does"
```

### Task 2: Derive-free schema drift fix — extract `schema_value()`, add a drift-detection test, close the gaps it finds

**Note on approach:** Considered deriving `schemars::JsonSchema` (root-cause-analysis.md's proposed ADR-0006) but rejected it here: `schemars` isn't a dependency anywhere in the workspace today (`grep schemars Cargo.toml` — nothing), and doing it properly means hand-writing `JsonSchema` impls for `PropType` (16 variants, ~250 lines in its existing hand-written `Serialize` impl alone), `CollectedType`, and `OpaqueReason`, mirroring ADR-0002's precedent. That's a multi-day addition, not a bite-sized fix, and this cluster is grouped with two unrelated small fixes. Taking the floor-level option instead: a drift-detection test that serializes a real `ExtractionOutput` and checks every field name it produces shows up somewhere in `schema.rs`'s hand-written schema. No ADR — root-cause-analysis.md's own text says ADR only applies if option (a) is taken.

**Files:**

- Modify: `crates/cli/src/commands/schema.rs` (whole file — extract the `json!()` literal out of `cmd_schema` into a `schema_value()` function, then fix the drift the new test finds)
- Test: inline `#[cfg(test)] mod tests` in `crates/cli/src/commands/schema.rs` (existing convention — the file already has one test there)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn schema_covers_every_field_name_the_real_output_serializes() {
    use oxc_react_docgen_core::types::{
        ComponentEntry, DefaultValue, Diagnostic, DiagnosticCode, DiagnosticSeverity, ExtractionOutput,
        ExtractionStats, InheritedLayer, ParsedProp, PropParent, PropType,
    };
    use std::collections::BTreeMap;

    let mut props = BTreeMap::new();
    props.insert(
        "variant".to_string(),
        ParsedProp {
            name: "variant".into(),
            prop_type: PropType::String,
            required: true,
            default_value: Some(DefaultValue { value: "\"a\"".into(), computed: false }),
            description: "desc".into(),
            tags: BTreeMap::new(),
            parent: Some(PropParent { name: "ButtonProps".into(), file_name: "Button.tsx".into() }),
            declarations: vec![],
        },
    );

    let mut components = BTreeMap::new();
    components.insert(
        "Button".to_string(),
        ComponentEntry {
            display_name: "Button".into(),
            file_path: "src/Button.tsx".into(),
            description: "A button".into(),
            props,
            inheritance: vec![InheritedLayer {
                type_name: "ButtonHTMLAttributes".into(),
                file_name: "react.d.ts".into(),
                omitted: vec![],
                html_element: Some("button".into()),
                total_props: 3,
            }],
            notable_inherited: BTreeMap::new(),
            discriminant_prop: Some("variant".into()),
            composes: vec!["SomeUnresolved".into()],
            tags: BTreeMap::from([("deprecated".to_string(), String::new())]),
            methods: vec![],
        },
    );

    let output = ExtractionOutput {
        components,
        enums: BTreeMap::new(),
        diagnostics: vec![Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: "msg".into(),
            file: Some("Button.tsx".into()),
            line: Some(10),
            column: Some(4),
            help: Some("try this".into()),
            code: DiagnosticCode::OpaqueType,
        }],
        stats: ExtractionStats {
            components_extracted: 1,
            components_skipped: 1,
            files_parsed: 1,
            dts_files_parsed: 1,
            dts_cache_hits: 0,
            duration_ms: 5,
            tier1_count: 1,
            tier3_count: 1,
            opaque_count: 1,
        },
    };

    let value = serde_json::to_value(&output).expect("ExtractionOutput must serialize");
    let mut real_fields: Vec<String> = Vec::new();
    for key in ["components", "diagnostics", "stats"] {
        assert!(value.get(key).is_some(), "fixture is missing top-level key {key}");
    }
    if let Some(obj) = value["components"]["Button"].as_object() {
        real_fields.extend(obj.keys().cloned());
    }
    if let Some(obj) = value["components"]["Button"]["props"]["variant"].as_object() {
        real_fields.extend(obj.keys().cloned());
    }
    if let Some(obj) = value["components"]["Button"]["inheritance"][0].as_object() {
        real_fields.extend(obj.keys().cloned());
    }
    if let Some(obj) = value["diagnostics"][0].as_object() {
        real_fields.extend(obj.keys().cloned());
    }
    if let Some(obj) = value["stats"].as_object() {
        real_fields.extend(obj.keys().cloned());
    }

    let schema_str = serde_json::to_string(&schema_value()).expect("schema must serialize");
    let missing: Vec<&String> = real_fields.iter().filter(|f| !schema_str.contains(f.as_str())).collect();
    assert!(missing.is_empty(), "schema.rs is missing field(s) present in real serialized output: {missing:?}");
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli schema_covers_every_field_name -- --nocapture` Expected: FAIL — first on compilation, because `schema_value()` doesn't exist yet (`cmd_schema` builds the `json!()` literal inline and returns `Result<()>`); after that function is extracted it still FAILs the assertion, reporting real fields missing from `schema.rs`'s hand-written schema: `tags`, `methods` (on the component), `tags`, `parent`, `declarations` (on the prop), `typeName`/`fileName`/`omitted`/`htmlElement`/`totalProps` (inheritance has no `items` schema at all today), `line`, `column`, `help` (on diagnostics), and `componentsSkipped`, `dtsFilesParsed`, `tier1Count`, `tier3Count`, `opaqueCount` (on stats).

- [ ] **Step 3: Write minimal implementation** Replace the whole file's non-test content:

```rust
use miette::Result;

/// Build the JSON Schema for the oxc-react-docgen `ExtractionOutput` format.
/// Hand-maintained per ADR-0002-style precedent — see the drift-detection
/// test below for the guard against it silently falling out of sync with the
/// real `ComponentEntry`/`ExtractionStats`/`Diagnostic` structs.
fn schema_value() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ExtractionOutput",
        "description": "Output schema for oxc-react-docgen component prop extraction",
        "type": "object",
        "required": ["components", "enums", "diagnostics", "stats"],
        "properties": {
            "components": {
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "required": ["displayName", "filePath", "props"],
                    "properties": {
                        "displayName": { "type": "string" },
                        "filePath": { "type": "string" },
                        "description": { "type": ["string", "null"] },
                        "props": {
                            "type": "object",
                            "additionalProperties": {
                                "type": "object",
                                "required": ["name", "required", "type"],
                                "properties": {
                                    "name": { "type": "string" },
                                    "required": { "type": "boolean" },
                                    "type": { "type": "object" },
                                    "description": { "type": ["string", "null"] },
                                    "tags": { "type": "object" },
                                    "parent": { "type": ["object", "null"] },
                                    "declarations": { "type": "array" }
                                }
                            }
                        },
                        "inheritance": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "typeName": { "type": "string" },
                                    "fileName": { "type": "string" },
                                    "omitted": { "type": "array", "items": { "type": "string" } },
                                    "htmlElement": { "type": ["string", "null"] },
                                    "totalProps": { "type": "integer" }
                                }
                            }
                        },
                        "notableInherited": { "type": "object" },
                        "discriminantProp": { "type": ["string", "null"] },
                        "composes": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "tags": { "type": "object" },
                        "methods": { "type": "array" }
                    }
                }
            },
            "enums": { "type": "object" },
            "diagnostics": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["severity", "message", "code"],
                    "properties": {
                        "severity": { "type": "string", "enum": ["error", "warning", "info"] },
                        "message": { "type": "string" },
                        "file": { "type": ["string", "null"] },
                        "line": { "type": ["integer", "null"] },
                        "column": { "type": ["integer", "null"] },
                        "help": { "type": ["string", "null"] },
                        "code": { "type": "string" }
                    }
                }
            },
            "stats": {
                "type": "object",
                "required": ["componentsExtracted", "filesParsed", "durationMs"],
                "properties": {
                    "componentsExtracted": { "type": "integer" },
                    "componentsSkipped": { "type": "integer" },
                    "filesParsed": { "type": "integer" },
                    "dtsFilesParsed": { "type": "integer" },
                    "dtsCacheHits": { "type": "integer" },
                    "durationMs": { "type": "integer" },
                    "tier1Count": { "type": "integer" },
                    "tier3Count": { "type": "integer" },
                    "opaqueCount": { "type": "integer" }
                }
            }
        }
    })
}

/// Output JSON schema for the oxc-react-docgen ExtractionOutput format.
pub fn cmd_schema() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&schema_value()).unwrap_or_default());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_schema_valid_json() {
        assert!(cmd_schema().is_ok());
    }

    // schema_covers_every_field_name_the_real_output_serializes goes here (Step 1)
}
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli -p oxc-react-docgen-core schema:: -- --nocapture` (or simply `cargo test -p oxc-react-docgen-cli`) Expected: PASS — both `test_cmd_schema_valid_json` and `schema_covers_every_field_name_the_real_output_serializes` pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/schema.rs
git commit -m "fix(cli): close ComponentEntry/ExtractionStats/Diagnostic drift in the hand-written schema, guard it with a drift test"
```

### Task 3: `ExtractionOutput::max_severity()` / `exit_code()`

**Files:**

- Modify: `crates/core/src/types/output.rs:1-25` (imports and the `ExtractionOutput` struct — add the impl block right after it, before `InheritedLayer`)
- Test: inline `#[cfg(test)] mod tests` in `crates/core/src/types/output.rs` (existing convention)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn exit_code_is_zero_with_no_diagnostics() {
    let output = ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: vec![],
        stats: ExtractionStats::default(),
    };
    assert_eq!(output.exit_code(false), 0);
    assert_eq!(output.exit_code(true), 0);
}

#[test]
fn exit_code_is_two_when_any_diagnostic_is_error_severity() {
    let output = ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: vec![crate::types::diagnostic::Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "boom".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: crate::types::diagnostic::DiagnosticCode::Unknown,
        }],
        stats: ExtractionStats::default(),
    };
    assert_eq!(output.exit_code(false), 2);
    assert_eq!(output.exit_code(true), 2);
}

#[test]
fn exit_code_is_one_only_when_strict_and_a_warning_is_present() {
    let output = ExtractionOutput {
        components: BTreeMap::new(),
        enums: BTreeMap::new(),
        diagnostics: vec![crate::types::diagnostic::Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: "heads up".into(),
            file: None,
            line: None,
            column: None,
            help: None,
            code: crate::types::diagnostic::DiagnosticCode::Unknown,
        }],
        stats: ExtractionStats::default(),
    };
    assert_eq!(output.exit_code(false), 0, "non-strict must not fail on warnings");
    assert_eq!(output.exit_code(true), 1, "strict must fail on warnings");
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core exit_code_is -- --nocapture` Expected: FAIL to compile — `ExtractionOutput` has no `exit_code` method yet.

- [ ] **Step 3: Write minimal implementation** Change the top-of-file import and add the impl block right after the `ExtractionOutput` struct definition:

```rust
use super::diagnostic::{Diagnostic, DiagnosticSeverity};
```

```rust
impl ExtractionOutput {
    /// Highest-severity diagnostic present, if any — `Error` outranks
    /// `Warning` outranks `Info`.
    pub fn max_severity(&self) -> Option<DiagnosticSeverity> {
        self.diagnostics.iter().map(|d| d.severity.clone()).max_by_key(|s| match s {
            DiagnosticSeverity::Error => 2,
            DiagnosticSeverity::Warning => 1,
            DiagnosticSeverity::Info => 0,
        })
    }

    /// Process exit code for this output: `2` if any diagnostic is
    /// `Error`-severity, `1` if `strict` and any diagnostic is at least
    /// `Warning`-severity, `0` otherwise. This is the CLI's shared exit-code
    /// contract — `oxc-react-docgen check --strict`'s mapping, reused as-is
    /// by `extract`, `watch`, and `inspect`.
    pub fn exit_code(&self, strict: bool) -> i32 {
        match self.max_severity() {
            Some(DiagnosticSeverity::Error) => 2,
            Some(DiagnosticSeverity::Warning) => {
                if strict {
                    1
                } else {
                    0
                }
            }
            Some(DiagnosticSeverity::Info) => 0,
            None => 0,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core exit_code_is -- --nocapture` Expected: PASS — all three new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/output.rs
git commit -m "feat(core): add ExtractionOutput::exit_code, the shared CLI exit-code contract"
```

### Task 4: Route `extract.rs` and `check.rs` through `exit_code()`

**Files:**

- Modify: `crates/cli/src/commands/extract.rs:67-73`
- Modify: `crates/cli/src/commands/check.rs:20-40`

Depends on Task 3. This is a pure refactor — no behavior change — proven by the two files' existing tests passing unmodified.

- [ ] **Step 1: Write the failing test** No new test — the existing tests already pin the exact behavior this refactor must preserve:
- `crates/cli/src/commands/extract.rs`'s `json_mode_still_returns_the_error_exit_code` and `non_json_mode_returns_the_same_error_exit_code`
- `crates/cli/src/commands/check.rs`'s `json_mode_still_returns_the_error_exit_code`

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli -- --nocapture` Expected: PASS as-is right now (nothing changed yet) — this step confirms the baseline the refactor must not break.

- [ ] **Step 3: Write minimal implementation** In `extract.rs`, replace:

```rust
    // Must run regardless of --json — this is the one thing CI actually
    // depends on the exit code for.
    let has_errors = output
        .diagnostics
        .iter()
        .any(|d| matches!(d.severity, oxc_react_docgen_core::types::DiagnosticSeverity::Error));

    Ok(if has_errors { 2 } else { 0 })
```

with:

```rust
    // Must run regardless of --json — this is the one thing CI actually
    // depends on the exit code for.
    Ok(output.exit_code(false))
```

In `check.rs`, replace:

```rust
    let errors: Vec<_> = output
        .diagnostics
        .iter()
        .filter(|d| matches!(d.severity, oxc_react_docgen_core::types::DiagnosticSeverity::Error))
        .collect();
    let warnings: Vec<_> = output
        .diagnostics
        .iter()
        .filter(|d| matches!(d.severity, oxc_react_docgen_core::types::DiagnosticSeverity::Warning))
        .collect();

    if args.json {
        println!("{}", serde_json::to_string(&output.diagnostics).into_diagnostic()?);
    } else if !quiet {
        print_summary(&output, quiet);
        print_diagnostics(&output.diagnostics);
    }

    if !errors.is_empty() {
        return Ok(2);
    }
    if args.strict && !warnings.is_empty() {
        return Ok(1);
    }

    Ok(0)
```

with:

```rust
    if args.json {
        println!("{}", serde_json::to_string(&output.diagnostics).into_diagnostic()?);
    } else if !quiet {
        print_summary(&output, quiet);
        print_diagnostics(&output.diagnostics);
    }

    Ok(output.exit_code(args.strict))
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli -- --nocapture` Expected: PASS — same three tests, unmodified, still pass, proving the refactor changed nothing observable.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/extract.rs crates/cli/src/commands/check.rs
git commit -m "refactor(cli): collapse extract/check's inline exit-code checks onto ExtractionOutput::exit_code"
```

### Task 5: `cmd_watch` surfaces the exit code instead of hardcoding 0

**Files:**

- Modify: `crates/cli/src/commands/watch.rs` (whole file)
- Modify: `crates/cli/src/main.rs:190-193` (the `Command::Watch` dispatch arm)
- Test: inline `#[cfg(test)] mod tests` in `crates/cli/src/commands/watch.rs` (new — file currently has no test module)

Depends on Task 3. `cmd_watch` itself blocks on a `watchexec` event loop and spawns threads (it cannot be driven end-to-end in a unit test — the crate has never had a test for it for the same reason), so the new test targets the one piece of this change that is pure and testable: computing the exit code from an `ExtractionOutput`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_exit_code_mirrors_extraction_output_exit_code_non_strict() {
        let clean = oxc_react_docgen_core::types::ExtractionOutput {
            components: Default::default(),
            enums: Default::default(),
            diagnostics: vec![],
            stats: Default::default(),
        };
        assert_eq!(watch_exit_code(&clean), 0);

        let with_error = oxc_react_docgen_core::types::ExtractionOutput {
            components: Default::default(),
            enums: Default::default(),
            diagnostics: vec![oxc_react_docgen_core::types::Diagnostic {
                severity: oxc_react_docgen_core::types::DiagnosticSeverity::Error,
                message: "boom".into(),
                file: None,
                line: None,
                column: None,
                help: None,
                code: oxc_react_docgen_core::types::DiagnosticCode::Unknown,
            }],
            stats: Default::default(),
        };
        assert_eq!(watch_exit_code(&with_error), 2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli watch_exit_code -- --nocapture` Expected: FAIL to compile — `watch_exit_code` doesn't exist yet in `watch.rs`.

- [ ] **Step 3: Write minimal implementation** Add the seam function near the top of `watch.rs` (module-private, called from `cmd_watch`):

```rust
/// Watch mode never runs `--strict` — there's no CLI flag for it — so this
/// is always `exit_code(false)`. Named seam so `cmd_watch`'s wiring has
/// something unit-testable without spinning up watchexec.
fn watch_exit_code(output: &oxc_react_docgen_core::types::ExtractionOutput) -> i32 {
    output.exit_code(false)
}
```

Change the signature and thread an `AtomicI32` through the update loop:

```rust
pub fn cmd_watch(args: crate::WatchArgs, quiet: bool, config_path: Option<&str>) -> Result<i32> {
```

After `let first = session.initialize();`:

```rust
    let exit_code = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(watch_exit_code(&first)));
```

Before the `Watchexec::new(move |action| {` closure (alongside the other `_inner`/`_clone` captures), clone it in:

```rust
        let exit_code_inner = exit_code.clone();
```

Inside the closure, right after `let update = session_inner.update_file(&utf8);`, add:

```rust
                        exit_code_inner.store(
                            oxc_react_docgen_core::types::ExtractionOutput {
                                components: Default::default(),
                                enums: Default::default(),
                                diagnostics: update.diagnostics.clone(),
                                stats: Default::default(),
                            }
                            .exit_code(false),
                            std::sync::atomic::Ordering::Relaxed,
                        );
```

And at the very end of the function, replace:

```rust
    running.store(false, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}
```

with:

```rust
    running.store(false, std::sync::atomic::Ordering::Relaxed);
    Ok(exit_code.load(std::sync::atomic::Ordering::Relaxed))
}
```

Update `main.rs`'s dispatch arm:

```rust
        Command::Watch(args) => cmd_watch(args, cli.quiet, cli.config.as_deref())?,
```

(replacing the old `{ cmd_watch(...)?; 0 }` block.)

Note: `IncrementalUpdate`'s exact field set wasn't fully re-verified beyond `diagnostics: Vec<Diagnostic>` (confirmed at `crates/core/src/pipeline/mod.rs:119-122`) — if `ExtractionOutput`'s other required fields don't default-construct as shown, wrap `update.diagnostics.clone()` through `oxc_react_docgen_core::types::Diagnostic::exit_code`-equivalent logic directly instead (i.e. reuse `watch_exit_code`'s severity-ranking match inline over `&update.diagnostics`) rather than fabricating a placeholder `ExtractionOutput`.

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli watch_exit_code -- --nocapture && cargo build -p oxc-react-docgen-cli` Expected: PASS — the unit test passes and the crate builds clean with `cmd_watch: Result<i32>` wired through `main.rs`.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/watch.rs crates/cli/src/main.rs
git commit -m "fix(cli): stop hardcoding exit 0 for watch mode, surface real diagnostic severity"
```

### Task 6: `cmd_inspect` surfaces the exit code from diagnostics elsewhere in the tree

**Files:**

- Modify: `crates/cli/src/commands/inspect.rs` (whole file — signature + return)
- Modify: `crates/cli/src/main.rs:194-197` (the `Command::Inspect` dispatch arm)
- Test: inline `#[cfg(test)] mod tests` in `crates/cli/src/commands/inspect.rs` (new — file currently has no test module; follow `extract.rs`'s tempfile+camino fixture convention)

Depends on Task 3.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_surfaces_error_exit_code_from_diagnostics_elsewhere_in_the_tree() {
        let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = tempfile::TempDir::new_in(manifest_dir).unwrap();
        std::fs::write(
            tmp.path().join("Comp.tsx"),
            r#"
export interface CompProps { label: string; }
export function Comp(props: CompProps) { return null; }
"#,
        )
        .unwrap();
        // Deliberately malformed, same fixture shape as
        // extractor::tests::test_parse_error_surfaced_as_diagnostic — unclosed
        // interface body triggers a ParseError diagnostic (Error severity).
        std::fs::write(
            tmp.path().join("Bad.tsx"),
            r#"
export interface BrokenProps {
    label: string;
"#,
        )
        .unwrap();

        let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap();
        let args = crate::InspectArgs { component: "Comp".into(), src: vec![dir.to_string()] };
        let code = cmd_inspect(args, None).expect("cmd_inspect should find Comp and not error");
        assert_eq!(code, 2, "expected exit code 2: Bad.tsx has a parse error even though the inspected component is fine");
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli inspect_surfaces_error_exit_code -- --nocapture` Expected: FAIL to compile — `cmd_inspect` currently returns `Result<()>`, so `.expect(...)` binds `code: ()` and `assert_eq!(code, 2)` is a type error; after fixing the signature it still FAILs the assertion since `cmd_inspect` never checks `output.diagnostics` and always implicitly returns without an exit code.

- [ ] **Step 3: Write minimal implementation** Change the signature:

```rust
pub fn cmd_inspect(args: crate::InspectArgs, config_path: Option<&str>) -> Result<i32> {
```

Replace the final lines of the function:

```rust
    println!();
    Ok(())
}
```

with:

```rust
    println!();
    Ok(output.exit_code(false))
}
```

Update `main.rs`'s dispatch arm:

```rust
        Command::Inspect(args) => cmd_inspect(args, cli.config.as_deref())?,
```

(replacing the old `{ cmd_inspect(...)?; 0 }` block.)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli inspect_surfaces_error_exit_code -- --nocapture` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/inspect.rs crates/cli/src/main.rs
git commit -m "fix(cli): make inspect surface a nonzero exit code when the tree has an error diagnostic"
```

---

## Part G: Standalone fixes

All source verified. Writing the final task group now.

### Task 1: Diagnose unfilled generic type parameters in `build_substitution`

**Files:**

- Modify: `crates/core/src/resolver/substitute.rs:18-43` (imports + `build_substitution`)
- Modify: `crates/core/src/resolver/substitute.rs:57-72` (`apply_generic_args` — add `diagnostics` param)
- Modify: `crates/core/src/resolver/chain.rs:152` (call site)
- Modify: `crates/core/src/resolver/primitives.rs:144` (call site)
- Modify: `crates/core/src/resolver/alias.rs:181-190`... (no change needed here — `generic_alias_with_structured_args` already has `diagnostics`, just thread it into `build_substitution` at `substitute.rs:105`)
- Modify: `crates/core/src/types/diagnostic.rs:60-64` (new `DiagnosticCode` variant)
- Test: inline `#[cfg(test)]` module in `crates/core/src/resolver/substitute.rs` (file currently has no test module — this project's convention is inline tests, per `crates/core/CLAUDE.md`)

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DiagnosticSeverity;
    use compact_str::CompactString;

    #[test]
    fn build_substitution_diagnoses_unfilled_trailing_type_params() {
        // `type Foo<T, U> = { a: T; b: U }` called as `Foo<string>` — `U` never
        // supplied, so it's left as a bare `Named` reference with no diagnostic
        // today. This should now warn.
        let params: Vec<CompactString> = vec![CompactString::from("T"), CompactString::from("U")];
        let args = vec![CollectedType::String];
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        let subst = build_substitution(&params, &args, Utf8Path::new("src/foo.ts"), &mut diagnostics);

        assert_eq!(subst.len(), 1, "only T should have been substituted");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert!(diagnostics[0].message.contains('U'), "message should name the unfilled param: {}", diagnostics[0].message);
    }

    #[test]
    fn build_substitution_is_silent_when_all_params_are_supplied() {
        let params: Vec<CompactString> = vec![CompactString::from("T")];
        let args = vec![CollectedType::String];
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        let subst = build_substitution(&params, &args, Utf8Path::new("src/foo.ts"), &mut diagnostics);

        assert_eq!(subst.len(), 1);
        assert!(diagnostics.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core build_substitution_diagnoses_unfilled_trailing_type_params -- --nocapture` Expected: FAIL to compile — `build_substitution` currently takes 3 args (`params`, `args`, `origin_file`), not 4; there's no `diagnostics` parameter yet.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/types/diagnostic.rs`, add a new variant just after `UnresolvableImport` (around line 61):

```rust
pub enum DiagnosticCode {
    UnresolvableImport,
    /// A generic type alias/interface was referenced with fewer type arguments
    /// than it declares parameters — the trailing, unfilled parameters were
    /// left as bare unresolved names in the substituted body.
    GenericArgumentMismatch,
    OpaqueType,
    MaxDepthExceeded,
    Unknown,
    // ...unchanged...
```

In `crates/core/src/resolver/substitute.rs`, update the import and `build_substitution`:

```rust
use crate::types::{CollectedObjectField, CollectedType, CollectedTypeAlias, Diagnostic, DiagnosticCode, DiagnosticSeverity};
```

```rust
/// Build a `Substitution` from declared parameter names and the caller's
/// arguments, tagging each argument with `origin_file` — the file the *caller*
/// wrote them in, which is where any further name lookups on them must happen.
///
/// If `params` declares more type parameters than `args` supplies (a call site
/// under-applying a generic alias, e.g. `Foo<string>` for `type Foo<T, U> = ...`),
/// the trailing unfilled parameters are silently dropped from the substitution —
/// pushes a `Warning` diagnostic naming them so this doesn't degrade silently.
pub(super) fn build_substitution<'a>(
    params: &'a [compact_str::CompactString],
    args: &[CollectedType],
    origin_file: &Utf8Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Substitution<'a> {
    if params.len() > args.len() {
        let unfilled: Vec<&str> = params[args.len()..].iter().map(|p| p.as_str()).collect();
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "Generic alias in '{}' declares {} type parameter(s) but only {} were supplied — '{}' left unsubstituted",
                origin_file,
                params.len(),
                args.len(),
                unfilled.join("', '")
            ),
            file: Some(origin_file.to_string()),
            line: None,
            column: None,
            help: Some("Check the call site supplies a type argument for every declared type parameter.".into()),
            code: DiagnosticCode::GenericArgumentMismatch,
        });
    }
    params
        .iter()
        .map(|p| p.as_str())
        .zip(args.iter().map(|a| CollectedType::AtFile { file: origin_file.to_owned(), inner: Box::new(a.clone()) }))
        .collect()
}
```

Update `apply_generic_args` to take and forward `diagnostics`:

```rust
pub(super) fn apply_generic_args(
    alias: CollectedTypeAlias,
    scoped_key: &str,
    type_args: &[String],
    consuming_file: &Utf8Path,
    ctx: &ResolutionContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> CollectedTypeAlias {
    let Some(params) = ctx.global.type_alias_params.get(scoped_key) else {
        return alias;
    };
    if params.is_empty() || type_args.is_empty() {
        return alias;
    }
    let args: Vec<CollectedType> = type_args.iter().map(|a| raw_arg_to_collected_type(a)).collect();
    let subst = build_substitution(params, &args, consuming_file, diagnostics);
    substitute_alias(&alias, &subst)
}
```

Update the call in `generic_alias_with_structured_args` (line ~105) to pass its existing `diagnostics` param through:

```rust
    let subst = build_substitution(params, args, consuming_file, diagnostics);
```

Update `crates/core/src/resolver/chain.rs:152` (inside `resolve_props_chain`, where `state.diagnostics` is already in scope):

```rust
        let alias = super::substitute::apply_generic_args(alias, &matched_key, type_args, consuming_file, ctx, &mut state.diagnostics);
```

Update `crates/core/src/resolver/primitives.rs:144` (inside the interface-field lookup branch, where `state.diagnostics` is already in scope):

```rust
                    Some(params) if !params.is_empty() && !obj_args.is_empty() => {
                        let subst = build_substitution(params, obj_args, consuming_file, &mut state.diagnostics);
                        substitute_type(&field.collected_type, &subst)
                    }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core build_substitution -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/resolver/substitute.rs crates/core/src/resolver/chain.rs crates/core/src/resolver/primitives.rs crates/core/src/types/diagnostic.rs
git commit -m "fix(resolver): diagnose generic aliases called with too few type args"
```

---

### Task 2: Make `watch --out` writes visible-on-failure and atomic

**Files:**

- Modify: `crates/cli/src/commands/watch.rs:107-112` (the `--out` write inside the `Watchexec::new` closure)
- Test: inline `#[cfg(test)]` module in `crates/cli/src/commands/watch.rs` (no test module exists there yet; matches this crate's convention of colocated tests per `crates/core/CLAUDE.md`'s inline-test norm)

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::write_atomic;

    #[test]
    fn write_atomic_surfaces_error_when_parent_dir_is_missing() {
        let result = write_atomic("/nonexistent-rdt-watch-dir-xyz-123/out.json", "{}");
        assert!(result.is_err(), "write to a missing parent directory should surface an error, not succeed silently");
    }

    #[test]
    fn write_atomic_writes_via_temp_then_rename() {
        let dir = std::env::temp_dir().join(format!("rdt-watch-atomic-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let target = dir.join("out.json");

        write_atomic(target.to_str().expect("utf8 path"), "{\"a\":1}").expect("write should succeed");

        let contents = std::fs::read_to_string(&target).expect("read back written file");
        assert_eq!(contents, "{\"a\":1}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-cli write_atomic -- --nocapture` Expected: FAIL to compile — `write_atomic` does not exist yet in `watch.rs`.

- [ ] **Step 3: Write minimal implementation**

Add a private helper above `cmd_watch` in `crates/cli/src/commands/watch.rs`:

```rust
/// Writes `contents` to `path` via a same-directory temp file + rename, so a
/// mid-write failure (disk full, permission revoked) can never leave `path`
/// truncated or half-written. Returns the `io::Error` on failure instead of
/// swallowing it — callers must report it, not discard the `Result`.
fn write_atomic(path: &str, contents: &str) -> std::io::Result<()> {
    let target = std::path::Path::new(path);
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let file_name = target.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("'{path}' has no file name component"))
    })?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(".tmp");
    let tmp_path = dir.join(tmp_name);
    std::fs::write(&tmp_path, contents)?;
    std::fs::rename(&tmp_path, target)?;
    Ok(())
}
```

Replace the silent write at the current `if let Some(ref p) = out_path { ... }` block (currently `let _ = std::fs::write(p, json);`):

```rust
                        if let Some(ref p) = out_path {
                            let snapshot = session_inner.snapshot();
                            if let Ok(json) = serde_json::to_string(&snapshot) {
                                if let Err(e) = write_atomic(p, &json) {
                                    print_diagnostics(&[oxc_react_docgen_core::types::Diagnostic {
                                        severity: oxc_react_docgen_core::types::DiagnosticSeverity::Error,
                                        message: format!("Failed to write '{p}': {e}"),
                                        file: Some(p.clone()),
                                        line: None,
                                        column: None,
                                        help: Some(
                                            "Check that the output path's parent directory exists and is writable.".into(),
                                        ),
                                        code: oxc_react_docgen_core::types::DiagnosticCode::IoError,
                                    }]);
                                }
                            }
                        }
```

(`print_diagnostics` is already imported at the top of this file — `use crate::output::{print_diagnostics, print_summary};` — and is this crate's established way of surfacing an error from inside a non-`Result`-returning closure, matching how `extract.rs`'s `--out` failure is wrapped via `wrap_err` in the top-level `Result`-returning path — this closure can't propagate `?` since `Watchexec::new`'s callback must return `action`, not a `Result`.)

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-cli write_atomic -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/commands/watch.rs
git commit -m "fix(cli): surface watch --out write failures instead of discarding them"
```

---

### Task 3: `ParsedProp::new` normalizes `required` when a default value is present

**Files:**

- Modify: `crates/core/src/types/output.rs:81-99` (struct fields → `pub(crate)`, add `ParsedProp::new`)
- Modify: `crates/core/src/known.rs:296-306` (`simple_prop`)
- Modify: `crates/core/src/resolver/chain.rs:244-253`
- Modify: `crates/core/src/resolver/alias.rs:181-190`
- Modify: `crates/core/src/resolver/mod.rs:295-310` (notable-attr synthesis)
- Modify: `crates/core/src/resolver/mod.rs:827-836`, `crates/core/src/resolver/mod.rs:840-849` (test literals)
- Modify: `crates/core/src/toon.rs:165-181` (test literal)
- Test: inline `#[cfg(test)]` module in `crates/core/src/types/output.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod parsed_prop_tests {
    use super::*;

    #[test]
    fn new_normalizes_required_false_when_default_value_present() {
        let prop = ParsedProp::new(
            "variant".to_string(),
            PropType::String,
            true, // caller (incorrectly) says required
            Some(DefaultValue { value: "\"primary\"".to_string(), computed: false }),
            "desc".to_string(),
            Default::default(),
            None,
            vec![],
        );

        assert!(!prop.required, "a prop with a default value must not be reported as required");
        assert!(prop.default_value.is_some());
    }

    #[test]
    fn new_preserves_required_true_when_no_default_value() {
        let prop = ParsedProp::new(
            "variant".to_string(),
            PropType::String,
            true,
            None,
            "desc".to_string(),
            Default::default(),
            None,
            vec![],
        );

        assert!(prop.required);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core new_normalizes_required_false_when_default_value_present -- --nocapture` Expected: FAIL to compile — `ParsedProp::new` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

`[corrected from the original draft]` — the original draft made `ParsedProp`'s fields `pub(crate)`, which does not work: `crates/cli` (`extract.rs`, `inspect.rs`) reads `ParsedProp`'s fields directly across the crate boundary, so `pub(crate)` is a hard compile error there. And even ignoring that, `pub(crate)` grants visibility to every same-crate call site — including the exact ones (`chain.rs`, `alias.rs`, `mod.rs`, `known.rs`) that are the actual problem — so it wouldn't stop the misuse it's meant to stop. Use a **sealed-field** pattern instead: keep the real fields `pub` (so all existing reads, same-crate and cross-crate, keep compiling unchanged), and add one private zero-sized "seal" field that only this module can construct, so a bare struct literal from _any_ other module — including ones in this same crate — no longer compiles; only `ParsedProp::new(...)` can build one.

In `crates/core/src/types/output.rs`:

```rust
pub struct ParsedProp {
    /// Prop name
    pub name: String,
    /// Resolved semantic type
    #[serde(rename = "type")]
    pub prop_type: PropType,
    /// Whether the prop is required
    pub required: bool,
    /// Default value if known (from destructured params or JSDoc)
    pub default_value: Option<DefaultValue>,
    /// JSDoc description of the prop
    pub description: String,
    /// JSDoc @tags on the prop (@deprecated, @since, @see, etc.)
    pub tags: BTreeMap<String, String>,
    /// Interface/type where this prop was originally declared
    pub parent: Option<PropParent>,
    /// All declarations of this prop name (for overloads/merging)
    pub declarations: Vec<PropParent>,
    /// Private, zero-sized, and unconstructible outside this module — its
    /// only purpose is to make a bare `ParsedProp { .. }` struct literal
    /// fail to compile anywhere else, including other modules in this same
    /// crate, so `required`/`default_value` can only be set together
    /// through `ParsedProp::new`'s normalization. Skipped in both
    /// directions of serde so the wire format is unaffected.
    #[serde(skip)]
    _seal: Seal,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Seal;

impl ParsedProp {
    /// Constructs a `ParsedProp`, normalizing the `required`/`default_value`
    /// relationship: RDT convention is that a supplied default value makes a
    /// prop effectively optional regardless of what `required` was computed
    /// as upstream (e.g. a destructured param with both a type annotation
    /// marking it required and a default expression).
    pub fn new(
        name: String,
        prop_type: PropType,
        required: bool,
        default_value: Option<DefaultValue>,
        description: String,
        tags: BTreeMap<String, String>,
        parent: Option<PropParent>,
        declarations: Vec<PropParent>,
    ) -> Self {
        let required = if default_value.is_some() { false } else { required };
        ParsedProp { name, prop_type, required, default_value, description, tags, parent, declarations, _seal: Seal }
    }
}
```

(Match `Seal`'s derives to whatever `ParsedProp` itself derives — if `ParsedProp` derives `Serialize`/`Deserialize` directly rather than going through `PropType`'s manual `to_json_value`, confirm `#[serde(skip)]` combined with `Seal: Default` lets deserialization construct a `ParsedProp` without needing `Seal` in the wire data; add `#[serde(default)]` alongside `#[serde(skip)]` on the field if the derive requires it explicitly.)

Update `crates/core/src/known.rs` `simple_prop`:

```rust
fn simple_prop(name: &str, prop_type: PropType, required: bool, description: &str) -> ParsedProp {
    ParsedProp::new(name.to_owned(), prop_type, required, None, description.to_owned(), Default::default(), None, vec![])
}
```

Update `crates/core/src/resolver/chain.rs:244` (the `chain.props.push(ParsedProp { ... })` after the default-value merge logic):

```rust
        chain.props.push(ParsedProp::new(
            raw_prop.name.clone(),
            prop_type,
            raw_prop.required,
            default_value,
            raw_prop.description.clone(),
            raw_prop.tags.clone(),
            Some(parent.clone()),
            vec![parent.clone()],
        ));
```

Update `crates/core/src/resolver/alias.rs:181`:

```rust
                chain.props.push(ParsedProp::new(
                    field.name.clone(),
                    prop_type,
                    field.required,
                    None,
                    field.description.clone(),
                    Default::default(),
                    None,
                    vec![],
                ));
```

Update `crates/core/src/resolver/mod.rs:295-310` (notable-attr synthesis):

```rust
                notable_inherited.insert(
                    attr_name.to_string(),
                    ParsedProp::new(
                        attr_name.to_string(),
                        prop_type,
                        false,
                        None,
                        String::new(),
                        Default::default(),
                        Some(PropParent {
                            name: format!("{}HTMLAttributes", html::capitalize_element(element)),
                            file_name: "node_modules/@types/react/index.d.ts".to_string(),
                        }),
                        vec![],
                    ),
                );
```

Update the two test literals in `crates/core/src/resolver/mod.rs` (lines ~827 and ~840):

```rust
                vec![ParsedProp::new(
                    "variant".into(),
                    PropType::StringLiteral("default".into()),
                    true,
                    None,
                    String::new(),
                    BTreeMap::new(),
                    None,
                    vec![],
                )],
```

```rust
                vec![ParsedProp::new(
                    "variant".into(),
                    PropType::StringLiteral("outline".into()),
                    true,
                    None,
                    String::new(),
                    BTreeMap::new(),
                    None,
                    vec![],
                )],
```

Update the test literal in `crates/core/src/toon.rs:165-181`:

```rust
        props.insert(
            "variant".to_string(),
            ParsedProp::new(
                "variant".into(),
                PropType::LiteralUnion { members: vec!["primary".into(), "secondary".into()], has_default: true },
                false,
                Some(crate::types::DefaultValue { value: "\"primary\"".into(), computed: false }),
                "Visual variant".into(),
                Default::default(),
                None,
                vec![],
            ),
        );
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core parsed_prop_tests -- --nocapture && cargo test -p oxc-react-docgen-core` Expected: PASS, and the full `crates/core` test suite (including `resolver::mod` and `toon` tests) still compiles and passes after the call-site rewrites.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/output.rs crates/core/src/known.rs crates/core/src/resolver/chain.rs crates/core/src/resolver/alias.rs crates/core/src/resolver/mod.rs crates/core/src/toon.rs
git commit -m "fix(types): force ParsedProp construction through a required/default-normalizing constructor"
```

---

### Task 4: Document `DiagnosticCode::Unknown` as reserved headroom (doc comment only, no test)

**Files:**

- Modify: `crates/core/src/types/diagnostic.rs` (the `Unknown` variant, immediately after Task 1's new `GenericArgumentMismatch` variant if done in the same branch — otherwise right after `UnresolvableImport`/`OpaqueType`/`MaxDepthExceeded`)

No test step: this is a documentation-only change to an intentionally-unused `#[non_exhaustive]` enum variant. There is no behavior to assert against — the only verifiable outcome is that the comment exists, checked by reading the file back.

- [ ] **Step 1: Add the doc comment**

```rust
pub enum DiagnosticCode {
    UnresolvableImport,
    OpaqueType,
    MaxDepthExceeded,
    /// Deliberately unconstructed within this crate — reserved headroom for
    /// external consumers synthesizing their own `Diagnostic` outside the
    /// extraction pipeline (e.g. a wrapping tool reporting its own issue
    /// through this crate's diagnostic shape). Do not remove for being
    /// "unused"; nothing in `crates/core` or `crates/cli` is expected to
    /// construct it.
    Unknown,
    /// JSDoc @default conflicts with code default value — code value was used.
    JsDocDefaultMismatch,
    // ...unchanged...
```

- [ ] **Step 2: Verify** Run: `cargo doc -p oxc-react-docgen-core --no-deps 2>&1 | tail -5` (or just re-read the file) — confirm the comment renders and no `-D warnings` clippy lint fires for it. Expected: doc builds cleanly; comment is present on `Unknown`.

- [ ] **Step 3: Commit**

```bash
git add crates/core/src/types/diagnostic.rs
git commit -m "docs(diagnostic): explain why DiagnosticCode::Unknown is unused but kept"
```

---

### Task 5: Fix NaN/Infinity round-trip on `PropType::NumberLiteral`

**Files:**

- Modify: `crates/core/src/types/output.rs:314` (`to_tagged_value`'s `NumberLiteral` arm)
- Modify: `crates/core/src/types/output.rs:423-425` (`from_tagged_value`'s `numberLiteral` arm)
- Test: inline `#[cfg(test)]` module in `crates/core/src/types/output.rs`

Chose a real fix over a comment-only punt: `serde_json::json!({"value": n})` on a non-finite `f64` already silently serializes to `null` (`serde_json::Number` can't represent NaN/Infinity), and `from_tagged_value` can't tell "field was null" apart from "field was absent", so it always falls back to `0.0`. Since the fix is small (tag non-finite values as strings instead of numbers) and the existing code already special-cases this variant, doing it properly is cheaper than documenting the gap and better than accepting silent data loss.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod number_literal_roundtrip_tests {
    use super::*;

    #[test]
    fn nan_number_literal_round_trips_as_nan_not_zero() {
        let original = PropType::NumberLiteral(f64::NAN);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert!(n.is_nan(), "expected NaN to survive the round-trip, got {n}"),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn infinity_number_literal_round_trips_as_infinity() {
        let original = PropType::NumberLiteral(f64::INFINITY);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        match restored {
            PropType::NumberLiteral(n) => assert_eq!(n, f64::INFINITY),
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
    }

    #[test]
    fn finite_number_literal_still_round_trips_normally() {
        let original = PropType::NumberLiteral(42.5);
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: PropType = serde_json::from_value(json).expect("deserialize");

        assert_eq!(restored, PropType::NumberLiteral(42.5));
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core nan_number_literal_round_trips_as_nan_not_zero -- --nocapture` Expected: FAIL — `n.is_nan()` is false because the round-trip currently produces `0.0` (NaN serialized to JSON `null`, then `as_f64().unwrap_or(0.0)` on read).

- [ ] **Step 3: Write minimal implementation**

In `to_tagged_value` (around line 314):

```rust
            PropType::NumberLiteral(n) => {
                // `serde_json::Number` cannot represent NaN/Infinity (they'd
                // silently become JSON `null`, then round-trip back as `0.0`
                // — see `from_tagged_value` below). Tag non-finite values as
                // strings instead so the read side can tell them apart from a
                // genuinely-absent value.
                let value = if n.is_finite() {
                    serde_json::json!(n)
                } else if n.is_nan() {
                    serde_json::json!("NaN")
                } else if *n > 0.0 {
                    serde_json::json!("Infinity")
                } else {
                    serde_json::json!("-Infinity")
                };
                serde_json::json!({"kind": "numberLiteral", "value": value})
            }
```

In `from_tagged_value` (around line 423):

```rust
            "numberLiteral" | "number_literal" => {
                let n = match v.get("value") {
                    Some(val) if val.is_string() => match val.as_str().unwrap_or("") {
                        "NaN" => f64::NAN,
                        "Infinity" => f64::INFINITY,
                        "-Infinity" => f64::NEG_INFINITY,
                        _ => 0.0,
                    },
                    Some(val) => val.as_f64().unwrap_or(0.0),
                    None => 0.0,
                };
                Ok(PropType::NumberLiteral(n))
            }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core number_literal_roundtrip_tests -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/output.rs
git commit -m "fix(types): round-trip NaN/Infinity NumberLiteral values instead of silently zeroing them"
```

---

### Task 6: Exclude zero/one-member `LiteralUnion`s from the RDT "enum" shape

**Files:**

- Modify: `crates/core/src/types/output.rs:210-219` (`PropType::is_literal_union`)
- Test: inline `#[cfg(test)]` module in `crates/core/src/types/output.rs`

Chose the exclude-and-fall-back fix over a comment: `is_literal_union` is the single source of truth `rdt_type_json` (`crates/cli/src/commands/extract.rs:84`) branches on, so fixing it here fixes the RDT `{"name":"enum","value":[]}` output for free without touching the CLI. A one-member "enum" is equally not a meaningful `<select>`, so both are excluded — falls back to plain `raw_string()` output via the existing `_ =>` branch in `rdt_type_json`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod is_literal_union_tests {
    use super::*;

    #[test]
    fn empty_literal_union_is_not_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec![], has_default: false };
        assert!(!pt.is_literal_union());
    }

    #[test]
    fn single_member_literal_union_is_not_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec!["only".to_string()], has_default: false };
        assert!(!pt.is_literal_union());
    }

    #[test]
    fn two_member_literal_union_is_still_treated_as_an_enum() {
        let pt = PropType::LiteralUnion { members: vec!["a".to_string(), "b".to_string()], has_default: false };
        assert!(pt.is_literal_union());
    }
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test -p oxc-react-docgen-core empty_literal_union_is_not_treated_as_an_enum -- --nocapture` Expected: FAIL — the current `PropType::LiteralUnion { .. } => true` arm ignores `members.len()` entirely, so an empty (or single-member) union is unconditionally reported as a literal union.

- [ ] **Step 3: Write minimal implementation**

```rust
    /// True if this type is a pure literal union (all members are literals).
    /// Used by serializers to choose between "enum" and "union" in RDT output.
    /// Requires at least 2 members — a 0- or 1-member "union" isn't a
    /// meaningful `<select>` shape, so RDT output falls back to plain
    /// `raw_string()` for those instead of an empty/single-option enum.
    pub fn is_literal_union(&self) -> bool {
        match self {
            PropType::Union(members) => {
                members.len() >= 2
                    && members.iter().all(|m| {
                        matches!(m, PropType::StringLiteral(_) | PropType::NumberLiteral(_) | PropType::BoolLiteral(_))
                    })
            }
            PropType::LiteralUnion { members, .. } => members.len() >= 2,
            _ => false,
        }
    }
```

- [ ] **Step 4: Run test to verify it passes** Run: `cargo test -p oxc-react-docgen-core is_literal_union_tests -- --nocapture` Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/types/output.rs
git commit -m "fix(types): don't treat a 0- or 1-member LiteralUnion as an RDT enum shape"
```

---

### Task 7: Document the mtime+size cache's same-tick staleness limitation (doc comment only, no test)

**Files:**

- Modify: `crates/core/src/cache.rs:181-191` (`DtsCache::key_for`)

No test step: per `docs/root-cause-analysis.md`, a content-hash fix is an explicit scoped follow-up, not something to implement now — it would change the cache's performance/complexity tradeoff. This task only documents the known limitation so it isn't rediscovered as a surprise later; there is no new behavior to assert against.

- [ ] **Step 1: Add the doc comment**

```rust
    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Builds the cache key from the file's current size + mtime.
    ///
    /// Known limitation: staleness detection is mtime+size only, no content
    /// hash. On filesystems with coarse mtime resolution (e.g. some
    /// configurations report only 1-second granularity), an edit that lands
    /// in the same tick as a prior write *and* happens to produce a
    /// same-length file will be served a stale cache hit — the key looks
    /// unchanged even though the content differs. A content hash would close
    /// this gap but trades away the cheap stat-only check this cache relies
    /// on for its speed; see `docs/root-cause-analysis.md` — this is a
    /// deliberate, scoped-for-later tradeoff, not an oversight.
    fn key_for(&self, path: &Utf8Path) -> Option<CacheKey> {
        let meta = std::fs::metadata(path.as_std_path()).ok()?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        Some(CacheKey { path: path.to_owned(), size: meta.len(), mtime_ns })
    }
```

- [ ] **Step 2: Verify** Run: `cargo doc -p oxc-react-docgen-core --no-deps 2>&1 | tail -5` (or re-read the file) — confirm the comment is present on `key_for` and doc build is clean. Expected: doc builds cleanly; comment present.

- [ ] **Step 3: Commit**

```bash
git add crates/core/src/cache.rs
git commit -m "docs(cache): document the mtime+size same-tick staleness limitation"
```
