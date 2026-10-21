use std::sync::{Arc, Barrier};
use std::thread;

use camino::Utf8PathBuf;

use super::WatchSession;
use crate::pipeline::PipelineOptions;

// Points at a real, empty directory rather than an empty `src_dirs` list —
// the latter is now its own error case (see
// pipeline::tests::test_extract_empty_src_dirs_produces_diagnostic), distinct
// from "a configured directory that happens to contain no files".
fn empty_options() -> PipelineOptions {
    let dir = tempfile::TempDir::new().unwrap().keep();
    PipelineOptions { src_dirs: vec![Utf8PathBuf::from_path_buf(dir).unwrap()], ..Default::default() }
}

#[test]
fn initialize_in_full_mode_does_not_emit_components_declared_in_merged_react_types() {
    let tmp = tempfile::TempDir::new_in(camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    std::fs::write(
        tmp.path().join("Button.tsx"),
        r#"
import * as React from "react";
export function Button(props: { variant?: "primary" | "secondary" } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return null;
}
"#,
    )
    .unwrap();
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(tmp.path().to_owned()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).unwrap()),
        html_attributes: crate::pipeline::HtmlAttributeMode::Full,
        ..Default::default()
    };

    let session = WatchSession::new(options);
    let initial = session.initialize();

    let names: Vec<&str> = initial.components.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["Button"]);
    let snapshot_names: Vec<String> = session.snapshot().components.keys().cloned().collect();
    assert_eq!(snapshot_names, vec!["Button".to_owned()]);
}

#[test]
fn initialize_is_idempotent() {
    let session = WatchSession::new(empty_options());
    let first = session.initialize();
    let second = session.initialize();
    assert_eq!(first.components.len(), second.components.len());
    assert_eq!(first.enums.len(), second.enums.len());
}

#[test]
fn concurrent_initialize_no_double_init() {
    // Barrier ensures all 8 threads reach initialize() simultaneously,
    // maximising the race window for the Mutex<bool> guard.
    let session = Arc::new(WatchSession::new(empty_options()));
    let barrier = Arc::new(Barrier::new(8));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let s = Arc::clone(&session);
            let b = Arc::clone(&barrier);
            thread::spawn(move || {
                b.wait();
                s.initialize()
            })
        })
        .collect();

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let expected = results[0].components.len();
    assert!(results.iter().all(|r| r.components.len() == expected));
}

#[test]
fn initialize_recovers_from_a_poisoned_lock_instead_of_panicking() {
    let session = WatchSession::new(empty_options());

    // Poison the writer lock the way an uncontained panic during a revision build would.
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = session.writer.lock().unwrap();
        panic!("simulated panic while holding the writer lock");
    }));
    assert!(poisoned.is_err(), "the panic should have unwound past the lock guard");
    assert!(session.writer.is_poisoned(), "the lock should now be poisoned");

    // Must recover instead of propagating a second panic via .expect(...).
    let output = session.initialize();
    assert!(output.components.is_empty());
}

#[test]
fn snapshot_after_initialize_with_no_files_has_no_diagnostics() {
    let session = WatchSession::new(empty_options());
    let first = session.initialize();
    assert!(first.diagnostics.is_empty(), "no files to read means nothing to report");
    let snap = session.snapshot();
    assert!(snap.diagnostics.is_empty());
}

#[test]
fn snapshot_stats_components_extracted_reflects_the_real_component_count() {
    // Found while validating SPEC-CLI-001b's AC-5: snapshot() always
    // returned ExtractionStats::default(), so componentsExtracted read 0
    // no matter how many components a watch session actually had.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    std::fs::write(dir.join("A.tsx").as_std_path(), "export function A(props: { x: string }) { return null; }\n")
        .expect("write A.tsx");
    std::fs::write(dir.join("B.tsx").as_std_path(), "export function B(props: { y: string }) { return null; }\n")
        .expect("write B.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    let snap = session.snapshot();
    assert_eq!(snap.components.len(), 2);
    assert_eq!(
        snap.stats.components_extracted, 2,
        "stats.components_extracted should match the real component count, got {:?}",
        snap.stats
    );
}

/// The session's snapshot must equal what a cold `extract()` of the same files returns. Stats carry timings, so they
/// are left out.
fn assert_matches_cold(session: &WatchSession) {
    let watched = session.snapshot();
    let cold = crate::pipeline::extract(&session.options);
    assert_eq!(watched.components, cold.components);
    assert_eq!(watched.enums, cold.enums);
    assert_eq!(watched.diagnostics, cold.diagnostics);
}

#[test]
fn a_path_outside_every_source_directory_changes_nothing() {
    let session = WatchSession::new(empty_options());
    let _ignored = session.initialize();

    let update = session.update_file(camino::Utf8Path::new("/nonexistent/does-not-exist.tsx"));
    assert!(update.updated_keys.is_empty() && update.removed_keys.is_empty() && update.diagnostics.is_empty());
    assert!(session.snapshot().diagnostics.is_empty());
}

/// A src dir holding `Widget.tsx` and the options over it; the DTS cache lives beside it, outside the watched tree.
fn widget_project(source: &str) -> (tempfile::TempDir, Utf8PathBuf, PipelineOptions) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().expect("canonicalize")).expect("utf8").join("src");
    std::fs::create_dir(&src).expect("create src");
    let widget = src.join("Widget.tsx");
    std::fs::write(&widget, source).expect("write Widget.tsx");
    let options = PipelineOptions {
        src_dirs: vec![src],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join("cache")).expect("utf8")),
        ..Default::default()
    };
    (tmp, widget, options)
}

const BROKEN: &str = "export const = ;\n";
const FIXED: &str = "export function Widget() { return null; }\n";

#[test]
fn update_file_reports_a_syntax_error_exactly_as_a_cold_extract_does() {
    let (_tmp, widget, options) = widget_project(FIXED);
    let session = WatchSession::new(options.clone());
    let _ignored = session.initialize();

    std::fs::write(&widget, BROKEN).expect("break Widget.tsx");
    let update = session.update_file(&widget);

    let cold = crate::pipeline::extract(&options);
    assert_eq!(cold.diagnostics.len(), 1, "a cold extract reports the syntax error: {:?}", cold.diagnostics);
    assert_eq!(cold.diagnostics[0].code, crate::types::DiagnosticCode::ParseError);
    assert_eq!(update.diagnostics, cold.diagnostics);
    assert_eq!(session.snapshot().diagnostics, cold.diagnostics);
}

#[test]
fn fixing_a_syntax_error_clears_it_from_the_update_and_the_snapshot() {
    let (_tmp, widget, options) = widget_project(BROKEN);
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    std::fs::write(&widget, FIXED).expect("fix Widget.tsx");
    let update = session.update_file(&widget);

    assert_eq!(update.diagnostics, vec![]);
    assert_eq!(session.snapshot().diagnostics, vec![]);
}

#[test]
fn a_panic_during_incremental_resolve_degrades_to_a_diagnostic_not_a_crash() {
    // Found while grounding SPEC-PIPELINE-001 in real source: extract()'s
    // one-shot resolve phase (pipeline/mod.rs) was wrapped in
    // `contain_panic`, but this incremental path was not — one component
    // panicking during an editor keystroke would have crashed the whole
    // watch session instead of degrading to a diagnostic for that file.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let button_path = dir.join("Button.tsx");
    let other_path = dir.join("Other.tsx");

    std::fs::write(button_path.as_std_path(), "export function Button(props: { label: string }) { return null; }\n")
        .expect("write Button.tsx");
    std::fs::write(
        other_path.as_std_path(),
        format!(
            "export function {}(props: {{ label: string }}) {{ return null; }}\n",
            super::super::RESOLVE_PANIC_TEST_SENTINEL
        ),
    )
    .expect("write Other.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    // Must not panic the test process — that's the whole point.
    let update = session.update_file(&other_path);

    assert!(
        update.diagnostics.iter().any(|d| d.code == crate::types::DiagnosticCode::InternalPanic),
        "expected an InternalPanic diagnostic, got {:?}",
        update.diagnostics
    );
}

#[test]
fn update_file_reflects_the_edited_files_new_content_not_its_pre_edit_content() {
    // Found while validating SPEC-PIPELINE-001: ArcSwap::rcu returns the
    // PRE-swap value (arc-swap's compare_and_swap convention), not the value
    // the closure just produced. `new_global` was bound directly to rcu's
    // return, so every incremental update resolved against the file's
    // content from BEFORE this edit — watch mode was permanently one edit
    // behind. A test asserting only component *identity* (name present) is
    // invariant under this bug; this test asserts prop *content* instead.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let path = dir.join("Widget.tsx");

    std::fs::write(
        path.as_std_path(),
        "export interface WidgetProps { label: string; }\nexport function Widget(props: WidgetProps) { return null; }\n",
    )
    .expect("write initial fixture");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let initial = session.initialize();
    assert!(
        initial.components.get("Widget").is_some_and(|c| c.props.contains_key("label")),
        "expected initial extraction to see 'label', got {:?}",
        initial.components.get("Widget")
    );

    std::fs::write(
        path.as_std_path(),
        "export interface WidgetProps { title: string; }\nexport function Widget(props: WidgetProps) { return null; }\n",
    )
    .expect("write edited fixture");

    let update = session.update_file(&path);
    let widget = update.updated_components.iter().find(|c| c.display_name == "Widget");
    assert!(
        widget.is_some_and(|c| c.props.contains_key("title") && !c.props.contains_key("label")),
        "expected update_file to resolve against the EDITED content ('title'), not the pre-edit content ('label'); got {:?}",
        widget
    );

    let snapshot = session.snapshot();
    let widget = snapshot.components.get("Widget");
    assert!(
        widget.is_some_and(|c| c.props.contains_key("title") && !c.props.contains_key("label")),
        "expected snapshot() to reflect the edited content too, got {:?}",
        widget
    );
}

#[test]
fn update_file_re_resolves_files_that_import_the_changed_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let base_path = dir.join("Base.tsx");
    let consumer_path = dir.join("Consumer.tsx");

    std::fs::write(base_path.as_std_path(), "export interface BaseProps { label: string; }").expect("write Base.tsx");
    std::fs::write(
        consumer_path.as_std_path(),
        r#"
            import { BaseProps } from './Base';
            interface ConsumerProps extends BaseProps { extra?: boolean; }
            export function Consumer(props: ConsumerProps) { return null; }
        "#,
    )
    .expect("write Consumer.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let initial = session.initialize();
    assert!(
        initial.components.contains_key("Consumer"),
        "Consumer should be found on cold init, got {:?}",
        initial.components.keys().collect::<Vec<_>>()
    );

    // Base.tsx itself declares no component — only Consumer.tsx (which
    // imports it) should be re-resolved.
    let update = session.update_file(&base_path);
    let updated_names: Vec<&str> = update.updated_components.iter().map(|c| c.display_name.as_str()).collect();
    assert!(
        updated_names.contains(&"Consumer"),
        "expected Consumer (which imports Base.tsx) to be re-resolved when Base.tsx changes, got {:?}",
        updated_names
    );
}

#[test]
fn update_file_still_resolves_ambient_globals_after_the_first_incremental_change() {
    // Adversarial review finding: WatchSession::initialize() used to rebuild
    // GlobalSourceData by hand, skipping Phase 3.5/3.6 (@types/react
    // Full-mode + ambient lib.d.ts merges) entirely. The cold `initialize()`
    // output was correct (it came from a real extract() call done
    // separately just for that output), but `self.global` — what every
    // subsequent update_file() resolves against — was missing those merges,
    // so the very next edit to the file itself would regress it to Opaque.
    // Uses a tempdir under CARGO_MANIFEST_DIR so the node_modules
    // ancestor-walk reaches this repo's real installed `typescript`
    // package, the same trick pipeline::tests's ambient-global tests use.
    let manifest_dir = camino::Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::tempdir_in(manifest_dir).expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let foo_path = dir.join("Foo.tsx");

    let source = r#"
        interface FooProps {
          dir?: HTMLDivElement["dir"];
        }
        export function Foo(props: FooProps) { return null; }
    "#;
    std::fs::write(foo_path.as_std_path(), source).expect("write Foo.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    // Re-save Foo.tsx itself (content unchanged) — this exercises the
    // exact path a real edit takes: re-parse, remove_file + merge back
    // into self.global, then re-resolve against that same self.global.
    let update = session.update_file(&foo_path);
    let foo = update.updated_components.iter().find(|c| c.display_name == "Foo").expect("Foo not re-resolved");
    let dir_prop = foo.props.get("dir").expect("'dir' prop not found");
    assert_eq!(
        dir_prop.prop_type,
        crate::types::PropType::String,
        "expected 'dir' to still resolve to String after the first incremental update, got {:?}",
        dir_prop.prop_type
    );
}

// ── TypeScript 7 lib files: extract, initialize() and update_file each get their own assertion ───────────────────
// `update_file` uses lib paths cached at `initialize()`, separately from the cold run's Phase 3.6.

/// A project whose `typescript` uses the TS 7 layout (`lib/` hidden behind `exports`).
fn typescript_7_project(platform_package_installed: bool, component: &str) -> (tempfile::TempDir, PipelineOptions) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let write = |rel: &str, content: &str| {
        let path = tmp.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(path, content).expect("write");
    };
    write(
        "node_modules/typescript/package.json",
        r#"{"name":"typescript","version":"7.0.2","exports":{"./package.json":"./package.json"},
            "optionalDependencies":{"@typescript/typescript-here":"7.0.2"}}"#,
    );
    if platform_package_installed {
        write(
            "node_modules/@typescript/typescript-here/package.json",
            r#"{"name":"@typescript/typescript-here","version":"7.0.2","exports":{"./package.json":"./package.json"}}"#,
        );
        write("node_modules/@typescript/typescript-here/lib/lib.es5.d.ts", "");
        write(
            "node_modules/@typescript/typescript-here/lib/lib.dom.d.ts",
            "interface HTMLElement { dir: string; }\ninterface HTMLDivElement extends HTMLElement {}\n",
        );
    }
    write("src/Foo.tsx", component);

    let src = Utf8PathBuf::from_path_buf(tmp.path().join("src")).expect("utf8 path");
    let cache_dir = Utf8PathBuf::from_path_buf(tmp.path().join("cache")).expect("utf8 path");
    let options = PipelineOptions { src_dirs: vec![src], cache_dir: Some(cache_dir), ..Default::default() };
    (tmp, options)
}

#[test]
fn typescript_7_lib_files_resolve_ambient_globals_on_extract_initialize_and_update_file() {
    let (_tmp, options) = typescript_7_project(
        true,
        r#"
        interface FooProps { dir?: HTMLDivElement["dir"]; }
        export function Foo(props: FooProps) { return null; }
        "#,
    );
    let foo_path = options.src_dirs[0].join("Foo.tsx");

    let cold = crate::pipeline::extract(&options);
    let session = WatchSession::new(options);
    let initialized = session.initialize();
    let updated = session.update_file(&foo_path);

    assert_eq!(cold.diagnostics, vec![]);
    assert_eq!(initialized.diagnostics, vec![]);
    assert_eq!(cold.components["Foo"].props["dir"].prop_type, crate::types::PropType::String);
    assert_eq!(initialized.components["Foo"].props["dir"].prop_type, crate::types::PropType::String);
    let foo = updated.updated_components.iter().find(|c| c.display_name == "Foo").expect("Foo not re-resolved");
    assert_eq!(foo.props["dir"].prop_type, crate::types::PropType::String);
}

#[test]
fn typescript_7_without_its_platform_package_is_reported_once_by_extract_and_initialize() {
    let (tmp, options) = typescript_7_project(false, "export function Foo(props: { a?: string }) { return null; }");
    let src = Utf8PathBuf::from_path_buf(tmp.path().join("src").canonicalize().expect("canonicalize")).expect("utf8");
    let expected = crate::resolver::resolve_ts_lib_paths(&src).diagnostic.expect("lib files are missing");

    let cold = crate::pipeline::extract(&options);
    let initialized = WatchSession::new(options).initialize();

    assert_eq!(cold.diagnostics, vec![expected.clone()]);
    assert_eq!(initialized.diagnostics, vec![expected]);
}

// ── SPEC-PIPELINE-001 AC-014: initialize() called twice does not re-run
// extraction — a plugin hook counter is unchanged on the second call, and
// the second call's diagnostics are empty regardless of the first call's.

#[test]
fn initialize_called_twice_does_not_rerun_extraction_or_replay_diagnostics() {
    use crate::plugin::{DocgenPlugin, PluginRegistry};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    struct CountingPlugin(StdArc<AtomicUsize>);
    impl DocgenPlugin for CountingPlugin {
        fn name(&self) -> &str {
            "counting-plugin"
        }
        fn on_file_extracted(&self, _path: &camino::Utf8Path, _data: &mut crate::types::SourceData) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    std::fs::write(dir.join("Button.tsx").as_std_path(), "export const Button = () => null;")
        .expect("write Button.tsx");

    let count = StdArc::new(AtomicUsize::new(0));
    let mut plugins = PluginRegistry::new();
    plugins.register(CountingPlugin(count.clone()));

    let options = PipelineOptions { src_dirs: vec![dir], plugins, ..Default::default() };
    let session = WatchSession::new(options);

    let first = session.initialize();
    let count_after_first = count.load(Ordering::SeqCst);
    assert!(count_after_first > 0, "expected the plugin hook to have run at least once");

    let second = session.initialize();
    assert_eq!(
        count.load(Ordering::SeqCst),
        count_after_first,
        "second initialize() call must not re-parse/re-extract"
    );
    assert_eq!(second, first, "a second initialize() returns the current snapshot unchanged");
}

// ── SPEC-PIPELINE-001 AC-016b: after a contained panic in one
// update_file() call, a subsequent call on a different, unrelated file
// still returns a normal IncrementalUpdate — session state isn't
// corrupted or deadlocked.

#[test]
fn session_recovers_after_a_contained_panic_and_serves_unrelated_files_normally() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let panicking_path = dir.join("Boom.tsx");
    let other_path = dir.join("Other.tsx");

    std::fs::write(
        panicking_path.as_std_path(),
        format!(
            "export function {}(props: {{ label: string }}) {{ return null; }}\n",
            super::super::RESOLVE_PANIC_TEST_SENTINEL
        ),
    )
    .expect("write Boom.tsx");
    std::fs::write(other_path.as_std_path(), "export function Other(props: { x: string }) { return null; }\n")
        .expect("write Other.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    // Must not panic the test process.
    let boom_update = session.update_file(&panicking_path);
    assert!(boom_update.diagnostics.iter().any(|d| d.code == crate::types::DiagnosticCode::InternalPanic));

    // Session must still be usable afterward.
    std::fs::write(
        other_path.as_std_path(),
        "export function Other(props: { x: string; y: string }) { return null; }\n",
    )
    .expect("rewrite Other.tsx");
    let other_update = session.update_file(&other_path);
    let other = other_update.updated_components.iter().find(|c| c.display_name == "Other");
    assert!(
        other.is_some(),
        "session should still serve unrelated files normally after a contained panic, got {:?}",
        other_update.updated_components
    );
    // Content, not presence: "Other" existed before the rewrite too, so checking
    // only its name survives is invariant under the exact rcu stale-global bug
    // this session guards against (the panic-containment path resolving against
    // pre-edit content would still produce an "Other" component — just the old
    // one, missing "y"). Checking the new prop proves this call resolved against
    // the just-rewritten content, not a stale snapshot from before the panic.
    assert!(
        other.unwrap().props.contains_key("y"),
        "expected the rewritten 'y' prop to be present, proving this update resolved against the new \
         content rather than a stale pre-rewrite snapshot, got props {:?}",
        other.unwrap().props.keys().collect::<Vec<_>>()
    );
}

// ── SPEC-PIPELINE-001 AC-017: update_file() with a non-canonical path (a symlinked directory segment) still matches
// the canonical paths discovery recorded. The changed file's own component would reappear either way; only a dependent
// in another file shows whether the path matched its recorded dependency.

#[test]
fn update_file_with_a_non_canonical_path_still_finds_the_canonical_entry() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let real_dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").join("real");
    std::fs::create_dir_all(real_dir.as_std_path()).expect("create real dir");
    let file_path = real_dir.join("Button.tsx");
    std::fs::write(
        file_path.as_std_path(),
        "export interface ButtonProps { label: string; }\n\
         export function Button(props: ButtonProps) { return null; }\n",
    )
    .expect("write Button.tsx");

    // A second file whose component's props come from Button.tsx via a
    // relative import — this is the file that must be re-resolved as a
    // TRANSITIVE dependent of the (non-canonically-referenced) change,
    // which only happens if `canonicalize_best_effort` translates the
    // symlinked path back to the canonical one before the reverse-dep
    // lookup.
    let consumer_path = real_dir.join("Consumer.tsx");
    std::fs::write(
        consumer_path.as_std_path(),
        "import { ButtonProps } from './Button';\n\
         export function Consumer(props: ButtonProps) { return null; }\n",
    )
    .expect("write Consumer.tsx");

    let link_dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(real_dir.as_std_path(), link_dir.as_std_path()).expect("create symlink");
    #[cfg(not(unix))]
    {
        // No portable symlink API on this platform — skip rather than fail.
        return;
    }

    let options = PipelineOptions { src_dirs: vec![real_dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    // Reference the changed file via its non-canonical (symlinked) path.
    let non_canonical_path = link_dir.join("Button.tsx");
    let update = session.update_file(&non_canonical_path);

    assert!(
        update.updated_components.iter().any(|c| c.display_name == "Button"),
        "a non-canonical path should still resolve to the canonical entry, got {:?}",
        update.updated_components
    );
    assert!(
        update.updated_components.iter().any(|c| c.display_name == "Consumer"),
        "Consumer.tsx transitively imports from the non-canonically-referenced file and must also be \
         re-resolved — this fails without canonicalize_best_effort, since recorded dependencies use the \
         canonical path discover_files reported at initialize() time, got {:?}",
        update.updated_components
    );
}

// ── SPEC-PIPELINE-001 AC-020: snapshot()/update_file() on a session whose
// initialize() was never called must not panic.

#[test]
fn snapshot_and_update_file_on_an_uninitialized_session_do_not_panic() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let file_path = dir.join("Widget.tsx");
    // onClick's type requires resolving React's own ambient HTMLAttributes-family
    // types — only wired up by initialize()'s ambient_global_files computation,
    // never called here — proving the cross-file/ambient-resolution degrade half
    // of AC-020, not just "doesn't panic".
    std::fs::write(
        file_path.as_std_path(),
        "export function Widget(props: { onClick: React.MouseEventHandler<HTMLButtonElement> }) { return null; }\n",
    )
    .expect("write Widget.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    // initialize() deliberately never called.

    let snap = session.snapshot();
    assert!(snap.components.is_empty(), "expected zero components, got {:?}", snap.components);
    assert!(snap.enums.is_empty());
    assert!(snap.diagnostics.is_empty());

    let update = session.update_file(&file_path);
    let widget = update.updated_components.iter().find(|c| c.display_name == "Widget");
    assert!(
        widget.is_some(),
        "expected the just-parsed file's own component present, got {:?}",
        update.updated_components
    );
}

// ── SPEC-PIPELINE-001 AC-015: update_file() after an editor's atomic
// replace (write to temp, rename over the original) reflects the new
// content — the session must not misinterpret it as a deletion.

#[test]
fn update_file_after_an_atomic_replace_reflects_the_new_content_not_a_deletion() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let file_path = dir.join("Widget.tsx");
    std::fs::write(file_path.as_std_path(), "export function Widget(props: { a: string }) { return null; }\n")
        .expect("write Widget.tsx");

    let options = PipelineOptions { src_dirs: vec![dir.clone()], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    // Atomic replace: write the new content to a same-directory temp file,
    // then rename it over the original path — the pattern editors (and
    // write_atomic itself) use, distinct from an in-place truncate+write.
    let tmp_file = dir.join(".Widget.tsx.tmp");
    std::fs::write(tmp_file.as_std_path(), "export function Widget(props: { b: string }) { return null; }\n")
        .expect("write temp file");
    std::fs::rename(tmp_file.as_std_path(), file_path.as_std_path()).expect("atomic rename over original");

    let update = session.update_file(&file_path);
    let widget = update.updated_components.iter().find(|c| c.display_name == "Widget");
    assert!(
        widget.is_some(),
        "expected Widget still present after the atomic replace, got {:?}",
        update.updated_components
    );
    assert!(
        widget.unwrap().props.contains_key("b"),
        "expected the replaced content's 'b' prop, not the pre-replace 'a' — a rename-based edit must not \
         be misread as a deletion, got props {:?}",
        widget.unwrap().props.keys().collect::<Vec<_>>()
    );

    let snap = session.snapshot();
    let widget_snap = snap.components.get("Widget").expect("expected Widget in snapshot after replace");
    assert!(widget_snap.props.contains_key("b"), "snapshot should also reflect the replaced content");
}

// ── SPEC-PIPELINE-001 AC-019: deleting a file removes its components and re-resolves its dependents.

#[test]
fn deleting_a_file_removes_its_components_and_re_resolves_dependents() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let f_path = dir.join("Base.tsx");
    std::fs::write(
        f_path.as_std_path(),
        "export interface BaseProps { x: string; }\nexport function Base(props: BaseProps) { return null; }\n",
    )
    .expect("write Base.tsx");
    let g_path = dir.join("Consumer.tsx");
    std::fs::write(
        g_path.as_std_path(),
        "import { BaseProps } from './Base';\nexport function Consumer(props: BaseProps) { return null; }\n",
    )
    .expect("write Consumer.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();
    assert!(session.snapshot().components["Consumer"].props.contains_key("x"), "sanity: Consumer resolves 'x'");

    std::fs::remove_file(f_path.as_std_path()).expect("delete Base.tsx");
    let update = session.update_file(&f_path);
    assert_eq!(update.removed_keys, vec!["Base".to_string()]);
    assert_eq!(update.updated_keys, vec!["Consumer".to_string()]);

    let snap = session.snapshot();
    assert_eq!(snap.components.keys().collect::<Vec<_>>(), vec!["Consumer"]);
    assert!(
        !matches!(
            snap.components["Consumer"].props.get("x").map(|p| &p.prop_type),
            Some(crate::types::PropType::String)
        ),
        "Consumer must not still resolve 'x' against the deleted declaration, got {:?}",
        snap.components["Consumer"].props.get("x")
    );
    assert_matches_cold(&session);
}

// ── SPEC-PIPELINE-001 AC-024: update_file() on a file whose on-disk
// content is not valid UTF-8 (distinct from AC-005's non-UTF8 *path*)
// surfaces an IoError and does not re-resolve that file's own component.

#[test]
fn update_file_with_non_utf8_content_surfaces_io_error_and_removes_that_files_own_component() {
    // Canonical, so the component's recorded file path can actually match (macOS temp dirs live under a symlink).
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().expect("canonicalize")).expect("utf8 path");
    let file_path = dir.join("Bad.tsx");
    std::fs::write(file_path.as_std_path(), "export function Bad(props: { x: string }) { return null; }\n")
        .expect("write Bad.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();
    assert_eq!(session.snapshot().components["Bad"].file_path, file_path, "sanity: the paths compare equal");

    std::fs::write(file_path.as_std_path(), [0x42, 0xFF, 0xFE, 0x00, 0xFF]).expect("write invalid utf8 bytes");
    let update = session.update_file(&file_path);

    assert!(
        update
            .diagnostics
            .iter()
            .any(|d| d.code == crate::types::DiagnosticCode::IoError && d.message.contains("Bad.tsx")),
        "expected an IoError diagnostic naming Bad.tsx, got {:?}",
        update.diagnostics
    );
    assert_eq!((update.updated_keys, update.removed_keys), (vec![], vec!["Bad".to_string()]));
    assert_matches_cold(&session);
}

#[test]
fn renaming_a_component_between_edits_removes_its_old_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let file_path = dir.join("Widget.tsx");
    std::fs::write(file_path.as_std_path(), "export function Card(props: { label: string }) { return null; }\n")
        .expect("write Widget.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();
    assert!(session.snapshot().components.contains_key("Card"), "sanity check: Card should exist after initialize()");

    // Rename Card -> Panel within the same file.
    std::fs::write(file_path.as_std_path(), "export function Panel(props: { label: string }) { return null; }\n")
        .expect("rewrite Widget.tsx");
    let update = session.update_file(&file_path);

    let keys: Vec<String> = session.snapshot().components.into_keys().collect();
    assert_eq!(keys, vec!["Panel".to_string()]);
    assert_eq!(update.updated_keys, vec!["Panel".to_string()]);
    assert_eq!(update.removed_keys, vec!["Card".to_string()]);
}

#[test]
fn editing_a_disambiguated_component_updates_its_own_key_not_the_bare_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = camino::Utf8Path::from_path(tmp.path()).expect("utf8 path").to_owned();
    let a_path = dir.join("A.tsx");
    let b_path = dir.join("B.tsx");
    // Both files declare a component named "Button" — A sorts first
    // lexically, so Phase 5 gives A the bare key and B the disambiguated one.
    std::fs::write(a_path.as_std_path(), "export function Button(props: { a: string }) { return null; }\n")
        .expect("write A.tsx");
    std::fs::write(b_path.as_std_path(), "export function Button(props: { b: string }) { return null; }\n")
        .expect("write B.tsx");
    // Canonicalize before comparing — on macOS the tempdir's own path
    // (/var/...) differs from its canonical form (/private/var/...),
    // same class of thing canonicalize_best_effort exists to handle, and
    // discover_files canonicalizes every path it reports at cold-extract
    // time, so the CACHED file_path is always the canonical form.
    let a_path = a_path.canonicalize_utf8().expect("canonicalize A.tsx");
    let b_path = b_path.canonicalize_utf8().expect("canonicalize B.tsx");

    let options = PipelineOptions { src_dirs: vec![dir], ..Default::default() };
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    let before = session.snapshot().components;
    assert_eq!(before["Button"].file_path, a_path, "expected A.tsx to hold the bare key (sorts first)");
    let disambiguated_key = format!("Button ({b_path})");
    assert!(
        before.contains_key(&disambiguated_key),
        "expected B.tsx's Button to be disambiguated, got keys {:?}",
        before.keys().collect::<Vec<_>>()
    );

    // Edit B.tsx only — A.tsx is untouched.
    std::fs::write(b_path.as_std_path(), "export function Button(props: { b: string; c: string }) { return null; }\n")
        .expect("rewrite B.tsx");
    let update = session.update_file(&b_path);
    assert_eq!(update.updated_keys, vec![disambiguated_key.clone()]);

    let after = session.snapshot().components;
    assert_eq!(after["Button"].file_path, a_path, "A's bare entry must not be overwritten by B's edit");
    assert_eq!(after["Button"].props.keys().collect::<Vec<_>>(), vec!["a"]);
    assert_eq!(after[&disambiguated_key].props.keys().collect::<Vec<_>>(), vec!["b", "c"]);
}

// ── parity with the one-shot extract() path ─────────────────────────────────

fn write_project(files: &[(&str, &str)]) -> (tempfile::TempDir, PipelineOptions) {
    let tmp = tempfile::TempDir::new().unwrap();
    for (name, body) in files {
        let path = tmp.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    let options = PipelineOptions {
        src_dirs: vec![Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap()],
        cache_dir: Some(Utf8PathBuf::from_path_buf(tmp.path().join(".cache")).unwrap()),
        ..Default::default()
    };
    (tmp, options)
}

/// The parts of an output a consumer sees, as JSON: components and public enums.
fn visible(output: &crate::types::ExtractionOutput) -> serde_json::Value {
    serde_json::json!({ "components": output.components, "enums": output.enums })
}

#[test]
fn snapshot_lists_public_enums_under_their_bare_names_like_extract() {
    let (_tmp, options) =
        write_project(&[("Size.ts", "export enum Size { Small = \"sm\" }\nenum Hidden { A = \"a\" }\n")]);
    let cold = crate::pipeline::extract(&options);
    let session = WatchSession::new(options);
    let _initial = session.initialize();

    assert_eq!(cold.enums.keys().collect::<Vec<_>>(), vec!["Size"]);
    assert_eq!(visible(&session.snapshot()), visible(&cold));
}

#[test]
fn a_new_import_edge_added_by_an_edit_makes_the_importer_re_resolve_on_later_edits() {
    let (tmp, options) = write_project(&[
        ("types.ts", "export interface BaseProps { a?: string }\n"),
        (
            "Button.tsx",
            "export interface OwnProps { own?: string }\nexport function Button(props: OwnProps) { return null }\n",
        ),
    ]);
    let session = WatchSession::new(options);
    let _initial = session.initialize();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();

    // Edit 1: Button starts extending a type imported from types.ts, a dependency edge that did not exist at init.
    std::fs::write(
        root.join("Button.tsx"),
        "import type { BaseProps } from \"./types\";\nexport interface OwnProps extends BaseProps { own?: string }\nexport function Button(props: OwnProps) { return null }\n",
    )
    .unwrap();
    let _update = session.update_file(&root.join("Button.tsx"));

    // Edit 2: change only the dependency. Button must be re-resolved with the new props.
    std::fs::write(root.join("types.ts"), "export interface BaseProps { a?: string; b?: number }\n").unwrap();
    let update = session.update_file(&root.join("types.ts"));

    let button = update.updated_components.iter().find(|c| c.display_name == "Button").expect("Button re-resolved");
    assert_eq!(button.props.keys().collect::<Vec<_>>(), vec!["a", "b", "own"]);
}

#[test]
fn an_edit_gives_the_same_visible_output_as_a_cold_extract_of_the_edited_project() {
    let (tmp, options) = write_project(&[
        (
            "Button.tsx",
            "export interface ButtonProps { label?: string }\nexport function Button(props: ButtonProps) { return null }\n",
        ),
        ("Size.ts", "export enum Size { Small = \"sm\" }\n"),
    ]);
    let session = WatchSession::new(options.clone());
    let _initial = session.initialize();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();

    std::fs::write(
        root.join("Button.tsx"),
        "export interface ButtonProps { label?: string; size?: number }\nexport function Button(props: ButtonProps) { return null }\n",
    )
    .unwrap();
    let _update = session.update_file(&root.join("Button.tsx"));

    assert_eq!(visible(&session.snapshot()), visible(&crate::pipeline::extract(&options)));
}

#[test]
fn a_wrapper_component_with_unresolvable_props_is_kept_by_update_file_like_extract() {
    let (tmp, options) =
        write_project(&[("Widgets.tsx", "export const Real = wrap<HTMLDivElement, { label?: string }>(\"div\");\n")]);
    let session = WatchSession::new(options.clone());
    let _initial = session.initialize();
    let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(tmp.path()).unwrap()).unwrap();

    std::fs::write(
        root.join("Widgets.tsx"),
        "import type { MissingProps } from \"./missing\";\nexport const Ghost = wrap<HTMLDivElement, MissingProps>(\"div\");\n",
    )
    .unwrap();
    let update = session.update_file(&root.join("Widgets.tsx"));

    assert_eq!(update.updated_components.iter().map(|c| c.display_name.as_str()).collect::<Vec<_>>(), vec!["Ghost"]);
    assert_eq!(visible(&session.snapshot()), visible(&crate::pipeline::extract(&options)));
}

/// One step of a watch session: write or delete each file, then report every touched path to `update_file`.
enum Step<'s> {
    Write(&'s str, &'s str),
    WriteBytes(&'s str, &'s [u8]),
    Delete(&'s str),
}

#[test]
fn every_kind_of_change_leaves_the_snapshot_equal_to_a_cold_extract() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().expect("canonicalize")).expect("utf8");
    let src = root.join("src");
    std::fs::create_dir_all(src.join("button")).expect("create src/button");
    let options =
        PipelineOptions { src_dirs: vec![src.clone()], cache_dir: Some(root.join("cache")), ..Default::default() };
    let files: &[(&str, &str)] = &[
        ("shared.ts", "export interface SharedProps { size: 'sm' | 'lg'; }\n"),
        ("index.ts", "export type { SharedProps } from './shared';\n"),
        ("button/Button.tsx", "import type { SharedProps } from '../index';\nexport function Button(props: SharedProps) { return null; }\n"),
        ("Card.tsx", "export function Card(props: { title: string }) { return null; }\n"),
        ("sizes.ts", "export const SIZES = ['sm', 'lg'] as const;\n"),
        (
            "Chip.tsx",
            "import { SIZES } from './sizes';\nimport type { Later } from './later';\nexport function Chip(props: { size: (typeof SIZES)[number]; later?: Later }) { return null; }\n",
        ),
    ];
    for (name, body) in files {
        std::fs::write(src.join(name), body).expect("write fixture");
    }
    let session = WatchSession::new(options);
    let _ignored = session.initialize();
    assert_matches_cold(&session);

    let steps: Vec<(&str, Vec<Step<'_>>)> = vec![
        (
            "edit a dependency two hops away",
            vec![Step::Write(
                "shared.ts",
                "export interface SharedProps { size: 'sm' | 'md' | 'lg'; tone?: string; }\n",
            )],
        ),
        (
            "edit the barrel",
            vec![Step::Write(
                "index.ts",
                "export type { SharedProps } from './shared';\nexport type Extra = { id: string };\n",
            )],
        ),
        (
            "edit a component's own file",
            vec![Step::Write(
                "Card.tsx",
                "export function Card(props: { title: string; footer?: string }) { return null; }\n",
            )],
        ),
        (
            "rename a component",
            vec![Step::Write("Card.tsx", "export function Panel(props: { title: string }) { return null; }\n")],
        ),
        (
            "create a component that collides with another name",
            vec![Step::Write("button/Panel.tsx", "export function Panel(props: { open: boolean }) { return null; }\n")],
        ),
        (
            "excluded story file",
            vec![Step::Write("Card.stories.tsx", "export function Story(props: { x: number }) { return null; }\n")],
        ),
        ("stop being a component", vec![Step::Write("button/Panel.tsx", "export const panelHeight = 4;\n")]),
        ("break a file", vec![Step::Write("Card.tsx", "export const = ;\n")]),
        ("make a file unreadable", vec![Step::WriteBytes("Card.tsx", &[0x42, 0xFF, 0xFE])]),
        ("fix it", vec![Step::Write("Card.tsx", "export function Card(props: { title: string }) { return null; }\n")]),
        ("delete a dependency", vec![Step::Delete("shared.ts")]),
        ("restore it", vec![Step::Write("shared.ts", "export interface SharedProps { size: 'sm'; }\n")]),
        (
            "edit a const array read by bare name",
            vec![Step::Write("sizes.ts", "export const SIZES = ['sm', 'md', 'lg'] as const;\n")],
        ),
        (
            "create the module an import was missing",
            vec![Step::Write("later.ts", "export interface Later { id: string; }\n")],
        ),
        (
            "move a file",
            vec![
                Step::Delete("Card.tsx"),
                Step::Write("button/Card.tsx", "export function Card(props: { title: string }) { return null; }\n"),
            ],
        ),
    ];
    for (what, changes) in steps {
        let mut touched = Vec::new();
        for change in &changes {
            match change {
                Step::Write(name, body) => {
                    std::fs::write(src.join(name), body).expect("write");
                    touched.push(src.join(name));
                }
                Step::WriteBytes(name, bytes) => {
                    std::fs::write(src.join(name), bytes).expect("write");
                    touched.push(src.join(name));
                }
                Step::Delete(name) => {
                    std::fs::remove_file(src.join(name)).expect("delete");
                    touched.push(src.join(name));
                }
            }
        }
        for path in &touched {
            let _ignored = session.update_file(path);
        }
        let watched = session.snapshot();
        let cold = crate::pipeline::extract(&session.options);
        assert_eq!(watched.components, cold.components, "components after: {what}");
        assert_eq!(watched.enums, cold.enums, "enums after: {what}");
        assert_eq!(watched.diagnostics, cold.diagnostics, "diagnostics after: {what}");
    }
}

#[test]
fn an_update_reports_the_keys_it_changed_and_removed() {
    let (_tmp, widget, options) = widget_project("export function Widget(props: { a: string }) { return null; }\n");
    let session = WatchSession::new(options);
    let _ignored = session.initialize();

    std::fs::write(&widget, "export function Gadget(props: { a: string }) { return null; }\n").expect("rename");
    let first = session.update_file(&widget);
    assert_eq!((first.updated_keys, first.removed_keys), (vec!["Gadget".to_string()], vec!["Widget".to_string()]));

    std::fs::write(&widget, "export const gadget = 1;\n").expect("stop being a component");
    let second = session.update_file(&widget);
    assert_eq!((second.updated_keys, second.removed_keys), (vec![], vec!["Gadget".to_string()]));
    assert!(second.revision > first.revision, "revisions increase: {} then {}", first.revision, second.revision);
}
