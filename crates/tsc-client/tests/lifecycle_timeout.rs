mod common;

use std::time::{Duration, Instant};

use caliper_docgen_tsc_client::{Supervisor, TscClientError};
use serde_json::json;

#[tokio::test]
async fn repeated_timeouts_leave_the_session_usable() {
    let (_dir, config) = common::spawn_fake("hang_after_n:1");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    client.request("warmup", json!({}), Duration::from_secs(2)).await.expect("first request is answered");
    for _ in 0..5 {
        let result = client.request("ping", json!({}), Duration::from_millis(50)).await;
        assert!(matches!(result, Err(TscClientError::Timeout { .. })), "got {result:?}");
    }
    assert!(client.closed().is_none(), "timeouts must not close the session");
}

#[tokio::test]
async fn a_request_the_child_never_answers_times_out() {
    let (_dir, config) = common::spawn_fake("never_respond");
    let cwd = config.cwd.clone();
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let result = supervisor.request("ping", serde_json::json!({}), Duration::from_millis(150)).await;
    match result {
        Err(TscClientError::Timeout { method, after }) => {
            assert_eq!(method, "ping");
            assert_eq!(after, Duration::from_millis(150));
        }
        Err(other) => panic!("expected Timeout, got {other:?}"),
        Ok(response) => panic!("expected Timeout, got a response: {response:?}"),
    }

    // A timeout on a live session is not a restart trigger.
    assert_eq!(common::settled_spawn_count(&cwd, 1).await, 1);

    supervisor.shutdown().await.expect("shutdown");
}

// Multi-threaded so the 24 requests' synchronous 1 MiB serialization does not delay one another's timer wakeups.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_child_that_stops_reading_stdin_cannot_hold_a_request_past_its_deadline() {
    let (_dir, config) = common::spawn_fake("never_read");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let deadline = Duration::from_millis(100);
    // More requests than the 16-frame write queue holds, so some block in `send().await` and only the deadline frees
    // them.
    let mut requests = tokio::task::JoinSet::new();
    for n in 0..24 {
        let client = client.clone();
        requests.spawn(async move {
            let params = json!({ "blob": "x".repeat(1024 * 1024), "n": n });
            let start = Instant::now();
            let result = client.request("big", params, deadline).await;
            (result, start.elapsed())
        });
    }
    // Bounded from outside so a regression fails fast instead of hanging the suite.
    let results = tokio::time::timeout(Duration::from_secs(10), requests.join_all())
        .await
        .expect("every request must return on its own deadline");

    assert_eq!(results.len(), 24);
    for (result, elapsed) in results {
        match result {
            Err(TscClientError::Timeout { after, .. }) => assert_eq!(after, deadline),
            other => panic!("expected Timeout, got {other:?}"),
        }
        // Old behavior blocked for seconds or forever; 1s leaves room for a scheduling stall under load.
        assert!(elapsed < Duration::from_secs(1), "request outlived its deadline: {elapsed:?}");
    }

    let start = Instant::now();
    supervisor.shutdown().await.expect("shutdown must succeed while a write is blocked");
    assert!(start.elapsed() < Duration::from_secs(5), "shutdown took {:?}", start.elapsed());
}
