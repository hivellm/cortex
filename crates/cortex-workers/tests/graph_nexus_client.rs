//! Integration tests for `cortex_workers::graph::nexus_client`.
//!
//! Moved out of `src/nexus_client.rs` so every test in this crate lives
//! under `tests/`. Exercises only the public surface; the private
//! `classify_message` helper is exercised through the public
//! `GraphClientError::classify` entry point (which delegates to it for
//! 4xx Api errors), and `json_to_sdk_value` is reachable through a
//! `#[doc(hidden)] pub` re-export so this conversion can stay covered
//! at unit granularity.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde_json::json;

use std::path::Path;

use cortex_workers::graph::config::GraphConfig;
use cortex_workers::graph::cypher::{load_from_dir, CypherTemplates, REQUIRED_TEMPLATES};
use cortex_workers::graph::nexus_client::{
    json_to_sdk_value, with_retry, GraphClient, GraphClientError, LiveNexusClient, MemoryCall,
    MemoryNexusClient, WriteStats,
};
use cortex_workers::graph::patch::{ConflictPolicy, EdgeOp, GraphPatch, NodeOp};
use cortex_workers::graph::schema;
use nexus_sdk::{NexusError, Value as SdkValue};

fn empty_props() -> BTreeMap<String, serde_json::Value> {
    BTreeMap::new()
}

fn node(label: &str, key: &str) -> NodeOp {
    NodeOp {
        label: label.to_string(),
        natural_key: key.to_string(),
        external_id: Some(key.to_string()),
        conflict_policy: ConflictPolicy::default(),
        props: empty_props(),
    }
}

fn edge(rel: &str, from_label: &str, from_key: &str, to_label: &str, to_key: &str) -> EdgeOp {
    EdgeOp {
        edge_type: rel.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props: empty_props(),
    }
}

#[tokio::test]
async fn memory_client_records_ensure_schema() {
    let client = MemoryNexusClient::new();
    client
        .ensure_schema(&["a".to_string(), "b".to_string()])
        .await
        .expect("ensure_schema must succeed against the fake");
    let calls = client.calls_snapshot();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        MemoryCall::EnsureSchema(stmts) => {
            assert_eq!(stmts, &vec!["a".to_string(), "b".to_string()]);
        }
        other => panic!("expected EnsureSchema, got {other:?}"),
    }
}

#[tokio::test]
async fn memory_client_records_write_tx_with_correct_counts() {
    let client = MemoryNexusClient::new();
    let patch = GraphPatch {
        nodes: vec![
            node("Turn", "T1"),
            node("Turn", "T2"),
            node("Session", "S1"),
        ],
        edges: vec![
            edge("HAS_TURN", "Session", "S1", "Turn", "T1"),
            edge("HAS_TURN", "Session", "S1", "Turn", "T2"),
            edge("HAS_TOOL_CALL", "Session", "S1", "ToolCall", "TC1"),
            edge("HAS_TOOL_CALL", "Session", "S1", "ToolCall", "TC2"),
        ],
    };
    let templates = CypherTemplates::default();
    let stats: WriteStats = client
        .run_write_tx(&patch, &templates)
        .await
        .expect("write_tx must succeed");
    assert_eq!(stats.nodes_upserted, 3);
    assert_eq!(stats.edges_upserted, 4);

    let calls = client.calls_snapshot();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        MemoryCall::WriteTx(captured) => {
            assert_eq!(captured.nodes.len(), 3);
            assert_eq!(captured.edges.len(), 4);
        }
        other => panic!("expected WriteTx, got {other:?}"),
    }
}

#[test]
fn key_field_for_returns_natural_key_for_artifact() {
    assert_eq!(LiveNexusClient::key_field_for("Artifact"), "natural_key");
    assert_eq!(LiveNexusClient::key_field_for("Repo"), "name");
    assert_eq!(LiveNexusClient::key_field_for("Turn"), "id");
    assert_eq!(LiveNexusClient::key_field_for("ToolCall"), "id");
    assert_eq!(LiveNexusClient::key_field_for("Decision"), "id");
    assert_eq!(LiveNexusClient::key_field_for("BrandNew"), "id");
}

#[test]
fn classify_message_detects_constraint_violation() {
    // Route through the public classifier — Api 4xx with a "constraint"
    // string lands in the same private branch the inline test used to
    // exercise directly.
    let err = GraphClientError::classify(
        NexusError::Api {
            status: 400,
            message: "Constraint violation: id already exists for Decision".into(),
        },
        "x",
    );
    match err {
        GraphClientError::ConstraintViolation { detail } => {
            assert!(detail.contains("Constraint"));
        }
        other => panic!("expected ConstraintViolation, got {other:?}"),
    }
}

#[test]
fn classify_message_detects_unauthorized() {
    let err = GraphClientError::classify(
        NexusError::Api {
            status: 400,
            message: "401 Unauthorized".into(),
        },
        "x",
    );
    assert!(matches!(err, GraphClientError::AuthFailed(_)));
}

#[test]
fn classify_api_500_is_transient() {
    let err = NexusError::Api {
        status: 503,
        message: "boom".to_string(),
    };
    assert!(GraphClientError::classify(err, "x").is_retriable());
}

#[test]
fn classify_connection_is_transient() {
    let err = NexusError::Connection("refused".into());
    assert!(GraphClientError::classify(err, "x").is_retriable());
}

#[test]
fn classify_api_400_is_not_retriable() {
    let err = NexusError::Api {
        status: 400,
        message: "bad request".to_string(),
    };
    assert!(!GraphClientError::classify(err, "x").is_retriable());
}

#[tokio::test]
async fn with_retry_recovers_after_transient_errors() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_for_op = counter.clone();
    let result: Result<u32, GraphClientError> = with_retry(3, move || {
        let counter = counter_for_op.clone();
        async move {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                Err(GraphClientError::TransientError("network blip".into()))
            } else {
                Ok(attempt)
            }
        }
    })
    .await;
    assert_eq!(result.expect("eventually succeeds"), 2);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn with_retry_does_not_retry_non_retriable() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_for_op = counter.clone();
    let result: Result<(), GraphClientError> = with_retry(3, move || {
        let counter = counter_for_op.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(GraphClientError::ConstraintViolation {
                detail: "duplicate".into(),
            })
        }
    })
    .await;
    assert!(matches!(
        result,
        Err(GraphClientError::ConstraintViolation { .. })
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn json_to_sdk_value_round_trips_primitives() {
    assert!(matches!(json_to_sdk_value(&json!(null)), SdkValue::Null));
    assert!(matches!(
        json_to_sdk_value(&json!(true)),
        SdkValue::Bool(true)
    ));
    assert!(matches!(json_to_sdk_value(&json!(42)), SdkValue::Int(42)));
    assert!(matches!(json_to_sdk_value(&json!(1.5)), SdkValue::Float(_)));
    match json_to_sdk_value(&json!("hi")) {
        SdkValue::String(s) => assert_eq!(s, "hi"),
        other => panic!("expected String, got {other:?}"),
    }
    match json_to_sdk_value(&json!([1, 2, 3])) {
        SdkValue::Array(v) => assert_eq!(v.len(), 3),
        other => panic!("expected Array, got {other:?}"),
    }
    match json_to_sdk_value(&json!({"k": 1})) {
        SdkValue::Object(map) => {
            assert!(matches!(map.get("k"), Some(SdkValue::Int(1))));
        }
        other => panic!("expected Object, got {other:?}"),
    }
}

// Removed: `missing_node_template_surfaces_as_hard_error`.
//
// The writer no longer dispatches through the `.cypher` template
// registry — Nexus 1.15.0 silently drops UNWIND-write and $param-write
// substitutions, so writes are now rendered as per-row Cypher with
// values escaped into the literal (see
// `phase1_graph_writer_nexus_compat`). The "missing template" error
// path is unreachable in this code path; the corresponding live-Nexus
// failure mode is now `GraphClientError::Nexus("... write not
// persisted ...")` and is covered by the live-stack probes below.

// ---- env-gated live probes (Round 5 owns full IT) -----------------

/// Smoke probe — only runs when `CORTEX_GRAPH_IT=1` is set so unit
/// runs stay hermetic.
#[tokio::test]
async fn schema_bootstrap_against_live_nexus_when_enabled() {
    if std::env::var("CORTEX_GRAPH_IT").as_deref() != Ok("1") {
        return;
    }
    let cfg = GraphConfig {
        nexus_url: "http://127.0.0.1:17002".into(),
        ..GraphConfig::default()
    };
    let client = LiveNexusClient::new(cfg).expect("construct live client");
    client
        .ensure_schema(&schema::statements())
        .await
        .expect("schema bootstrap");
    client
        .ensure_schema(&schema::statements())
        .await
        .expect("idempotent re-run");
}

/// Round-trip probe — also `CORTEX_GRAPH_IT=1` gated.
#[tokio::test]
async fn write_tx_against_live_nexus_when_enabled() {
    if std::env::var("CORTEX_GRAPH_IT").as_deref() != Ok("1") {
        return;
    }
    let cfg = GraphConfig {
        nexus_url: "http://127.0.0.1:17002".into(),
        ..GraphConfig::default()
    };
    let client = LiveNexusClient::new(cfg).expect("construct live client");
    client
        .ensure_schema(&schema::statements())
        .await
        .expect("schema bootstrap");

    let session_key = format!("cortex-graph-test-session-{}", ulid::Ulid::new());
    let turn_key = format!("cortex-graph-test-turn-{}", ulid::Ulid::new());
    let patch = GraphPatch {
        nodes: vec![node("Session", &session_key), node("Turn", &turn_key)],
        edges: vec![edge("HAS_TURN", "Session", &session_key, "Turn", &turn_key)],
    };
    let cypher_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("cypher");
    let templates = load_from_dir(&cypher_dir).expect("load shipped cypher dir");
    templates
        .ensure_required(REQUIRED_TEMPLATES)
        .expect("shipped cypher dir must satisfy REQUIRED_TEMPLATES");
    let stats = client
        .run_write_tx(&patch, &templates)
        .await
        .expect("write_tx");
    assert_eq!(stats.nodes_upserted, 2);
    assert_eq!(stats.edges_upserted, 1);

    // Cleanup so the test is rerunnable.
    for key in [&session_key, &turn_key] {
        let _ = client
            .sdk()
            .execute_cypher(
                "MATCH (n { id: $id }) DETACH DELETE n",
                Some({
                    let mut p = HashMap::new();
                    p.insert("id".to_string(), SdkValue::String(key.clone()));
                    p
                }),
            )
            .await;
    }
}
