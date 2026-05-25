//! ADR-012 §3.3 — graph worker write-back stamps
//! `event_identity.nexus_id` after a successful patch flush.

use async_trait::async_trait;
use cortex_core::events::Kind;
use cortex_storage::{Backend, IdentityIndex as _, MetadataStore, SqliteIdentityIndex};
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::embedder::EnrichedEvent;
use cortex_workers::graph::metrics::Metrics as GraphMetrics;
use cortex_workers::graph::nexus_client::GraphClientError;
use cortex_workers::graph::patch::EdgeDeleteFilter;
use cortex_workers::graph::{
    ConsumedMessage, GraphConfig, GraphPatch, GraphWriteReport, GraphWriter, MemorySynapConsumer,
    MemorySynapPublisher, Worker,
};
use serde_json::json;
use std::sync::Arc;

/// Minimal in-test `GraphWriter` double — `write_patches` always
/// succeeds with an empty report. The graph worker's identity
/// write-back path only cares that the patch flush returned `Ok`;
/// per-batch metrics get exercised by the dedicated `graph_writer`
/// suite and are irrelevant here.
#[derive(Default)]
struct FakeGraphWriter;

#[async_trait]
impl GraphWriter for FakeGraphWriter {
    async fn write_batch(
        &self,
        _events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        Ok(GraphWriteReport::default())
    }

    async fn write_patches(
        &self,
        _patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        Ok(GraphWriteReport::default())
    }

    async fn delete_edges_by_filter(
        &self,
        _filter: EdgeDeleteFilter,
    ) -> Result<u64, GraphClientError> {
        Ok(0)
    }
}

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
async fn graph_worker_stamps_event_identity_nexus_id_after_success() {
    let store = MetadataStore::open_in_memory().expect("metadata opens");
    let metadata = Arc::new(std::sync::Mutex::new(store));

    let writer = Arc::new(FakeGraphWriter);
    let config = GraphConfig::default();
    let metrics = Arc::new(GraphMetrics::default());
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::default());

    let worker = Worker::new(config, writer, consumer.clone(), publisher.clone(), metrics)
        .with_metadata(metadata.clone());

    let event = make_enriched("01HXGRAPH00000000000000001");
    enqueue_event(&consumer, &event, 1);

    let handled = worker.run_once().await.expect("run_once ok");
    assert_eq!(handled, 1, "exactly one enriched event drained");

    let guard = metadata.lock().expect("metadata mutex");
    let idx = SqliteIdentityIndex::new(guard.conn());
    let row = idx
        .lookup("01HXGRAPH00000000000000001")
        .expect("lookup ok")
        .expect("identity row present after graph write");
    assert_eq!(
        row.nexus_id.as_deref(),
        Some("01HXGRAPH00000000000000001"),
        "nexus_id native value must equal event_id per spec-07 §Node keys"
    );
    let by_native = idx
        .lookup_by_native(Backend::Nexus, "01HXGRAPH00000000000000001")
        .expect("reverse lookup ok")
        .expect("identity row found by nexus_id");
    assert_eq!(by_native.event_id, "01HXGRAPH00000000000000001");
    assert!(row.vec_id.is_none());
    assert!(row.meili_id.is_none());
    assert!(row.archive_partition.is_none());
}

#[tokio::test]
async fn graph_worker_skips_identity_writeback_when_metadata_absent() {
    let writer = Arc::new(FakeGraphWriter);
    let config = GraphConfig::default();
    let metrics = Arc::new(GraphMetrics::default());
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::default());

    let worker = Worker::new(config, writer, consumer.clone(), publisher.clone(), metrics);

    let event = make_enriched("01HXGRAPH00000000000000002");
    enqueue_event(&consumer, &event, 1);

    let handled = worker.run_once().await.expect("run_once ok");
    assert_eq!(handled, 1);
    assert_eq!(consumer.remaining(), 0, "worker must consume the message");
}
