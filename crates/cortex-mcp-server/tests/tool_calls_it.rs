//! Phase19 §1.3 + §5.1 — wiremock IT for `cortex_tool_calls`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn tool_calls_happy_path_returns_native_meili_shape() {
    let mock = MockServer::start().await;
    let canned = json!({
        "hits": [
            {"event_id": "01EV0", "tool_name": "Bash", "outcome": "ok", "duration_ms": 120, "repo": "cortex"},
            {"event_id": "01EV1", "tool_name": "Bash", "outcome": "error", "duration_ms": 50, "repo": "cortex"}
        ],
        "processing_time_ms": 2,
        "estimated_total_hits": 2
    });
    Mock::given(method("POST"))
        .and(path("/v1/search/tool-calls"))
        .and(body_partial_json(json!({"tool_name": "Bash"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_tool_calls",
            "arguments": {"tool_name": "Bash", "repo": "cortex", "limit": 10}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["hits"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["hits"][0]["tool_name"], "Bash");
}

#[tokio::test]
async fn tool_calls_surfaces_bad_input_on_unknown_outcome() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search/tool-calls"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "reason": "bad_input",
            "detail": "unknown outcome `success`; allowed: ok, transient, rejected, task_failed, error"
        })))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_tool_calls",
            "arguments": {"outcome": "success"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["reason"], "bad_input");
}

#[tokio::test]
async fn tool_calls_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_tool_calls",
            "arguments": {"tool_name": "Read"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["reason"], "api_unreachable");
}

#[tokio::test]
async fn tool_calls_descriptor_pins_outcome_enum() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {}});
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    let desc = tools
        .iter()
        .find(|t| t["name"] == "cortex_tool_calls")
        .expect("cortex_tool_calls in tool list");
    let enums = desc["inputSchema"]["properties"]["outcome"]["enum"]
        .as_array()
        .expect("outcome.enum");
    let strings: Vec<&str> = enums.iter().filter_map(|v| v.as_str()).collect();
    for required in ["ok", "transient", "rejected", "task_failed", "error"] {
        assert!(
            strings.contains(&required),
            "descriptor enum missing `{required}`; got {strings:?}"
        );
    }
}
