//! Phase19 §3.1 + §5.1 — wiremock IT for `cortex_law_violations`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn law_violations_happy_path_returns_hits_with_law_id_filter() {
    let mock = MockServer::start().await;
    let canned = json!({
        "index": "cortex-cortex-governance",
        "hits": [
            {
                "event_id": "01HZLV01",
                "kind": "law_violation",
                "law_id": "LAW-007",
                "law_severity": "critical",
                "session_id": "01HSESS01",
                "ts": 1_700_000_000_000_i64
            }
        ],
        "processing_time_ms": 3,
        "estimated_total_hits": 1
    });
    Mock::given(method("POST"))
        .and(path("/v1/laws/violations"))
        .and(body_partial_json(json!({
            "repo": "cortex",
            "law_id": "LAW-007"
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
            "name": "cortex_law_violations",
            "arguments": {"repo": "cortex", "law_id": "LAW-007"}
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
    assert_eq!(parsed["index"], "cortex-cortex-governance");
    assert_eq!(parsed["hits"][0]["law_id"], "LAW-007");
}

#[tokio::test]
async fn law_violations_rejects_missing_repo_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_law_violations",
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
async fn law_violations_rejects_non_alphanumeric_repo() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_law_violations",
            "arguments": {"repo": "bad/slash"}
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
async fn law_violations_propagates_session_and_window_filters() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/laws/violations"))
        .and(body_partial_json(json!({
            "repo": "cortex",
            "session_id": "01HSESS01",
            "since": "2026-05-01T00:00:00Z",
            "until": "2026-05-26T00:00:00Z"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "index": "cortex-cortex-governance",
            "hits": [],
            "processing_time_ms": 1,
            "estimated_total_hits": 0
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_law_violations",
            "arguments": {
                "repo": "cortex",
                "session_id": "01HSESS01",
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
async fn law_violations_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_law_violations",
            "arguments": {"repo": "cortex"}
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
async fn law_violations_descriptor_pins_repo_required_and_limit_bounds() {
    use cortex_mcp_server::{LawViolationsTool, Tool};
    let descriptor = LawViolationsTool::new().descriptor();
    let required = descriptor["inputSchema"]["required"]
        .as_array()
        .expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("repo")));
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["maximum"],
        50
    );
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["minimum"],
        1
    );
}
