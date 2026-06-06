//! Integration tests for `cortex_workers::embedder::worker`.

use async_trait::async_trait;
use cortex_core::events::Kind;
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::embedder::{
    Chunk, CollectionSchema, ConsumedMessage, EmbedderConfig, EnrichedEvent, MemoryCall,
    MemorySynapConsumer, MemorySynapPublisher, MemoryVectorizerClient, Metrics, UpsertReport,
    VectorizerClient, VectorizerClientError, VectorizerEmbedder, Worker, STREAM_EMBEDDED,
    STREAM_INVALID,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn classifier(summary: Option<&str>) -> ClassifierOutput {
    ClassifierOutput {
        event_id: "evt".into(),
        kind_refinement: None,
        topics: vec!["t".into()],
        severity: Severity::Info,
        pii_risk: PiiRisk::Low,
        redaction_suggestions: vec![],
        summary: summary.map(|s| s.to_string()),
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

fn make_enriched(id: &str, content: &str, path: Option<&str>) -> EnrichedEvent {
    EnrichedEvent {
        event_id: id.to_string(),
        kind: Kind::ToolCall,
        content_hash: format!("parent-{id}"),
        redacted_payload: json!({ "content": content }),
        classifier: classifier(None),
        context_repo: None,
        context_path: path.map(|s| s.to_string()),
        parent_event_id: None,
        session_id: None,
        occurred_at_ms: 0,
    }
}

fn enqueue_event(consumer: &MemorySynapConsumer, offset: u64, event: &EnrichedEvent) {
    consumer.enqueue(ConsumedMessage {
        offset,
        kind: "enriched".into(),
        payload: serde_json::to_value(event).unwrap(),
        event_id: Some(event.event_id.clone()),
    });
}

fn make_worker(
    consumer: Arc<MemorySynapConsumer>,
    publisher: Arc<MemorySynapPublisher>,
    client: Arc<dyn VectorizerClient>,
) -> Arc<Worker> {
    let config = EmbedderConfig {
        workers: 8,
        ..EmbedderConfig::default()
    };
    let embedder = Arc::new(VectorizerEmbedder::new(config.clone(), client));
    let metrics = Arc::new(Metrics::new());
    Arc::new(Worker::new(config, embedder, consumer, publisher, metrics))
}

#[tokio::test]
async fn happy_path_publishes_embedded_for_each_event() {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let vec_client = Arc::new(MemoryVectorizerClient::default());
    let worker = make_worker(consumer.clone(), publisher.clone(), vec_client.clone());

    let events = [
        make_enriched("a", "fn a() {}", Some("a.rs")),
        make_enriched("b", "fn b() {}", Some("b.rs")),
        make_enriched("c", "fn c() {}", Some("c.rs")),
    ];
    for (i, e) in events.iter().enumerate() {
        enqueue_event(&consumer, i as u64, e);
    }

    let handled = worker.run_once().await.unwrap();
    assert_eq!(handled, 3);

    let embedded = publisher.calls_on(STREAM_EMBEDDED);
    assert_eq!(embedded.len(), 3);
    assert!(publisher.calls_on(STREAM_INVALID).is_empty());

    let upsert_calls = vec_client
        .calls()
        .into_iter()
        .filter(|c| matches!(c, MemoryCall::Upsert(_, _)))
        .count();
    assert_eq!(upsert_calls, 3);
}

#[tokio::test]
async fn oversize_without_summary_yields_invalid() {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let vec_client = Arc::new(MemoryVectorizerClient::default());
    let worker = make_worker(consumer.clone(), publisher.clone(), vec_client.clone());

    let big = "lorem ipsum dolor sit amet consectetur. ".repeat(200);
    let md = format!("# Big\n{body}\n", body = big);
    let mut event = make_enriched("over", &md, Some("doc.md"));
    event.kind = Kind::Artifact;
    enqueue_event(&consumer, 0, &event);

    let _ = worker.run_once().await.unwrap();

    let invalid = publisher.calls_on(STREAM_INVALID);
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0]["cause"], "oversize_without_summary");
    assert_eq!(invalid[0]["event_id"], "over");
    assert!(publisher.calls_on(STREAM_EMBEDDED).is_empty());
}

#[tokio::test]
async fn malformed_envelope_yields_deserialize_failed() {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let vec_client = Arc::new(MemoryVectorizerClient::default());
    let worker = make_worker(consumer.clone(), publisher.clone(), vec_client.clone());

    consumer.enqueue(ConsumedMessage {
        offset: 0,
        kind: "enriched".into(),
        payload: json!({ "event_id": "broken", "not": "an EnrichedEvent" }),
        event_id: Some("broken".into()),
    });

    let _ = worker.run_once().await.unwrap();
    let invalid = publisher.calls_on(STREAM_INVALID);
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0]["cause"], "deserialize_failed");
    assert_eq!(invalid[0]["event_id"], "broken");
    assert!(publisher.calls_on(STREAM_EMBEDDED).is_empty());
}

// Rate-limited client used by the backpressure test — wraps the real
// `MemoryVectorizerClient` and rejects the first N upserts with
// `RateLimited` so we can drive the worker through its backoff path.

struct RateLimitedClient {
    inner: Arc<MemoryVectorizerClient>,
    fail_remaining: AtomicU32,
}

impl RateLimitedClient {
    fn new(inner: Arc<MemoryVectorizerClient>, fail_n: u32) -> Self {
        Self {
            inner,
            fail_remaining: AtomicU32::new(fail_n),
        }
    }
}

#[async_trait]
impl VectorizerClient for RateLimitedClient {
    async fn ensure_collection(
        &self,
        name: &str,
        schema: &CollectionSchema,
    ) -> std::result::Result<(), VectorizerClientError> {
        self.inner.ensure_collection(name, schema).await
    }
    async fn upsert_chunks(
        &self,
        collection: &str,
        chunks: &[Chunk],
    ) -> std::result::Result<UpsertReport, VectorizerClientError> {
        if self.fail_remaining.load(Ordering::SeqCst) > 0 {
            self.fail_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(VectorizerClientError::RateLimited);
        }
        self.inner.upsert_chunks(collection, chunks).await
    }
    async fn exists_by_dedup_key(
        &self,
        collection: &str,
        dedup_keys: &[String],
    ) -> std::result::Result<BTreeSet<String>, VectorizerClientError> {
        self.inner.exists_by_dedup_key(collection, dedup_keys).await
    }
    async fn delete_vectors(
        &self,
        collection: &str,
        ids: &[String],
    ) -> std::result::Result<vectorizer_sdk::models::DeleteReport, VectorizerClientError> {
        self.inner.delete_vectors(collection, ids).await
    }
    async fn move_vectors(
        &self,
        src: &str,
        dst: &str,
        ids: &[String],
    ) -> std::result::Result<vectorizer_sdk::models::MoveReport, VectorizerClientError> {
        self.inner.move_vectors(src, dst, ids).await
    }
}

#[tokio::test]
async fn rate_limit_arms_backpressure_and_halts_batch() {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let inner = Arc::new(MemoryVectorizerClient::default());
    let vec_client: Arc<dyn VectorizerClient> = Arc::new(RateLimitedClient::new(inner.clone(), 2));
    let worker = make_worker(consumer.clone(), publisher.clone(), vec_client);

    let event = make_enriched("rl", "fn f() {}", Some("f.rs"));
    enqueue_event(&consumer, 0, &event);
    enqueue_event(&consumer, 1, &event);

    let _ = worker.run_once().await.unwrap();

    assert!(worker.backpressure.is_active());
    assert_eq!(consumer.remaining(), 0, "batch drained in one fetch");
    assert!(publisher.calls_on(STREAM_INVALID).is_empty());
    assert!(publisher.calls_on(STREAM_EMBEDDED).is_empty());

    assert!(!worker.backpressure.is_paused());

    worker
        .backpressure
        .force_since(std::time::Instant::now() - Duration::from_secs(31));
    assert!(worker.backpressure.is_paused());

    let handled_while_paused = worker.run_once().await.unwrap();
    assert_eq!(handled_while_paused, 0);

    worker.backpressure.record_success();
    assert!(!worker.backpressure.is_paused());
    assert!(!worker.backpressure.is_active());
}

#[tokio::test]
async fn replayed_event_is_deduped_by_inmemory_guard() {
    let consumer = Arc::new(MemorySynapConsumer::new());
    let publisher = Arc::new(MemorySynapPublisher::new());
    let vec_client = Arc::new(MemoryVectorizerClient::default());
    let worker = make_worker(consumer.clone(), publisher.clone(), vec_client.clone());

    let event = make_enriched("dup", "fn f() {}", Some("f.rs"));
    enqueue_event(&consumer, 0, &event);
    enqueue_event(&consumer, 1, &event);

    let handled = worker.run_once().await.unwrap();
    assert_eq!(handled, 2);

    let upsert_calls = vec_client
        .calls()
        .into_iter()
        .filter(|c| matches!(c, MemoryCall::Upsert(_, _)))
        .count();
    assert_eq!(upsert_calls, 1);

    let embedded = publisher.calls_on(STREAM_EMBEDDED);
    assert_eq!(embedded.len(), 1);
}
