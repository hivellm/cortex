//! Integration tests for the Meili-backed `FulltextIndexer`.

use std::sync::Arc;

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_fulltext::meili_client::MemoryCall;
use cortex_fulltext::{
    EnrichedEvent, FulltextConfig, FulltextIndexer, MeiliFulltextIndexer, MemoryMeiliClient,
    Metrics,
};
use serde_json::json;

fn classifier(event_id: &str) -> ClassifierOutput {
    ClassifierOutput {
        event_id: event_id.to_string(),
        kind_refinement: None,
        topics: vec!["topic".into()],
        severity: Severity::Info,
        pii_risk: PiiRisk::Low,
        redaction_suggestions: vec![],
        summary: None,
        entities: Vec::new(),
        relations: Vec::new(),
        source: ClassifierSource::StaticFallback,
        prompt_version: "v1".into(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}

fn event(event_id: &str, kind: Kind, payload: serde_json::Value) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: Some("Vectorizer".into()),
        context_path: None,
        parent_event_id: None,
        session_id: None,
    }
}

fn build_indexer() -> (Arc<MemoryMeiliClient>, Arc<MeiliFulltextIndexer>, Arc<Metrics>) {
    let client = Arc::new(MemoryMeiliClient::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        upsert_batch: 4,
        ..FulltextConfig::default()
    };
    let indexer = Arc::new(MeiliFulltextIndexer::new(
        cfg,
        client.clone(),
        metrics.clone(),
    ));
    (client, indexer, metrics)
}

#[tokio::test]
async fn index_batch_groups_events_per_index() {
    let (client, indexer, _metrics) = build_indexer();
    let events = vec![
        event(
            "tc-1",
            Kind::ToolCall,
            json!({
                "tool_name": "Edit",
                "input": { "command": "x" },
                "outcome": "success",
                "touched": []
            }),
        ),
        event(
            "tc-2",
            Kind::ToolCall,
            json!({
                "tool_name": "Read",
                "input": { "command": "y" },
                "outcome": "success",
                "touched": []
            }),
        ),
        event(
            "dec-1",
            Kind::Decision,
            json!({
                "decision_id": "DEC-1",
                "title": "x",
                "status": "accepted",
                "body": "details"
            }),
        ),
    ];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 3);
    // Per-repo isolation: events with `context_repo = "Vectorizer"` route to
    // `cortex-vectorizer-{family}` instead of the legacy shared `cortex-{family}`.
    assert_eq!(report.by_index.get("cortex-vectorizer-code").copied(), Some(2));
    assert_eq!(report.by_index.get("cortex-vectorizer-decisions").copied(), Some(1));

    let calls = client.calls_snapshot();
    let mut indexes_seen: Vec<String> = calls
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::UpsertDocuments { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    indexes_seen.sort();
    assert_eq!(
        indexes_seen,
        vec![
            "cortex-vectorizer-code".to_string(),
            "cortex-vectorizer-decisions".to_string()
        ]
    );
}

#[tokio::test]
async fn empty_payload_event_is_counted_as_skipped() {
    let (_client, indexer, metrics) = build_indexer();
    let events = vec![event(
        "evt-empty",
        Kind::Artifact,
        json!({ "artifact_type": "file" }),
    )];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 0);
    assert_eq!(report.documents_skipped, 1);
    assert_eq!(metrics.skipped_empty.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[tokio::test]
async fn oversize_event_increments_truncated_counter_and_flag() {
    let (_client, indexer, metrics) = build_indexer();
    let raw = "z".repeat(20 * 1024);
    let events = vec![event(
        "evt-big",
        Kind::Turn,
        json!({ "user_message": raw }),
    )];
    // Small max_body_bytes to force truncation regardless of payload.
    let small_cfg = FulltextConfig {
        upsert_batch: 4,
        max_body_bytes: 1024,
        ..FulltextConfig::default()
    };
    let mc = Arc::new(MemoryMeiliClient::new());
    let metrics2 = Arc::new(Metrics::new());
    let indexer2 = MeiliFulltextIndexer::new(small_cfg, mc.clone(), metrics2.clone());
    let report = indexer2.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 1);
    assert_eq!(report.documents_truncated, 1);
    assert_eq!(metrics2.truncated.load(std::sync::atomic::Ordering::Relaxed), 1);
    let _ = (indexer, metrics);
}

/// Spec-08 §Routing matrix: a mixed batch should fan out across every
/// destination family (`code`, `docs`, `decisions`, `governance`,
/// `turns`, `misc`). The `routed_total` counter must mirror the
/// `IndexReport.by_index` numbers exactly.
#[tokio::test]
async fn routing_matrix_distributes_mixed_batch_across_families() {
    let (_client, indexer, metrics) = build_indexer();

    // ToolCall → cortex-vectorizer-code.
    let mut tool_call = event(
        "tc-1",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": { "command": "edit foo" },
            "outcome": "success",
            "touched": []
        }),
    );
    tool_call.classifier.topics = vec!["code".into()];

    // AgentCall → cortex-vectorizer-turns (per spec-08).
    let mut agent_call = event(
        "ag-1",
        Kind::AgentCall,
        json!({
            "agent_type": "researcher",
            "description": "research",
            "outcome": "success",
            "duration_ms": 5
        }),
    );
    agent_call.classifier.topics = vec!["agent".into()];

    // Turn → cortex-vectorizer-turns.
    let turn = event(
        "tu-1",
        Kind::Turn,
        json!({
            "user_message": "hi",
            "assistant_message": "hello"
        }),
    );

    // Decision → cortex-vectorizer-decisions.
    let decision = event(
        "dec-1",
        Kind::Decision,
        json!({
            "decision_id": "DEC-1",
            "title": "Adopt X",
            "status": "accepted",
            "body": "rationale"
        }),
    );

    // LawViolation → cortex-vectorizer-governance.
    let law = event(
        "lv-1",
        Kind::LawViolation,
        json!({
            "violation_id": "VIO-1",
            "law_id": "LAW-1",
            "severity": "critical",
            "tier": 1,
            "message": "broke a rule",
            "evidence": null
        }),
    );

    // Artifact (.rs path) → cortex-vectorizer-code.
    let mut code_artifact = event(
        "art-rs",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "src/lib.rs",
            "body": "fn main() {}"
        }),
    );
    code_artifact.context_path = Some("src/lib.rs".into());

    // Artifact (.md path) → cortex-vectorizer-docs.
    let mut doc_artifact = event(
        "art-md",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "docs/spec-08.md",
            "body": "# Spec"
        }),
    );
    doc_artifact.context_path = Some("docs/spec-08.md".into());

    // Artifact with unknown ext + no topics → cortex-vectorizer-misc.
    let mut misc_artifact = event(
        "art-bin",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "tools/blob.bin",
            "body": "raw"
        }),
    );
    misc_artifact.context_path = Some("tools/blob.bin".into());
    misc_artifact.classifier.topics = vec![];

    let events = vec![
        tool_call,
        agent_call,
        turn,
        decision,
        law,
        code_artifact,
        doc_artifact,
        misc_artifact,
    ];

    let report = indexer.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 8);

    // Every destination from the spec-08 matrix should be populated.
    let want = [
        ("cortex-vectorizer-code", 2),         // tool_call + .rs artifact
        ("cortex-vectorizer-turns", 2),        // turn + agent_call
        ("cortex-vectorizer-decisions", 1),
        ("cortex-vectorizer-governance", 1),
        ("cortex-vectorizer-docs", 1),         // .md artifact
        ("cortex-vectorizer-misc", 1),         // unknown-ext artifact
    ];
    for (idx, expected) in want {
        assert_eq!(
            report.by_index.get(idx).copied(),
            Some(expected),
            "index {idx} count drift: by_index={:?}",
            report.by_index
        );
    }

    // The routed_total counter must mirror by_index exactly so the
    // operator dashboard and the per-batch report cannot diverge.
    let routed = metrics.routed_snapshot();
    for (idx, expected) in want {
        assert_eq!(
            routed.get(idx).copied(),
            Some(expected as u64),
            "routed_total mismatch for {idx}: snapshot={:?}",
            routed
        );
    }
}

#[tokio::test]
async fn batches_chunk_by_upsert_batch_size() {
    let (client, indexer, _metrics) = build_indexer();
    // 9 ToolCall events with upsert_batch=4 ⇒ 3 chunks for the
    // `cortex-code` index (4 + 4 + 1).
    let mut events = Vec::with_capacity(9);
    for i in 0..9 {
        events.push(event(
            &format!("tc-{i}"),
            Kind::ToolCall,
            json!({
                "tool_name": "Edit",
                "input": { "command": format!("cmd-{i}") },
                "outcome": "success",
                "touched": []
            }),
        ));
    }
    indexer.index_batch(&events).await.expect("index_batch");
    let upserts: Vec<usize> = client
        .calls_snapshot()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::UpsertDocuments { docs, .. } => Some(docs.len()),
            _ => None,
        })
        .collect();
    assert_eq!(upserts, vec![4, 4, 1]);
}
