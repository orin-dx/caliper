use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use caliper_docgen_core::pipeline::{PipelineOptions, WatchSession};
use caliper_docgen_core::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity, ExtractionOutput};
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
        no_cross_package: false,
        react_version: None,
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

    let exit_code = Arc::new(AtomicI32::new(watch_exit_code(&first)));
    Ok((options, session, exit_code))
}

/// `Some(exit code)` when `key` quits. Raw mode turns off ISIG, so Ctrl-C arrives here as a key, not SIGINT. `r` only
/// refreshes the tracked exit code: past its first call `initialize()` returns the current snapshot.
fn handle_key(key: KeyEvent, session: &WatchSession, exit_code: &AtomicI32) -> Option<i32> {
    match key.code {
        KeyCode::Char('q') => Some(exit_code.load(Ordering::Relaxed)),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(exit_code.load(Ordering::Relaxed)),
        KeyCode::Char('r') => {
            exit_code.store(watch_exit_code(&session.initialize()), Ordering::Relaxed);
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

fn spawn_keyboard_thread(session: Arc<WatchSession>, exit_code: Arc<AtomicI32>) {
    std::thread::spawn(move || {
        if let Err(error) = crossterm::terminal::enable_raw_mode() {
            tracing::warn!("key shortcuts unavailable: {error}");
            return;
        }
        let quit = read_keys(crossterm::event::read, |key| handle_key(key, &session, &exit_code));
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
    })
}

/// One file-change event: re-resolves a TypeScript `path`, tracks the exit code, prints the delta, rewrites `--out`.
fn handle_change(session: &WatchSession, path: &Path, quiet: bool, out: Option<&str>, exit_code: &AtomicI32) {
    if !matches!(path.extension().and_then(|e| e.to_str()), Some("ts" | "tsx")) {
        return;
    }
    let Ok(path) = camino::Utf8PathBuf::from_path_buf(path.to_owned()) else { return };

    let update = session.update_file(&path);
    // The cumulative snapshot, not this event's delta: a clean update to one file must not reset the exit code while
    // an earlier file in the session still has an unresolved error.
    let snapshot = session.snapshot();
    exit_code.store(watch_exit_code(&snapshot), Ordering::Relaxed);

    if !quiet {
        use owo_colors::OwoColorize;
        let names: Vec<_> = update.updated_components.iter().map(|c| c.display_name.as_str()).collect();
        if !names.is_empty() {
            println!("  {}  {}", path.file_name().unwrap_or("?").dimmed(), names.join(", ").bold());
        }
        print_diagnostics(&update.diagnostics);
    }
    if let Some(failure) = out.and_then(|out| write_snapshot(out, &snapshot)) {
        print_diagnostics(&[failure]);
    }
}

pub fn cmd_watch(args: crate::WatchArgs, quiet: bool, config_path: Option<&str>) -> Result<i32> {
    // Without a terminal on stdin (CI, a pipe, a service) there are no keys to read, and reading anyway fails at once.
    let interactive = std::io::stdin().is_terminal();
    let (options, session, exit_code) = start_session(&args, quiet, interactive, config_path)?;

    let _restore_terminal = interactive.then_some(RestoreTerminal);
    if interactive {
        spawn_keyboard_thread(session.clone(), exit_code.clone());
    }

    // watchexec's constructor is synchronous even though its event loop is async.
    let src_dirs: Vec<std::path::PathBuf> = options.src_dirs.iter().map(|p| p.as_std_path().to_owned()).collect();

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

        wx.config.pathset(src_dirs);
        wx.main().await.into_diagnostic()??;
        Ok::<(), miette::Error>(())
    })?;

    Ok(exit_code.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    use camino::Utf8PathBuf;
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
    fn a_clean_change_does_not_reset_the_exit_code_of_an_earlier_error() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let widget = src.path().join("Widget.tsx");
        std::fs::write(&widget, WIDGET).unwrap();
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        let exit_code = AtomicI32::new(0);

        // An unreadable file is an error-severity diagnostic that stays in the session's snapshot.
        handle_change(&session, &src.path().join("Missing.tsx"), true, None, &exit_code);
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

        assert_eq!(handle_key(key('c'), &session, &exit_code), None);
        assert_eq!(handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &session, &exit_code), Some(2));
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

        assert_eq!(handle_key(key('q'), &session, &exit_code), Some(2));
        assert_eq!(handle_key(key('x'), &session, &exit_code), None);
        assert_eq!(exit_code.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn r_refreshes_the_tracked_exit_code_from_the_session() {
        let (src, scratch) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let session = session_over(&src, &scratch);
        let _ignored = session.initialize();
        // An unreadable file leaves an error-severity diagnostic in the session.
        handle_change(&session, &src.path().join("Missing.tsx"), true, None, &AtomicI32::new(0));
        let exit_code = AtomicI32::new(0);

        assert_eq!(handle_key(key('r'), &session, &exit_code), None);
        assert_eq!(exit_code.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn start_session_runs_the_first_extraction_quiet_or_not() {
        for quiet in [true, false] {
            let src = TempDir::new().unwrap();
            std::fs::write(src.path().join("Widget.tsx"), WIDGET).unwrap();
            let dir = src.path().to_str().unwrap().to_owned();
            let args = crate::WatchArgs { src: vec![dir.clone()], out: None };

            let (options, session, exit_code) = start_session(&args, quiet, false, None).unwrap();

            assert_eq!(options.src_dirs, vec![Utf8PathBuf::from(dir)]);
            let components: Vec<String> = session.snapshot().components.keys().cloned().collect();
            assert_eq!(components, vec!["Widget".to_owned()]);
            assert_eq!(exit_code.load(Ordering::Relaxed), 0);
        }
    }
}
