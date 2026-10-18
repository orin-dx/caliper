mod common;

use std::time::Duration;

use caliper_docgen_tsc_client::Supervisor;

#[tokio::test]
async fn seq_reflects_wire_arrival_order_not_issue_order() {
    // Answers the second request before the first — proves `seq` can't be assumed to
    // follow issue order, which is exactly the gap the "stale notification" gate is
    // about: a caller must be able to tell whether an answer could possibly reflect an
    // update it already saw acknowledged, and issue order alone can't tell it that.
    let (_dir, config) = common::spawn_fake("reorder_ack");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let first = client.request("first", serde_json::json!({}), Duration::from_secs(2));
    let second = client.request("second", serde_json::json!({}), Duration::from_secs(2));
    let (first_result, second_result) = tokio::join!(first, second);

    let first_response = first_result.expect("first request should still succeed, just answered later");
    let second_response = second_result.expect("second request should succeed, answered first");

    assert_eq!(second_response.value["order"], serde_json::json!("second"));
    assert_eq!(first_response.value["order"], serde_json::json!("first"));
    assert!(
        second_response.seq < first_response.seq,
        "expected the second request's answer (arrived first on the wire) to carry the lower seq: \
         second.seq={}, first.seq={}",
        second_response.seq,
        first_response.seq
    );
}
