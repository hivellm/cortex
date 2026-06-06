//! ADR-012 §3.2 — fulltext worker write-back stamps
//! `event_identity.meili_id` after a successful index batch.

use cortex_core::events::Kind;
use cortex_storage::{Backend, IdentityIndex as _, MetadataStore, SqliteIdentityIndex};
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::embedder::EnrichedEvent;
use cortex_workers::fulltext::{
    ConsumedMessage, FulltextConfig, MeiliFulltextIndexer, MemoryMeiliClient, MemorySynapConsumer,
    MemorySynapPublisher, Metrics as FulltextMetrics, Worker,
};
use serde_json::json;
use std::sync::Arc;

fn make_enriched(event_id: &str) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind: Kind::ToolCall,
        content_hash: format!("parent-{event_id}"),
        redacted_payload: json!({
            "tool_name": "Bash",
            "input": { "command": "echo hi" },
            "outcome": "success"
        }),
        classifier: ClassifierOutput {
            event_id: event_id.to_string(),
            kind_refinement: None,
            topics: vec!["topic".into()],
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: Some("cortex".into()),
        context_path: None,
        parent_event_id: None,
        session_id: None,
        occurred_at_ms: 0,
    }
}

fn enqueue_event(consumer: &MemorySynapConsumer, event: &EnrichedEvent, offset: u64) {
    consumer.enqueue(ConsumedMessage {
        offset,
        kind: "enriched".to_string(),
        payload: serde_json::to_value(event).expect("event serialises"),
        event_id: Some(event.event_id.clone()),
    });
}

#[tokio::test]
async fn fulltext_worker_stamps_event_identity_meili_id_after_success() {
    let store = MetadataStore::open_in_memory().expect("metadata opens");
    let metadata = Arc::new(std::sync::Mutex::new(store));

    let meili = Arc::new(MemoryMeiliClient::default());
    let config = FulltextConfig::default();
    let metrics = Arc::new(FulltextMetrics::default());
    let indexer = Arc::new(MeiliFulltextIndexer::new(
        config.clone(),
        meili.clone(),
        metrics.clone(),
    ));
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::default());

    let worker = Worker::new(
        config,
        indexer,
        consumer.clone(),
        publisher.clone(),
        metrics,
    )
    .with_metadata(metadata.clone());

    let event = make_enriched("01HXFULL000000000000000001");
    enqueue_event(&consumer, &event, 1);

    let handled = worker.run_once().await.expect("run_once ok");
    assert_eq!(handled, 1, "exactly one enriched event drained");

    let guard = metadata.lock().expect("metadata mutex");
    let idx = SqliteIdentityIndex::new(guard.conn());
    let row = idx
        .lookup("01HXFULL000000000000000001")
        .expect("lookup ok")
        .expect("identity row present after index");
    assert_eq!(
        row.meili_id.as_deref(),
        Some("01HXFULL000000000000000001"),
        "meili_id native value must equal event_id per Document::id contract"
    );
    let by_native = idx
        .lookup_by_native(Backend::Meili, "01HXFULL000000000000000001")
        .expect("reverse lookup ok")
        .expect("identity row found by meili_id");
    assert_eq!(by_native.event_id, "01HXFULL000000000000000001");
    // Sibling columns untouched.
    assert!(row.vec_id.is_none());
    assert!(row.nexus_id.is_none());
    assert!(row.archive_partition.is_none());
}

#[tokio::test]
async fn fulltext_worker_skips_identity_writeback_when_metadata_absent() {
    let meili = Arc::new(MemoryMeiliClient::default());
    let config = FulltextConfig::default();
    let metrics = Arc::new(FulltextMetrics::default());
    let indexer = Arc::new(MeiliFulltextIndexer::new(
        config.clone(),
        meili.clone(),
        metrics.clone(),
    ));
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::default());

    let worker = Worker::new(
        config,
        indexer,
        consumer.clone(),
        publisher.clone(),
        metrics,
    );

    let event = make_enriched("01HXFULL000000000000000002");
    enqueue_event(&consumer, &event, 1);

    let handled = worker.run_once().await.expect("run_once ok");
    assert_eq!(handled, 1);
    assert_eq!(consumer.remaining(), 0, "worker must consume the message");
}
