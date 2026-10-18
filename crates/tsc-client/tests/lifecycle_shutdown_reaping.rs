mod common;

use std::time::{Duration, Instant};

use oxc_react_docgen_tsc_client::{Supervisor, TscClientError};
use serde_json::json;

#[tokio::test]
async fn shutdown_force_kills_and_reaps_a_process_that_ignores_it() {
    // Survives stdin EOF for far longer than the supervisor's grace period.
    let (_dir, config) = common::spawn_fake("shutdown_slow:30000");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let start = Instant::now();
    let status = supervisor.shutdown().await.expect("shutdown must force-kill and still succeed, not hang for 30s");
    let elapsed = start.elapsed();

    assert!(!status.success(), "a force-killed process must not report success: {status:?}");
    // The 2s grace period plus kill/reap overhead, well under the fake's 30s stubbornness.
    assert!(elapsed >= Duration::from_secs(2), "expected shutdown to wait out the 2s grace period, took {elapsed:?}");
    assert!(elapsed < Duration::from_secs(10), "expected shutdown to force-kill well before 10s, took {elapsed:?}");
}

#[tokio::test]
async fn shutdown_returns_quickly_when_the_process_exits_promptly() {
    let (_dir, config) = common::spawn_fake("shutdown_slow:0");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let start = Instant::now();
    supervisor.shutdown().await.expect("shutdown");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "expected a prompt exit to not wait out the grace period, took {elapsed:?}"
    );
}

#[tokio::test]
async fn shutdown_closes_stdin_while_a_client_clone_is_held() {
    let (_dir, config) = common::spawn_fake("echo");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let _clone = supervisor.client();

    let start = Instant::now();
    let status = supervisor.shutdown().await.expect("shutdown");
    let elapsed = start.elapsed();

    assert!(status.success(), "the child exits cleanly on stdin EOF: {status:?}");
    assert!(elapsed < Duration::from_secs(1), "the grace window must not be consumed, took {elapsed:?}");
}

#[tokio::test]
async fn shutdown_fails_in_flight_requests_on_clones_with_session_closed() {
    let (_dir, config) = common::spawn_fake("never_respond");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();
    let in_flight = tokio::spawn(async move {
        let result = client.request("ping", json!({}), Duration::from_secs(30)).await;
        (result, Instant::now())
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let shutdown_started = Instant::now();
    supervisor.shutdown().await.expect("shutdown");

    let (result, failed_at) = in_flight.await.expect("request task");
    assert!(matches!(result, Err(TscClientError::SessionClosed)), "got {result:?}");
    let waited = failed_at.duration_since(shutdown_started);
    assert!(waited < Duration::from_secs(1), "the in-flight request failed {waited:?} after shutdown started");
}

#[tokio::test]
async fn a_clone_held_across_shutdown_reports_session_closed() {
    let (_dir, config) = common::spawn_fake("echo");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let stale = supervisor.client();
    supervisor.shutdown().await.expect("shutdown");

    assert!(matches!(stale.closed(), Some(TscClientError::SessionClosed)));
    let start = Instant::now();
    let result = stale.request("ping", json!({}), Duration::from_secs(2)).await;
    assert!(matches!(result, Err(TscClientError::SessionClosed)), "got {result:?}");
    assert!(start.elapsed() < Duration::from_millis(100), "took {:?}", start.elapsed());
}

#[tokio::test]
async fn shutdown_returns_the_reaped_exit_status() {
    let (_dir, config) = common::spawn_fake("exit_after:1:7");
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let first = supervisor.request("first", json!({}), Duration::from_secs(2)).await;
    assert!(first.is_ok(), "expected the first request to succeed, got {first:?}");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let status = supervisor.shutdown().await.expect("shutdown");
    assert_eq!(status.code(), Some(7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_wakes_requests_blocked_on_a_full_write_queue() {
    // A child that never reads stdin fills the pipe, then the 16-frame queue, so later requests block in the enqueue.
    // Shutdown must stop the writer even from inside its blocked write; otherwise those requests only end at their own
    // (here 30 s) deadline with `Timeout` instead of `SessionClosed`.
    let (_dir, config) = common::spawn_fake("never_read");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let mut requests = tokio::task::JoinSet::new();
    for n in 0..24 {
        let client = client.clone();
        requests.spawn(async move {
            let params = json!({ "blob": "x".repeat(1024 * 1024), "n": n });
            let result = client.request("big", params, Duration::from_secs(30)).await;
            (result, Instant::now())
        });
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    let shutdown_started = Instant::now();
    supervisor.shutdown().await.expect("shutdown must succeed while the writer is blocked");
    while let Some(joined) = requests.join_next().await {
        let (result, finished) = joined.expect("request task must not panic");
        match result {
            Err(TscClientError::SessionClosed) => {}
            other => panic!("expected SessionClosed, got {other:?}"),
        }
        // Shutdown itself takes the full 2 s grace plus a kill against a child that ignores EOF, and that kill would
        // wake a stuck writer by itself. A request has to wake as soon as the session closes, well inside the grace.
        let woke_after = finished.duration_since(shutdown_started);
        assert!(woke_after < Duration::from_millis(1000), "request woke {woke_after:?} after shutdown started");
    }
}
