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
    assert_eq!(report.by_index.get("cortex-code").copied(), Some(2));
    assert_eq!(report.by_index.get("cortex-decisions").copied(), Some(1));

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
        vec!["cortex-code".to_string(), "cortex-decisions".to_string()]
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
