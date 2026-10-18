mod common;

use std::time::Duration;

use oxc_react_docgen_tsc_client::{Supervisor, TscClientError};

#[tokio::test]
async fn a_process_that_crashes_on_every_restart_gives_up_after_the_cap() {
    // Crashes immediately on every spawn, including every respawn attempt — drives
    // the supervisor's bounded restart policy to exhaustion.
    let (_dir, config) = common::spawn_fake("crash_immediately:9");
    let mut supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    // First call: the crashed process answers nothing, restart is attempted and the
    // freshly-restarted process also crashes immediately, so the retried call also
    // fails — this consumes one restart attempt per call until the cap trips.
    let mut storm_detected = false;
    for _ in 0..10 {
        match supervisor.request("ping", serde_json::json!({}), Duration::from_secs(2)).await {
            Err(TscClientError::RestartStormDetected { .. }) => {
                storm_detected = true;
                break;
            }
            _ => continue,
        }
    }

    assert!(storm_detected, "expected RestartStormDetected within 10 calls, restart cap never tripped");

    // Once tripped, the supervisor short-circuits every further call without touching
    // the process again.
    match supervisor.request("ping", serde_json::json!({}), Duration::from_secs(2)).await {
        Err(TscClientError::RestartStormDetected { .. }) => {}
        other => panic!("expected the supervisor to stay permanently failed, got {other:?}"),
    }
}
