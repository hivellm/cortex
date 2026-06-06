//! ADR-012 §3.1 — embedder worker write-back stamps
//! `event_identity.vec_id` after a successful embed batch.

use cortex_core::events::Kind;
use cortex_storage::{Backend, IdentityIndex as _, MetadataStore, SqliteIdentityIndex};
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::embedder::metrics::Metrics as EmbedderMetrics;
use cortex_workers::embedder::{
    Chunk, ChunkMetadata, ChunkSource, ConsumedMessage, EmbedderConfig, EnrichedEvent,
    MemorySynapConsumer, MemorySynapPublisher, MemoryVectorizerClient, VectorizerEmbedder, Worker,
    STREAM_ENRICHED,
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
async fn embedder_worker_stamps_event_identity_vec_id_after_success() {
    // Wire metadata store + the in-memory worker stack.
    let store = MetadataStore::open_in_memory().expect("metadata opens");
    let metadata = Arc::new(std::sync::Mutex::new(store));

    let vec_client = Arc::new(MemoryVectorizerClient::default());
    let embedder = Arc::new(VectorizerEmbedder::new(
        EmbedderConfig::default(),
        vec_client.clone(),
    ));
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::default());
    let metrics = Arc::new(EmbedderMetrics::default());

    let worker = Worker::new(
        EmbedderConfig::default(),
        embedder,
        consumer.clone(),
        publisher.clone(),
        metrics,
    )
    .with_metadata(metadata.clone());

    let event = make_enriched("01HXEMBED000000000000000001");
    enqueue_event(&consumer, &event, 1);

    let handled = worker.run_once().await.expect("run_once ok");
    assert_eq!(handled, 1, "exactly one enriched event drained");

    // ADR-012 — the worker's stamp_identity_after_success path
    // should have written one row to event_identity with the
    // representative vec_id (first chunk's server_id).
    let guard = metadata.lock().expect("metadata mutex");
    let idx = SqliteIdentityIndex::new(guard.conn());
    let row = idx
        .lookup("01HXEMBED000000000000000001")
        .expect("lookup ok")
        .expect("identity row present after embed");
    let vec_id = row.vec_id.clone().expect("vec_id stamped");
    assert!(
        !vec_id.is_empty(),
        "vec_id must carry the Vectorizer server id, got empty string"
    );
    // Reverse lookup by native id resolves back to the same event.
    let by_native = idx
        .lookup_by_native(Backend::Vectorizer, &vec_id)
        .expect("reverse lookup ok")
        .expect("identity row found by vec_id");
    assert_eq!(by_native.event_id, "01HXEMBED000000000000000001");
    // Sibling columns untouched — other workers stamp those.
    assert!(row.nexus_id.is_none());
    assert!(row.meili_id.is_none());
    assert!(row.archive_partition.is_none());
}

#[tokio::test]
async fn embedder_worker_skips_identity_writeback_when_metadata_absent() {
    // Worker without `with_metadata(...)` — write-back is a silent
    // no-op and the legacy embed path keeps working unchanged.
    let vec_client = Arc::new(MemoryVectorizerClient::default());
    let embedder = Arc::new(VectorizerEmbedder::new(
        EmbedderConfig::default(),
        vec_client.clone(),
    ));
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::default());
    let metrics = Arc::new(EmbedderMetrics::default());

    let worker = Worker::new(
        EmbedderConfig::default(),
        embedder,
        consumer.clone(),
        publisher.clone(),
        metrics,
    );

    let event = make_enriched("01HXEMBED000000000000000002");
    enqueue_event(&consumer, &event, 1);

    let handled = worker.run_once().await.expect("run_once ok");
    assert_eq!(handled, 1);
    // No metadata handle → no row written. The test asserts only
    // that the embed succeeds + the worker drains the message
    // (publish_success ran without panicking on the absent stamp).
    assert_eq!(
        consumer.remaining(),
        0,
        "worker must consume the enqueued message"
    );
    let _ = (
        Chunk {
            // silence unused-import: types are wired through embedder
            dedup_key: String::new(),
            parent_event_id: String::new(),
            parent_content_hash: String::new(),
            chunk_content_hash: String::new(),
            collection: String::new(),
            text: String::new(),
            metadata: ChunkMetadata {
                kind: Kind::ToolCall,
                topics: Vec::new(),
                severity: Severity::Info,
                repo: None,
                path: None,
                symbol: None,
                byte_range: None,
                language: None,
                source: ChunkSource::FallbackWindow,
                prompt_version: None,
                project_id: None,
                branch_id: None,
                lifecycle: None,
                valid_from_unix: None,
                valid_to_unix: None,
                superseded_at_unix: None,
            },
        },
        STREAM_ENRICHED,
    );
}
