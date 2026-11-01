#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
mod common;

use caliper_docgen_tsc_client::{Supervisor, SupervisorConfig, TscClientError};

async fn spawn_with(installed: &str, accepted: &'static [&'static str]) -> Result<(), TscClientError> {
    let dir = tempfile::tempdir().expect("temp dir");
    let (tsc_path, cwd) = common::layout_with_version(&dir, installed);
    let supervisor = Supervisor::spawn(SupervisorConfig::new(tsc_path, cwd, accepted)).await?;
    supervisor.shutdown().await.expect("shuts down");
    Ok(())
}

#[tokio::test]
async fn a_series_entry_takes_every_release_of_that_series() {
    for installed in ["7.1.0-beta", "7.1.0-rc.1", "7.1.0", "7.1.4"] {
        spawn_with(installed, &["7.1"]).await.unwrap_or_else(|error| panic!("{installed}: {error}"));
    }
}

#[tokio::test]
async fn a_series_entry_rejects_other_series_and_near_misses() {
    for installed in ["7.10.0", "7.2.0", "7.0.2", "6.0.3", "7.1x"] {
        match spawn_with(installed, &["7.1"]).await {
            Err(TscClientError::UnsupportedVersion { found, .. }) => assert_eq!(found, installed),
            other => panic!("{installed}: expected UnsupportedVersion, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn an_exact_entry_still_takes_only_that_version() {
    spawn_with("7.1.0-beta", &["7.1.0-beta"]).await.expect("the exact version");
    assert!(matches!(spawn_with("7.1.0-rc.1", &["7.1.0-beta"]).await, Err(TscClientError::UnsupportedVersion { .. })));
}
