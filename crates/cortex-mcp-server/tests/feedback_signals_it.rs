//! Phase19 §3.2 + §5.1 — wiremock IT for `cortex_feedback_signals`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn feedback_signals_happy_path_returns_rows_newest_first() {
    let mock = MockServer::start().await;
    let canned = json!({
        "rows": [
            {
                "query_id": "01HXQUERY00000000000000B",
                "intent": "explain",
                "helpful": true,
                "files_cited": ["src/lib.rs"],
                "recorded_at": "2026-05-26T09:00:00Z"
            },
            {
                "query_id": "01HXQUERY00000000000000A",
                "intent": "explain",
                "helpful": true,
                "files_cited": [],
                "recorded_at": "2026-05-25T09:00:00Z"
            }
        ],
        "total": 2
    });
    Mock::given(method("POST"))
        .and(path("/v1/feedback/list"))
        .and(body_partial_json(json!({
            "helpful": true,
            "intent": "explain",
            "limit": 10
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
            "name": "cortex_feedback_signals",
            "arguments": {"helpful": true, "intent": "explain", "limit": 10}
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
    assert_eq!(parsed["total"], 2);
    assert_eq!(parsed["rows"][0]["query_id"], "01HXQUERY00000000000000B");
}

#[tokio::test]
async fn feedback_signals_rejects_repo_filter_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_feedback_signals",
            "arguments": {"repo": "cortex"}
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
async fn feedback_signals_propagates_window_filter() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/feedback/list"))
        .and(body_partial_json(json!({
            "since": "2026-05-01T00:00:00Z",
            "until": "2026-05-26T00:00:00Z"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "rows": [],
            "total": 0
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_feedback_signals",
            "arguments": {
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
async fn feedback_signals_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_feedback_signals",
            "arguments": {}
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
async fn feedback_signals_descriptor_pins_limit_bounds_and_no_repo() {
    use cortex_mcp_server::{FeedbackSignalsTool, Tool};
    let descriptor = FeedbackSignalsTool::new().descriptor();
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["maximum"],
        50
    );
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["minimum"],
        1
    );
    // `repo` is intentionally absent from the descriptor properties.
    assert!(descriptor["inputSchema"]["properties"]
        .get("repo")
        .is_none());
}
