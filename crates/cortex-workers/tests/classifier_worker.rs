//! Integration tests for the classifier worker.
//!
//! Drive the worker with the in-memory consumer/publisher so the
//! Synap → classify → enriched flow can be validated without spinning
//! up a real Synap instance. The tests cover:
//!
//! 1. A bootstrap envelope produces an `EnrichedEvent` on `cortex.events.enriched`.
//! 2. A canonical envelope produces an `EnrichedEvent` on `cortex.events.enriched`.
//! 3. A duplicate `event_id` is acked but not re-published (replay dedup).
//! 4. Halted budget swaps the active backend for the static fallback.

use std::collections::BTreeMap;
use std::sync::Arc;

use cortex_classifier::{
    build_offline_stack, BudgetTracker, Classifier, ClassifierError, ClassifierOutput,
    ClassifierSource, EnrichmentInput, InMemoryCache, PiiRisk, PricingTable, Severity,
};
use cortex_workers::classifier::{
    ConsumedMessage, MemorySynapConsumer, MemorySynapPublisher, Worker, STREAM_BOOTSTRAP,
    STREAM_ENRICHED, STREAM_RAW,
};
use cortex_core::events::{Context as EvtContext, Envelope, Kind, Stream};
use cortex_workers::embedder::EnrichedEvent;
use cortex_storage::MetadataStore;
use serde_json::{json, Value};

fn worker_with_offline_stack() -> (
    Arc<MemorySynapConsumer>,
    Arc<MemorySynapPublisher>,
    Arc<Worker>,
) {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let budget = Arc::new(BudgetTracker::new(2000, PricingTable::HAIKU_4_5));
    let cache: Box<dyn cortex_classifier::ClassifierCache> = Box::new(InMemoryCache::default());
    let stack = build_offline_stack(cache, budget);
    let cfg = cortex_workers::classifier::ClassifierWorkerConfig::default();
    let worker = Worker::with_stack(cfg, stack, consumer.clone(), publisher.clone());
    (consumer, publisher, Arc::new(worker))
}

fn bootstrap_event(event_id: &str, kind: &str, repo: &str, path: &str) -> Value {
    json!({
        "event_id": event_id,
        "ts": 1_000_000_000_000_i64,
        "kind": kind,
        "adapter": "bootstrap",
        "stream": "cortex.events.bootstrap",
        "source": {
            "repo": repo,
            "path": path,
            "git_ref": "abc1234"
        },
        "redacted_payload": {
            "text": "fn hello() {}",
            "language": "rust"
        },
        "content_hash": "sha256:deadbeef",
        "redactions": 0
    })
}

fn canonical_envelope(event_id: &str, kind: Kind) -> Envelope {
    Envelope {
        event_id: event_id.to_string(),
        schema_version: "1".to_string(),
        occurred_at: "2026-04-26T18:00:00Z".to_string(),
        ingested_at: Some("2026-04-26T18:00:01Z".to_string()),
        session_id: "01HXY1234567890ABCDEFGHIJK".to_string(),
        stream: Stream::Live,
        tool: "claude-code".to_string(),
        model: Some("claude-opus-4-7".to_string()),
        kind,
        context: EvtContext {
            repo: Some("Cortex".into()),
            branch: Some("main".into()),
            commit: Some("abc1234".into()),
            cwd: None,
            user: None,
            platform: "win32".into(),
            ide: None,
            extras: BTreeMap::new(),
        },
        payload: json!({
            "role": "user",
            "message": "ship it"
        }),
        redactions: vec![],
        content_hash: "sha256:cafef00d".to_string(),
        parent_event_id: None,
    }
}

#[tokio::test]
async fn bootstrap_event_publishes_enriched() {
    let (consumer, publisher, worker) = worker_with_offline_stack();
    consumer.enqueue(
        STREAM_BOOTSTRAP,
        ConsumedMessage {
            offset: 1,
            kind: "artifact.code".into(),
            payload: bootstrap_event("01HBOOTSTRAP", "artifact.code", "Cortex", "src/lib.rs"),
            event_id: Some("01HBOOTSTRAP".into()),
        },
    );

    let handled = worker.run_once().await.expect("run_once");
    assert_eq!(handled, 1);

    let calls = publisher.calls_on(STREAM_ENRICHED);
    assert_eq!(calls.len(), 1, "exactly one enriched envelope");
    let enriched: EnrichedEvent =
        serde_json::from_value(calls[0].clone()).expect("deserialize EnrichedEvent");
    assert_eq!(enriched.event_id, "01HBOOTSTRAP");
    assert_eq!(enriched.kind, Kind::Artifact);
    assert_eq!(enriched.context_repo.as_deref(), Some("Cortex"));
    assert_eq!(enriched.context_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(enriched.content_hash, "sha256:deadbeef");
    assert_eq!(
        enriched.classifier.source.as_str(),
        "static_fallback",
        "default mode is static fallback"
    );
}

#[tokio::test]
async fn canonical_envelope_publishes_enriched() {
    let (consumer, publisher, worker) = worker_with_offline_stack();
    let env = canonical_envelope("01HRAWLIVE", Kind::Turn);
    let payload = serde_json::to_value(&env).expect("serialize envelope");
    consumer.enqueue(
        STREAM_RAW,
        ConsumedMessage {
            offset: 7,
            kind: "turn".into(),
            payload,
            event_id: Some("01HRAWLIVE".into()),
        },
    );

    let handled = worker.run_once().await.expect("run_once");
    assert_eq!(handled, 1);

    let calls = publisher.calls_on(STREAM_ENRICHED);
    assert_eq!(calls.len(), 1);
    let enriched: EnrichedEvent =
        serde_json::from_value(calls[0].clone()).expect("deserialize EnrichedEvent");
    assert_eq!(enriched.event_id, "01HRAWLIVE");
    assert_eq!(enriched.kind, Kind::Turn);
    assert_eq!(enriched.context_repo.as_deref(), Some("Cortex"));
    assert_eq!(enriched.content_hash, "sha256:cafef00d");
}

#[tokio::test]
async fn replay_is_deduped_within_lifetime() {
    let (consumer, publisher, worker) = worker_with_offline_stack();
    let payload = bootstrap_event("01HDEDUP", "artifact.code", "Cortex", "src/main.rs");
    consumer.enqueue(
        STREAM_BOOTSTRAP,
        ConsumedMessage {
            offset: 1,
            kind: "artifact.code".into(),
            payload: payload.clone(),
            event_id: Some("01HDEDUP".into()),
        },
    );

    worker.run_once().await.expect("first iter");
    assert_eq!(publisher.calls_on(STREAM_ENRICHED).len(), 1);

    // Re-deliver the same message at a later offset.
    consumer.enqueue(
        STREAM_BOOTSTRAP,
        ConsumedMessage {
            offset: 2,
            kind: "artifact.code".into(),
            payload,
            event_id: Some("01HDEDUP".into()),
        },
    );

    worker.run_once().await.expect("second iter");
    assert_eq!(
        publisher.calls_on(STREAM_ENRICHED).len(),
        1,
        "duplicate event_id must not re-publish"
    );
}

/// Backend that stamps a fixed token bill on every classification —
/// lets the spend-recording test assert the metadata roll-up converts
/// tokens → cents via the configured [`PricingTable`].
struct FixedTokenBackend {
    tokens_in: u32,
    tokens_out: u32,
}

#[async_trait::async_trait]
impl Classifier for FixedTokenBackend {
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, ClassifierError> {
        Ok(events
            .iter()
            .map(|e| ClassifierOutput {
                event_id: e.event_id.clone(),
                kind_refinement: None,
                topics: vec![],
                severity: Severity::Info,
                pii_risk: PiiRisk::Low,
                redaction_suggestions: vec![],
                summary: None,
                entities: Vec::new(),
                relations: Vec::new(),
                source: ClassifierSource::HaikuCli,
                prompt_version: "test-v1".into(),
                model: "claude-haiku-4-5".into(),
                latency_ms: 0,
                tokens_in: self.tokens_in,
                tokens_out: self.tokens_out,
            })
            .collect())
    }
}

#[tokio::test]
async fn metadata_store_records_classifier_spend_after_publish() {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let store = Arc::new(std::sync::Mutex::new(MetadataStore::open_in_memory().unwrap()));

    // 10k input + 4k output tokens against HAIKU_4_5 pricing
    // ($0.001/1k in, $0.005/1k out) = $0.01 + $0.02 = $0.03 = 3 cents.
    let backend: Arc<dyn Classifier> = Arc::new(FixedTokenBackend {
        tokens_in: 10_000,
        tokens_out: 4_000,
    });
    let cfg = cortex_workers::classifier::ClassifierWorkerConfig::default();
    let worker = Worker::new(cfg, backend, consumer.clone(), publisher.clone())
        .with_metadata(store.clone())
        .with_pricing(PricingTable::HAIKU_4_5);

    consumer.enqueue(
        STREAM_RAW,
        ConsumedMessage {
            offset: 1,
            kind: "turn".into(),
            payload: serde_json::to_value(&canonical_envelope("01HSPEND", Kind::Turn)).unwrap(),
            event_id: Some("01HSPEND".into()),
        },
    );

    worker.run_once().await.expect("run_once");
    assert_eq!(publisher.calls_on(STREAM_ENRICHED).len(), 1);

    // The hourly row for "now" should carry 1 call + the pricing
    // table's cents conversion.
    let now = chrono::Utc::now();
    let hour = cortex_storage::hour_bucket_rfc3339(now);
    let guard = store.lock().unwrap();
    let rows = guard
        .classifier_spend_hourly_window(&hour, &hour)
        .expect("hourly window");
    assert_eq!(rows.len(), 1, "exactly one hourly bucket touched");
    assert_eq!(rows[0].spend.calls, 1);
    assert_eq!(rows[0].spend.tokens_in, 10_000);
    assert_eq!(rows[0].spend.tokens_out, 4_000);
    assert_eq!(rows[0].spend.est_usd_cents, 3);

    // Daily roll-up mirrors the hourly accumulation so the legacy
    // `classifier_spend` table stays usable for callers that already
    // poll it.
    let day = now.format("%Y-%m-%d").to_string();
    let daily = guard.classifier_spend(&day).unwrap().unwrap();
    assert_eq!(daily.calls, 1);
    assert_eq!(daily.est_usd_cents, 3);
}

#[tokio::test]
async fn worker_without_metadata_skips_spend_recording() {
    // Sanity check: the legacy code path (no metadata wired) still
    // classifies + publishes, and never panics trying to touch a
    // store that isn't there.
    let (consumer, publisher, worker) = worker_with_offline_stack();
    consumer.enqueue(
        STREAM_RAW,
        ConsumedMessage {
            offset: 1,
            kind: "turn".into(),
            payload: serde_json::to_value(&canonical_envelope("01HNOMETA", Kind::Turn)).unwrap(),
            event_id: Some("01HNOMETA".into()),
        },
    );
    worker.run_once().await.expect("run_once");
    assert_eq!(publisher.calls_on(STREAM_ENRICHED).len(), 1);
}

#[tokio::test]
async fn budget_halt_uses_static_fallback() {
    // Build a fully-loaded stack and force the tracker over its halt
    // threshold so the BudgetedClassifier shunts to StaticClassifier.
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let budget = Arc::new(BudgetTracker::new(100, PricingTable::HAIKU_4_5));
    budget.set_spend_cents_for_test(10_000); // 100x over limit -> Halt.
    let cache: Box<dyn cortex_classifier::ClassifierCache> = Box::new(InMemoryCache::default());
    let stack = build_offline_stack(cache, budget);
    let cfg = cortex_workers::classifier::ClassifierWorkerConfig::default();
    let worker = Worker::with_stack(cfg, stack, consumer.clone(), publisher.clone());

    consumer.enqueue(
        STREAM_BOOTSTRAP,
        ConsumedMessage {
            offset: 1,
            kind: "memory.imported".into(),
            payload: bootstrap_event("01HHALT", "memory.imported", "Cortex", "CLAUDE.md"),
            event_id: Some("01HHALT".into()),
        },
    );

    worker.run_once().await.expect("run_once");
    let calls = publisher.calls_on(STREAM_ENRICHED);
    assert_eq!(calls.len(), 1);
    let enriched: EnrichedEvent = serde_json::from_value(calls[0].clone()).unwrap();
    assert_eq!(enriched.classifier.source.as_str(), "static_fallback");
    assert_eq!(enriched.kind, Kind::Memory);
}
