//! Phase19 §2.5 + §5.1 — wiremock IT for `cortex_consolidation_lineage`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn consolidation_lineage_happy_path_returns_decisions_sessions_files_cost() {
    let mock = MockServer::start().await;
    let id = "01HZCONS01";
    let canned = json!({
        "consolidation_id": "cons-ses-aaaaaaaaaaaaaaaaaaaaaaaa",
        "source_session_ids": ["01HSESS01", "01HSESS02"],
        "decisions": [
            {"decision_id": "DEC-0042"},
            {"decision_id": "DEC-0099"}
        ],
        "files": [
            {"path": "crates/cortex-api/src/lib.rs"}
        ],
        "cost": {
            "model": "claude-opus-4-7"
        },
        "match_strategy": "doc_only"
    });
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/consolidations/[^/]+/lineage$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_lineage",
            "arguments": {"id": id}
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
    assert_eq!(parsed["match_strategy"], "doc_only");
    assert_eq!(parsed["source_session_ids"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["decisions"][0]["decision_id"], "DEC-0042");
    assert_eq!(parsed["files"][0]["path"], "crates/cortex-api/src/lib.rs");
    assert_eq!(parsed["cost"]["model"], "claude-opus-4-7");
}

#[tokio::test]
async fn consolidation_lineage_rejects_missing_id_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "cortex_consolidation_lineage", "arguments": {}}
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn consolidation_lineage_rejects_non_alphanumeric_id() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "cortex_consolidation_lineage", "arguments": {"id": "bad/id"}}
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn consolidation_lineage_surfaces_not_found_on_unknown_id() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/consolidations/[^/]+/lineage$"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "reason": "not_found",
            "detail": "no consolidation matches id `01HZUNKNOWN`"
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_lineage",
            "arguments": {"id": "01HZUNKNOWN"}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["reason"], "not_found");
}

#[tokio::test]
async fn consolidation_lineage_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_consolidation_lineage",
            "arguments": {"id": "01HZCONS01"}
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
