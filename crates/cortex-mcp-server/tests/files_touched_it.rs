//! Phase19 §1.4 + §5.1 — wiremock IT for `cortex_files_touched`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn files_touched_session_mode_uses_get_route() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/sessions/[^/]+/files-touched"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count": 2,
            "truncated": false,
            "paths": [
                {"path":"/a.rs","read_count":2,"write_count":1,"other_count":0,"last_touched_ts":"2026-05-26T07:10:00Z"},
                {"path":"/b.rs","read_count":1,"write_count":0,"other_count":0,"last_touched_ts":"2026-05-26T07:01:00Z"}
            ]
        })))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_files_touched",
            "arguments": {"session_id": "01HZSESS0000000000000000ZZ", "limit": 20}
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
    assert_eq!(parsed["count"], 2);
    assert_eq!(parsed["paths"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn files_touched_window_mode_uses_post_route() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search/files-touched"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count": 0,
            "truncated": false,
            "paths": []
        })))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_files_touched",
            "arguments": {"repo": "cortex", "since": "2026-05-26T00:00:00Z"}
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
async fn files_touched_rejects_non_alphanumeric_session_id() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_files_touched",
            "arguments": {"session_id": "bad/path"}
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
async fn files_touched_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_files_touched",
            "arguments": {"repo": "cortex"}
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
