mod common;

use std::time::Duration;

use oxc_react_docgen_tsc_client::{Supervisor, TscClientError};

#[tokio::test]
async fn a_live_supervisor_reports_alive() {
    let (_dir, config) = common::spawn_fake("echo");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    assert!(supervisor.is_alive());
}

#[tokio::test]
async fn one_failing_call_does_not_affect_a_concurrently_in_flight_sibling() {
    let (_dir, config) = common::spawn_fake("echo");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let failing = client.request("componentA", serde_json::json!({ "fail": true }), Duration::from_secs(2));
    let succeeding = client.request("componentB", serde_json::json!({ "fail": false }), Duration::from_secs(2));
    let (failing_result, succeeding_result) = tokio::join!(failing, succeeding);

    match failing_result {
        Err(TscClientError::RpcError { code, .. }) => assert_eq!(code, -32000),
        other => panic!("expected componentA's RpcError to be scoped to it, got {other:?}"),
    }
    match succeeding_result {
        Ok(response) => assert_eq!(response.value["echo"]["fail"], serde_json::json!(false)),
        other => panic!("expected componentB to succeed despite componentA failing, got {other:?}"),
    }
}
