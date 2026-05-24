//! Embedder worker.
//!
//! Consumes `cortex.events.enriched` from Synap, embeds each event through
//! [`crate::Embedder`], and republishes the outcome on
//! `cortex.events.embedded` (success) or `cortex.events.invalid` (failure).
//!
//! Synap 0.11 exposes a **pull** consume API
//! (`StreamManager::consume(room, offset, limit)`) — there is no durable
//! consumer-group or ack primitive in the SDK at this version. The worker
//! therefore tracks the last processed offset locally (`OffsetTracker`) and
//! skips any event whose `event_id` is already in an in-memory processed-id
//! set, giving at-least-once delivery within a single worker-process
//! lifetime. When Synap grows consumer groups we can swap the tracker for a
//! server-side cursor without touching callers.
//!
//! Backpressure: two consecutive `RateLimited` errors from Vectorizer flip
//! the `BackpressureState` gauge. The loop then idles for five-second
//! windows until a successful embed is observed; the 30-second pause-soak
//! threshold in spec 06 §Worker concurrency is enforced by gating
//! `record_rate_limit` against wall-clock — once a rate-limit has been
//! active for 30 s the consumer idles until the next `record_success`.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use synap_sdk::stream::StreamManager;
use synap_sdk::types::Event;
use synap_sdk::{SynapClient, SynapConfig};

use super::config::EmbedderConfig;
use super::embedder::{EmbedError, EmbedReport, Embedder, EnrichedEvent};
use super::metrics::Metrics;

/// Default stream name for enriched events.
pub const STREAM_ENRICHED: &str = "cortex.events.enriched";
/// Stream name for successful embed outcomes.
pub const STREAM_EMBEDDED: &str = "cortex.events.embedded";
/// Stream name for events that could not be embedded.
pub const STREAM_INVALID: &str = "cortex.events.invalid";
/// Threshold for Vectorizer rate-limit soak before the worker halts
/// consumption entirely, per spec 06 §Worker concurrency.
pub const BACKPRESSURE_SOAK: Duration = Duration::from_secs(30);

// ---------- Consumer abstraction ----------------------------------------

/// One message delivered by a [`SynapConsumer`].
#[derive(Debug, Clone)]
pub struct ConsumedMessage {
    /// Stream offset — monotonically increasing per room.
    pub offset: u64,
    /// Event kind label from the envelope (Synap's `event` field).
    pub kind: String,
    /// Event payload (the envelope body).
    pub payload: Value,
    /// Pre-extracted event id, when the payload already contained one.
    pub event_id: Option<String>,
}

/// Synap-consumer abstraction.
#[async_trait]
pub trait SynapConsumer: Send + Sync + 'static {
    /// Fetch up to `max` un-processed messages from `room`.
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>>;

    /// Mark a message as processed. The default in-memory consumers advance
    /// their offset cursor; the live consumer persists the offset to the
    /// local [`OffsetTracker`] so reconnects pick up from the right spot.
    async fn ack(&self, room: &str, offset: u64) -> Result<()>;
}

// ---------- Publisher abstraction ---------------------------------------

/// Synap-publisher abstraction — local to this crate so there is no
/// cross-crate dependency on `cortex-ingestion`.
#[async_trait]
pub trait SynapPublisher: Send + Sync + 'static {
    /// Publish `envelope` onto `room`. The `event` label Synap stores with
    /// each record is the envelope's `kind` field, falling back to the
    /// room name itself when absent.
    async fn publish(&self, room: &str, envelope: &Value) -> Result<()>;
}

// ---------- Offset tracking --------------------------------------------

/// Per-room offset cursor used by [`LiveSynapConsumer`]. Thread-safe.
#[derive(Debug, Default)]
pub struct OffsetTracker {
    /// Next offset to request from Synap.
    next: AtomicU64,
}

impl OffsetTracker {
    /// Create a new tracker starting at offset 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the current cursor.
    pub fn current(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }

    /// Advance past the given offset.
    pub fn advance_past(&self, offset: u64) {
        loop {
            let current = self.next.load(Ordering::Relaxed);
            let proposed = offset.saturating_add(1).max(current);
            match self
                .next
                .compare_exchange(current, proposed, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }
}

// ---------- Backpressure -----------------------------------------------

/// Tracks whether Vectorizer has been rate-limiting for long enough that
/// the worker should stop consuming new messages.
#[derive(Debug, Default)]
pub struct BackpressureState {
    /// Instant the first `RateLimited` observation became "live", or `None`
    /// when Vectorizer is currently healthy.
    since: Mutex<Option<Instant>>,
    /// Whether the gauge is currently armed.
    active: AtomicBool,
}

impl BackpressureState {
    /// Fresh, healthy state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a `RateLimited` observation. No-ops if already armed.
    pub fn record_rate_limit(&self) {
        if let Ok(mut guard) = self.since.lock() {
            if guard.is_none() {
                *guard = Some(Instant::now());
            }
        }
        self.active.store(true, Ordering::Relaxed);
    }

    /// Record a successful embed; clears the rate-limit timestamp and
    /// disarms the gauge.
    pub fn record_success(&self) {
        if let Ok(mut guard) = self.since.lock() {
            *guard = None;
        }
        self.active.store(false, Ordering::Relaxed);
    }

    /// Raw active flag — `true` whenever a rate-limit has been observed
    /// more recently than a success.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Return `true` once the rate-limit condition has persisted for at
    /// least [`BACKPRESSURE_SOAK`] (30 s by default). The worker halts
    /// consumption in that state.
    pub fn is_paused(&self) -> bool {
        let guard = match self.since.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        match *guard {
            Some(t) => t.elapsed() >= BACKPRESSURE_SOAK,
            None => false,
        }
    }

    /// Force-arm the state at a specific `since` instant. Used by tests to
    /// avoid real 30 s sleeps.
    #[doc(hidden)]
    pub fn force_since(&self, instant: Instant) {
        if let Ok(mut guard) = self.since.lock() {
            *guard = Some(instant);
        }
        self.active.store(true, Ordering::Relaxed);
    }
}

// ---------- Live Synap consumer / publisher ----------------------------

/// Synap client handle built from the embedder configuration.
pub struct SynapHandle {
    streams: StreamManager,
}

impl SynapHandle {
    /// Connect to Synap at `base_url` and return a handle for stream I/O.
    pub fn new(base_url: &str) -> Result<Self> {
        let cfg = SynapConfig::new(base_url);
        let client = SynapClient::new(cfg).map_err(|e| anyhow::anyhow!("synap client: {e}"))?;
        Ok(Self {
            streams: client.stream(),
        })
    }

    /// Shared stream manager.
    pub fn streams(&self) -> &StreamManager {
        &self.streams
    }
}

/// Live Synap consumer using the 0.11 pull API.
///
/// Tracks the next offset locally — the SDK's `consume(room, offset, limit)`
/// call is stateless. `ack(offset)` advances the cursor so the next poll
/// starts at `offset + 1`. Data loss is bounded to the in-flight batch if
/// the worker crashes before acking; that tradeoff is spec-06-acceptable
/// until durable consumer groups land server-side.
pub struct LiveSynapConsumer {
    handle: Arc<SynapHandle>,
    tracker: Arc<OffsetTracker>,
}

impl LiveSynapConsumer {
    /// Build a new live consumer.
    pub fn new(handle: Arc<SynapHandle>) -> Self {
        Self {
            handle,
            tracker: Arc::new(OffsetTracker::new()),
        }
    }

    /// Expose the offset tracker (useful for metrics / tests).
    pub fn tracker(&self) -> Arc<OffsetTracker> {
        self.tracker.clone()
    }
}

#[async_trait]
impl SynapConsumer for LiveSynapConsumer {
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>> {
        let offset = self.tracker.current();
        let events: Vec<Event> = self
            .handle
            .streams()
            .consume(room, Some(offset), Some(max))
            .await
            .map_err(|e| anyhow::anyhow!("synap consume: {e}"))?;

        Ok(events
            .into_iter()
            .map(|e| {
                let event_id = e
                    .data
                    .get("event_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                ConsumedMessage {
                    offset: e.offset,
                    kind: e.event,
                    payload: e.data,
                    event_id,
                }
            })
            .collect())
    }

    async fn ack(&self, _room: &str, offset: u64) -> Result<()> {
        self.tracker.advance_past(offset);
        Ok(())
    }
}

/// Live Synap publisher.
pub struct LiveSynapPublisher {
    handle: Arc<SynapHandle>,
}

impl LiveSynapPublisher {
    /// Build a new live publisher.
    pub fn new(handle: Arc<SynapHandle>) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl SynapPublisher for LiveSynapPublisher {
    async fn publish(&self, room: &str, envelope: &Value) -> Result<()> {
        let kind = envelope
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or(room);
        self.handle
            .streams()
            .publish(room, kind, envelope.clone())
            .await
            .map(|_offset| ())
            .map_err(|e| anyhow::anyhow!("synap publish: {e}"))
    }
}

// ---------- Memory consumer / publisher (tests) ------------------------

/// In-memory consumer for tests.
///
/// Pre-loaded with a queue of messages; `next_batch` drains up to `max`,
/// `ack` is a no-op (messages are removed at delivery time — the test
/// decides what to re-queue).
#[derive(Default)]
pub struct MemorySynapConsumer {
    queue: Mutex<VecDeque<ConsumedMessage>>,
}

impl MemorySynapConsumer {
    /// Fresh consumer with an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a message to be delivered by a future `next_batch` call.
    pub fn enqueue(&self, msg: ConsumedMessage) {
        if let Ok(mut q) = self.queue.lock() {
            q.push_back(msg);
        }
    }

    /// Remaining queued messages.
    pub fn remaining(&self) -> usize {
        self.queue.lock().map(|q| q.len()).unwrap_or(0)
    }
}

#[async_trait]
impl SynapConsumer for MemorySynapConsumer {
    async fn next_batch(&self, _room: &str, max: usize) -> Result<Vec<ConsumedMessage>> {
        let mut out = Vec::with_capacity(max);
        if let Ok(mut q) = self.queue.lock() {
            while out.len() < max {
                match q.pop_front() {
                    Some(m) => out.push(m),
                    None => break,
                }
            }
        }
        Ok(out)
    }

    async fn ack(&self, _room: &str, _offset: u64) -> Result<()> {
        Ok(())
    }
}

/// In-memory publisher for tests.
#[derive(Default)]
pub struct MemorySynapPublisher {
    /// Recorded publish calls, `(room, envelope)` in order.
    pub calls: Mutex<Vec<(String, Value)>>,
}

impl MemorySynapPublisher {
    /// Fresh publisher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of the recorded calls.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Filter recorded calls by stream.
    pub fn calls_on(&self, room: &str) -> Vec<Value> {
        self.calls()
            .into_iter()
            .filter(|(r, _)| r == room)
            .map(|(_, v)| v)
            .collect()
    }
}

#[async_trait]
impl SynapPublisher for MemorySynapPublisher {
    async fn publish(&self, room: &str, envelope: &Value) -> Result<()> {
        if let Ok(mut g) = self.calls.lock() {
            g.push((room.to_string(), envelope.clone()));
        }
        Ok(())
    }
}

// ---------- Worker ------------------------------------------------------

/// Embedder worker that drives the Synap → Embedder → Synap pipeline.
pub struct Worker {
    /// Embedder configuration.
    pub config: EmbedderConfig,
    /// Embedder implementation.
    pub embedder: Arc<dyn Embedder>,
    /// Synap consumer.
    pub consumer: Arc<dyn SynapConsumer>,
    /// Synap publisher.
    pub publisher: Arc<dyn SynapPublisher>,
    /// Metrics registry.
    pub metrics: Arc<Metrics>,
    /// Backpressure gauge.
    pub backpressure: Arc<BackpressureState>,
    /// In-memory record of already-processed event ids; keeps at-least-once
    /// delivery from double-embedding during a single worker lifetime.
    processed: Mutex<BTreeSet<String>>,
    /// ADR-012 — optional `MetadataStore` handle for stamping
    /// `event_identity.vec_id` after every successful embed batch.
    /// `None` keeps the legacy worker path (pre-phase13d) running
    /// unchanged. The phase13d boot path in
    /// `cortex-embedder-worker/main.rs` builds the store from the
    /// resolved DB path and chains `with_metadata(store)` after
    /// `Worker::new`.
    pub metadata: Option<Arc<Mutex<cortex_storage::MetadataStore>>>,
}

impl Worker {
    /// Build a new worker.
    pub fn new(
        config: EmbedderConfig,
        embedder: Arc<dyn Embedder>,
        consumer: Arc<dyn SynapConsumer>,
        publisher: Arc<dyn SynapPublisher>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            config,
            embedder,
            consumer,
            publisher,
            metrics,
            backpressure: Arc::new(BackpressureState::new()),
            processed: Mutex::new(BTreeSet::new()),
            metadata: None,
        }
    }

    /// ADR-012 — wire the `MetadataStore` handle for
    /// `event_identity.vec_id` write-back. The handle is shared
    /// across the worker's lifetime; each successful embed batch
    /// takes the mutex briefly to stamp one row.
    pub fn with_metadata(
        mut self,
        metadata: Arc<Mutex<cortex_storage::MetadataStore>>,
    ) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Stream name this worker consumes from.
    pub fn enriched_stream(&self) -> &'static str {
        STREAM_ENRICHED
    }

    /// Single iteration of the run loop — fetches one batch, processes it,
    /// publishes outcomes. Exposed so tests can drive the loop without
    /// spinning an infinite task.
    ///
    /// Returns the number of messages handled in this iteration.
    pub async fn run_once(&self) -> Result<usize> {
        if self.backpressure.is_paused() {
            self.metrics.set_backpressure(true);
            return Ok(0);
        }
        self.metrics.set_backpressure(self.backpressure.is_active());

        let batch = self
            .consumer
            .next_batch(STREAM_ENRICHED, self.config.workers.max(1))
            .await?;
        if batch.is_empty() {
            return Ok(0);
        }

        let processed_count = batch.len();
        for msg in batch {
            // `handle_message` returns an error only on fatal (Rate-limited)
            // back-pressure; other failures publish an invalid envelope and
            // keep draining the batch.
            if let Some(control) = self.handle_message(msg).await? {
                match control {
                    LoopControl::Halt => break,
                }
            }
        }
        Ok(processed_count)
    }

    /// Long-running loop. Polls `run_once`, idling briefly when the room is
    /// empty, and honouring backpressure pauses. Exits cleanly when
    /// `shutdown.load(Relaxed)` becomes true.
    pub async fn run_forever(&self, shutdown: Arc<AtomicBool>) -> Result<()> {
        tracing::info!(
            workers = self.config.workers,
            chunker_concurrency = self.config.chunker_concurrency,
            upsert_batch = self.config.upsert_batch,
            vectorizer = %self.config.vectorizer_url,
            synap = %self.config.synap_url,
            "cortex-embedder worker started"
        );
        while !shutdown.load(Ordering::Relaxed) {
            if self.backpressure.is_paused() {
                self.metrics.set_backpressure(true);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            let handled = match self.run_once().await {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "run_once failed; backing off");
                    self.metrics.incr_vectorizer_errors(1);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            // Phase8b — stamp jobs_processed + last_job_ts. The bump
            // is conditional on handled > 0 so an idle poll never
            // looks like ongoing work to the freshness aggregator.
            self.metrics.record_jobs_processed(handled as u64);
            if handled == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        tracing::info!("cortex-embedder worker stopped");
        Ok(())
    }

    /// Spawn `config.workers` copies of the loop onto the current Tokio
    /// runtime and join them. Every copy shares the embedder, consumer, and
    /// publisher via `Arc`, so the backpressure gauge and metrics are
    /// naturally shared too.
    pub async fn run_pool(self: Arc<Self>, shutdown: Arc<AtomicBool>) -> Result<()> {
        let count = self.config.workers.max(1);
        let mut handles = Vec::with_capacity(count);
        for idx in 0..count {
            let this = self.clone();
            let shut = shutdown.clone();
            handles.push(tokio::spawn(async move {
                tracing::debug!(worker = idx, "pool worker starting");
                this.run_forever(shut).await
            }));
        }
        for h in handles {
            if let Err(e) = h.await {
                tracing::warn!(error = %e, "worker join failed");
            }
        }
        Ok(())
    }

    /// Dispatch a single message. Returns `Ok(Some(Halt))` when the caller
    /// should stop draining the current batch (rate-limit observed).
    async fn handle_message(&self, msg: ConsumedMessage) -> Result<Option<LoopControl>> {
        let offset = msg.offset;

        // 1. Deserialize.
        let event = match serde_json::from_value::<EnrichedEvent>(msg.payload.clone()) {
            Ok(event) => event,
            Err(err) => {
                let victim_id = msg
                    .event_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                self.publish_invalid(&victim_id, "deserialize_failed", &err.to_string())
                    .await;
                self.ack(STREAM_ENRICHED, offset).await;
                return Ok(None);
            }
        };

        // 2. In-memory dedup — at-least-once delivery guard. Drop the
        //    mutex guard before awaiting so the future stays `Send`.
        let is_replay = {
            let mut seen = match self.processed.lock() {
                Ok(g) => g,
                Err(_) => {
                    // Poisoned mutex — treat as "not seen" rather than panic;
                    // the Vectorizer layer will still dedupe via chunk ids.
                    return Ok(None);
                }
            };
            !seen.insert(event.event_id.clone())
        };
        if is_replay {
            tracing::debug!(event_id = %event.event_id, "skipping already-processed event");
            self.ack(STREAM_ENRICHED, offset).await;
            return Ok(None);
        }

        // 3. Embed.
        let report = match self
            .embedder
            .embed_batch(std::slice::from_ref(&event))
            .await
        {
            Ok(r) => r,
            Err(err) => {
                // Fatal embedder error (not a per-event issue).
                tracing::warn!(error = %err, event_id = %event.event_id, "embed_batch failed");
                self.metrics.incr_vectorizer_errors(1);
                self.publish_invalid(&event.event_id, "embedder_error", &err.to_string())
                    .await;
                self.ack(STREAM_ENRICHED, offset).await;
                return Ok(None);
            }
        };

        // 4. Classify the report.
        if report.errors.is_empty() {
            self.publish_success(&event, &report).await;
            // ADR-012 §3.1 — stamp `event_identity.vec_id` with the
            // representative server id (first chunk wins). The
            // `vec_id` column is per-event (1:1) so a many-chunk
            // event collapses to a single representative for the
            // doctor + forget lookups; subsequent chunks share the
            // dedup_key path the Vectorizer SDK already exposes.
            self.stamp_identity_after_success(&event, &report);
            self.ack(STREAM_ENRICHED, offset).await;
            self.backpressure.record_success();
            return Ok(None);
        }

        // Report has per-event errors. The worker processes one event per
        // message, so a single-error classification is well-defined.
        let primary_error = &report.errors[0];
        match primary_error {
            EmbedError::OversizeWithoutSummary { event_id } => {
                self.metrics.incr_oversize_without_summary();
                self.publish_invalid(event_id, "oversize_without_summary", "")
                    .await;
                self.ack(STREAM_ENRICHED, offset).await;
                Ok(None)
            }
            EmbedError::Chunker { event_id, detail } => {
                tracing::warn!(event_id = %event_id, detail = %detail, "embed chunker_failed");
                self.publish_invalid(event_id, "chunker_failed", detail)
                    .await;
                self.ack(STREAM_ENRICHED, offset).await;
                Ok(None)
            }
            EmbedError::Vectorizer { event_id, detail } => {
                tracing::warn!(event_id = %event_id, detail = %detail, "embed vectorizer_error");
                self.metrics.incr_vectorizer_errors(1);
                // Detect rate-limit signatures coming back from the client
                // so we can apply backpressure without the trait surface
                // needing a dedicated discriminator.
                let lower = detail.to_ascii_lowercase();
                if lower.contains("rate") || lower.contains("429") {
                    self.backpressure.record_rate_limit();
                    self.metrics.set_backpressure(true);
                    // Do NOT ack — let the message redeliver when we un-pause.
                    // Also remove the id from the processed set so re-delivery
                    // doesn't get short-circuited by dedup. Mutex guard is
                    // dropped at the end of this block before the next await.
                    {
                        if let Ok(mut seen) = self.processed.lock() {
                            seen.remove(event_id);
                        }
                    }
                    return Ok(Some(LoopControl::Halt));
                }
                self.publish_invalid(event_id, "vectorizer_error", detail)
                    .await;
                self.ack(STREAM_ENRICHED, offset).await;
                Ok(None)
            }
        }
    }

    async fn publish_success(&self, event: &EnrichedEvent, report: &EmbedReport) {
        // Optional `server_ids` array carries the server-assigned UUIDs for
        // newly-written chunks so downstream consumers (graph writer,
        // query API) can join back to Vectorizer without re-querying.
        // Keyed-nullable: empty vec when the whole batch was deduped.
        let server_ids: Vec<&str> = report
            .new_records
            .iter()
            .map(|r| r.server_id.as_str())
            .collect();
        let envelope = json!({
            "kind": "embedded",
            "event_id": event.event_id,
            "chunks_written": report.chunks_written,
            "chunks_deduped": report.chunks_deduped,
            "by_collection": report.by_collection,
            "latency_ms": report.latency_ms,
            "server_ids": server_ids,
        });
        if let Err(e) = self.publisher.publish(STREAM_EMBEDDED, &envelope).await {
            tracing::warn!(error = %e, "failed to publish embedded envelope");
        }
    }

    async fn publish_invalid(&self, event_id: &str, cause: &str, detail: &str) {
        let envelope = json!({
            "kind": "invalid",
            "event_id": event_id,
            "cause": cause,
            "detail": detail,
        });
        if let Err(e) = self.publisher.publish(STREAM_INVALID, &envelope).await {
            tracing::warn!(error = %e, "failed to publish invalid envelope");
        }
    }

    async fn ack(&self, room: &str, offset: u64) {
        if let Err(e) = self.consumer.ack(room, offset).await {
            tracing::debug!(error = %e, "consumer ack failed");
        }
    }

    /// ADR-012 §3.1 — stamp `event_identity.vec_id` with the
    /// representative server id for `event` after a successful
    /// embed batch. Best-effort: a poisoned mutex or a SQLite
    /// failure logs at WARN but does NOT undo the embed. The
    /// representative id is the FIRST entry in `new_records`
    /// (chunk insertion order); the `vec_id` column is per-event
    /// (1:1) so a many-chunk event collapses to one canonical
    /// vector for downstream lookups. An empty `new_records`
    /// (every chunk was deduped) is a no-op — the row was already
    /// stamped on a prior embed.
    fn stamp_identity_after_success(&self, event: &EnrichedEvent, report: &EmbedReport) {
        let Some(metadata) = self.metadata.as_ref() else {
            return;
        };
        let Some(first) = report.new_records.first() else {
            return;
        };
        let event_id = event.event_id.as_str();
        let server_id = first.server_id.as_str();
        if event_id.is_empty() || server_id.is_empty() {
            return;
        }
        match metadata.lock() {
            Ok(guard) => {
                use cortex_storage::IdentityIndex as _;
                let idx = cortex_storage::SqliteIdentityIndex::new(guard.conn());
                if let Err(e) = idx.upsert_identity(
                    event_id,
                    cortex_storage::Backend::Vectorizer,
                    server_id,
                ) {
                    tracing::warn!(
                        event_id = %event_id,
                        vec_id = %server_id,
                        error = %e,
                        "identity index: vectorizer upsert failed (post-embed)"
                    );
                }
            }
            Err(poisoned) => {
                tracing::warn!(
                    event_id = %event_id,
                    error = %poisoned,
                    "identity index: metadata mutex poisoned; skipping vectorizer upsert"
                );
            }
        }
    }
}

/// Control signal returned by `handle_message` when the caller should stop
/// draining the current batch early.
#[derive(Debug, Clone, Copy)]
enum LoopControl {
    /// Halt the current batch (rate-limit observed).
    Halt,
}
