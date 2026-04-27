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

use cortex_classifier::{build_offline_stack, BudgetTracker, InMemoryCache, PricingTable};
use cortex_classifier_worker::{
    ConsumedMessage, MemorySynapConsumer, MemorySynapPublisher, Worker, STREAM_BOOTSTRAP,
    STREAM_ENRICHED, STREAM_RAW,
};
use cortex_core::events::{Context as EvtContext, Envelope, Kind, Stream};
use cortex_embedder::EnrichedEvent;
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
    let cfg = cortex_classifier_worker::ClassifierWorkerConfig::default();
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
    let cfg = cortex_classifier_worker::ClassifierWorkerConfig::default();
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
