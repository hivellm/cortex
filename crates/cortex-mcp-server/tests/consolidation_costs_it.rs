//! Phase19 §3.4 + §5.1 — wiremock IT for `cortex_consolidation_costs`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidation_costs_happy_path_returns_buckets_grouped_by_grain() {
    let mock = MockServer::start().await;
    let canned = json!({
        "group_by": ["grain"],
        "buckets": [
            {
                "key": {"grain": "session"},
                "consolidation_count": 7,
                "distinct_models": ["claude-haiku-4-5"]
            },
            {
                "key": {"grain": "topic"},
                "consolidation_count": 3,
                "distinct_models": ["claude-opus-4-7"]
            }
        ],
        "total": 10,
        "truncated": false,
        "match_strategy": "doc_only"
    });
    Mock::given(method("POST"))
        .and(path("/v1/consolidations/costs"))
        .and(body_partial_json(json!({
            "since": "2026-05-01T00:00:00Z",
            "until": "2026-05-26T00:00:00Z",
            "group_by": ["grain"]
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
            "name": "cortex_consolidation_costs",
            "arguments": {
                "since": "2026-05-01T00:00:00Z",
                "until": "2026-05-26T00:00:00Z",
                "group_by": ["grain"]
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
    assert_eq!(parsed["total"], 10);
    assert_eq!(parsed["buckets"][0]["consolidation_count"], 7);
    assert_eq!(parsed["match_strategy"], "doc_only");
}

#[tokio::test]
async fn consolidation_costs_rejects_missing_group_by_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_costs",
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
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn consolidation_costs_rejects_unknown_axis_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_costs",
            "arguments": {
                "since": "2026-05-01T00:00:00Z",
                "until": "2026-05-26T00:00:00Z",
                "group_by": ["grain", "topic"]
            }
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
async fn consolidation_costs_rejects_malformed_since_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_costs",
            "arguments": {
                "since": "not-a-date",
                "until": "2026-05-26T00:00:00Z",
                "group_by": ["grain"]
            }
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
async fn consolidation_costs_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_costs",
            "arguments": {
                "since": "2026-05-01T00:00:00Z",
                "until": "2026-05-26T00:00:00Z",
                "group_by": ["grain"]
            }
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
async fn consolidation_costs_descriptor_pins_axis_enum_and_required_fields() {
    use cortex_mcp_server::{ConsolidationCostsTool, Tool};
    let descriptor = ConsolidationCostsTool::new().descriptor();
    let required = descriptor["inputSchema"]["required"]
        .as_array()
        .expect("required array");
    for f in ["since", "until", "group_by"] {
        assert!(required.iter().any(|v| v.as_str() == Some(f)));
    }
    let axis_enum = descriptor["inputSchema"]["properties"]["group_by"]["items"]["enum"]
        .as_array()
        .expect("axis enum");
    let names: Vec<&str> = axis_enum.iter().filter_map(Value::as_str).collect();
    for required in ["grain", "model", "day"] {
        assert!(
            names.contains(&required),
            "descriptor must list `{required}`"
        );
    }
}
