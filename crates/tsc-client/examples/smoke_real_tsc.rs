#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! Manual smoke check against the real `typescript@next` nightly — not a `cargo test`
//! (CI has no reason to depend on an unstable nightly build), but a one-off run to
//! confirm the fake-tsc harness's assumptions about wire behavior (framing, exit
//! codes) still match reality. Run from the repo root:
//!
//! ```sh
//! cargo run -p caliper-docgen-tsc-client --example smoke_real_tsc
//! ```
//!
//! Requires `apps/validate`'s `typescript-next` dependency to be installed (`pnpm
//! install` from the repo root) — see docs/type-checker-integration.md.

use std::time::Duration;

use caliper_docgen_tsc_client::{Supervisor, SupervisorConfig};
use camino::Utf8PathBuf;

#[tokio::main]
async fn main() {
    // Assumes invocation from the repo root, per this file's doc comment.
    let repo_root = Utf8PathBuf::from_path_buf(std::env::current_dir().expect("cwd")).expect("repo root is UTF-8");

    let tsc_path = repo_root.join("apps/validate/node_modules/typescript-next/bin/tsc");
    let package_json_path = repo_root.join("apps/validate/node_modules/typescript-next/package.json");
    let version = std::fs::read_to_string(&package_json_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|value| value.get("version").and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_else(|| panic!("could not read version from {package_json_path}"));

    println!("Spawning {tsc_path} (version {version})...");

    // Leaked to get a `&'static str` cheaply for this one-off binary — not a pattern
    // to repeat in real caller code, which knows its pinned versions at compile time.
    let expected: &'static str = Box::leak(version.into_boxed_str());
    let expected: &'static [&'static str] = Box::leak(vec![expected].into_boxed_slice());
    let mut config = SupervisorConfig::new(tsc_path, repo_root, expected);
    config.handshake_timeout = Duration::from_secs(30); // generous for a cold nightly spawn

    let mut supervisor = Supervisor::spawn(config).await.expect("spawn real tsc");
    println!("Spawned. Sending `initialize`...");

    let response = supervisor
        .request("initialize", serde_json::json!(null), Duration::from_secs(10))
        .await
        .expect("initialize request");
    println!("Response (seq {}): {}", response.seq, response.value);

    let started = std::time::Instant::now();
    let status = supervisor.shutdown().await.expect("shutdown");
    println!("Shut down in {:?} (exit status: {status}).", started.elapsed());
}
