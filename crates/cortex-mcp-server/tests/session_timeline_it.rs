//! Phase19 §1.2 + §5.1 — wiremock IT for `cortex_session_timeline`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{method, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn session_timeline_happy_path_returns_chronological_rows() {
    let mock = MockServer::start().await;
    let session_id = "01HZSESS0000000000000000ZZ";
    let canned = json!({
        "session_id": session_id,
        "count": 3,
        "truncated": false,
        "events": [
            {"ts":"2026-05-26T07:00:00Z","kind":"turn","title":"explain dispatcher","event_id":"01EV0","deltas":{}},
            {"ts":"2026-05-26T07:00:05Z","kind":"tool_call","title":"tool: Read","event_id":"01EV1","deltas":{"tool_name":"Read","exit_code":0}},
            {"ts":"2026-05-26T07:00:30Z","kind":"turn","title":"reply","event_id":"01EV2","deltas":{"model":"opus-4-7"}}
        ]
    });
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/sessions/[^/]+/timeline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_session_timeline",
            "arguments": {"session_id": session_id, "limit": 50}
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
    assert_eq!(parsed["session_id"], session_id);
    assert_eq!(parsed["count"], 3);
    assert_eq!(parsed["events"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn session_timeline_rejects_missing_session_id_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "cortex_session_timeline", "arguments": {}}
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    // tools::ToolError::invalid_input surfaces as a JSON-RPC error
    // (-32602), NOT a soft-error envelope.
    assert!(
        resp.get("error").is_some(),
        "expected JSON-RPC error envelope: {resp}"
    );
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn session_timeline_rejects_non_alphanumeric_session_id() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "cortex_session_timeline", "arguments": {"session_id": "bad/path"}}
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert!(resp.get("error").is_some());
    assert_eq!(resp["error"]["data"]["reason"], "invalid_input");
}

#[tokio::test]
async fn session_timeline_propagates_kind_filter_via_query_string() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/sessions/[^/]+/timeline$"))
        .and(query_param("kind", "tool_call"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "session_id": "01HZSESS0000000000000000ZZ",
            "count": 0,
            "truncated": false,
            "events": []
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_session_timeline",
            "arguments": {"session_id": "01HZSESS0000000000000000ZZ", "kind": "tool_call", "limit": 10}
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    // Either soft success or successful body — the contract under
    // test is that wiremock matched the query-string filter.
    assert_eq!(resp["result"]["isError"], false);
}
