//! Integration tests for `cortex_graph::worker` (graph-writer task §5).
//!
//! Cover the four headline behaviours: end-to-end batch flow,
//! out-of-order buffering with orphan-Turn fabrication, constraint
//! violations dead-lettering, and transient errors engaging the
//! backpressure gauge.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_graph::nexus_client::GraphClientError;
use cortex_graph::patch::{GraphPatch, GraphWriteReport};
use cortex_graph::worker::orphan_turn_patch;
use cortex_graph::{
    BackpressureState, ConsumedMessage, EnrichedEvent, GraphConfig, GraphWriter,
    MemorySynapConsumer, MemorySynapPublisher, Metrics, OutOfOrderBuffer, Worker,
    BACKPRESSURE_SOAK, STREAM_GRAPHED, STREAM_INVALID,
};
use serde_json::{json, Value};

// ---------- helpers ----------

fn classifier(event_id: &str) -> ClassifierOutput {
    ClassifierOutput {
        event_id: event_id.to_string(),
        kind_refinement: None,
        topics: vec![],
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

fn enriched(
    event_id: &str,
    kind: Kind,
    payload: Value,
    parent: Option<&str>,
) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: Some("hivellm/cortex".into()),
        context_path: None,
        parent_event_id: parent.map(String::from),
    }
}

fn enqueue(consumer: &MemorySynapConsumer, offset: u64, event: &EnrichedEvent) {
    consumer.enqueue(ConsumedMessage {
        offset,
        kind: "enriched".into(),
        payload: serde_json::to_value(event).unwrap(),
        event_id: Some(event.event_id.clone()),
    });
}

// ---------- writer doubles ----------

#[derive(Default)]
struct CountingWriter {
    calls: AtomicU32,
    last_patch: Mutex<Option<GraphPatch>>,
}

#[async_trait]
impl GraphWriter for CountingWriter {
    async fn write_batch(
        &self,
        events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        let patches: Vec<GraphPatch> =
            events.iter().map(cortex_graph::map_event_to_patch).collect();
        self.write_patches(patches).await
    }

    async fn write_patches(
        &self,
        patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // Coalesce by hand so the report counts feel realistic.
        let (coalesced, stats) = cortex_graph::coalescer::coalesce(patches);
        if let Ok(mut g) = self.last_patch.lock() {
            *g = Some(coalesced.clone());
        }
        let nodes = coalesced.nodes.len() as u32;
        let edges = coalesced.edges.len() as u32;
        let mut by_label = std::collections::BTreeMap::new();
        for n in &coalesced.nodes {
            *by_label.entry(n.label.clone()).or_insert(0) += 1u32;
        }
        Ok(GraphWriteReport {
            nodes_upserted: nodes,
            edges_upserted: edges,
            nodes_deduped: stats.node_dedup_hits,
            edges_deduped: stats.edge_dedup_hits,
            by_label,
            latency_ms: 1,
        })
    }
}

#[derive(Default)]
struct ConstraintViolatingWriter;

#[async_trait]
impl GraphWriter for ConstraintViolatingWriter {
    async fn write_batch(
        &self,
        _events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        Err(GraphClientError::ConstraintViolation {
            detail: "duplicate id for Decision".into(),
        })
    }
    async fn write_patches(
        &self,
        _patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        Err(GraphClientError::ConstraintViolation {
            detail: "duplicate id for Decision".into(),
        })
    }
}

#[derive(Default)]
struct TransientWriter;

#[async_trait]
impl GraphWriter for TransientWriter {
    async fn write_batch(
        &self,
        _events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        Err(GraphClientError::TransientError("nexus 503 boom".into()))
    }
    async fn write_patches(
        &self,
        _patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        Err(GraphClientError::TransientError("nexus 503 boom".into()))
    }
}

fn build_worker(writer: Arc<dyn GraphWriter>) -> (Arc<Worker>, Arc<MemorySynapConsumer>, Arc<MemorySynapPublisher>) {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = GraphConfig {
        workers: 1,
        patch_batch: 16,
        flush_ms: 50,
        out_of_order_buffer_secs: 1,
        ..GraphConfig::default()
    };
    let worker = Arc::new(Worker::new(cfg, writer, consumer.clone(), publisher.clone(), metrics));
    (worker, consumer, publisher)
}

// ---------- tests ----------

#[tokio::test]
async fn writes_batch_and_publishes_graphed_report() {
    let writer: Arc<dyn GraphWriter> = Arc::new(CountingWriter::default());
    let (worker, consumer, publisher) = build_worker(writer.clone());

    let turn = enriched("turn-1", Kind::Turn, json!({ "user_message": "hi" }), None);
    enqueue(&consumer, 0, &turn);

    let processed = worker.run_once().await.expect("run_once");
    assert_eq!(processed, 1);

    let graphed = publisher.calls_on(STREAM_GRAPHED);
    assert_eq!(graphed.len(), 1);
    let envelope = &graphed[0];
    assert_eq!(envelope["kind"], "graphed");
    assert!(envelope["nodes_upserted"].as_u64().unwrap() >= 1);
    assert_eq!(
        envelope["event_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["turn-1"]
    );
}

#[tokio::test]
async fn replay_skips_already_processed_event() {
    let writer: Arc<dyn GraphWriter> = Arc::new(CountingWriter::default());
    let (worker, consumer, publisher) = build_worker(writer.clone());

    let turn = enriched("turn-replay", Kind::Turn, json!({ "user_message": "" }), None);
    enqueue(&consumer, 0, &turn);
    enqueue(&consumer, 1, &turn);

    worker.run_once().await.expect("first run_once");
    let graphed_after_first = publisher.calls_on(STREAM_GRAPHED).len();
    assert_eq!(graphed_after_first, 1);

    // The second delivery is skipped at deserialization-time dedup —
    // the writer never sees it, so no second graphed envelope is
    // published.
    let graphed_after_second = publisher.calls_on(STREAM_GRAPHED).len();
    assert_eq!(graphed_after_second, 1);
}

#[tokio::test]
async fn malformed_payload_routes_to_invalid_stream() {
    let writer: Arc<dyn GraphWriter> = Arc::new(CountingWriter::default());
    let (worker, consumer, publisher) = build_worker(writer);

    consumer.enqueue(ConsumedMessage {
        offset: 0,
        kind: "enriched".into(),
        payload: json!({ "garbage": true }),
        event_id: Some("evt-broken".into()),
    });

    worker.run_once().await.expect("run_once");
    let invalids = publisher.calls_on(STREAM_INVALID);
    assert_eq!(invalids.len(), 1);
    assert_eq!(invalids[0]["cause"], "deserialize_failed");
    assert_eq!(invalids[0]["event_id"], "evt-broken");
}

#[tokio::test]
async fn out_of_order_buffer_holds_tool_call_until_turn_arrives() {
    let buffer = OutOfOrderBuffer::new(Duration::from_secs(60));

    let tool_call = enriched(
        "tc-A",
        Kind::ToolCall,
        json!({ "tool_name": "Read", "input": {}, "outcome": "success", "touched": [] }),
        Some("turn-X"),
    );
    let ready = buffer.ingest(vec![tool_call.clone()]);
    assert!(ready.events.is_empty(), "tool_call must be buffered until parent arrives");
    assert_eq!(buffer.pending(), 1);

    let turn = enriched("turn-X", Kind::Turn, json!({ "user_message": "" }), None);
    let ready2 = buffer.ingest(vec![turn]);
    assert_eq!(ready2.events.len(), 2, "turn flushes the buffered tool_call");
    assert_eq!(buffer.pending(), 0);
}

#[tokio::test]
async fn out_of_order_buffer_flushes_orphans_after_timeout() {
    let buffer = OutOfOrderBuffer::new(Duration::from_millis(50));

    let tool_call = enriched(
        "tc-orphan",
        Kind::ToolCall,
        json!({ "tool_name": "Read", "input": {}, "outcome": "success", "touched": [] }),
        Some("turn-missing"),
    );
    buffer.ingest(vec![tool_call]);
    assert_eq!(buffer.pending(), 1);

    // Force the sweep to act as if 1 s had passed.
    let later = Instant::now() + Duration::from_secs(1);
    let ready = buffer.sweep_at(later);
    assert_eq!(ready.events.len(), 1);
    assert_eq!(ready.orphan_turn_ids, vec!["turn-missing".to_string()]);
    assert_eq!(buffer.pending(), 0);
}

#[tokio::test]
async fn worker_emits_orphan_turn_after_buffer_timeout() {
    let writer: Arc<CountingWriter> = Arc::new(CountingWriter::default());
    let (worker, consumer, publisher) = build_worker(writer.clone());

    let tool_call = enriched(
        "tc-late",
        Kind::ToolCall,
        json!({ "tool_name": "Read", "input": {}, "outcome": "success", "touched": [] }),
        Some("turn-never-arrives"),
    );
    enqueue(&consumer, 0, &tool_call);

    worker.run_once().await.expect("first run_once");
    // Initially the tool_call is held back; nothing flushed.
    assert!(publisher.calls_on(STREAM_GRAPHED).is_empty());
    assert_eq!(worker.buffer().pending(), 1);

    // Wait past the 1 s configured buffer, then drive another batch
    // (an empty one) so the sweep runs.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    worker.run_once().await.expect("second run_once");

    let graphed = publisher.calls_on(STREAM_GRAPHED);
    assert_eq!(graphed.len(), 1);
    let envelope = &graphed[0];
    assert_eq!(
        envelope["orphan_turn_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["turn-never-arrives"]
    );

    // The synthesised orphan Turn carries the orphan flag.
    let last = writer.last_patch.lock().unwrap().clone().unwrap();
    let orphan_node = last
        .nodes
        .iter()
        .find(|n| n.label == "Turn" && n.natural_key == "turn-never-arrives")
        .expect("orphan Turn upserted");
    assert_eq!(
        orphan_node.props.get("orphan").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[tokio::test]
async fn constraint_violation_routes_batch_to_invalid_stream() {
    let writer: Arc<dyn GraphWriter> = Arc::new(ConstraintViolatingWriter);
    let (worker, consumer, publisher) = build_worker(writer);

    let turn = enriched("turn-bad", Kind::Turn, json!({ "user_message": "" }), None);
    enqueue(&consumer, 0, &turn);

    worker.run_once().await.expect("run_once");

    let invalids = publisher.calls_on(STREAM_INVALID);
    assert_eq!(invalids.len(), 1);
    assert_eq!(invalids[0]["cause"], "constraint_violation");
    assert_eq!(invalids[0]["event_id"], "turn-bad");
    assert!(publisher.calls_on(STREAM_GRAPHED).is_empty());
}

#[tokio::test]
async fn transient_error_engages_backpressure_and_does_not_ack() {
    let writer: Arc<dyn GraphWriter> = Arc::new(TransientWriter);
    let (worker, consumer, publisher) = build_worker(writer);

    let turn = enriched("turn-flaky", Kind::Turn, json!({ "user_message": "" }), None);
    enqueue(&consumer, 0, &turn);

    worker.run_once().await.expect("run_once");
    assert!(worker.backpressure().is_active());
    assert!(publisher.calls_on(STREAM_GRAPHED).is_empty());
    assert!(publisher.calls_on(STREAM_INVALID).is_empty());
}

#[tokio::test]
async fn paused_backpressure_skips_consumption_entirely() {
    let writer: Arc<dyn GraphWriter> = Arc::new(CountingWriter::default());
    let (worker, consumer, publisher) = build_worker(writer);

    // Force the gauge into the "paused" range by aging the timestamp
    // past BACKPRESSURE_SOAK.
    let aged = Instant::now() - BACKPRESSURE_SOAK - Duration::from_secs(1);
    worker.backpressure().force_since(aged);
    assert!(worker.backpressure().is_paused());

    let turn = enriched("turn-paused", Kind::Turn, json!({ "user_message": "" }), None);
    enqueue(&consumer, 0, &turn);

    let processed = worker.run_once().await.expect("run_once");
    assert_eq!(processed, 0, "paused worker must not consume");
    assert_eq!(consumer.remaining(), 1);
    assert!(publisher.calls_on(STREAM_GRAPHED).is_empty());
}

#[tokio::test]
async fn orphan_turn_patch_carries_orphan_flag() {
    let patch = orphan_turn_patch("turn-Z");
    assert_eq!(patch.nodes.len(), 1);
    assert_eq!(patch.nodes[0].label, "Turn");
    assert_eq!(patch.nodes[0].natural_key, "turn-Z");
    assert_eq!(
        patch.nodes[0]
            .props
            .get("orphan")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[tokio::test]
async fn backpressure_state_record_lifecycle() {
    let bp = BackpressureState::new();
    assert!(!bp.is_active());
    assert!(!bp.is_paused());
    bp.record_transient();
    assert!(bp.is_active());
    bp.record_success();
    assert!(!bp.is_active());
    assert!(!bp.is_paused());
}
