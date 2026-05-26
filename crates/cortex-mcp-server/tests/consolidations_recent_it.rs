//! Phase19 §2.2 + §5.1 — wiremock IT for `cortex_consolidations_recent`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidations_recent_happy_path_returns_hits_sorted_newest_first() {
    let mock = MockServer::start().await;
    let canned = json!({
        "hits": [
            {
                "event_id": "01HZCONS01",
                "kind": "consolidation",
                "title": "Session digest for 01HZSESS01",
                "repo": "cortex",
                "grain": "session",
                "occurred_at": 1716700000000_i64,
                "ext": {"consolidation": {"consolidation_id": "cons-ses-aaaaaaaaaaaaaaaaaaaaaaaa", "grain": "session"}}
            },
            {
                "event_id": "01HZCONS02",
                "kind": "consolidation",
                "title": "Session digest for 01HZSESS02",
                "repo": "cortex",
                "grain": "session",
                "occurred_at": 1716600000000_i64,
                "ext": {"consolidation": {"consolidation_id": "cons-ses-bbbbbbbbbbbbbbbbbbbbbbbb", "grain": "session"}}
            }
        ],
        "processing_time_ms": 4,
        "estimated_total_hits": 2
    });
    Mock::given(method("GET"))
        .and(path("/v1/consolidations/recent"))
        .and(query_param("repo", "cortex"))
        .and(query_param("grain", "session"))
        .and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_recent",
            "arguments": {"repo": "cortex", "grain": "session", "limit": 5}
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
    assert_eq!(parsed["hits"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["hits"][0]["event_id"], "01HZCONS01");
    assert_eq!(parsed["estimated_total_hits"], 2);
}

#[tokio::test]
async fn consolidations_recent_rejects_non_snake_grain_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_recent",
            "arguments": {"grain": "Session"}
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
async fn consolidations_recent_propagates_since_until_window_to_query_string() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/consolidations/recent"))
        .and(query_param("since", "2026-05-01T00:00:00Z"))
        .and(query_param("until", "2026-05-26T23:59:59Z"))
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
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_recent",
            "arguments": {"since": "2026-05-01T00:00:00Z", "until": "2026-05-26T23:59:59Z"}
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
    assert_eq!(parsed["estimated_total_hits"], 0);
}

#[tokio::test]
async fn consolidations_recent_surfaces_api_unreachable_when_upstream_is_dead() {
    // Bind nothing — the tool must fold the connection error into
    // a soft error with reason `api_unreachable` (phase14i).
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_recent",
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
async fn consolidations_recent_descriptor_pins_grain_enum_and_limit_bounds() {
    use cortex_mcp_server::{ConsolidationsRecentTool, Tool};
    let descriptor = ConsolidationsRecentTool::new().descriptor();
    let enum_values = descriptor["inputSchema"]["properties"]["grain"]["enum"]
        .as_array()
        .expect("grain enum present");
    let names: Vec<&str> = enum_values.iter().filter_map(Value::as_str).collect();
    for required in ["session", "topic", "decision_trace"] {
        assert!(
            names.contains(&required),
            "descriptor must list `{required}`"
        );
    }
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["maximum"],
        30
    );
    assert_eq!(
        descriptor["inputSchema"]["properties"]["limit"]["minimum"],
        1
    );
}
