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
    let mut supervisor = caliper_docgen_tsc_client::Supervisor::spawn(config).await.expect("spawn fake tsc");

    let first = supervisor.request("first", serde_json::json!({}), Duration::from_secs(2)).await;
    assert!(first.is_ok(), "expected the first request to succeed, got {first:?}");

    // The second request may report Crashed directly, or (since Supervisor::request
    // auto-restarts once on Crashed) succeed against the restarted process — both are
    // correct; what matters is it isn't left hanging or silently wrong. Use the
    // low-level Client directly instead, which never restarts, to observe the crash
    // itself deterministically.
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
async fn a_fatal_frame_from_a_live_child_is_restarted_not_timed_out() {
    let (_dir, config) = common::spawn_fake("fatal_frame_once_then_echo");
    let cwd = config.cwd.clone();
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let first = supervisor.request("ping", json!({ "n": 1 }), Duration::from_secs(2)).await.expect("restarted request");
    assert_eq!(first.value["echo"], json!({ "n": 1 }));
    assert!(supervisor.is_alive());
    assert_eq!(common::spawn_count(&cwd), 2);

    let second = supervisor.request("ping", json!({ "n": 2 }), Duration::from_secs(2)).await.expect("second request");
    assert_eq!(second.value["echo"], json!({ "n": 2 }));
    assert_eq!(common::spawn_count(&cwd), 2);
}

#[tokio::test]
async fn a_child_that_always_sends_a_fatal_frame_consumes_the_restart_budget() {
    let (_dir, config) = common::spawn_fake("fatal_frame_then_alive");
    let cwd = config.cwd.clone();
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    for call in 1..=3 {
        // The deadline is only an upper bound: the frame error returns as soon as the fake writes it, but a loaded
        // machine can take longer than a few hundred ms to start the restarted child.
        let result = supervisor.request("ping", json!({}), Duration::from_secs(5)).await;
        assert!(matches!(result, Err(TscClientError::MalformedFrame { .. })), "call {call}: got {result:?}");
    }
    let fourth = supervisor.request("ping", json!({}), Duration::from_secs(5)).await;
    assert!(matches!(fourth, Err(TscClientError::RestartStormDetected { attempts: 3, .. })), "got {fourth:?}");
    assert_eq!(common::spawn_count(&cwd), 4);
    assert!(!supervisor.is_alive(), "a supervisor that hit the restart cap is not alive");
}

#[tokio::test]
async fn a_closed_session_is_not_alive_even_before_the_restart_cap_trips() {
    let (_dir, config) = common::spawn_fake("crash_immediately:9");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let give_up = Instant::now() + Duration::from_secs(5);
    while client.closed().is_none() && Instant::now() < give_up {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(client.closed().is_some(), "the child exited, so the session must close");
    assert!(!supervisor.is_alive(), "closed session, restart budget untouched");
}

#[tokio::test]
async fn a_clone_from_before_a_restart_fails_immediately_with_the_old_sessions_reason() {
    let (_dir, config) = common::spawn_fake("fatal_frame_once_then_echo");
    let cwd = config.cwd.clone();
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let stale = supervisor.client();

    supervisor.request("first", json!({}), Duration::from_secs(5)).await.expect("request restarts and succeeds");

    let start = Instant::now();
    let result = stale.request("ping", json!({}), Duration::from_secs(2)).await;
    assert!(start.elapsed() < Duration::from_millis(100), "took {:?}", start.elapsed());
    assert!(matches!(result, Err(TscClientError::MalformedFrame { .. })), "got {result:?}");
    assert!(matches!(stale.closed(), Some(TscClientError::MalformedFrame { .. })), "got {:?}", stale.closed());

    let live = supervisor.client().request("ping", json!({}), Duration::from_secs(2)).await;
    assert!(live.is_ok(), "the current session must be live, got {live:?}");
    assert_eq!(common::spawn_count(&cwd), 2);
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
