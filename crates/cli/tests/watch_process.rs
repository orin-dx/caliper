#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Process-level tests for `watch`: signals, exit codes and idling can only be observed on the real binary.

#![cfg(unix)]

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use rstest::rstest;
use rustix::pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags};
use rustix::termios::{tcgetattr, LocalModes};

mod common;
use common::strip_ansi;

const WIDGET: &str = "export function Widget(props: { label: string }) { return null; }\n";
const TIMEOUT: Duration = Duration::from_secs(20);

/// A pseudo-terminal: the child gets `slave` as its terminal, the test types into and reads from `master`.
struct Pty {
    master: File,
    slave: File,
}

fn open_pty() -> Pty {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let name = ptsname(&master, Vec::new()).unwrap();
    let slave = OpenOptions::new().read(true).write(true).open(Path::new(OsStr::from_bytes(name.to_bytes()))).unwrap();
    Pty { master: File::from(master), slave }
}

impl Pty {
    fn is_raw(&self) -> bool {
        !tcgetattr(&self.slave).unwrap().local_modes.contains(LocalModes::ICANON)
    }

    fn type_bytes(&self, bytes: &[u8]) {
        (&self.master).write_all(bytes).unwrap();
    }
}

/// A running `watch` over a temp dir, with its stdout and stderr lines as they arrive. Stdin is `/dev/null` unless it
/// runs in a [`Pty`].
struct Watch {
    child: Child,
    output: Receiver<String>,
    src: tempfile::TempDir,
    seen: Vec<String>,
    pty: Option<Pty>,
}

fn watched_dir() -> tempfile::TempDir {
    let src = tempfile::TempDir::new().unwrap();
    std::fs::write(src.path().join("Widget.tsx"), WIDGET).unwrap();
    src
}

fn spawn_watch(src: &Path, stdin: Stdio, stdout: Stdio, stderr: Stdio) -> Child {
    Command::new(env!("CARGO_BIN_EXE_caliper-docgen"))
        .args(["watch", "--src"])
        .arg(src)
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .expect("failed to spawn the real watch binary")
}

fn forward_lines(reader: impl Read + Send + 'static, lines: Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if lines.send(strip_ansi(line.trim_end_matches('\r'))).is_err() {
                return;
            }
        }
    });
}

impl Watch {
    fn start() -> Watch {
        let src = watched_dir();
        let mut child = spawn_watch(src.path(), Stdio::null(), Stdio::piped(), Stdio::piped());
        let (lines, output) = mpsc::channel();
        forward_lines(child.stdout.take().unwrap(), lines.clone());
        forward_lines(child.stderr.take().unwrap(), lines);
        Watch { child, output, src, seen: vec![], pty: None }
    }

    fn start_in_terminal() -> Watch {
        let src = watched_dir();
        let pty = open_pty();
        let terminal = || Stdio::from(pty.slave.try_clone().unwrap());
        let child = spawn_watch(src.path(), terminal(), terminal(), terminal());
        let (lines, output) = mpsc::channel();
        forward_lines(pty.master.try_clone().unwrap(), lines);
        Watch { child, output, src, seen: vec![], pty: Some(pty) }
    }

    fn pty(&self) -> &Pty {
        self.pty.as_ref().expect("started in a terminal")
    }

    /// Waits for the keyboard thread to put the terminal in raw mode, after which typed keys reach `watch` unbuffered.
    fn wait_until_raw(&self) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.pty().is_raw() {
            assert!(Instant::now() < deadline, "watch never put the terminal in raw mode; saw {:?}", self.seen);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn widget(&self) -> PathBuf {
        self.src.path().join("Widget.tsx")
    }

    /// The first line containing `needle` within `timeout`.
    fn line_containing(&mut self, needle: &str, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.output.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if line.contains(needle) {
                        return Some(line);
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn expect_line(&mut self, needle: &str) -> String {
        self.line_containing(needle, TIMEOUT)
            .unwrap_or_else(|| panic!("no output containing {needle:?}; saw {:?}", self.seen))
    }

    /// Rewrites the widget until `watch` reports the change, which proves its file watcher and signal handling are up.
    fn wait_until_watching(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        for n in 0.. {
            std::fs::write(self.widget(), format!("{WIDGET}// {n}\n")).unwrap();
            if self.line_containing("Widget.tsx", Duration::from_millis(300)).is_some() {
                return;
            }
            assert!(Instant::now() < deadline, "watch never reacted to a change; saw {:?}", self.seen);
        }
    }

    fn signal(&self, name: &str) {
        let status = Command::new("kill").args(["-s", name, &self.child.id().to_string()]).status().unwrap();
        assert!(status.success(), "kill -s {name} failed");
    }

    fn exit_code_within(&mut self, timeout: Duration) -> Option<Option<i32>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status.code());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    /// The exit code once the process has exited: `Some(code)` for a normal exit, `None` if a signal killed it.
    fn expect_exit(&mut self, after: &str) -> Option<i32> {
        self.exit_code_within(Duration::from_secs(10))
            .unwrap_or_else(|| panic!("watch was still running 10s after {after}; saw {:?}", self.seen))
    }

    /// CPU seconds the process has used, from `ps` (`[[dd-]hh:]mm:ss[.ff]`).
    fn cpu_seconds(&self) -> f64 {
        let out = Command::new("ps").args(["-o", "cputime=", "-p", &self.child.id().to_string()]).output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let (days, clock) = text.trim().split_once('-').map_or((0.0, text.trim()), |(d, c)| (d.parse().unwrap(), c));
        let clock = clock.split(':').fold(0.0, |total, part| total * 60.0 + part.parse::<f64>().unwrap());
        days * 86_400.0 + clock
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[rstest]
#[case("TERM")]
#[case("INT")]
#[case("HUP")]
#[case("QUIT")]
fn a_termination_signal_ends_a_clean_session_with_exit_code_0(#[case] signal: &str) {
    let mut watch = Watch::start();
    watch.wait_until_watching();

    watch.signal(signal);

    assert_eq!(watch.expect_exit(signal), Some(0));
}

#[test]
fn a_termination_signal_reports_the_exit_code_of_an_unresolved_error() {
    let mut watch = Watch::start();
    watch.wait_until_watching();
    std::fs::write(watch.widget(), [0xff, 0xfe, 0xfd]).unwrap();
    watch.expect_line("Failed to read");

    watch.signal("TERM");

    assert_eq!(watch.expect_exit("SIGTERM"), Some(2));
}

#[test]
fn watch_stays_idle_when_stdin_is_not_a_terminal() {
    let mut watch = Watch::start();
    watch.wait_until_watching();

    let before = watch.cpu_seconds();
    std::thread::sleep(Duration::from_secs(3));
    let used = watch.cpu_seconds() - before;

    // A busy loop uses ~3s here; `ps` reports whole seconds on Linux.
    assert!(used < 2.0, "used {used}s of CPU over 3s of wall time");
}

#[test]
fn the_banner_only_advertises_key_shortcuts_when_stdin_is_a_terminal() {
    let mut watch = Watch::start();

    let banner = watch.expect_line("watching");

    assert_eq!(banner, format!("  ⚡  caliper-docgen watching {}", watch.src.path().display()));
}

// ── In a terminal: the keys and the terminal mode, which only a pty can observe ─────────────────────────────────────

#[test]
fn a_terminal_session_advertises_its_key_shortcuts() {
    let mut watch = Watch::start_in_terminal();

    let banner = watch.expect_line("watching");

    assert_eq!(
        banner,
        format!("  ⚡  caliper-docgen watching {}  (press q to quit, r to re-extract)", watch.src.path().display())
    );
}

#[test]
fn q_quits_a_terminal_session_with_its_tracked_exit_code() {
    let mut watch = Watch::start_in_terminal();
    watch.wait_until_raw();

    watch.pty().type_bytes(b"q");

    assert_eq!(watch.expect_exit("q"), Some(0));
}

#[test]
fn ctrl_c_quits_a_terminal_session_but_a_bare_c_does_not() {
    let mut watch = Watch::start_in_terminal();
    watch.wait_until_raw();

    watch.pty().type_bytes(b"c");
    assert_eq!(watch.exit_code_within(Duration::from_millis(700)), None, "a bare c must not quit");

    watch.pty().type_bytes(&[0x03]);
    assert_eq!(watch.expect_exit("Ctrl-C"), Some(0));
}

#[test]
fn a_signal_restores_the_terminal_mode_the_session_changed() {
    let mut watch = Watch::start_in_terminal();
    watch.wait_until_raw();

    watch.signal("TERM");

    assert_eq!(watch.expect_exit("SIGTERM"), Some(0));
    assert!(!watch.pty().is_raw(), "the terminal was left in raw mode");
}
