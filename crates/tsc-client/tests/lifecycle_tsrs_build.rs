#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
//! The `tsrs` builds of the checker: their version reads `<tsrs version>-ts<typescript version>`, and the binary is
//! either the Node shim `tsrs/bin/tsrs` or the platform package's `@tsrs/<platform>/tsrs`, whose `package.json` is
//! beside it instead of one level up.
mod common;

use caliper_docgen_tsc_client::{Supervisor, SupervisorConfig, TscClientError};

const TSRS: &str = "0.23.0-ts7.1.0-beta";

async fn spawn_at(
    package_dir: &str,
    binary: &str,
    installed: &str,
    accepted: &'static [&'static str],
) -> Result<(), TscClientError> {
    let dir = tempfile::tempdir().expect("temp dir");
    let (path, cwd) = common::layout_at(&dir, package_dir, binary, installed);
    let supervisor = Supervisor::spawn(SupervisorConfig::new(path, cwd, accepted)).await?;
    supervisor.shutdown().await.expect("shuts down");
    Ok(())
}

#[tokio::test]
async fn a_traced_tsrs_build_of_an_allowed_typescript_is_accepted_through_the_shim() {
    spawn_at("tsrs", "tsrs/bin/tsrs", TSRS, &["7.1", TSRS]).await.expect("the traced build");
}

#[tokio::test]
async fn the_platform_binary_reads_the_package_json_beside_it() {
    spawn_at("@tsrs/darwin-arm64", "@tsrs/darwin-arm64/tsrs", TSRS, &["7.1", TSRS]).await.expect("the platform binary");
}

#[tokio::test]
async fn a_tsrs_build_whose_typescript_is_not_allowed_is_rejected_even_when_pinned() {
    // The pin names the tsrs build, but no traced TypeScript series covers its checker.
    match spawn_at("tsrs", "tsrs/bin/tsrs", TSRS, &[TSRS]).await {
        Err(TscClientError::UnsupportedVersion { found, .. }) => assert_eq!(found, TSRS),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
    match spawn_at("tsrs", "tsrs/bin/tsrs", TSRS, &["7.2", TSRS]).await {
        Err(TscClientError::UnsupportedVersion { .. }) => {}
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[tokio::test]
async fn a_tsrs_build_that_was_not_traced_is_rejected_whatever_its_typescript() {
    for installed in ["0.23.1-ts7.1.0-beta", "0.24.0-ts7.1.0-beta", "0.23.0-ts7.1.0", "0.23.0-ts7.1.0-beta.1"] {
        match spawn_at("tsrs", "tsrs/bin/tsrs", installed, &["7.1", TSRS]).await {
            Err(TscClientError::UnsupportedVersion { found, .. }) => assert_eq!(found, installed),
            other => panic!("{installed}: expected UnsupportedVersion, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_tsrs_version_is_not_taken_for_a_series_by_its_prefix() {
    match spawn_at("tsrs", "tsrs/bin/tsrs", TSRS, &["0.23", "7.1"]).await {
        Err(TscClientError::UnsupportedVersion { .. }) => {}
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}
