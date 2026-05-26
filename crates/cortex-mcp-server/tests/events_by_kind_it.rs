//! Phase19 §1.1 + §5.1 — wiremock IT for `cortex_events_by_kind`.
//!
//! Boots a wiremock cortex-api stub backing `POST /v1/search/events`
//! and drives the MCP tool through `Server::handle_frame`. Covers:
//!
//! 1. Happy path — 2xx body lands as `ToolResult::ok` with native
//!    envelope shape preserved.
//! 2. Unknown kind — 4xx with `bad_input` reason surfaces as a soft
//!    error (parity with phase14i `ToolError` contract).
//! 3. Upstream unreachable — `api_unreachable` reason.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn events_by_kind_happy_path_returns_native_meili_shape() {
    let mock = MockServer::start().await;
    let canned = json!({
        "kind": "consolidation",
        "index": "cortex_consolidations",
        "hits": [
            {"event_id": "01KSH2KCWFQ50816C8WRS1BDYK", "kind": "consolidation",
             "summary": "Feature-gated integration tests", "repo": "vectorizer",
             "occurred_at_ms": 1779763360657u64},
            {"event_id": "01KSH2JTWAV32K8ANV8KCYPQT0", "kind": "consolidation",
             "summary": "Go and C# SDK parity", "repo": "vectorizer",
             "occurred_at_ms": 1779763342000u64}
        ],
        "processing_time_ms": 4,
        "estimated_total_hits": 2
    });
    Mock::given(method("POST"))
        .and(path("/v1/search/events"))
        .and(body_partial_json(json!({"kind": "consolidation"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_events_by_kind",
            "arguments": {
                "kind": "consolidation",
                "repo": "vectorizer",
                "limit": 5
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("content text");
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["kind"], "consolidation");
    assert_eq!(parsed["index"], "cortex_consolidations");
    assert_eq!(parsed["hits"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["hits"][0]["repo"], "vectorizer");
    assert_eq!(parsed["estimated_total_hits"], 2);
}

#[tokio::test]
async fn events_by_kind_surfaces_bad_input_on_unknown_kind() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search/events"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "reason": "bad_input",
            "detail": "unknown kind `bogus`"
        })))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_events_by_kind",
            "arguments": { "kind": "bogus" }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["reason"], "bad_input");
}

#[tokio::test]
async fn events_by_kind_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_events_by_kind",
            "arguments": { "kind": "turn" }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    assert_eq!(resp["id"], 3);
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["reason"], "api_unreachable");
}

#[tokio::test]
async fn events_by_kind_descriptor_lists_every_canonical_kind() {
    // Pin the schema's enum so a future drop of a kind surfaces here.
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
        .find(|t| t["name"] == "cortex_events_by_kind")
        .expect("cortex_events_by_kind in tool list");
    let kinds = desc["inputSchema"]["properties"]["kind"]["enum"]
        .as_array()
        .expect("kind.enum");
    let strings: Vec<&str> = kinds.iter().filter_map(|v| v.as_str()).collect();
    for required in [
        "turn",
        "tool_call",
        "consolidation",
        "decision",
        "analysis",
        "memory",
        "law",
        "knowledge",
        "learning",
        "topic_card",
    ] {
        assert!(
            strings.contains(&required),
            "descriptor enum missing `{required}`; got {strings:?}"
        );
    }
}
