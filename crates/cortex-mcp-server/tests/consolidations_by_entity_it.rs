//! Phase19 §2.3 + §5.1 — wiremock IT for `cortex_consolidations_by_entity`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidations_by_entity_repo_kind_routes_to_filter_strategy() {
    let mock = MockServer::start().await;
    let canned = json!({
        "hits": [
            {
                "event_id": "01HZCONS01",
                "kind": "consolidation",
                "repo": "cortex",
                "title": "Session digest"
            }
        ],
        "processing_time_ms": 3,
        "estimated_total_hits": 1,
        "match_strategy": "filter"
    });
    Mock::given(method("POST"))
        .and(path("/v1/consolidations/by-entity"))
        .and(body_partial_json(json!({
            "entity": {"kind": "repo", "value": "cortex"}
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
            "name": "cortex_consolidations_by_entity",
            "arguments": {"entity": {"kind": "repo", "value": "cortex"}}
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
    assert_eq!(parsed["match_strategy"], "filter");
    assert_eq!(parsed["hits"][0]["repo"], "cortex");
}

#[tokio::test]
async fn consolidations_by_entity_decision_id_kind_routes_to_keyword_strategy() {
    let mock = MockServer::start().await;
    let canned = json!({
        "hits": [],
        "processing_time_ms": 1,
        "estimated_total_hits": 0,
        "match_strategy": "q"
    });
    Mock::given(method("POST"))
        .and(path("/v1/consolidations/by-entity"))
        .and(body_partial_json(json!({
            "entity": {"kind": "decision_id", "value": "DEC-0042"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_by_entity",
            "arguments": {"entity": {"kind": "decision_id", "value": "DEC-0042"}}
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
    assert_eq!(parsed["match_strategy"], "q");
}

#[tokio::test]
async fn consolidations_by_entity_rejects_unknown_kind_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_by_entity",
            "arguments": {"entity": {"kind": "person", "value": "alice"}}
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
async fn consolidations_by_entity_rejects_missing_entity_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_by_entity",
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
async fn consolidations_by_entity_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidations_by_entity",
            "arguments": {"entity": {"kind": "repo", "value": "cortex"}}
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
async fn consolidations_by_entity_descriptor_pins_kind_enum_and_limit_bounds() {
    use cortex_mcp_server::{ConsolidationsByEntityTool, Tool};
    let descriptor = ConsolidationsByEntityTool::new().descriptor();
    let enum_values = descriptor["inputSchema"]["properties"]["entity"]["properties"]["kind"]
        ["enum"]
        .as_array()
        .expect("kind enum present");
    let names: Vec<&str> = enum_values.iter().filter_map(Value::as_str).collect();
    for required in ["file", "function", "decision_id", "repo", "model"] {
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
