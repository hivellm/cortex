//! Phase19 §2.4 + §5.1 — wiremock IT for `cortex_consolidations_search`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidations_search_happy_path_returns_hits_and_bm25_strategy() {
    let mock = MockServer::start().await;
    let canned = json!({
        "hits": [
            {
                "event_id": "01HZCONS01",
                "kind": "consolidation",
                "title": "Retention daemon sweep",
                "summary_markdown": "## Retention\nFolds retention rules…",
                "repo": "cortex",
                "grain": "session"
            }
        ],
        "processing_time_ms": 4,
        "estimated_total_hits": 1,
        "match_strategy": "bm25",
        "intent_hint": "decision_lookup"
    });
    Mock::given(method("POST"))
        .and(path("/v1/consolidations/search"))
        .and(body_partial_json(json!({
            "query": "retention daemon",
            "intent_hint": "decision_lookup",
            "k": 5
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
            "name": "cortex_consolidations_search",
            "arguments": {
                "query": "retention daemon",
                "intent_hint": "decision_lookup",
                "k": 5
            }
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
    assert_eq!(parsed["match_strategy"], "bm25");
    assert_eq!(parsed["intent_hint"], "decision_lookup");
    assert_eq!(parsed["hits"][0]["title"], "Retention daemon sweep");
}

#[tokio::test]
async fn consolidations_search_rejects_missing_query_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_search",
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
async fn consolidations_search_rejects_unknown_grain_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_search",
            "arguments": {"query": "x", "grain": "Session"}
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
async fn consolidations_search_rejects_unknown_intent_hint_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_search",
            "arguments": {"query": "x", "intent_hint": "decisions"}
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
async fn consolidations_search_propagates_repo_and_grain_filters() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/consolidations/search"))
        .and(body_partial_json(json!({
            "query": "x",
            "repo": "cortex",
            "grain": "topic"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [],
            "processing_time_ms": 1,
            "estimated_total_hits": 0,
            "match_strategy": "bm25"
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_search",
            "arguments": {"query": "x", "repo": "cortex", "grain": "topic"}
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
async fn consolidations_search_surfaces_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_search",
            "arguments": {"query": "x"}
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
async fn consolidations_search_descriptor_pins_intent_enum_and_k_bounds() {
    use cortex_mcp_server::{ConsolidationsSearchTool, Tool};
    let descriptor = ConsolidationsSearchTool::new().descriptor();
    let enum_values = descriptor["inputSchema"]["properties"]["intent_hint"]["enum"]
        .as_array()
        .expect("intent_hint enum present");
    let names: Vec<&str> = enum_values.iter().filter_map(Value::as_str).collect();
    for required in [
        "pre_change_context",
        "decision_lookup",
        "similar_problems",
        "law_check",
        "free_search",
        "explain",
    ] {
        assert!(
            names.contains(&required),
            "descriptor must list `{required}`"
        );
    }
    assert_eq!(descriptor["inputSchema"]["properties"]["k"]["maximum"], 20);
    assert_eq!(descriptor["inputSchema"]["properties"]["k"]["minimum"], 1);
}
