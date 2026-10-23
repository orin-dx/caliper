use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use caliper_docgen_core::pipeline::{PipelineOptions, WatchSession};
use caliper_docgen_core::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity, ExtractionOutput};
use camino::Utf8PathBuf;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use miette::{IntoDiagnostic, Result};
use watchexec_signals::Signal;

use crate::config::{build_options, BuildOptionsArgs};
use crate::output::{print_diagnostics, print_summary, write_atomic};

/// Watch mode never runs `--strict` — there's no CLI flag for it — so this is always `exit_code(false)`.
fn watch_exit_code(output: &ExtractionOutput) -> i32 {
    output.exit_code(false)
}

/// Builds the options, prints the banner, and runs the first extraction. Key shortcuts are advertised only when
/// `interactive`.
fn start_session(
    args: &crate::WatchArgs,
    quiet: bool,
    interactive: bool,
    config_path: Option<&str>,
) -> Result<(PipelineOptions, Arc<WatchSession>, Arc<AtomicI32>)> {
    use indicatif::{ProgressBar, ProgressStyle};
    use owo_colors::OwoColorize;

    let options = build_options(BuildOptionsArgs {
        src: &args.src,
        cache_dir: None,
        html_attributes: None,
        config_path,
        extra_builtins: &[],
    })?;

    if !quiet {
        println!();
        let hint = if interactive { "  (press q to quit, r to re-extract)" } else { "" };
        println!(
            "  {}  {} watching {}{}",
            "⚡".yellow(),
            "caliper-docgen".bold(),
            options.src_dirs.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ").cyan(),
            hint.dimmed()
        );
        println!();
    }

    let pb = if !quiet {
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}").into_diagnostic()?);
        pb.set_message("Extracting...");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };

    let session = Arc::new(WatchSession::new(options.clone()));
    let first = session.initialize();

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }
    if !quiet {
        print_summary(&first, quiet);
        print_diagnostics(&first.diagnostics);
    }

    let exit_code = Arc::new(AtomicI32::new(0));
    publish(&first, args.out.as_deref(), &exit_code);
    Ok((options, session, exit_code))
}

/// Tracks the exit code from the whole `snapshot` and rewrites `--out` with it.
fn publish(snapshot: &ExtractionOutput, out: Option<&str>, exit_code: &AtomicI32) {
    exit_code.store(watch_exit_code(snapshot), Ordering::Relaxed);
    if let Some(failure) = out.and_then(|out| write_snapshot(out, snapshot)) {
        print_diagnostics(&[failure]);
    }
}

/// `Some(exit code)` when `key` quits. Raw mode turns off ISIG, so Ctrl-C arrives here as a key, not SIGINT. `r`
/// extracts everything again, for changes the watcher cannot see (a package's files in `node_modules`).
fn handle_key(
    key: KeyEvent,
    session: &WatchSession,
    quiet: bool,
    out: Option<&str>,
    exit_code: &AtomicI32,
) -> Option<i32> {
    match key.code {
        KeyCode::Char('q') => Some(exit_code.load(Ordering::Relaxed)),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(exit_code.load(Ordering::Relaxed)),
        KeyCode::Char('r') => {
            let output = session.rebuild();
            if !quiet {
                print_summary(&output, quiet);
                print_diagnostics(&output.diagnostics);
            }
            publish(&output, out, exit_code);
            None
        }
        _ => None,
    }
}

/// Feeds key presses to `on_key` until it returns an exit code; `Err` once input can no longer be read.
fn read_keys(
    mut next: impl FnMut() -> std::io::Result<Event>,
    mut on_key: impl FnMut(KeyEvent) -> Option<i32>,
) -> std::io::Result<i32> {
    loop {
        if let Event::Key(key) = next()? {
            if let Some(code) = on_key(key) {
                return Ok(code);
            }
        }
    }
}

/// Raw mode is for reading single keys, but it also turns off output processing, so every `\n` printed during the
/// session would leave the cursor in its column. Turn output processing back on; input stays raw.
#[cfg(unix)]
fn keep_output_processing() -> std::io::Result<()> {
    use rustix::termios::{tcgetattr, tcsetattr, OptionalActions, OutputModes};

    let stdin = std::io::stdin();
    let mut termios = tcgetattr(&stdin)?;
    termios.output_modes |= OutputModes::OPOST | OutputModes::ONLCR;
    tcsetattr(&stdin, OptionalActions::Now, &termios)?;
    Ok(())
}

#[cfg(not(unix))]
fn keep_output_processing() -> std::io::Result<()> {
    Ok(())
}

fn spawn_keyboard_thread(session: Arc<WatchSession>, quiet: bool, out: Option<String>, exit_code: Arc<AtomicI32>) {
    std::thread::spawn(move || {
        if let Err(error) = crossterm::terminal::enable_raw_mode() {
            tracing::warn!("key shortcuts unavailable: {error}");
            return;
        }
        if let Err(error) = keep_output_processing() {
            tracing::warn!("output lines may not start at column 0: {error}");
        }
        let quit =
            read_keys(crossterm::event::read, |key| handle_key(key, &session, quiet, out.as_deref(), &exit_code));
        crossterm::terminal::disable_raw_mode().ok();
        match quit {
            // watchexec has no graceful-quit handle reachable from this thread, so this hard-exits, with the tracked
            // code so a session that quit on an unresolved error still fails the shell.
            Ok(code) => std::process::exit(code),
            Err(error) => tracing::warn!("key shortcuts stopped: {error}; stop watching with Ctrl-C"),
        }
    });
}

/// watchexec swallows the signals its handler doesn't act on, so these must end the session explicitly.
fn ends_session(signal: Signal) -> bool {
    matches!(signal, Signal::Interrupt | Signal::Terminate | Signal::Hangup | Signal::Quit)
}

/// Puts the terminal back in cooked mode when the session ends, however it ends.
struct RestoreTerminal;

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        crossterm::terminal::disable_raw_mode().ok();
    }
}

/// Returns the diagnostic to report when the snapshot can't be written to `path`.
fn write_snapshot(path: &str, snapshot: &ExtractionOutput) -> Option<Diagnostic> {
    let json = serde_json::to_string(snapshot).ok()?;
    let error = write_atomic(path, &json).err()?;
    Some(Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: format!("Failed to write '{path}': {error}"),
        file: Some(path.to_owned()),
        line: None,
        column: None,
        help: Some("Check that the output path's parent directory exists and is writable.".into()),
        code: DiagnosticCode::IoError,
        affects: Vec::new(),
    })
}

/// One file-change event: applies `path` to the session, prints the delta, tracks the exit code and rewrites `--out`.
/// An event that produced no new revision (a non-source file, an excluded story) changes nothing.
fn handle_change(session: &WatchSession, path: &Path, quiet: bool, out: Option<&str>, exit_code: &AtomicI32) {
    let Ok(path) = camino::Utf8PathBuf::from_path_buf(path.to_owned()) else { return };

    let before = session.revision();
    let update = session.update_file(&path);
    if update.revision == before {
        return;
    }

    if !quiet {
        use owo_colors::OwoColorize;
        let file = path.file_name().unwrap_or("?");
        if !update.updated_keys.is_empty() {
            println!("  {}  {}", file.dimmed(), update.updated_keys.join(", ").bold());
        }
        if !update.removed_keys.is_empty() {
            println!("  {}  removed {}", file.dimmed(), update.removed_keys.join(", "));
        }
        print_diagnostics(&update.diagnostics);
    }
    // The cumulative snapshot, not this event's delta: a clean update to one file must not reset the exit code while
    // an earlier file in the session still has an unresolved error.
    publish(&session.snapshot(), out, exit_code);
}

/// The source directories, and the tsconfigs, `package.json` files and lockfiles above them: editing one rebuilds the
/// session (`WatchSession::update_file`).
fn watched_paths(options: &PipelineOptions, session: &WatchSession) -> Vec<std::path::PathBuf> {
    let src_dirs = options.src_dirs.iter().map(|dir| dir.as_std_path().to_owned());
    src_dirs.chain(session.resolution_config_files().into_iter().map(Utf8PathBuf::into_std_path_buf)).collect()
}

pub fn cmd_watch(args: crate::WatchArgs, quiet: bool, config_path: Option<&str>) -> Result<i32> {
    // Without a terminal on stdin (CI, a pipe, a service) there are no keys to read, and reading anyway fails at once.
    let interactive = std::io::stdin().is_terminal();
    let (options, session, exit_code) = start_session(&args, quiet, interactive, config_path)?;

    let _restore_terminal = interactive.then_some(RestoreTerminal);
    if interactive {
        spawn_keyboard_thread(session.clone(), quiet, args.out.clone(), exit_code.clone());
    }

    // watchexec's constructor is synchronous even though its event loop is async.
    let watched = watched_paths(&options, &session);

    let rt = tokio::runtime::Runtime::new().into_diagnostic()?;
    let handler_exit_code = exit_code.clone();
    rt.block_on(async move {
        use watchexec::Watchexec;

        let wx = Watchexec::new(move |mut action| {
            if action.signals().any(ends_session) {
                action.quit();
                return action;
            }
            for event in action.events.iter() {
                for (path, _) in event.paths() {
                    handle_change(&session, path, quiet, args.out.as_deref(), &handler_exit_code);
                }
            }
            action
        })
        .into_diagnostic()?;

        wx.config.pathset(watched);
        wx.main().await.into_diagnostic()??;
        Ok::<(), miette::Error>(())
    })?;

    Ok(exit_code.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn empty_output() -> ExtractionOutput {
        ExtractionOutput {
            components: Default::default(),
            enums: Default::default(),
            diagnostics: vec![],
            stats: Default::default(),
        }
    }

    #[test]
    fn watch_exit_code_mirrors_extraction_output_exit_code_non_strict() {
        assert_eq!(watch_exit_code(&empty_output()), 0);

        let with_error = ExtractionOutput {
            diagnostics: vec![Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: "boom".into(),
                file: None,
                line: None,
                column: None,
                help: None,
                code: DiagnosticCode::Unknown,
                affects: Vec::new(),
            }],
            ..empty_output()
        };
        assert_eq!(watch_exit_code(&with_error), 2);
    }

    const WIDGET: &str = "export function Widget(props: { label: string }) { return null; }\n";

    /// A session over `src`; the DTS cache lives in `scratch` so it never lands inside the watched tree.
    fn session_over(src: &TempDir, scratch: &TempDir) -> WatchSession {
        WatchSession::new(PipelineOptions {
            src_dirs: vec![Utf8PathBuf::from_path_buf(src.path().to_owned()).unwrap()],
            cache_dir: Some(Utf8PathBuf::from_path_buf(scratch.path().join("cache")).unwrap()),
            ..Default::default()
        })
    }

    fn prop_names(written: &serde_json::Value, component: &str) -> Vec<String> {
        written["components"][component]["props"].as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn a_tsconfig_change_rebuilds_and_rewrites_out() {
        let root = TempDir::new().unwrap();
        let root_dir = Utf8PathBuf::from_path_buf(root.path().canonicalize().unwrap()).unwrap();
        std::fs::create_dir_all(root_dir.join("src/ui")).unwrap();
        std::fs::write(root_dir.join("tsconfig.json"), "{}").unwrap();
        std::fs::write(root_dir.join("package.json"), "{}").unwrap();
        std::fs::write(root_dir.join("src/ui/types.ts"), "export interface ButtonProps { label: string }\n").unwrap();
        std::fs::write(
            root_dir.join("src/Button.tsx"),
            "import type { ButtonProps } from '@ui/types';\nexport function Button(props: ButtonProps) { return null; }\n",
        )
        .unwrap();
        let options = PipelineOptions {
            src_dirs: vec![root_dir.join("src")],
            tsconfig_path: Some(root_dir.join("tsconfig.json")),
            cache_dir: Some(root_dir.join("cache")),
            ..Default::default()
        };
        let session = WatchSession::new(options.clone());
        let _ignored = session.initialize();

        let watched: Vec<std::path::PathBuf> =
            watched_paths(&options, &session).into_iter().filter(|p| p.starts_with(&root_dir)).collect();
        let expected: Vec<std::path::PathBuf> =
            ["src", "package.json", "tsconfig.json"].iter().map(|p| root_dir.join(p).into_std_path_buf()).collect();
        assert_eq!(watched, expected);

        std::fs::write(
            root_dir.join("tsconfig.json"),
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@ui/*": ["src/ui/*"] } } }"#,
        )
        .unwrap();
        let out = root_dir.join("out.json");
        let exit_code = AtomicI32::new(0);
        handle_change(&session, root_dir.join("tsconfig.json").as_std_path(), true, Some(out.as_str()), &exit_code);

        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(prop_names(&written, "Button"), ["label"]);
    }

    #[test]
    fn a_typescript_change_rewrites_out_with_the_cumulative_snapshot() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let widget = src.path().join("Widget.tsx");
        std::fs::write(&widget, WIDGET).unwrap();
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();

        std::fs::write(&widget, "export function Widget(props: { label: string; size: number }) { return null; }\n")
            .unwrap();
        let out = scratch.path().join("out.json");
        let exit_code = AtomicI32::new(7);
        handle_change(&session, &widget, true, out.to_str(), &exit_code);

        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(prop_names(&written, "Widget"), vec!["label", "size"]);
        assert_eq!(exit_code.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_non_typescript_event_changes_nothing() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let out = scratch.path().join("out.json");
        let exit_code = AtomicI32::new(7);

        handle_change(&session, &src.path().join("notes.md"), true, out.to_str(), &exit_code);

        assert!(!out.exists(), "a non-TS event must not rewrite --out");
        assert_eq!(exit_code.load(Ordering::Relaxed), 7);
    }

    #[test]
    fn an_excluded_file_event_changes_nothing() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let out = scratch.path().join("out.json");
        let story = src.path().join("Widget.stories.tsx");
        std::fs::write(&story, WIDGET).unwrap();

        handle_change(&session, &story, true, out.to_str(), &AtomicI32::new(7));

        assert!(!out.exists(), "an excluded file must not rewrite --out");
        assert_eq!(session.revision(), 1);
    }

    #[test]
    fn a_deleted_file_is_removed_from_out() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let widget = src.path().join("Widget.tsx");
        std::fs::write(&widget, WIDGET).unwrap();
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let out = scratch.path().join("out.json");

        std::fs::remove_file(&widget).unwrap();
        handle_change(&session, &widget, true, out.to_str(), &AtomicI32::new(7));

        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written["components"], serde_json::json!({}));
    }

    #[test]
    fn a_clean_change_does_not_reset_the_exit_code_of_an_earlier_error() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let widget = src.path().join("Widget.tsx");
        std::fs::write(&widget, WIDGET).unwrap();
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let exit_code = AtomicI32::new(0);

        // An unreadable file is an error-severity diagnostic that stays in the session's snapshot.
        let bad = src.path().join("Bad.tsx");
        std::fs::write(&bad, [0xff, 0xfe, 0xfd]).unwrap();
        handle_change(&session, &bad, true, None, &exit_code);
        assert_eq!(exit_code.load(Ordering::Relaxed), 2);

        handle_change(&session, &widget, true, None, &exit_code);
        assert_eq!(exit_code.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn an_unwritable_out_path_is_reported_as_an_io_error_naming_the_path() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("missing-dir").join("out.json");
        let path = path.to_str().unwrap();

        let failure = write_snapshot(path, &empty_output()).expect("writing into a missing directory must fail");

        assert_eq!(
            (failure.severity, failure.code, failure.file.as_deref()),
            (DiagnosticSeverity::Error, DiagnosticCode::IoError, Some(path))
        );
        assert!(failure.message.starts_with(&format!("Failed to write '{path}': ")), "message: {}", failure.message);
        assert_eq!(write_snapshot(scratch.path().join("ok.json").to_str().unwrap(), &empty_output()), None);
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn ctrl_c_quits_with_the_tracked_exit_code_but_a_bare_c_does_not() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let exit_code = AtomicI32::new(2);

        assert_eq!(handle_key(key('c'), &session, true, None, &exit_code), None);
        assert_eq!(
            handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &session, true, None, &exit_code),
            Some(2)
        );
    }

    #[test]
    fn keys_are_read_until_one_quits_and_other_events_are_skipped() {
        let mut events =
            vec![Ok(Event::Key(key('x'))), Ok(Event::FocusGained), Ok(Event::Key(key('q'))), Ok(Event::Key(key('z')))]
                .into_iter();
        let mut seen = vec![];

        let code = read_keys(
            || events.next().unwrap(),
            |key| {
                seen.push(key.code);
                (key.code == KeyCode::Char('q')).then_some(3)
            },
        );

        assert_eq!(code.unwrap(), 3);
        assert_eq!(seen, vec![KeyCode::Char('x'), KeyCode::Char('q')]);
    }

    #[test]
    fn reading_keys_stops_at_the_first_input_error_instead_of_retrying() {
        let mut reads = 0;

        let result = read_keys(
            || {
                reads += 1;
                Err(std::io::Error::other("no terminal"))
            },
            |_| None,
        );

        assert_eq!(result.unwrap_err().to_string(), "no terminal");
        assert_eq!(reads, 1);
    }

    #[test]
    fn termination_signals_end_the_session_and_user_signals_do_not() {
        for signal in [Signal::Interrupt, Signal::Terminate, Signal::Hangup, Signal::Quit] {
            assert!(ends_session(signal), "{signal:?}");
        }
        for signal in [Signal::User1, Signal::User2] {
            assert!(!ends_session(signal), "{signal:?}");
        }
    }

    #[test]
    fn q_quits_with_the_tracked_exit_code_and_other_keys_keep_running() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let exit_code = AtomicI32::new(2);

        assert_eq!(handle_key(key('q'), &session, true, None, &exit_code), Some(2));
        assert_eq!(handle_key(key('x'), &session, true, None, &exit_code), None);
        assert_eq!(exit_code.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn r_extracts_again_and_rewrites_out() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let out = scratch.path().join("out.json");
        let exit_code = AtomicI32::new(0);
        // Changes no event reported: a new component and an unreadable file.
        std::fs::write(src.path().join("Widget.tsx"), WIDGET).unwrap();
        std::fs::write(src.path().join("Bad.tsx"), [0xff, 0xfe, 0xfd]).unwrap();

        assert_eq!(handle_key(key('r'), &session, true, out.to_str(), &exit_code), None);

        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(prop_names(&written, "Widget"), vec!["label"]);
        assert_eq!(exit_code.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn start_session_runs_the_first_extraction_quiet_or_not() {
        for quiet in [true, false] {
            let src = TempDir::new().unwrap();
            std::fs::write(src.path().join("Widget.tsx"), WIDGET).unwrap();
            let dir = src.path().to_str().unwrap().to_owned();
            let out = src.path().join("out.json");
            let args = crate::WatchArgs { src: vec![dir.clone()], out: Some(out.to_str().unwrap().to_owned()) };

            let (options, session, exit_code) = start_session(&args, quiet, false, None).unwrap();

            assert_eq!(options.src_dirs, vec![Utf8PathBuf::from(dir)]);
            let components: Vec<String> = session.snapshot().components.keys().cloned().collect();
            assert_eq!(components, vec!["Widget".to_owned()]);
            assert_eq!(exit_code.load(Ordering::Relaxed), 0);
            let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
            assert_eq!(prop_names(&written, "Widget"), vec!["label"], "--out is written before the first change");
        }
    }
}
