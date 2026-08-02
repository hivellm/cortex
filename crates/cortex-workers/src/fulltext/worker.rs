//! Full-text indexer worker.
//!
//! Consumes `cortex.events.enriched` from Synap, runs each batch
//! through [`crate::FulltextIndexer`], and republishes per-batch
//! [`crate::IndexReport`]s on `cortex.events.fulltext_indexed`.
//! 4xx rejects route to `cortex.events.invalid`; sustained 503/429
//! pressure flips a backpressure gauge that pauses consumption.
//!
//! Same shape as [`crate::graph::worker`] and [`crate::embedder::worker`]
//! — the three Phase-1 workers share the Synap pull-API integration so
//! operations work the same way across all of them.

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

use super::config::FulltextConfig;
use super::indexer::FulltextIndexer;
use super::meili_client::MeiliError;
use super::metrics::Metrics;
use crate::embedder::EnrichedEvent;

/// Default stream name for enriched events the worker consumes.
pub const STREAM_ENRICHED: &str = "cortex.events.enriched";
/// Stream name for per-batch index reports.
pub const STREAM_FULLTEXT_INDEXED: &str = "cortex.events.fulltext_indexed";
/// Stream name for events that could not be indexed.
pub const STREAM_INVALID: &str = "cortex.events.invalid";

/// Threshold above which a sustained transient Meili error halts
/// consumption entirely.
pub const BACKPRESSURE_SOAK: Duration = Duration::from_secs(30);

/// Phase28 §1.4 — half-open retry window. While paused, the state
/// reads as un-paused again once the MOST RECENT transient is older
/// than this, letting the loop attempt one probe batch per window so
/// `record_success` can ever fire again after Meilisearch recovers.
pub const BACKPRESSURE_RETRY: Duration = Duration::from_secs(60);

// ---------- Consumer / Publisher abstraction ---------------------------

/// One message delivered by a [`SynapConsumer`].
#[derive(Debug, Clone)]
pub struct ConsumedMessage {
    /// Stream offset.
    pub offset: u64,
    /// Event kind label from the envelope.
    pub kind: String,
    /// Event payload.
    pub payload: Value,
    /// Pre-extracted event id when present.
    pub event_id: Option<String>,
}

/// Synap consumer abstraction.
#[async_trait]
pub trait SynapConsumer: Send + Sync + 'static {
    /// Fetch up to `max` un-processed messages from `room`.
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>>;
    /// Mark a message as processed.
    async fn ack(&self, room: &str, offset: u64) -> Result<()>;
}

/// Synap publisher abstraction.
#[async_trait]
pub trait SynapPublisher: Send + Sync + 'static {
    /// Publish `envelope` onto `room`.
    async fn publish(&self, room: &str, envelope: &Value) -> Result<()>;
}

// ---------- OffsetTracker -----------------------------------------------

/// Per-room offset cursor used by [`LiveSynapConsumer`].
#[derive(Debug, Default)]
pub struct OffsetTracker {
    next: AtomicU64,
}

impl OffsetTracker {
    /// Fresh tracker.
    pub fn new() -> Self {
        Self::default()
    }
    /// Read current cursor.
    pub fn current(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }
    /// Advance past `offset`.
    pub fn advance_past(&self, offset: u64) {
        loop {
            let cur = self.next.load(Ordering::Relaxed);
            let proposed = offset.saturating_add(1).max(cur);
            match self
                .next
                .compare_exchange(cur, proposed, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }
}

// ---------- BackpressureState -----------------------------------------

/// Tracks whether Meilisearch has been returning transient errors
/// long enough to halt consumption.
///
/// Phase28 §1.4 — the pause is half-open, not permanent (same latent
/// deadlock as the graph worker's 2026-06-27 stall: a paused worker
/// never attempts an index batch, so `record_success` could never fire
/// and the pause outlived Meilisearch's recovery). The pause expires
/// [`BACKPRESSURE_RETRY`] after the MOST RECENT transient so a probe
/// batch can flow: success disarms, another transient re-arms.
#[derive(Debug, Default)]
pub struct BackpressureState {
    since: Mutex<Option<Instant>>,
    /// Most recent transient — drives the half-open retry window.
    last_transient: Mutex<Option<Instant>>,
    active: AtomicBool,
}

impl BackpressureState {
    /// Fresh, healthy state.
    pub fn new() -> Self {
        Self::default()
    }
    /// Record a transient observation. Arms the gauge, stamps the
    /// first-observation instant once, and refreshes the
    /// most-recent-observation instant every time.
    pub fn record_transient(&self) {
        if let Ok(mut guard) = self.since.lock() {
            if guard.is_none() {
                *guard = Some(Instant::now());
            }
        }
        if let Ok(mut guard) = self.last_transient.lock() {
            *guard = Some(Instant::now());
        }
        self.active.store(true, Ordering::Relaxed);
    }
    /// Record a successful index batch.
    pub fn record_success(&self) {
        if let Ok(mut guard) = self.since.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.last_transient.lock() {
            *guard = None;
        }
        self.active.store(false, Ordering::Relaxed);
    }
    /// Whether the gauge is currently armed.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
    /// Whether the soak has reached the pause threshold AND the most
    /// recent transient is younger than [`BACKPRESSURE_RETRY`] (the
    /// half-open probe window — see the struct docs).
    pub fn is_paused(&self) -> bool {
        let soaked = match self.since.lock() {
            Ok(g) => matches!(*g, Some(t) if t.elapsed() >= BACKPRESSURE_SOAK),
            Err(_) => false,
        };
        if !soaked {
            return false;
        }
        match self.last_transient.lock() {
            Ok(g) => matches!(*g, Some(t) if t.elapsed() < BACKPRESSURE_RETRY),
            Err(_) => false,
        }
    }
    /// Force-arm with a specific instant (test hook).
    #[doc(hidden)]
    pub fn force_since(&self, instant: Instant) {
        if let Ok(mut guard) = self.since.lock() {
            *guard = Some(instant);
        }
        if let Ok(mut guard) = self.last_transient.lock() {
            *guard = Some(Instant::now());
        }
        self.active.store(true, Ordering::Relaxed);
    }
    /// Force the most-recent-transient instant independently of
    /// `since` (test hook for the half-open window).
    #[doc(hidden)]
    pub fn force_last_transient(&self, instant: Instant) {
        if let Ok(mut guard) = self.last_transient.lock() {
            *guard = Some(instant);
        }
    }
}

// ---------- Live Synap consumer / publisher ---------------------------

/// Synap client handle.
pub struct SynapHandle {
    streams: StreamManager,
}

impl SynapHandle {
    /// Connect to Synap at `base_url`.
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

/// Live Synap consumer.
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
    /// Expose the offset tracker.
    pub fn tracker(&self) -> Arc<OffsetTracker> {
        self.tracker.clone()
    }
}

#[async_trait]
impl SynapConsumer for LiveSynapConsumer {
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>> {
        let offset = self.tracker.current();
        // A Synap restart drops every ephemeral room; the upstream
        // publisher recreates it lazily on its next publish. Until
        // then, consuming a missing room must read as an empty batch
        // (not a hard error) so the worker idles instead of
        // error-spinning. Mirrors the classifier consumer and the
        // ingestion publisher's room-lifecycle handling.
        // Phase29b §2.1 — self-heal: on "Room not found", re-declare
        // the room (idempotent get_or_create) and retry ONCE within
        // this poll, so a synap restart resumes the pipeline without a
        // worker restart and the server stops logging an ERROR per
        // poll. A still-missing room after the re-declare degrades to
        // the previous empty-batch idle.
        let mut redeclared = false;
        let events: Vec<Event> = loop {
            match self
                .handle
                .streams()
                .consume(room, Some(offset), Some(max))
                .await
            {
                Ok(v) => break v,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("not found") && msg.contains("Room") {
                        if !redeclared {
                            redeclared = true;
                            if self
                                .handle
                                .streams()
                                .get_or_create_room(room, None)
                                .await
                                .is_ok()
                            {
                                continue;
                            }
                        }
                        return Ok(Vec::new());
                    }
                    return Err(anyhow::anyhow!("synap consume: {e}"));
                }
            }
        };
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
        // Publish-or-create-then-republish. Synap does not auto-create
        // rooms, so the first publish to an output stream
        // (fulltext_indexed) after a fresh or restarted Synap returns
        // "Room … not found". Cortex owns the room lifecycle — create
        // it and retry once. Mirrors the classifier and ingestion
        // publishers.
        match self
            .handle
            .streams()
            .publish(room, kind, envelope.clone())
            .await
        {
            Ok(_offset) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if !(msg.contains("not found") && msg.contains("Room")) {
                    return Err(anyhow::anyhow!("synap publish: {e}"));
                }
                if let Err(create_err) = self.handle.streams().create_room(room, None).await {
                    tracing::debug!(
                        room,
                        error = %create_err,
                        "synap create_room returned an error; will still retry publish"
                    );
                }
                self.handle
                    .streams()
                    .publish(room, kind, envelope.clone())
                    .await
                    .map(|_offset| ())
                    .map_err(|e2| {
                        anyhow::anyhow!(
                            "synap publish after create_room still failed: {e2} (initial: {msg})"
                        )
                    })
            }
        }
    }
}

// ---------- Memory consumer / publisher (tests) -----------------------

/// In-memory consumer for tests.
#[derive(Default)]
pub struct MemorySynapConsumer {
    queue: Mutex<VecDeque<ConsumedMessage>>,
}

impl MemorySynapConsumer {
    /// Fresh consumer.
    pub fn new() -> Self {
        Self::default()
    }
    /// Enqueue a message.
    pub fn enqueue(&self, msg: ConsumedMessage) {
        if let Ok(mut q) = self.queue.lock() {
            q.push_back(msg);
        }
    }
    /// Remaining messages.
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
    /// Recorded `(room, envelope)` pairs.
    pub calls: Mutex<Vec<(String, Value)>>,
}

impl MemorySynapPublisher {
    /// Fresh publisher.
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot of recorded calls.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().map(|g| g.clone()).unwrap_or_default()
    }
    /// Filter by room.
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

// ---------- Worker -----------------------------------------------------

/// Full-text indexer worker that drives the Synap → indexer →
/// Meilisearch → Synap pipeline.
pub struct Worker {
    config: FulltextConfig,
    indexer: Arc<dyn FulltextIndexer>,
    consumer: Arc<dyn SynapConsumer>,
    publisher: Arc<dyn SynapPublisher>,
    /// Shared metrics registry. Public so the binary entrypoint can
    /// read the counters from its admin `/healthz` listener
    /// (phase8a) without threading a separate handle.
    pub metrics: Arc<Metrics>,
    backpressure: Arc<BackpressureState>,
    processed: Mutex<BTreeSet<String>>,
    /// ADR-012 — optional `MetadataStore` handle for stamping
    /// `event_identity.meili_id` after every successful index
    /// batch. `None` keeps the legacy worker path (pre-phase13d)
    /// running unchanged. The phase13d boot path in
    /// `cortex-fulltext-worker/main.rs` builds the store from the
    /// resolved DB path and chains `with_metadata(store)` after
    /// `Worker::new`.
    metadata: Option<Arc<Mutex<cortex_storage::MetadataStore>>>,
}

impl Worker {
    /// Build a new worker.
    pub fn new(
        config: FulltextConfig,
        indexer: Arc<dyn FulltextIndexer>,
        consumer: Arc<dyn SynapConsumer>,
        publisher: Arc<dyn SynapPublisher>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            config,
            indexer,
            consumer,
            publisher,
            metrics,
            backpressure: Arc::new(BackpressureState::new()),
            processed: Mutex::new(BTreeSet::new()),
            metadata: None,
        }
    }

    /// ADR-012 — wire the `MetadataStore` handle for
    /// `event_identity.meili_id` write-back. Held for the worker's
    /// lifetime; each successful batch takes the mutex once to
    /// stamp every event in the batch.
    pub fn with_metadata(mut self, metadata: Arc<Mutex<cortex_storage::MetadataStore>>) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Borrow runtime configuration.
    pub fn config(&self) -> &FulltextConfig {
        &self.config
    }

    /// Borrow metrics registry.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Borrow backpressure gauge.
    pub fn backpressure(&self) -> &Arc<BackpressureState> {
        &self.backpressure
    }

    /// Stream name this worker consumes from.
    pub fn enriched_stream(&self) -> &'static str {
        STREAM_ENRICHED
    }

    /// Single iteration.
    pub async fn run_once(&self) -> Result<usize> {
        if self.backpressure.is_paused() {
            self.metrics.set_backpressure(true);
            return Ok(0);
        }
        self.metrics.set_backpressure(self.backpressure.is_active());

        let batch_size = self.config.upsert_batch.max(1);
        let msgs = self
            .consumer
            .next_batch(STREAM_ENRICHED, batch_size)
            .await?;
        let received = msgs.len();
        self.handle_batch(msgs).await?;
        Ok(received)
    }

    /// Phase14h — delegate to the shared `synap_worker` runtime.
    /// See [`crate::synap_worker::run_pool`].
    pub async fn run_pool(self: Arc<Self>, shutdown: Arc<AtomicBool>) -> Result<()> {
        crate::synap_worker::run_pool(self, shutdown)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn handle_batch(&self, msgs: Vec<ConsumedMessage>) -> Result<()> {
        if msgs.is_empty() {
            return Ok(());
        }

        // Deserialize + dedupe.
        let mut events: Vec<EnrichedEvent> = Vec::with_capacity(msgs.len());
        let mut to_ack: Vec<u64> = Vec::with_capacity(msgs.len());
        for msg in &msgs {
            match serde_json::from_value::<EnrichedEvent>(msg.payload.clone()) {
                Ok(event) => {
                    let is_replay = match self.processed.lock() {
                        Ok(mut seen) => !seen.insert(event.event_id.clone()),
                        Err(_) => false,
                    };
                    if is_replay {
                        tracing::debug!(event_id = %event.event_id, "skipping already-processed event");
                        to_ack.push(msg.offset);
                        continue;
                    }
                    events.push(event);
                    to_ack.push(msg.offset);
                }
                Err(err) => {
                    let victim_id = msg
                        .event_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    self.publish_invalid(&victim_id, "deserialize_failed", &err.to_string())
                        .await;
                    to_ack.push(msg.offset);
                }
            }
        }

        if events.is_empty() {
            for offset in &to_ack {
                self.ack_offset(*offset).await;
            }
            return Ok(());
        }

        let event_count = events.len();
        match self.indexer.index_batch(&events).await {
            Ok(report) => {
                tracing::info!(
                    events = event_count,
                    documents_upserted = report.documents_upserted,
                    documents_skipped = report.documents_skipped,
                    documents_truncated = report.documents_truncated,
                    latency_ms = report.latency_ms,
                    outcome = "ok",
                    "fulltext batch indexed"
                );
                self.publish_report(&events, &report).await;
                // ADR-012 §3.2 — stamp `event_identity.meili_id`
                // for every event the batch indexed. The Meili
                // doc id for live envelopes IS `event_id` per
                // `Document::id` (`docs/specs/08-fulltext.md`
                // §Index keys); using `event_id` as the native
                // value keeps `lookup_by_native(Meili, event_id)`
                // a one-row lookup for the doctor + forget paths.
                self.stamp_identity_after_success(&events);
                self.backpressure.record_success();
                for offset in &to_ack {
                    self.ack_offset(*offset).await;
                }
            }
            Err(MeiliError::TransientError(detail)) => {
                tracing::warn!(
                    events = event_count,
                    outcome = "transient",
                    detail = %detail,
                    "transient meili error; engaging backpressure"
                );
                self.backpressure.record_transient();
                self.metrics.set_backpressure(true);
                self.metrics.incr_errors("transient");
                self.untrack(&events);
            }
            Err(MeiliError::Rejected { status, detail }) => {
                tracing::warn!(
                    events = event_count,
                    status,
                    outcome = "rejected",
                    detail = %detail,
                    "meili rejected batch; routing to invalid"
                );
                self.metrics.incr_errors(&format!("status_{status}"));
                for evt in &events {
                    self.publish_invalid(&evt.event_id, "meili_rejected", &detail)
                        .await;
                }
                for offset in &to_ack {
                    self.ack_offset(*offset).await;
                }
            }
            Err(MeiliError::TaskNotSucceeded { task, status }) => {
                let reason = format!("{status:?}");
                tracing::warn!(
                    events = event_count,
                    task,
                    status = %reason,
                    outcome = "task_failed",
                    "meili task did not succeed"
                );
                self.metrics.incr_task_failure(&reason);
                for evt in &events {
                    self.publish_invalid(&evt.event_id, "task_failed", &reason)
                        .await;
                }
                for offset in &to_ack {
                    self.ack_offset(*offset).await;
                }
            }
            Err(other) => {
                tracing::warn!(
                    events = event_count,
                    outcome = "error",
                    error = %other,
                    "index_batch failed"
                );
                self.metrics.incr_errors("other");
                self.untrack(&events);
            }
        }

        Ok(())
    }

    async fn publish_report(&self, events: &[EnrichedEvent], report: &super::indexer::IndexReport) {
        let event_ids: Vec<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
        let envelope = json!({
            "kind": "fulltext_indexed",
            "event_ids": event_ids,
            "documents_upserted": report.documents_upserted,
            "documents_skipped": report.documents_skipped,
            "documents_truncated": report.documents_truncated,
            "by_index": report.by_index,
            "latency_ms": report.latency_ms,
        });
        if let Err(e) = self
            .publisher
            .publish(STREAM_FULLTEXT_INDEXED, &envelope)
            .await
        {
            tracing::warn!(error = %e, "failed to publish fulltext_indexed envelope");
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

    async fn ack_offset(&self, offset: u64) {
        if let Err(e) = self.consumer.ack(STREAM_ENRICHED, offset).await {
            tracing::debug!(error = %e, "consumer ack failed");
        }
    }

    fn untrack(&self, events: &[EnrichedEvent]) {
        if let Ok(mut seen) = self.processed.lock() {
            for e in events {
                seen.remove(&e.event_id);
            }
        }
    }

    /// ADR-012 §3.2 — stamp `event_identity.meili_id` for every
    /// event in `events` after a successful `index_batch`. Best-
    /// effort: a poisoned mutex or per-row SQLite failure logs at
    /// WARN but does NOT undo the index batch. Empty `events` is
    /// a no-op (every msg in the batch was an upstream parse
    /// failure routed to `invalid` already).
    ///
    /// Native value: `event_id` itself. The Meili doc id for
    /// live envelopes IS `event_id` (`Document::id` contract per
    /// `docs/specs/08-fulltext.md` §Index keys), and bootstrap
    /// envelopes still resolve via Meili's `event_id`
    /// filterableAttribute — using `event_id` as the native value
    /// keeps `lookup_by_native(Meili, event_id)` a one-row hit
    /// from the doctor + forget paths.
    fn stamp_identity_after_success(&self, events: &[EnrichedEvent]) {
        let Some(metadata) = self.metadata.as_ref() else {
            return;
        };
        if events.is_empty() {
            return;
        }
        match metadata.lock() {
            Ok(guard) => {
                use cortex_storage::IdentityIndex as _;
                let idx = cortex_storage::SqliteIdentityIndex::new(guard.conn());
                for event in events {
                    let event_id = event.event_id.as_str();
                    if event_id.is_empty() {
                        continue;
                    }
                    if let Err(e) =
                        idx.upsert_identity(event_id, cortex_storage::Backend::Meili, event_id)
                    {
                        tracing::warn!(
                            event_id = %event_id,
                            error = %e,
                            "identity index: meili upsert failed (post-index)"
                        );
                    }
                }
            }
            Err(poisoned) => {
                tracing::warn!(
                    error = %poisoned,
                    "identity index: metadata mutex poisoned; skipping meili upserts"
                );
            }
        }
    }
}

#[async_trait]
impl crate::synap_worker::SynapWorker for Worker {
    fn worker_name(&self) -> &'static str {
        "fulltext"
    }

    fn pool_size(&self) -> usize {
        self.config.workers.max(1)
    }

    fn idle_duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.config.flush_ms.max(50))
    }

    async fn run_once(&self) -> anyhow::Result<usize> {
        Worker::run_once(self).await
    }

    fn backpressure(&self) -> crate::synap_worker::BackpressureGate {
        if self.backpressure.is_paused() {
            crate::synap_worker::BackpressureGate::Paused
        } else {
            self.metrics.set_backpressure(self.backpressure.is_active());
            crate::synap_worker::BackpressureGate::Active
        }
    }

    fn on_run_once_ok(&self, handled: usize) {
        self.metrics.record_jobs_processed(handled as u64);
    }

    fn on_run_once_err(&self, _err: &anyhow::Error, _consecutive: u32) {
        self.metrics.incr_errors("other");
    }

    fn on_backpressure_pause(&self) {
        self.metrics.set_backpressure(true);
    }
}
