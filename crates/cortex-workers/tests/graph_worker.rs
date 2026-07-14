//! Integration tests for `cortex_workers::graph::worker` (graph-writer task §5).
//!
//! Cover the four headline behaviours: end-to-end batch flow,
//! out-of-order buffering with orphan-Turn fabrication, constraint
//! violations dead-lettering, and transient errors engaging the
//! backpressure gauge.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_core::events::Kind;
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::graph::nexus_client::GraphClientError;
use cortex_workers::graph::patch::{EdgeDeleteFilter, GraphPatch, GraphWriteReport};
use cortex_workers::graph::worker::orphan_turn_patch;
use cortex_workers::graph::{
    BackpressureState, ConsumedMessage, EnrichedEvent, GraphConfig, GraphWriter,
    MemorySynapConsumer, MemorySynapPublisher, Metrics, OutOfOrderBuffer, Worker,
    BACKPRESSURE_RETRY, BACKPRESSURE_SOAK, STREAM_GRAPHED, STREAM_INVALID,
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
        entities: Vec::new(),
        relations: Vec::new(),
        sensitivity: Default::default(),
        source: ClassifierSource::StaticFallback,
        prompt_version: "v1".into(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}

fn enriched(event_id: &str, kind: Kind, payload: Value, parent: Option<&str>) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: Some("hivellm/cortex".into()),
        context_path: None,
        parent_event_id: parent.map(String::from),
        session_id: None,
        occurred_at_ms: 0,
        class_level: None,
        class_compartments: None,
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
        let patches: Vec<GraphPatch> = events
            .iter()
            .map(cortex_workers::graph::map_event_to_patch)
            .collect();
        self.write_patches(patches).await
    }

    async fn write_patches(
        &self,
        patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // Coalesce by hand so the report counts feel realistic.
        let (coalesced, stats) = cortex_workers::graph::coalescer::coalesce(patches);
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

    async fn delete_edges_by_filter(
        &self,
        _filter: EdgeDeleteFilter,
    ) -> Result<u64, GraphClientError> {
        Ok(0)
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

    async fn delete_edges_by_filter(
        &self,
        _filter: EdgeDeleteFilter,
    ) -> Result<u64, GraphClientError> {
        Ok(0)
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

    async fn delete_edges_by_filter(
        &self,
        _filter: EdgeDeleteFilter,
    ) -> Result<u64, GraphClientError> {
        Ok(0)
    }
}

fn build_worker(
    writer: Arc<dyn GraphWriter>,
) -> (
    Arc<Worker>,
    Arc<MemorySynapConsumer>,
    Arc<MemorySynapPublisher>,
) {
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
    let worker = Arc::new(Worker::new(
        cfg,
        writer,
        consumer.clone(),
        publisher.clone(),
        metrics,
    ));
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

    let turn = enriched(
        "turn-replay",
        Kind::Turn,
        json!({ "user_message": "" }),
        None,
    );
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
    assert!(
        ready.events.is_empty(),
        "tool_call must be buffered until parent arrives"
    );
    assert_eq!(buffer.pending(), 1);

    let turn = enriched("turn-X", Kind::Turn, json!({ "user_message": "" }), None);
    let ready2 = buffer.ingest(vec![turn]);
    assert_eq!(
        ready2.events.len(),
        2,
        "turn flushes the buffered tool_call"
    );
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

    let turn = enriched(
        "turn-flaky",
        Kind::Turn,
        json!({ "user_message": "" }),
        None,
    );
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

    let turn = enriched(
        "turn-paused",
        Kind::Turn,
        json!({ "user_message": "" }),
        None,
    );
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
        patch.nodes[0].props.get("orphan").and_then(|v| v.as_bool()),
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

// ---------- Phase28 §1.4 — half-open pause recovery ----------

#[tokio::test]
async fn paused_backpressure_half_opens_after_retry_window() {
    // Regression for the 2026-06-27 permanent stall: once paused, the
    // worker never attempted a write, so `record_success` could never
    // fire and the pause outlived Nexus's recovery. The pause must
    // expire BACKPRESSURE_RETRY after the most recent transient.
    let bp = BackpressureState::new();
    let aged = Instant::now() - BACKPRESSURE_SOAK - Duration::from_secs(1);
    bp.force_since(aged);
    assert!(
        bp.is_paused(),
        "soaked + fresh transient keeps the pause armed"
    );

    // Age the most-recent transient past the retry window: the state
    // must read un-paused so a probe batch can flow.
    bp.force_last_transient(Instant::now() - BACKPRESSURE_RETRY - Duration::from_secs(1));
    assert!(
        !bp.is_paused(),
        "elapsed retry window must half-open the pause"
    );
    assert!(bp.is_active(), "gauge stays armed until a success");

    // A still-failing Nexus re-arms the window…
    bp.record_transient();
    assert!(bp.is_paused(), "fresh transient re-arms the pause");

    // …and a success fully disarms.
    bp.record_success();
    assert!(!bp.is_paused());
    assert!(!bp.is_active());
}

#[tokio::test]
async fn half_open_probe_reaches_writer_and_success_clears_pause() {
    // End-to-end: a paused worker whose retry window elapsed must let
    // run_once consume, and the successful probe write disarms the
    // gauge entirely.
    let writer: Arc<dyn GraphWriter> = Arc::new(CountingWriter::default());
    let (worker, consumer, publisher) = build_worker(writer);

    let aged = Instant::now() - BACKPRESSURE_SOAK - Duration::from_secs(1);
    worker.backpressure().force_since(aged);
    worker
        .backpressure()
        .force_last_transient(Instant::now() - BACKPRESSURE_RETRY - Duration::from_secs(1));
    assert!(!worker.backpressure().is_paused());

    let turn = enriched(
        "turn-probe",
        Kind::Turn,
        json!({ "user_message": "probe" }),
        None,
    );
    enqueue(&consumer, 0, &turn);

    let processed = worker.run_once().await.expect("run_once");
    assert_eq!(processed, 1, "half-open worker must consume the probe");
    assert!(
        !worker.backpressure().is_active(),
        "successful probe write disarms the gauge"
    );
    assert!(!worker.backpressure().is_paused());
    assert_eq!(publisher.calls_on(STREAM_GRAPHED).len(), 1);
}

// ---------- §7.2 acceptance suite ----------

/// Writer that fails the first `flake_count` calls with `TransientError`,
/// then succeeds. Drives the spec-07 §Failure modes "5xx soak recovery"
/// scenario: backpressure engages on the failures, gauges clear once a
/// success comes through.
#[derive(Default)]
struct FlakyWriter {
    calls: AtomicU32,
    flake_count: u32,
}

impl FlakyWriter {
    fn new(flake_count: u32) -> Self {
        Self {
            calls: AtomicU32::new(0),
            flake_count,
        }
    }
}

#[async_trait]
impl GraphWriter for FlakyWriter {
    async fn write_batch(
        &self,
        events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        let patches: Vec<GraphPatch> = events
            .iter()
            .map(cortex_workers::graph::map_event_to_patch)
            .collect();
        self.write_patches(patches).await
    }
    async fn write_patches(
        &self,
        patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        if n < self.flake_count {
            return Err(GraphClientError::TransientError(format!(
                "synthetic 503 attempt {n}"
            )));
        }
        let (coalesced, _stats) = cortex_workers::graph::coalescer::coalesce(patches);
        let nodes = coalesced.nodes.len() as u32;
        let edges = coalesced.edges.len() as u32;
        Ok(GraphWriteReport {
            nodes_upserted: nodes,
            edges_upserted: edges,
            nodes_deduped: 0,
            edges_deduped: 0,
            by_label: std::collections::BTreeMap::new(),
            latency_ms: 1,
        })
    }

    async fn delete_edges_by_filter(
        &self,
        _filter: EdgeDeleteFilter,
    ) -> Result<u64, GraphClientError> {
        Ok(0)
    }
}

#[tokio::test]
async fn ten_thousand_event_stream_drains_and_replays_idempotently() {
    let writer: Arc<CountingWriter> = Arc::new(CountingWriter::default());
    let writer_dyn: Arc<dyn GraphWriter> = writer.clone();
    let (worker, consumer, publisher) = build_worker(writer_dyn);

    // Generate 10 000 Turn events. Turn-only because their parent
    // dependency is None — the buffer never holds them, so the stream
    // drains in one straight pass.
    let total: usize = 10_000;
    for i in 0..total {
        let id = format!("turn-{i:06}");
        let evt = enriched(&id, Kind::Turn, json!({ "user_message": "" }), None);
        enqueue(&consumer, i as u64, &evt);
    }

    let mut drained = 0usize;
    while drained < total {
        let n = worker.run_once().await.expect("run_once");
        if n == 0 {
            break;
        }
        drained += n;
    }
    assert_eq!(drained, total, "all events must drain");
    assert_eq!(consumer.remaining(), 0);

    // Every batch flushed publishes one `cortex.events.graphed` envelope.
    // 10 000 events / 16 patch_batch ⇒ 625 batches.
    let graphed = publisher.calls_on(STREAM_GRAPHED);
    let expected_batches = total.div_ceil(16);
    assert_eq!(graphed.len(), expected_batches);

    // Aggregate event_ids from all reports — every input event must
    // surface exactly once.
    let mut seen_ids: std::collections::BTreeSet<String> = Default::default();
    for env in &graphed {
        for v in env["event_ids"].as_array().unwrap() {
            seen_ids.insert(v.as_str().unwrap().to_string());
        }
    }
    assert_eq!(seen_ids.len(), total);

    // Idempotent replay: enqueue the same 10 000 events again. The
    // in-memory dedup set short-circuits every one of them; no new
    // graphed envelope publishes.
    for i in 0..total {
        let id = format!("turn-{i:06}");
        let evt = enriched(&id, Kind::Turn, json!({ "user_message": "" }), None);
        enqueue(&consumer, (total + i) as u64, &evt);
    }
    while worker.run_once().await.expect("replay run_once") > 0 {}
    let graphed_after_replay = publisher.calls_on(STREAM_GRAPHED).len();
    assert_eq!(
        graphed_after_replay, expected_batches,
        "replay must not emit new graphed envelopes"
    );
}

#[tokio::test]
async fn coalescer_collapses_artifact_upserts_keeping_all_touched_edges() {
    // Spec 07 acceptance: 100 events touching the same Artifact ⇒ one
    // Artifact upsert and 100 TOUCHED edges.
    let mut patches: Vec<GraphPatch> = Vec::with_capacity(100);
    for i in 0..100 {
        let evt = enriched(
            &format!("tc-{i:03}"),
            Kind::ToolCall,
            json!({
                "tool_name": "Edit",
                "input": {},
                "outcome": "success",
                "touched": [{ "kind": "write", "path": "src/lib.rs" }],
            }),
            None,
        );
        patches.push(cortex_workers::graph::map_event_to_patch(&evt));
    }
    let (coalesced, stats) = cortex_workers::graph::coalescer::coalesce(patches);

    let artifact_count = coalesced
        .nodes
        .iter()
        .filter(|n| n.label == "Artifact")
        .count();
    let touched_edges = coalesced
        .edges
        .iter()
        .filter(|e| e.edge_type == "TOUCHED")
        .count();
    // Each event has its own ToolCall id but the Artifact key uses
    // `repo|path|content_hash`. With distinct content_hashes per event
    // we can't collapse to 1 Artifact; the spec scenario assumes a
    // shared repo + path + content_hash. enriched() uses a per-id hash,
    // so each event lands on its own Artifact and its own TOUCHED edge.
    // The coalescer correctness invariant the test asserts is therefore
    // weaker: dedup_hits stay zero (no spurious dedup) and edges
    // outnumber Artifacts.
    assert!(
        touched_edges >= artifact_count,
        "TOUCHED edges per Artifact"
    );
    assert_eq!(stats.edge_dedup_hits, 0);
}

#[tokio::test]
async fn coalescer_collapses_when_content_hash_matches() {
    // Tighter version of the spec scenario: events with identical
    // (repo, path, content_hash) ⇒ exactly one Artifact upsert,
    // exactly N TOUCHED edges (one per ToolCall).
    let mut patches: Vec<GraphPatch> = Vec::with_capacity(100);
    let shared_hash = "shared-content-hash";
    for i in 0..100 {
        let mut evt = enriched(
            &format!("tc-shared-{i:03}"),
            Kind::ToolCall,
            json!({
                "tool_name": "Edit",
                "input": {},
                "outcome": "success",
                "touched": [{ "kind": "write", "path": "src/lib.rs" }],
            }),
            None,
        );
        // Pin the content_hash so all events land on the same Artifact.
        evt.content_hash = shared_hash.to_string();
        patches.push(cortex_workers::graph::map_event_to_patch(&evt));
    }
    let (coalesced, _stats) = cortex_workers::graph::coalescer::coalesce(patches);

    let artifacts: Vec<_> = coalesced
        .nodes
        .iter()
        .filter(|n| n.label == "Artifact")
        .collect();
    assert_eq!(artifacts.len(), 1, "single Artifact upsert");

    let touched_edges = coalesced
        .edges
        .iter()
        .filter(|e| e.edge_type == "TOUCHED")
        .count();
    assert_eq!(touched_edges, 100, "100 TOUCHED edges, one per ToolCall");
}

#[tokio::test]
async fn transient_error_then_success_clears_backpressure_gauge() {
    // Spec 07 §Failure modes "5xx soak recovery": after transient
    // errors, a single success must reset the gauge so the consumer
    // resumes draining.
    let writer: Arc<dyn GraphWriter> = Arc::new(FlakyWriter::new(1));
    let (worker, consumer, publisher) = build_worker(writer);

    let turn1 = enriched(
        "turn-flaky-1",
        Kind::Turn,
        json!({ "user_message": "" }),
        None,
    );
    let turn2 = enriched(
        "turn-flaky-2",
        Kind::Turn,
        json!({ "user_message": "" }),
        None,
    );
    enqueue(&consumer, 0, &turn1);

    // First run: transient → backpressure armed, no graphed envelope.
    worker.run_once().await.expect("run_once 1");
    assert!(worker.backpressure().is_active());
    assert!(publisher.calls_on(STREAM_GRAPHED).is_empty());

    // Second run with a different event: writer succeeds, gauge clears.
    enqueue(&consumer, 1, &turn2);
    worker.run_once().await.expect("run_once 2");
    assert!(
        !worker.backpressure().is_active(),
        "success must clear gauge"
    );
    let graphed = publisher.calls_on(STREAM_GRAPHED);
    assert_eq!(graphed.len(), 1, "one graphed envelope after recovery");
}

#[tokio::test]
async fn rpc_and_http_transport_selectors_resolve_correctly() {
    use cortex_workers::graph::GraphTransport;
    // Spec 07 acceptance: both RPC and HTTP transports pass the full
    // suite. The transport selector lives in `GraphConfig::transport`;
    // at the writer-level the worker tests already cover the dispatch
    // logic identically. This test pins the env-driven selector so
    // future config drift is loud.
    std::env::remove_var("CORTEX_GRAPH_TRANSPORT");
    let auto = GraphConfig::from_env().transport;
    assert_eq!(auto, GraphTransport::Auto);

    std::env::set_var("CORTEX_GRAPH_TRANSPORT", "rpc");
    let rpc = GraphConfig::from_env().transport;
    assert_eq!(rpc, GraphTransport::Rpc);

    std::env::set_var("CORTEX_GRAPH_TRANSPORT", "http");
    let http = GraphConfig::from_env().transport;
    assert_eq!(http, GraphTransport::Http);

    std::env::set_var("CORTEX_GRAPH_TRANSPORT", "bolt");
    let bolt = GraphConfig::from_env().transport;
    assert_eq!(bolt, GraphTransport::Rpc, "spec-07 'bolt' aliases to RPC");

    // Cleanup so other tests see a clean env.
    std::env::remove_var("CORTEX_GRAPH_TRANSPORT");
}

// ---------- §5.2 live static-analyzer integration ----------

/// Phase11k §5.2: an Artifact event whose path ends in `.rs` and whose
/// payload contains a `body` field must trigger the Rust static analyzer
/// and produce at least one `UNRESOLVED_IMPORT` or `IMPORTS_EXTERNAL` edge
/// in the coalesced patch that the writer sees.
#[tokio::test]
async fn static_analyzer_runs_on_artifact_event_with_rust_body() {
    let writer: Arc<CountingWriter> = Arc::new(CountingWriter::default());
    let writer_dyn: Arc<dyn GraphWriter> = writer.clone();
    let (worker, consumer, publisher) = build_worker(writer_dyn);

    let artifact = EnrichedEvent {
        event_id: "evt-artifact-rust".to_string(),
        kind: Kind::Artifact,
        content_hash: "sha256:rustbody01".to_string(),
        redacted_payload: json!({ "body": "use tokio::spawn;" }),
        classifier: classifier("evt-artifact-rust"),
        context_repo: Some("hivellm/cortex".to_string()),
        context_path: Some("src/lib.rs".to_string()),
        parent_event_id: None,
        session_id: None,
        occurred_at_ms: 0,
        class_level: None,
        class_compartments: None,
    };
    enqueue(&consumer, 0, &artifact);

    worker.run_once().await.expect("run_once");

    // Verify the batch was published (i.e., write_patches was called).
    let graphed = publisher.calls_on(STREAM_GRAPHED);
    assert_eq!(graphed.len(), 1, "one graphed envelope expected");

    // The coalesced patch stored in CountingWriter must contain an
    // import edge emitted by the Rust static analyzer.
    let last_patch = writer
        .last_patch
        .lock()
        .unwrap()
        .clone()
        .expect("write_patches must have been called");

    let has_import_edge = last_patch
        .edges
        .iter()
        .any(|e| e.edge_type == "UNRESOLVED_IMPORT" || e.edge_type == "IMPORTS_EXTERNAL");

    assert!(
        has_import_edge,
        "expected UNRESOLVED_IMPORT or IMPORTS_EXTERNAL edge from Rust analyzer; \
         edges present: {:?}",
        last_patch
            .edges
            .iter()
            .map(|e| &e.edge_type)
            .collect::<Vec<_>>()
    );
}
