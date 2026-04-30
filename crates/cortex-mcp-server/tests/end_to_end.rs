//! End-to-end MCP server tests:
//! - `tools/call cortex_query` against a wiremock'd `cortex-api`.
//! - `tools/call cortex_status` against a wiremock'd `/v1/status`.
//! - `tools/call cortex_query` returns a soft-error envelope when
//!   the upstream API is unreachable.
//!
//! Each test boots a [`Server`] with a [`ToolContext`] pointing at a
//! mock HTTP server, drives one frame through [`Server::handle_frame`]
//! and asserts on the JSON-RPC response.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn tools_call_query_round_trips_through_wiremock() {
    let mock = MockServer::start().await;

    let canned = json!({
        "intent": "free_search",
        "query_id": "q_test_1",
        "scope_resolved": {},
        "results": {
            "snippets": [],
            "decisions": [],
            "violations": [],
            "graph_neighbors": [],
            "similar_turns": []
        },
        "laws_active": [],
        "budget": { "used_ms": 5, "cap_ms": 200, "cache": "miss" },
        "debug": { "lanes": {} }
    });

    Mock::given(method("POST"))
        .and(path("/v1/query"))
        .and(header("x-cortex-caller", "claude-code-plugin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_query",
            "arguments": {
                "intent": "free_search",
                "query": "ef_search",
                "scope": {},
                "limit": 5,
                "k": 20,
                "include": ["snippets"],
                "budget_ms": 200
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    assert_eq!(resp["id"], 1);
    assert!(resp["result"].is_object());
    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("content text");
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["query_id"], "q_test_1");
    assert_eq!(parsed["intent"], "free_search");

    let metrics = server.metrics().snapshot();
    assert_eq!(metrics.invocations_query, 1);
    assert_eq!(metrics.errors_query, 0);
}

#[tokio::test]
async fn tools_call_query_surfaces_spec_11_reason_on_4xx() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({ "reason": "scope_forbidden" })),
        )
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "cortex_query",
            "arguments": {
                "intent": "free_search",
                "query": "x",
                "scope": { "repo": "secret" },
                "limit": 5, "k": 20, "include": [], "budget_ms": 100
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    assert_eq!(resp["id"], 7);
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["reason"], "scope_forbidden");

    let metrics = server.metrics().snapshot();
    assert_eq!(metrics.errors_query, 1);
}

#[tokio::test]
async fn tools_call_query_returns_api_unreachable_when_upstream_dead() {
    // 127.0.0.1:1 — port 1 refuses fast on every platform.
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": {
            "name": "cortex_query",
            "arguments": {
                "intent": "free_search",
                "query": "x",
                "limit": 1, "k": 1, "include": [], "budget_ms": 50
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    assert_eq!(resp["id"], 9);
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["reason"], "api_unreachable");
}

#[tokio::test]
async fn tools_call_status_reports_reachable_when_v1_status_responds() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "pid": 4242,
            "queue_depth": 3,
            "wal_bytes": 1024
        })))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": { "name": "cortex_status", "arguments": {} }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["api_reachable"], true);
    assert_eq!(parsed["daemon"]["pid"], 4242);
    assert_eq!(parsed["mcp_server"]["name"], "cortex-mcp-server");
}

#[tokio::test]
async fn tools_call_status_marks_api_unreachable_on_dead_upstream() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "tools/call",
        "params": { "name": "cortex_status", "arguments": {} }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["api_reachable"], false);
    assert!(parsed["mcp_server"]["pid"].is_number());
}

#[tokio::test]
async fn tools_call_pre_thinking_returns_bundle_when_upstream_returns_results() {
    let mock = MockServer::start().await;

    let canned = json!({
        "intent": "free_search",
        "query_id": "pt_q_1",
        "scope_resolved": {},
        "results": {
            "snippets": [
                {
                    "rank": 1,
                    "source": "vector",
                    "path": "a.rs",
                    "text": "ef_search default = 64",
                    "score": 0.9
                }
            ],
            "decisions": [],
            "violations": [],
            "graph_neighbors": [],
            "similar_turns": []
        },
        "laws_active": [],
        "budget": { "used_ms": 8, "cap_ms": 600, "cache": "miss" },
        "debug": { "lanes": {} }
    });

    Mock::given(method("POST"))
        .and(path("/v1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned))
        .mount(&mock)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 21,
        "method": "tools/call",
        "params": {
            "name": "cortex_pre_thinking",
            "arguments": {
                "user_prompt": "what is the ef_search default?",
                "cwd": tmp.path().to_string_lossy(),
                "session_id": "ses_test",
                "turn_id": "turn_test",
                "budget_bytes": 8192,
                "budget_ms": 800
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    let bundle = parsed["bundle"].as_str().unwrap_or("");
    assert!(
        bundle.starts_with("<!-- cortex:") && bundle.contains("query_id=pt_q_1"),
        "bundle missing the spec-12 leading comment, got: {bundle}"
    );
}

#[tokio::test]
async fn tools_call_query_injects_x_cortex_cwd_header() {
    // Phase6a §3.3 — `cortex_query` MUST stamp the operator's cwd on
    // the outbound request so the daemon's scope resolver can derive
    // `scope.repo` from the basename when the body omits it. The
    // wiremock matcher `header_exists("x-cortex-cwd")` rejects the
    // request unless the header is present, which surfaces as the
    // upstream returning 404 → MCP error envelope.
    let mock = MockServer::start().await;

    let canned = json!({
        "intent": "free_search",
        "query_id": "q_cwd_1",
        "scope_resolved": { "repo": "vectorizer" },
        "results": {
            "snippets": [],
            "decisions": [],
            "violations": [],
            "graph_neighbors": [],
            "similar_turns": []
        },
        "laws_active": [],
        "budget": { "used_ms": 1, "cap_ms": 200, "cache": "miss" },
        "debug": { "lanes": {} }
    });

    Mock::given(method("POST"))
        .and(path("/v1/query"))
        .and(wiremock::matchers::header_exists("x-cortex-cwd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 41,
        "method": "tools/call",
        "params": {
            "name": "cortex_query",
            "arguments": {
                "intent": "free_search",
                "query": "x",
                "limit": 1, "k": 1, "include": ["snippets"], "budget_ms": 200
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["id"], 41);
    assert_eq!(
        resp["result"]["isError"], false,
        "wiremock matched x-cortex-cwd header — request reached the canned response"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["query_id"], "q_cwd_1");
}

// ---------------------------------------------------------------------
// Phase10j — cortex_audit / cortex_capture_memory / cortex_session_replay
// ---------------------------------------------------------------------

#[tokio::test]
async fn tools_call_audit_returns_envelope_when_query_id_known() {
    let mock = MockServer::start().await;
    let canned = json!({
        "kind": "query_audit",
        "caller": "claude-code-plugin",
        "query_id": "01KQDX",
        "intent": "free_search",
        "scope_resolution": "explicit",
        "counts": { "snippets": 0, "decisions": 0 },
        "latency_ms": 12,
        "cache": "miss"
    });
    Mock::given(method("GET"))
        .and(path("/v1/audit/01KQDX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 51,
        "method": "tools/call",
        "params": {
            "name": "cortex_audit",
            "arguments": { "query_id": "01KQDX" }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], false);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["query_id"], "01KQDX");
    assert_eq!(parsed["intent"], "free_search");
}

#[tokio::test]
async fn tools_call_audit_surfaces_404_as_soft_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/audit/missing"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({ "reason": "audit_envelope_not_found" })),
        )
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 53,
        "method": "tools/call",
        "params": {
            "name": "cortex_audit",
            "arguments": { "query_id": "missing" }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], true);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["reason"], "audit_envelope_not_found");
}

#[tokio::test]
async fn tools_call_capture_memory_round_trips_through_v1_ingest() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ingest"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({
                "event_id": "evt_test_1",
                "content_hash": "deadbeef",
                "indexed_at": "2026-04-30T12:00:00Z"
            })),
        )
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 61,
        "method": "tools/call",
        "params": {
            "name": "cortex_capture_memory",
            "arguments": {
                "kind": "memory",
                "body": "Phase9k uses per-name semaphore",
                "repo": "cortex"
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], false);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["event_id"], "evt_test_1");
    assert_eq!(parsed["content_hash"], "deadbeef");
}

#[tokio::test]
async fn tools_call_capture_memory_rejects_oversize_body_locally() {
    // No mock server — the tool must short-circuit before any HTTP call.
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let big = "x".repeat(8 * 1024 + 1);
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 63,
        "method": "tools/call",
        "params": {
            "name": "cortex_capture_memory",
            "arguments": {
                "kind": "memory",
                "body": big,
                "repo": "cortex"
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], true);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["reason"], "body_too_large");
}

#[tokio::test]
async fn tools_call_session_replay_returns_capped_turns() {
    let mock = MockServer::start().await;
    let canned = json!({
        "session_id": "01QDKXMY4TG7",
        "repos": ["cortex"],
        "turns": [
            { "turn_id": "t1", "user_message": "u1", "assistant_message": "a1", "started_at_ms": 100, "completed_at_ms": 110 },
            { "turn_id": "t2", "user_message": "u2", "assistant_message": "a2", "started_at_ms": 200, "completed_at_ms": 210 },
            { "turn_id": "t3", "user_message": "u3", "assistant_message": "a3", "started_at_ms": 300, "completed_at_ms": 310 }
        ]
    });
    Mock::given(method("GET"))
        .and(path("/v1/dashboard/conversations/01QDKXMY4TG7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 71,
        "method": "tools/call",
        "params": {
            "name": "cortex_session_replay",
            "arguments": {
                "session_id": "01QDKXMY4TG7",
                "max_turns": 2
            }
        }
    });
    let raw = server
        .handle_frame(frame.to_string().as_bytes())
        .await
        .expect("response bytes");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["result"]["isError"], false);
    let parsed: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["session_id"], "01QDKXMY4TG7");
    assert_eq!(parsed["total_turns"], 3);
    // 2 dashboard turns × 2 rows each (user + assistant) = 4 returned rows.
    assert_eq!(parsed["returned_turns"], 4);
    let turns = parsed["turns"].as_array().unwrap();
    assert_eq!(turns[0]["role"], "user");
    assert_eq!(turns[0]["summary"], "u1");
    assert_eq!(turns[1]["role"], "assistant");
    assert_eq!(turns[1]["summary"], "a1");
}
