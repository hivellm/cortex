//! Integration tests for `cortex_workers::ingestion::publisher`.

use cortex_workers::ingestion::{MemoryPublisher, Publisher};

#[tokio::test]
async fn memory_publisher_records_calls() {
    let p = MemoryPublisher::default();
    p.publish("cortex.events.raw", &serde_json::json!({ "kind": "turn" }))
        .await
        .unwrap();
    p.publish("cortex.events.raw", &serde_json::json!({ "kind": "tool_call" }))
        .await
        .unwrap();
    let calls = p.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "cortex.events.raw");
}
