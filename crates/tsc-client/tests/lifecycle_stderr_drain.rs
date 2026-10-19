mod common;

use std::time::Duration;

use caliper_docgen_tsc_client::Supervisor;
use serde_json::json;

#[tokio::test]
async fn a_child_that_floods_stderr_is_not_blocked_by_a_full_pipe() {
    // The fake writes 2 MiB to stderr before it reads stdin; without a drain it blocks on the full pipe.
    let (_dir, config) = common::spawn_fake("noisy_stderr");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");

    let response = supervisor.client().request("ping", json!({ "n": 1 }), Duration::from_secs(5)).await;
    let response = response.expect("the request must complete once stderr is drained");
    assert_eq!(response.value["echo"], json!({ "n": 1 }));
}
