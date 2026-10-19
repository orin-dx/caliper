mod common;

use std::time::Duration;

use caliper_docgen_tsc_client::Supervisor;
use serde_json::json;

#[tokio::test]
async fn a_server_notification_reaches_the_single_consumer_and_requests_still_round_trip() {
    let (_dir, config) = common::spawn_fake("notify_then_echo");
    let supervisor = Supervisor::spawn(config).await.expect("spawn fake tsc");
    let client = supervisor.client();

    let mut notifications = client.take_notifications().expect("the first call hands out the receiver");
    assert!(client.take_notifications().is_none(), "the receiver is single-consumer");
    assert!(supervisor.client().take_notifications().is_none(), "clones share the same single receiver");

    let notification = tokio::time::timeout(Duration::from_secs(5), notifications.recv())
        .await
        .expect("notification within 5s")
        .expect("channel open");
    assert_eq!(notification.method, "tsc/test");
    assert_eq!(notification.params, json!({ "n": 1 }));

    let response = client.request("ping", json!({ "n": 2 }), Duration::from_secs(2)).await.expect("request");
    assert_eq!(response.value["echo"], json!({ "n": 2 }));
}
