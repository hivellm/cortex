//! JSON-RPC-to-HTTP contracts for explicit daily memory, without optional services.

use cortex_mcp_server::tools::{CaptureMemoryTool, MemoryReadTool};
use cortex_mcp_server::{Server, Tool, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn rpc(server: &Server, method: &str, params: Value) -> Value {
    let frame = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let result = server
        .handle_frame(&serde_json::to_vec(&frame).unwrap())
        .await
        .unwrap();
    serde_json::from_slice(&result).unwrap()
}

fn payload(response: &Value) -> Value {
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn initialized_client_name_overrides_argument_without_granting_trust() {
    let upstream = MockServer::start().await;
    let receipt = json!({ "event_id": "legacy-event", "memory_id": "ADR-12", "version": 1, "local_accepted": true, "index_complete": false });
    Mock::given(method("POST"))
        .and(path("/v1/ingest"))
        .and(header("x-cortex-caller", "cortex-mcp-server"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&receipt))
        .expect(1)
        .mount(&upstream)
        .await;
    let server = Server::new(ToolContext::new(upstream.uri()).with_api_key("test-key"));
    rpc(
        &server,
        "initialize",
        json!({ "clientInfo": { "name": "codex", "version": "observed-version" } }),
    )
    .await;
    let args = json!({
        "kind": "DECISION", "body": "Explicit correction", "repo": "Exact/Repo", "title": "Use durable store", "classify": true,
        "memory_id": "ADR-12", "status": "accepted", "agent": "forged-agent", "turn_id": "observed-turn",
        "retry_key": "attempt-1", "expected_revision": 7, "supersedes": "ADR-11",
        "branch": "main", "path": "src/api", "pinned": true, "global": false,
        "sources": [{ "event_id": "source-event", "kind": "learning", "reference": "src/api.rs", "content_hash": "sha256:original", "revision": "commit" }],
        "attribution": "trusted_transport", "caller": "admin"
    });
    let response = rpc(
        &server,
        "tools/call",
        json!({ "name": "cortex_capture_memory", "arguments": args }),
    )
    .await;
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(payload(&response), receipt);
    let requests = upstream.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["kind"], "decision");
    assert_eq!(sent["agent"], "codex");
    for field in [
        "body",
        "classify",
        "repo",
        "title",
        "memory_id",
        "status",
        "turn_id",
        "retry_key",
        "expected_revision",
        "supersedes",
        "branch",
        "path",
        "pinned",
        "global",
        "sources",
    ] {
        assert_eq!(sent[field], args[field], "field: {field}");
    }
    for absent in ["attribution", "caller", "session_id"] {
        assert!(
            sent.get(absent).is_none(),
            "must not manufacture or trust {absent}"
        );
    }
}

#[tokio::test]
async fn legacy_capture_keeps_unknown_agent_and_session_when_initialize_has_no_name() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ingest"))
        .and(header("x-cortex-caller", "cortex-mcp-server"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "memory_id": "generated-once" })),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let server = Server::new(ToolContext::new(upstream.uri()));
    rpc(&server, "initialize", json!({})).await;
    let args = json!({ "kind": "memory", "body": "Exact fact", "repo": "cortex", "topic": "retrieval", "severity": "notable" });
    let response = rpc(
        &server,
        "tools/call",
        json!({ "name": "cortex_capture_memory", "arguments": args }),
    )
    .await;
    assert_eq!(response["result"]["isError"], false);
    let requests = upstream.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let mut expected = args;
    expected["classify"] = json!(false);
    assert_eq!(sent, expected);
    assert!(requests[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn capture_descriptor_defaults_local_and_forwards_explicit_classification_consent() {
    let descriptor = CaptureMemoryTool::new().descriptor();
    let property = &descriptor["inputSchema"]["properties"]["classify"];
    assert_eq!(property["type"], "boolean");
    assert_eq!(property["default"], false);
    assert!(property["description"]
        .as_str()
        .unwrap()
        .contains("external TypeSafe"));
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ingest"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"memory_id":"local"})))
        .expect(3)
        .mount(&upstream)
        .await;
    let server = Server::new(ToolContext::new(upstream.uri()));
    for consent in [None, Some(false), Some(true)] {
        let mut args = json!({"kind":"memory","body":"Synthetic local fact","repo":"repo"});
        if let Some(value) = consent {
            args["classify"] = json!(value);
        }
        let result = rpc(
            &server,
            "tools/call",
            json!({"name":"cortex_capture_memory","arguments":args}),
        )
        .await;
        assert_eq!(result["result"]["isError"], false);
    }
    let requests = upstream.received_requests().await.unwrap();
    for (request, expected) in requests.iter().zip([false, false, true]) {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["classify"], expected);
    }
}

#[tokio::test]
async fn malformed_initialize_label_does_not_replace_explicit_client_report() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ingest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "memory_id": "id" })))
        .expect(1)
        .mount(&upstream)
        .await;
    let server = Server::new(ToolContext::new(upstream.uri()));
    rpc(
        &server,
        "initialize",
        json!({ "clientInfo": { "name": "bad\nname" } }),
    )
    .await;
    let response = rpc(&server, "tools/call", json!({ "name": "cortex_capture_memory", "arguments": {
        "kind": "learning", "body": "Client-reported fact", "repo": "cortex", "agent": "kimi", "session_id": "reported-session"
    } })).await;
    assert_eq!(response["result"]["isError"], false);
    let requests = upstream.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["agent"], "kimi");
    assert_eq!(sent["session_id"], "reported-session");
    assert!(sent.get("attribution").is_none());
}

#[tokio::test]
async fn memory_read_preserves_durable_prepared_receipt_and_caps_delivery_request() {
    let upstream = MockServer::start().await;
    let delivered = json!({
        "context": "ADR-12 v2 [source-event]: Use durable store.", "query_id": "delivery-1", "delivery_id": "delivery-1",
        "revision": 8, "global_revision": 2, "outcome": "prepared",
        "memories": [{ "id": "ADR-12", "version": 2, "event_id": "decision-event", "scope": { "repository": "cortex" } }]
    });
    Mock::given(method("POST"))
        .and(path("/v1/memory/query"))
        .and(header("x-cortex-caller", "cortex-mcp-server"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&delivered))
        .expect(1)
        .mount(&upstream)
        .await;
    let server = Server::new(ToolContext::new(upstream.uri()).with_api_key("test-key"));
    let response = rpc(&server, "tools/call", json!({ "name": "cortex_memory_read", "arguments": {
        "repo": "cortex", "query": "durable store", "branch": "main", "paths": ["src/api.rs"], "include_global": true,
        "limit": 1000, "budget_bytes": 999999
    } })).await;
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(payload(&response), delivered);
    let requests = upstream.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        sent,
        json!({ "repo": "cortex", "query": "durable store", "branch": "main", "paths": ["src/api.rs"], "include_global": true, "limit": 5, "budget_bytes": 1536 })
    );
}

#[tokio::test]
async fn memory_read_defaults_exclude_globals_and_upstream_failure_stays_error() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/memory/query"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({ "error": "daily_memory_unavailable" })),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let result = MemoryReadTool::new()
        .call(
            &ToolContext::new(upstream.uri()),
            json!({ "repo": "cortex", "query": "fact" }),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    let failure: Value = serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(failure["reason"], "daily_memory_unavailable");
    let requests = upstream.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        sent,
        json!({ "repo": "cortex", "query": "fact", "paths": [], "include_global": false, "limit": 5, "budget_bytes": 1536 })
    );
}

#[tokio::test]
async fn invalid_memory_read_short_circuits_and_capture_keeps_body_and_repo_checks() {
    let ctx = ToolContext::new("http://127.0.0.1:1");
    for args in [
        json!({ "query": "missing repo" }),
        json!({ "repo": "", "query": "empty repo" }),
        json!({ "repo": "Exact\u{0000}/Repo", "query": "invalid repo" }),
        json!({ "repo": "repo", "query": "x".repeat(4097) }),
        json!({ "repo": "repo", "query": "x", "limit": -1 }),
        json!({ "repo": "repo", "query": "x", "paths": vec!["src/file"; 33] }),
    ] {
        let error = MemoryReadTool::new().call(&ctx, args).await.unwrap_err();
        assert_eq!(error.reason, "invalid_input");
    }
    for repo in [
        "",
        " Exact/Repo",
        "Exact/Repo ",
        "Exact\0/Repo",
        "Exact\n/Repo",
        &"x".repeat(257),
    ] {
        let invalid_repo = CaptureMemoryTool::new()
            .call(
                &ctx,
                json!({ "kind": "decision", "body": "x", "repo": repo }),
            )
            .await
            .unwrap_err();
        assert_eq!(invalid_repo.reason, "invalid_input");
    }
    let oversized = CaptureMemoryTool::new()
        .call(
            &ctx,
            json!({ "kind": "decision", "body": "x".repeat(8193), "repo": "cortex" }),
        )
        .await
        .unwrap();
    assert!(oversized.is_error);
}

#[tokio::test]
async fn oversized_upstream_memory_response_cannot_overflow_mcp_transport() {
    for text in ["x".repeat(50 * 1024), "\"".repeat(20 * 1024)] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/memory/query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "context": text })))
            .expect(1)
            .mount(&upstream)
            .await;
        let result = MemoryReadTool::new()
            .call(
                &ToolContext::new(upstream.uri()),
                json!({ "repo": "cortex", "query": "fact" }),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        let failure: Value =
            serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(failure["reason"], "budget_exceeded");
        assert!(serde_json::to_vec(&result).unwrap().len() < 48 * 1024);
    }
}
