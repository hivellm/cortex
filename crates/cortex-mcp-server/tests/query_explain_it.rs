//! Phase19 §3.5 + §5.1 — wiremock IT for `cortex_query_explain`.

use cortex_mcp_server::{Server, ToolContext};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn query_explain_happy_path_returns_lanes_and_fusion_math() {
    let mock = MockServer::start().await;
    let canned = json!({
        "intent": "free_search",
        "per_lane_hits": [
            {"lane": "vector", "ms": 12},
            {"lane": "keyword", "ms": 34},
            {"lane": "graph"}
        ],
        "fusion_math": {
            "rrf_k": 60.0,
            "alpha": 0.7,
            "recency_decay_lambda": 0.0,
            "drops": []
        },
        "final_envelope": {
            "intent": "free_search",
            "query_id": "01HZQ01",
            "results": {"snippets": [], "decisions": [], "violations": [], "graph_neighbors": [], "similar_turns": [], "topic_cards": [], "consolidations": []},
            "scope_resolved": {},
            "budget": {"used_ms": 50, "limit_ms": 500, "intent": "free_search", "lanes_run": [], "cache": "miss"},
            "debug": {"lanes": {"vector_ms": 12, "keyword_ms": 34}}
        },
        "match_strategy": "envelope_only"
    });
    Mock::given(method("POST"))
        .and(path("/v1/query/explain"))
        .and(body_partial_json(json!({"query": "retention"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned.clone()))
        .mount(&mock)
        .await;

    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cortex_query_explain",
            "arguments": {"query": "retention"}
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
    assert_eq!(parsed["match_strategy"], "envelope_only");
    assert_eq!(parsed["fusion_math"]["rrf_k"], 60.0);
    assert_eq!(parsed["per_lane_hits"][0]["lane"], "vector");
}

#[tokio::test]
async fn query_explain_rejects_missing_query_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cortex_query_explain",
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
async fn query_explain_rejects_unknown_intent_with_invalid_input() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "cortex_query_explain",
            "arguments": {"query": "x", "intent": "Explain"}
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
async fn query_explain_propagates_intent_and_scope() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query/explain"))
        .and(body_partial_json(json!({
            "query": "retention",
            "intent": "law_check",
            "scope": {"repo": "cortex"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "intent": "law_check",
            "per_lane_hits": [],
            "fusion_math": {"rrf_k": 60.0, "alpha": 0.7, "recency_decay_lambda": 0.0, "drops": []},
            "final_envelope": {
                "intent": "law_check",
                "query_id": "01HZ02",
                "results": {"snippets": [], "decisions": [], "violations": [], "graph_neighbors": [], "similar_turns": [], "topic_cards": [], "consolidations": []},
                "scope_resolved": {"repo": "cortex"},
                "budget": {"used_ms": 10, "limit_ms": 500, "intent": "law_check", "lanes_run": [], "cache": "miss"},
                "debug": {"lanes": {}}
            },
            "match_strategy": "envelope_only"
        })))
        .mount(&mock)
        .await;
    let server = Server::new(ToolContext::new(mock.uri()));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "cortex_query_explain",
            "arguments": {
                "query": "retention",
                "intent": "law_check",
                "scope": {"repo": "cortex"}
            }
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
async fn query_explain_returns_api_unreachable_when_upstream_dead() {
    let server = Server::new(ToolContext::new("http://127.0.0.1:1"));
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "cortex_query_explain",
            "arguments": {"query": "x"}
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
async fn query_explain_descriptor_pins_intent_enum_and_required_query() {
    use cortex_mcp_server::{QueryExplainTool, Tool};
    let descriptor = QueryExplainTool::new().descriptor();
    let required = descriptor["inputSchema"]["required"]
        .as_array()
        .expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("query")));
    let enum_values = descriptor["inputSchema"]["properties"]["intent"]["enum"]
        .as_array()
        .expect("intent enum");
    let names: Vec<&str> = enum_values.iter().filter_map(Value::as_str).collect();
    for required in [
        "pre_change_context",
        "decision_lookup",
        "similar_problems",
        "law_check",
        "free_search",
        "explain",
    ] {
        assert!(
            names.contains(&required),
            "descriptor must list `{required}`"
        );
    }
}
