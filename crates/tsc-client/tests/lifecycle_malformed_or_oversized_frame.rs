#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
mod common;

use std::time::{Duration, Instant};

use caliper_docgen_tsc_client::{MalformedFrameReason, Supervisor, TscClientError};

async fn expect_malformed(scenario: &str, expected_reason: MalformedFrameReason) {
    let (_dir, config) = common::spawn_fake(scenario);
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    // The session ends with the frame error, so the supervisor restarts once and retries against a fresh child that
    // misbehaves the same way; the caller still sees the variant.
    let start = Instant::now();
    let result = supervisor.request("ping", serde_json::json!({}), Duration::from_secs(2)).await;
    assert!(start.elapsed() < Duration::from_secs(2), "scenario {scenario:?} took {:?}", start.elapsed());
    match result {
        Err(TscClientError::MalformedFrame { reason }) => {
            assert_eq!(reason, expected_reason, "scenario {scenario:?}")
        }
        Err(other) => panic!("scenario {scenario:?}: expected MalformedFrame({expected_reason:?}), got {other:?}"),
        Ok(response) => panic!("scenario {scenario:?}: expected an error, got a response: {response:?}"),
    }
}

#[tokio::test]
async fn garbage_bytes_are_reported_as_a_missing_content_length_header() {
    expect_malformed("garbage_bytes", MalformedFrameReason::NoContentLengthHeader).await;
}

#[tokio::test]
async fn an_unparsable_content_length_is_reported_distinctly() {
    expect_malformed("bad_content_length", MalformedFrameReason::UnparsableContentLength).await;
}

#[tokio::test]
async fn a_truncated_body_is_reported_distinctly() {
    expect_malformed("truncated_body", MalformedFrameReason::TruncatedBody).await;
}

#[tokio::test]
async fn an_oversized_declared_length_is_rejected_before_reading_the_body() {
    let (_dir, config) = common::spawn_fake("oversized_frame:100000000"); // 100 MB, well past the 16 MiB cap
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let start = Instant::now();
    let result = supervisor.request("ping", serde_json::json!({}), Duration::from_secs(2)).await;
    assert!(start.elapsed() < Duration::from_secs(2), "took {:?}", start.elapsed());
    match result {
        Err(TscClientError::OversizedFrame { declared, cap }) => {
            assert_eq!(declared, 100_000_000);
            assert_eq!(cap, caliper_docgen_tsc_client::transport::DEFAULT_MAX_FRAME_BYTES);
        }
        Err(other) => panic!("expected OversizedFrame, got {other:?}"),
        Ok(response) => panic!("expected OversizedFrame, got a response: {response:?}"),
    }
}

#[tokio::test]
async fn a_request_after_the_reader_ended_fails_immediately_with_the_closed_reason() {
    let (_dir, config) = common::spawn_fake("garbage_bytes");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let first = client.request("ping", serde_json::json!({}), Duration::from_secs(2)).await;
    assert!(
        matches!(first, Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::NoContentLengthHeader })),
        "got {first:?}"
    );

    let start = Instant::now();
    let second = client.request("ping", serde_json::json!({}), Duration::from_secs(3)).await;
    assert!(
        matches!(second, Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::NoContentLengthHeader })),
        "got {second:?}"
    );
    assert!(start.elapsed() < Duration::from_millis(100), "took {:?}", start.elapsed());

    for _ in 0..2 {
        assert!(matches!(
            client.closed(),
            Some(TscClientError::MalformedFrame { reason: MalformedFrameReason::NoContentLengthHeader })
        ));
    }
}
