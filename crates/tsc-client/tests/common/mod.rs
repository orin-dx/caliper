#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Shared helper for the Lifecycle-gate integration tests: builds a `SupervisorConfig`
//! pointing at a fresh copy of the `fake-tsc` binary inside a `node_modules/typescript/
//! bin/` layout, so `check_version`'s `package.json` lookup finds a real, matching
//! version rather than needing every test to fake that out itself.
//!
//! Each `tests/*.rs` file compiles this module separately as its own crate, so a
//! helper unused by one particular test binary is expected, not a real dead-code
//! finding — hence the blanket allow below.
#![allow(dead_code)]

use caliper_docgen_tsc_client::SupervisorConfig;
use camino::Utf8PathBuf;

pub const PINNED_VERSION: &str = "7.1.0-test";

/// Returns the temp dir (kept alive for the caller's test duration) and a config ready
/// to `Supervisor::spawn`, with `FAKE_TSC_SCENARIO` set to `scenario`.
pub fn spawn_fake(scenario: &str) -> (tempfile::TempDir, SupervisorConfig) {
    let dir = tempfile::tempdir().expect("create temp dir for fake-tsc layout");
    let (tsc_path, cwd) = layout_with_version(&dir, PINNED_VERSION);
    let mut config = SupervisorConfig::new(tsc_path, cwd, &[PINNED_VERSION]);
    config.extra_env.push(("FAKE_TSC_SCENARIO".to_string(), scenario.to_string()));
    (dir, config)
}

/// Number of processes the fake has started in `cwd`: one line per spawn in `fake-tsc-spawns.log`.
pub fn spawn_count(cwd: &camino::Utf8Path) -> usize {
    std::fs::read_to_string(cwd.join("fake-tsc-spawns.log")).map_or(0, |contents| contents.lines().count())
}

/// Waits until the fake has logged at least `at_least` spawns, then lets a spawn that should not have happened show
/// up. A child that is slow to start may not have logged yet when a short-deadline request has already returned.
pub async fn settled_spawn_count(cwd: &camino::Utf8Path, at_least: usize) -> usize {
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while spawn_count(cwd) < at_least && std::time::Instant::now() < give_up {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    spawn_count(cwd)
}

/// Builds the `node_modules/typescript/{package.json,bin/tsc}` layout with the given
/// version string and a copy of the real `fake-tsc` binary at `bin/tsc`. Returns
/// `(tsc_path, cwd)`.
pub fn layout_with_version(dir: &tempfile::TempDir, version: &str) -> (Utf8PathBuf, Utf8PathBuf) {
    let typescript_dir = dir.path().join("node_modules").join("typescript");
    let bin_dir = typescript_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create node_modules/typescript/bin");
    std::fs::write(typescript_dir.join("package.json"), format!(r#"{{"version": "{version}"}}"#))
        .expect("write package.json");

    let fake_tsc_src = std::path::PathBuf::from(env!("CARGO_BIN_EXE_fake-tsc"));
    let tsc_dest = bin_dir.join(if cfg!(windows) { "tsc.exe" } else { "tsc" });
    std::fs::copy(&fake_tsc_src, &tsc_dest).expect("copy fake-tsc binary into place");

    (
        Utf8PathBuf::from_path_buf(tsc_dest).expect("temp path is UTF-8"),
        Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("temp path is UTF-8"),
    )
}

/// Same layout, but without ever placing a binary at `bin/tsc` — for the missing-binary
/// case.
pub fn layout_without_binary(dir: &tempfile::TempDir, version: &str) -> (Utf8PathBuf, Utf8PathBuf) {
    let typescript_dir = dir.path().join("node_modules").join("typescript");
    std::fs::create_dir_all(&typescript_dir).expect("create node_modules/typescript");
    std::fs::write(typescript_dir.join("package.json"), format!(r#"{{"version": "{version}"}}"#))
        .expect("write package.json");
    let tsc_path = typescript_dir.join("bin").join(if cfg!(windows) { "tsc.exe" } else { "tsc" });
    (
        Utf8PathBuf::from_path_buf(tsc_path).expect("temp path is UTF-8"),
        Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("temp path is UTF-8"),
    )
}
