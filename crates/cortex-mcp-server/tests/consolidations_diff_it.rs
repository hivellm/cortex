//! Phase19 §2.6 + §5.1 — wiremock IT for `cortex_consolidations_diff`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidations_diff_happy_path_returns_hits_sorted_ascending() {
    let mock = MockServer::start().await;
    let canned = json!({
        "since_ts": 1_700_000_000_000_i64,
        "hits": [
            {
                "event_id": "01HZCONS01",
                "kind": "consolidation",
                "occurred_at": 1_700_000_001_000_i64,
                "title": "Older digest"
            },
            {
                "event_id": "01HZCONS02",
                "kind": "consolidation",
                "occurred_at": 1_700_000_002_000_i64,
                "title": "Newer digest"
            }
        ],
        "processing_time_ms": 3,
        "estimated_total_hits": 2
    });
    Mock::given(method("GET"))
        .and(path("/v1/consolidations/diff"))
        .and(query_param("since_ts", "1700000000000"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_diff",
            "arguments": {"since_ts": 1_700_000_000_000_i64, "limit": 10}
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
    assert_eq!(parsed["since_ts"], 1_700_000_000_000_i64);
    assert_eq!(parsed["hits"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["hits"][0]["event_id"], "01HZCONS01");
}

#[tokio::test]
async fn consolidations_diff_propagates_repo_filter() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/consolidations/diff"))
        .and(query_param("since_ts", "0"))
        .and(query_param("repo", "cortex"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "since_ts": 0,
            "hits": [],
            "processing_time_ms": 1,
            "estimated_total_hits": 0
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_diff",
            "arguments": {"since_ts": 0, "repo": "cortex"}
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
async fn consolidations_diff_rejects_missing_since_ts_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_diff",
            "arguments": {}
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
async fn consolidations_diff_rejects_negative_since_ts_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_diff",
            "arguments": {"since_ts": -1}
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
async fn consolidations_diff_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_diff",
            "arguments": {"since_ts": 0}
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
async fn consolidations_diff_descriptor_pins_limit_bounds_and_since_ts_required() {
    use cortex_mcp_server::{ConsolidationsDiffTool, Tool};
    let descriptor = ConsolidationsDiffTool::new().descriptor();
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["maximum"],
        200
    );
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["minimum"],
        1
    );
    let required = descriptor["inputSchema"]["required"]
        .as_array()
        .expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("since_ts")));
}
