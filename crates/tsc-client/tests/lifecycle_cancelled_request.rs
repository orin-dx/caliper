#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
mod common;

use std::time::Duration;

use caliper_docgen_tsc_client::Supervisor;
use serde_json::json;

#[tokio::test]
async fn dropping_a_request_future_mid_write_never_wedges_the_session() {
    let (_dir, config) = common::spawn_fake("echo");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let big = client.request("big", json!({ "blob": "x".repeat(8 * 1024 * 1024) }), Duration::from_secs(10));
    let elapsed = tokio::time::timeout(Duration::from_millis(1), big).await;
    assert!(elapsed.is_err(), "the 8 MiB request must still be in flight when its future is dropped");

    let ping = client.request("ping", json!({}), Duration::from_secs(2)).await.expect("ping after a dropped request");
    assert_eq!(ping.value["echo"], json!({}));
    assert!(supervisor.is_alive());
}
