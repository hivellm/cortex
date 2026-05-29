//! Integration tests for the full-text worker.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_core::events::Kind;
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::fulltext::indexer::IndexReport;
use cortex_workers::fulltext::meili_client::MeiliError;
use cortex_workers::fulltext::{
    BackpressureState, ConsumedMessage, EnrichedEvent, FulltextConfig, FulltextIndexer,
    MemorySynapConsumer, MemorySynapPublisher, Metrics, Worker, BACKPRESSURE_SOAK,
    STREAM_FULLTEXT_INDEXED, STREAM_INVALID,
};
use serde_json::json;

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
        source: ClassifierSource::StaticFallback,
        prompt_version: "v1".into(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}

fn enriched(event_id: &str, kind: Kind, payload: serde_json::Value) -> EnrichedEvent {
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
    occurred_at_ms: 0,
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

// ---------- Indexer doubles ----------

#[derive(Default)]
struct CountingIndexer {
    calls: AtomicU32,
    last_count: Mutex<u32>,
}

#[async_trait]
impl FulltextIndexer for CountingIndexer {
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let n = events.len() as u32;
        if let Ok(mut g) = self.last_count.lock() {
            *g = n;
        }
        Ok(IndexReport {
            documents_upserted: n,
            documents_skipped: 0,
            documents_truncated: 0,
            by_index: std::collections::BTreeMap::new(),
            latency_ms: 1,
        })
    }
}

#[derive(Default)]
struct TransientIndexer;

#[async_trait]
impl FulltextIndexer for TransientIndexer {
    async fn index_batch(&self, _events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError> {
        Err(MeiliError::TransientError("503 boom".into()))
    }
}

#[derive(Default)]
struct RejectingIndexer;

#[async_trait]
impl FulltextIndexer for RejectingIndexer {
    async fn index_batch(&self, _events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError> {
        Err(MeiliError::Rejected {
            status: 400,
            detail: "schema mismatch".into(),
        })
    }
}

struct FlakyIndexer {
    flakes: u32,
    calls: AtomicU32,
}

impl FlakyIndexer {
    fn new(flakes: u32) -> Self {
        Self {
            flakes,
            calls: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl FulltextIndexer for FlakyIndexer {
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        if n < self.flakes {
            return Err(MeiliError::TransientError(format!("attempt {n}")));
        }
        Ok(IndexReport {
            documents_upserted: events.len() as u32,
            documents_skipped: 0,
            documents_truncated: 0,
            by_index: std::collections::BTreeMap::new(),
            latency_ms: 1,
        })
    }
}

fn build_worker(
    indexer: Arc<dyn FulltextIndexer>,
) -> (
    Arc<Worker>,
    Arc<MemorySynapConsumer>,
    Arc<MemorySynapPublisher>,
) {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        workers: 1,
        upsert_batch: 16,
        flush_ms: 50,
        ..FulltextConfig::default()
    };
    let worker = Arc::new(Worker::new(
        cfg,
        indexer,
        consumer.clone(),
        publisher.clone(),
        metrics,
    ));
    (worker, consumer, publisher)
}

// ---------- tests ----------

#[tokio::test]
async fn writes_batch_and_publishes_indexed_envelope() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(CountingIndexer::default());
    let (worker, consumer, publisher) = build_worker(indexer.clone());

    let evt = enriched("turn-1", Kind::Turn, json!({ "user_message": "hello" }));
    enqueue(&consumer, 0, &evt);

    let processed = worker.run_once().await.expect("run_once");
    assert_eq!(processed, 1);

    let indexed = publisher.calls_on(STREAM_FULLTEXT_INDEXED);
    assert_eq!(indexed.len(), 1);
    let env = &indexed[0];
    assert_eq!(env["kind"], "fulltext_indexed");
    assert_eq!(env["documents_upserted"], 1);
    assert_eq!(env["event_ids"][0], "turn-1");
}

#[tokio::test]
async fn replay_skips_already_processed_event() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(CountingIndexer::default());
    let (worker, consumer, publisher) = build_worker(indexer);

    let evt = enriched("turn-replay", Kind::Turn, json!({ "user_message": "" }));
    enqueue(&consumer, 0, &evt);
    enqueue(&consumer, 1, &evt);

    worker.run_once().await.expect("first run_once");
    let after_first = publisher.calls_on(STREAM_FULLTEXT_INDEXED).len();
    assert_eq!(after_first, 1);

    worker.run_once().await.expect("second run_once");
    // Second delivery: same event_id ⇒ dedup skips, no new index call,
    // no new envelope.
    let after_second = publisher.calls_on(STREAM_FULLTEXT_INDEXED).len();
    assert_eq!(after_second, 1);
}

#[tokio::test]
async fn malformed_payload_routes_to_invalid_stream() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(CountingIndexer::default());
    let (worker, consumer, publisher) = build_worker(indexer);

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
}

#[tokio::test]
async fn meili_rejection_routes_batch_to_invalid_stream() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(RejectingIndexer);
    let (worker, consumer, publisher) = build_worker(indexer);

    let evt = enriched("turn-bad", Kind::Turn, json!({ "user_message": "hi" }));
    enqueue(&consumer, 0, &evt);

    worker.run_once().await.expect("run_once");
    let invalids = publisher.calls_on(STREAM_INVALID);
    assert_eq!(invalids.len(), 1);
    assert_eq!(invalids[0]["cause"], "meili_rejected");
    assert!(publisher.calls_on(STREAM_FULLTEXT_INDEXED).is_empty());
}

#[tokio::test]
async fn transient_error_engages_backpressure_without_acking() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(TransientIndexer);
    let (worker, consumer, publisher) = build_worker(indexer);

    let evt = enriched("turn-flaky", Kind::Turn, json!({ "user_message": "hi" }));
    enqueue(&consumer, 0, &evt);

    worker.run_once().await.expect("run_once");
    assert!(worker.backpressure().is_active());
    assert!(publisher.calls_on(STREAM_FULLTEXT_INDEXED).is_empty());
    assert!(publisher.calls_on(STREAM_INVALID).is_empty());
}

#[tokio::test]
async fn transient_then_success_clears_backpressure_gauge() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(FlakyIndexer::new(1));
    let (worker, consumer, publisher) = build_worker(indexer);

    enqueue(
        &consumer,
        0,
        &enriched("turn-1", Kind::Turn, json!({ "user_message": "" })),
    );
    worker.run_once().await.expect("run_once 1");
    assert!(worker.backpressure().is_active());

    enqueue(
        &consumer,
        1,
        &enriched("turn-2", Kind::Turn, json!({ "user_message": "" })),
    );
    worker.run_once().await.expect("run_once 2");
    assert!(
        !worker.backpressure().is_active(),
        "success must clear gauge"
    );
    let envs = publisher.calls_on(STREAM_FULLTEXT_INDEXED);
    assert_eq!(envs.len(), 1);
}

#[tokio::test]
async fn paused_backpressure_skips_consumption_entirely() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(CountingIndexer::default());
    let (worker, consumer, publisher) = build_worker(indexer);

    let aged = Instant::now() - BACKPRESSURE_SOAK - Duration::from_secs(1);
    worker.backpressure().force_since(aged);
    assert!(worker.backpressure().is_paused());

    enqueue(
        &consumer,
        0,
        &enriched("turn-paused", Kind::Turn, json!({ "user_message": "" })),
    );
    let processed = worker.run_once().await.expect("run_once");
    assert_eq!(processed, 0);
    assert_eq!(consumer.remaining(), 1);
    assert!(publisher.calls_on(STREAM_FULLTEXT_INDEXED).is_empty());
}

#[tokio::test]
async fn ten_thousand_event_stream_drains_idempotently() {
    let indexer: Arc<dyn FulltextIndexer> = Arc::new(CountingIndexer::default());
    let (worker, consumer, publisher) = build_worker(indexer);

    let total: usize = 10_000;
    for i in 0..total {
        let id = format!("turn-{i:06}");
        let evt = enriched(&id, Kind::Turn, json!({ "user_message": "x" }));
        enqueue(&consumer, i as u64, &evt);
    }
    while worker.run_once().await.expect("run_once") > 0 {}
    let indexed = publisher.calls_on(STREAM_FULLTEXT_INDEXED);
    let expected_batches = total.div_ceil(16);
    assert_eq!(indexed.len(), expected_batches);

    // Aggregate event_ids — every input event surfaces exactly once.
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for env in &indexed {
        for v in env["event_ids"].as_array().unwrap() {
            seen.insert(v.as_str().unwrap().to_string());
        }
    }
    assert_eq!(seen.len(), total);

    // Replay: same event_ids ⇒ no new envelopes.
    for i in 0..total {
        let id = format!("turn-{i:06}");
        let evt = enriched(&id, Kind::Turn, json!({ "user_message": "x" }));
        enqueue(&consumer, (total + i) as u64, &evt);
    }
    while worker.run_once().await.expect("replay run_once") > 0 {}
    let after_replay = publisher.calls_on(STREAM_FULLTEXT_INDEXED).len();
    assert_eq!(after_replay, expected_batches);
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
}
