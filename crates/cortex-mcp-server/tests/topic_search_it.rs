//! Phase19 §1.5 + §5.1 — wiremock IT for `cortex_topic_search`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn topic_search_happy_path_returns_topic_cards() {
    let mock = MockServer::start().await;
    let canned = json!({
        "topic_prefix": "tool:claude-code",
        "hits": [
            {"topic_id": "t1", "title": "Claude Code automation patterns", "topics": ["tool:claude-code"], "repo": "cortex"},
            {"topic_id": "t2", "title": "Hook contract conventions", "topics": ["tool:claude-code", "spec:10"], "repo": "cortex"}
        ],
        "processing_time_ms": 3,
        "estimated_total_hits": 2
    });
    Mock::given(method("POST"))
        .and(path("/v1/topic-cards/search"))
        .and(body_partial_json(
            json!({"topic_prefix": "tool:claude-code"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_topic_search",
            "arguments": {"topic_prefix": "tool:claude-code", "repo": "cortex", "limit": 10}
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
    assert_eq!(parsed["topic_prefix"], "tool:claude-code");
    assert_eq!(parsed["hits"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["hits"][0]["topics"][0], "tool:claude-code");
}

#[tokio::test]
async fn topic_search_surfaces_bad_input_on_empty_prefix() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/topic-cards/search"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "reason": "bad_input",
            "detail": "`topic_prefix` is required"
        })))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_topic_search",
            "arguments": {"topic_prefix": ""}
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
    assert_eq!(parsed["reason"], "bad_input");
}

#[tokio::test]
async fn topic_search_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_topic_search",
            "arguments": {"topic_prefix": "tool:claude-code"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["reason"], "api_unreachable");
}
