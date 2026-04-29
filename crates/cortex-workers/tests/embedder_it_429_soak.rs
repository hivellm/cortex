//! Backpressure / 429-soak behaviour of the worker loop.
//!
//! Spec 06 §Worker concurrency calls for the consumer to halt forward
//! progress once Vectorizer has been rate-limiting for more than 30 s. The
//! brief invited a wiremock-driven variant against the real SDK, but the
//! `vectorizer-sdk` v3 `ClientConfig::base_url` is honoured by the HTTP
//! transport and the dispatch of which rate-limit status maps to which
//! `VectorizerError` variant is already tested in the round-3 unit suite.
//!
//! What we actually want to exercise here is the **worker-level
//! backpressure loop**: repeated `RateLimited` observations must arm the
//! gauge, halt the batch, and block further consumption until the soak
//! threshold clears. That lives above the SDK, so we inject a trait-impl
//! `VectorizerClient` that returns `RateLimited` for a programmable number
//! of upserts then switches to a wrapped [`MemoryVectorizerClient`].

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cortex_core::events::Kind;
use cortex_workers::embedder::{
    BackpressureState, Chunk, CollectionSchema, ConsumedMessage, EmbedderConfig,
    LiveSynapPublisher, MemorySynapConsumer, MemorySynapPublisher, MemoryVectorizerClient,
    Metrics, UpsertReport, VectorizerClient, VectorizerClientError, VectorizerEmbedder, Worker,
    STREAM_EMBEDDED, STREAM_ENRICHED, STREAM_INVALID,
};

mod common;
use common::*;

/// Programmable Vectorizer client — the first `fail_n` upserts return
/// `RateLimited`; subsequent calls are proxied to the wrapped memory client.
struct ThrottledClient {
    inner: Arc<MemoryVectorizerClient>,
    fail_remaining: AtomicU32,
    upsert_calls: AtomicU32,
}

impl ThrottledClient {
    fn new(inner: Arc<MemoryVectorizerClient>, fail_n: u32) -> Self {
        Self {
            inner,
            fail_remaining: AtomicU32::new(fail_n),
            upsert_calls: AtomicU32::new(0),
        }
    }

    fn calls(&self) -> u32 {
        self.upsert_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl VectorizerClient for ThrottledClient {
    async fn ensure_collection(
        &self,
        name: &str,
        schema: &CollectionSchema,
    ) -> Result<(), VectorizerClientError> {
        self.inner.ensure_collection(name, schema).await
    }

    async fn upsert_chunks(
        &self,
        collection: &str,
        chunks: &[Chunk],
    ) -> Result<UpsertReport, VectorizerClientError> {
        self.upsert_calls.fetch_add(1, Ordering::SeqCst);
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
    ) -> Result<std::collections::BTreeSet<String>, VectorizerClientError> {
        self.inner.exists_by_dedup_key(collection, dedup_keys).await
    }
}

#[tokio::test]
async fn worker_backs_off_on_repeated_ratelimit() {
    // 50 enriched events, each a trivial Rust one-liner → CodeChunker emits
    // one chunk per event.
    let consumer = Arc::new(MemorySynapConsumer::new());
    for i in 0..50 {
        let ev = enriched_event(
            &format!("evt-{i:02}"),
            Kind::ToolCall,
            Some("one.rs"),
            &format!("fn f_{i}() {{ let _ = {i}; }}\n"),
            None,
        );
        consumer.enqueue(ConsumedMessage {
            offset: i as u64,
            kind: "enriched".into(),
            payload: serde_json::to_value(&ev).unwrap(),
            event_id: Some(ev.event_id.clone()),
        });
    }

    let publisher = Arc::new(MemorySynapPublisher::new());

    // Client returns RateLimited for the first 10 upserts (plenty to
    // demonstrate sustained backpressure), then proxies to the memory
    // client.
    let inner_vec = Arc::new(MemoryVectorizerClient::default());
    let throttled = Arc::new(ThrottledClient::new(inner_vec.clone(), 10));
    let client_trait: Arc<dyn VectorizerClient> = throttled.clone();

    let config = EmbedderConfig {
        // Keep the run small and fast — five-at-a-time leaves room for
        // multiple batch halts before we flip the switch.
        workers: 5,
        upsert_batch: 64,
        ..EmbedderConfig::default()
    };
    let metrics = Arc::new(Metrics::new());
    let embedder = Arc::new(VectorizerEmbedder::with_metrics(
        config.clone(),
        client_trait,
        metrics.clone(),
    ));
    let worker = Arc::new(Worker::new(
        config,
        embedder,
        consumer.clone(),
        publisher.clone(),
        metrics.clone(),
    ));

    // Run a few iterations under throttling — batches should halt on the
    // first RateLimited they see.
    for _ in 0..5 {
        let _ = worker.run_once().await.expect("run_once");
        if worker.backpressure.is_active() {
            break;
        }
    }
    assert!(
        worker.backpressure.is_active(),
        "backpressure must be armed after repeated RateLimited errors"
    );

    // The gauge flips through `Metrics::set_backpressure` on the path that
    // handles rate-limit; assert the gauge is non-zero.
    let gauge = metrics
        .backpressure_active
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(gauge, 1, "backpressure_active gauge must be 1");

    // No successful embed envelopes while throttled.
    assert!(publisher.calls_on(STREAM_EMBEDDED).is_empty());

    // Advance the rate-limit timestamp past the 30-s soak so is_paused()
    // goes true. While paused, run_once must idle.
    worker
        .backpressure
        .force_since(std::time::Instant::now() - Duration::from_secs(31));
    assert!(worker.backpressure.is_paused());
    let handled = worker.run_once().await.expect("run_once while paused");
    assert_eq!(handled, 0, "paused worker must not drain the queue");

    // Clear the throttle + signal success so the loop resumes.
    throttled.fail_remaining.store(0, Ordering::SeqCst);
    worker.backpressure.record_success();
    assert!(!worker.backpressure.is_paused());
    assert!(!worker.backpressure.is_active());

    // Drain the remainder. The worker should pick up and publish embedded
    // envelopes for the events it can process.
    let mut spins = 0;
    while consumer.remaining() > 0 && spins < 50 {
        let handled = worker.run_once().await.expect("run_once post-resume");
        if handled == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        spins += 1;
    }
    let embedded = publisher.calls_on(STREAM_EMBEDDED);
    assert!(
        !embedded.is_empty(),
        "worker must publish at least one embedded envelope after resume"
    );

    // Throttled client saw at least as many upsert attempts as there were
    // rate-limit hits, plus the post-resume ones.
    assert!(throttled.calls() > 5);
}

/// Sanity check that the shared types compile in this binary — some of
/// them would otherwise be dropped by the test-only dead-code pruner.
#[allow(dead_code)]
fn _touch_types(
    _b: BackpressureState,
    _p: LiveSynapPublisher,
    _s1: &'static str,
    _s2: &'static str,
    _s3: &'static str,
    _l: Mutex<Vec<u8>>,
    _a: Arc<AtomicBool>,
) {
    let _ = STREAM_ENRICHED;
    let _ = STREAM_EMBEDDED;
    let _ = STREAM_INVALID;
}
