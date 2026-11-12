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

use caliper_docgen_tsc_client::{Client, Supervisor, TscClientError};
use serde_json::json;

#[tokio::test]
async fn a_process_that_exits_mid_session_fails_the_next_request_as_crashed() {
    // Answers the first request, then exits — a second request gets no response.
    let (_dir, config) = common::spawn_fake("exit_after:1:7");
    let supervisor = caliper_docgen_tsc_client::Supervisor::spawn(config).await.expect("spawn fake tsc");

    let first = supervisor.client().request("first", serde_json::json!({}), Duration::from_secs(2)).await;
    assert!(first.is_ok(), "expected the first request to succeed, got {first:?}");

    let raw_client: Client = supervisor.client();
    // Give the crashed process's reader task a moment to notice EOF.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = raw_client.request("second", serde_json::json!({}), Duration::from_secs(2)).await;
    match second {
        // Whether the crash surfaces as `Crashed` (reader task noticed EOF first) or
        // `Write` (the stdin pipe was already closed) is a timing race, not something
        // this crate controls — both mean the same thing: the process is gone.
        Err(TscClientError::Crashed { .. } | TscClientError::Write(_)) => {}
        Err(other) => panic!("expected Crashed or Write, got {other:?}"),
        Ok(response) => panic!("expected an error, got a response: {response:?}"),
    }
}

#[tokio::test]
async fn a_fatal_frame_from_a_live_child_ends_the_session_instead_of_timing_out() {
    let (_dir, config) = common::spawn_fake("fatal_frame_then_alive");
    let cwd = config.cwd.clone();
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let start = Instant::now();
    let result = client.request("ping", json!({}), Duration::from_secs(5)).await;
    assert!(matches!(result, Err(TscClientError::MalformedFrame { .. })), "got {result:?}");
    assert!(start.elapsed() < Duration::from_secs(5), "the frame error must not wait out the deadline");
    assert!(matches!(client.closed(), Some(TscClientError::MalformedFrame { .. })), "got {:?}", client.closed());
    assert!(!supervisor.is_alive(), "the child is still running, but the session is over");
    assert_eq!(common::spawn_count(&cwd), 1, "nothing restarts the child");
}

#[tokio::test]
async fn a_closed_session_is_not_alive() {
    let (_dir, config) = common::spawn_fake("crash_immediately:9");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let give_up = Instant::now() + Duration::from_secs(5);
    while client.closed().is_none() && Instant::now() < give_up {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(client.closed().is_some(), "the child exited, so the session must close");
    assert!(!supervisor.is_alive(), "the session is closed");
}

#[tokio::test]
async fn the_first_close_reason_wins() {
    let (_dir, config) = common::spawn_fake("exit_after:1:7");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let start = Instant::now();
    let (a, b) = tokio::join!(
        client.request("a", json!({}), Duration::from_secs(2)),
        client.request("b", json!({}), Duration::from_secs(2)),
    );
    assert!(start.elapsed() < Duration::from_secs(1), "took {:?}", start.elapsed());

    let is_crash = |result: &Result<_, TscClientError>| {
        matches!(result, Err(TscClientError::Crashed { .. } | TscClientError::Write(_)))
    };
    assert!(
        (a.is_ok() && is_crash(&b)) || (b.is_ok() && is_crash(&a)),
        "expected exactly one Ok and one Crashed/Write, got a={a:?} b={b:?}"
    );

    let reasons: Vec<_> = (0..3).map(|_| client.closed().map(|reason| std::mem::discriminant(&reason))).collect();
    assert!(reasons[0].is_some());
    assert!(reasons.iter().all(|reason| *reason == reasons[0]));
}
