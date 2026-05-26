//! Phase19 §3.3 + §5.1 — wiremock IT for `cortex_decision_search`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn decision_search_happy_path_returns_hits_for_status_and_tag() {
    let mock = MockServer::start().await;
    let canned = json!({
        "hits": [
            {
                "event_id": "01HZDEC01",
                "kind": "decision",
                "decision_id": "DEC-0042",
                "decision_status": "accepted",
                "title": "Adopt Meilisearch",
                "topics": ["retention", "infra"]
            }
        ],
        "processing_time_ms": 2,
        "estimated_total_hits": 1
    });
    Mock::given(method("POST"))
        .and(path("/v1/decisions/search"))
        .and(body_partial_json(json!({
            "status": "accepted",
            "tag": "retention"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_decision_search",
            "arguments": {"status": "accepted", "tag": "retention"}
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
    assert_eq!(parsed["hits"][0]["decision_id"], "DEC-0042");
}

#[tokio::test]
async fn decision_search_rejects_supersedes_filter_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_decision_search",
            "arguments": {"supersedes": "DEC-0001"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn decision_search_rejects_superseded_by_filter_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_decision_search",
            "arguments": {"superseded_by": "DEC-0099"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn decision_search_rejects_unknown_status_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_decision_search",
            "arguments": {"status": "Accepted"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn decision_search_propagates_repo_and_window_filters() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/decisions/search"))
        .and(body_partial_json(json!({
            "repo": "cortex",
            "since": "2026-05-01T00:00:00Z",
            "until": "2026-05-26T00:00:00Z"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [],
            "processing_time_ms": 1,
            "estimated_total_hits": 0
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_decision_search",
            "arguments": {
                "repo": "cortex",
                "since": "2026-05-01T00:00:00Z",
                "until": "2026-05-26T00:00:00Z"
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], false);
}

#[tokio::test]
async fn decision_search_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "cortex_decision_search",
            "arguments": {"q": "retention"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], true);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["reason"], "api_unreachable");
}

#[tokio::test]
async fn decision_search_descriptor_pins_status_enum_and_limit_bounds() {
    use cortex_mcp_server::{DecisionSearchTool, Tool};
    let descriptor = DecisionSearchTool::new().descriptor();
    let enum_values = descriptor["inputSchema"]["properties"]["status"]["enum"]
        .as_array()
        .expect("status enum present");
    let names: Vec<&str> = enum_values.iter().filter_map(Value::as_str).collect();
    for required in ["proposed", "accepted", "superseded", "deprecated"] {
        assert!(
            names.contains(&required),
            "descriptor must list `{required}`"
        );
    }
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["maximum"],
        50
    );
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["minimum"],
        1
    );
}
