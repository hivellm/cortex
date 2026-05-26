//! Phase19 §2.1 + §5.1 — wiremock IT for `cortex_consolidation_get`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidation_get_happy_path_returns_full_document() {
    let mock = MockServer::start().await;
    let id = "01HZCONS00000000000000000Z";
    let canned = json!({
        "document": {
            "event_id": id,
            "kind": "consolidation",
            "title": "Phase19 granular search rollout",
            "ext": {"consolidation": {"consolidation_id": id, "grain": "topic", "model": "claude-opus-4-7"}},
            "repo": "cortex"
        },
        "matched_consolidation_id": true
    });
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/consolidations/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_get",
            "arguments": {"id": id}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], false);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["document"]["event_id"], id);
    assert_eq!(parsed["matched_consolidation_id"], true);
}

#[tokio::test]
async fn consolidation_get_rejects_missing_id_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "cortex_consolidation_get", "arguments": {}}
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn consolidation_get_rejects_non_alphanumeric_id() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "cortex_consolidation_get", "arguments": {"id": "bad/path"}}
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn consolidation_get_surfaces_not_found_on_unknown_id() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/consolidations/[^/]+$"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "reason": "not_found",
            "detail": "no consolidation matches id `01HZUNKNOWN`"
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_get",
            "arguments": {"id": "01HZUNKNOWN"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["reason"], "not_found");
}
