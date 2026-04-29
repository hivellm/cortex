//! Graph-writer worker.
//!
//! Consumes `cortex.events.enriched` from Synap, batches a window of
//! events through the [`crate::GraphWriter`], and republishes the
//! per-batch [`crate::GraphWriteReport`] on `cortex.events.graphed`.
//! Constraint violations dead-letter to `cortex.events.invalid`;
//! transient Nexus 5xx pressure flips a backpressure gauge that pauses
//! consumption (spec 07 §Failure modes).
//!
//! Mirrors the Synap pull-API integration in
//! [`crate::embedder::worker`] — same `OffsetTracker`,
//! `BackpressureState`, and Memory consumer/publisher shape. Synap 0.11
//! has no durable consumer-group surface in the SDK; the worker tracks
//! the next offset locally and dedupes by `event_id` in-memory inside a
//! single process lifetime. When Synap grows server-side cursors the
//! tracker is the only thing that needs swapping.
//!
//! ## Out-of-order handling (spec 07 §Failure modes)
//!
//! `tool_call.*` events arrive carrying `parent_event_id` that points at
//! the parent `Turn`. When the parent has not been seen yet, we hold the
//! event in [`OutOfOrderBuffer`] for up to
//! `CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS` (default 30 s). Once the
//! `turn.start` arrives we flush the buffered children alongside it; if
//! the timeout elapses first we synthesise a Turn node carrying
//! `orphan: true` so the dashboard can surface it without dropping the
//! children.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use cortex_core::events::Kind;
use serde_json::{json, Value};
use synap_sdk::stream::StreamManager;
use synap_sdk::types::Event;
use synap_sdk::{SynapClient, SynapConfig};

use super::config::GraphConfig;
use super::metrics::Metrics;
use super::nexus_client::GraphClientError;
use super::patch::{GraphPatch, NodeOp};
use super::writer::GraphWriter;
use crate::embedder::EnrichedEvent;

/// Default stream name for enriched events the writer consumes.
pub const STREAM_ENRICHED: &str = "cortex.events.enriched";
/// Stream name for per-batch graph-write reports.
pub const STREAM_GRAPHED: &str = "cortex.events.graphed";
/// Stream name for events that could not be written to the graph.
pub const STREAM_INVALID: &str = "cortex.events.invalid";

/// Threshold above which a sustained transient Nexus error halts
/// consumption entirely. Mirrors spec 06's value.
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

    /// Mark a message as processed.
    async fn ack(&self, room: &str, offset: u64) -> Result<()>;
}

// ---------- Publisher abstraction ---------------------------------------

/// Synap-publisher abstraction.
#[async_trait]
pub trait SynapPublisher: Send + Sync + 'static {
    /// Publish `envelope` onto `room`.
    async fn publish(&self, room: &str, envelope: &Value) -> Result<()>;
}

// ---------- Offset tracking --------------------------------------------

/// Per-room offset cursor used by [`LiveSynapConsumer`]. Thread-safe.
#[derive(Debug, Default)]
pub struct OffsetTracker {
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
            match self.next.compare_exchange(
                current,
                proposed,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }
}

// ---------- Backpressure -----------------------------------------------

/// Tracks whether Nexus has been returning transient 5xx errors long
/// enough that the worker should stop consuming new messages.
#[derive(Debug, Default)]
pub struct BackpressureState {
    /// Instant the first transient observation became "live", or `None`
    /// when Nexus is currently healthy.
    since: Mutex<Option<Instant>>,
    /// Whether the gauge is currently armed.
    active: AtomicBool,
}

impl BackpressureState {
    /// Fresh, healthy state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a transient observation. No-ops if already armed.
    pub fn record_transient(&self) {
        if let Ok(mut guard) = self.since.lock() {
            if guard.is_none() {
                *guard = Some(Instant::now());
            }
        }
        self.active.store(true, Ordering::Relaxed);
    }

    /// Record a successful write; clears the timestamp and disarms the
    /// gauge.
    pub fn record_success(&self) {
        if let Ok(mut guard) = self.since.lock() {
            *guard = None;
        }
        self.active.store(false, Ordering::Relaxed);
    }

    /// Raw active flag — `true` whenever a transient has been observed
    /// more recently than a success.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Return `true` once the transient condition has persisted for at
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

    /// Force-arm the state at a specific `since` instant. Used by tests
    /// to avoid real 30 s sleeps.
    #[doc(hidden)]
    pub fn force_since(&self, instant: Instant) {
        if let Ok(mut guard) = self.since.lock() {
            *guard = Some(instant);
        }
        self.active.store(true, Ordering::Relaxed);
    }
}

// ---------- Live Synap consumer / publisher ----------------------------

/// Synap client handle built from the writer configuration.
pub struct SynapHandle {
    streams: StreamManager,
}

impl SynapHandle {
    /// Connect to Synap at `base_url` and return a handle for stream I/O.
    pub fn new(base_url: &str) -> Result<Self> {
        let cfg = SynapConfig::new(base_url);
        let client =
            SynapClient::new(cfg).map_err(|e| anyhow::anyhow!("synap client: {e}"))?;
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

// ---------- Out-of-order buffer ----------------------------------------

/// Result of feeding a batch through the [`OutOfOrderBuffer`]. Splits the
/// incoming events into the subset that's ready to write now plus the
/// list of Turn ids that timed out and need an orphan node fabricated
/// alongside the flush.
#[derive(Debug, Default)]
pub struct ReadyBatch {
    /// Events whose dependencies (parent Turn, when applicable) are
    /// satisfied or whose buffer timeout has elapsed.
    pub events: Vec<EnrichedEvent>,
    /// Turn ids referenced by aged-out child events whose parent
    /// `turn.start` never arrived. Each one upserts a Turn node carrying
    /// `orphan: true`.
    pub orphan_turn_ids: Vec<String>,
}

/// Buffer for `tool_call.*` events that arrive before their parent
/// `turn.start`. Holds them up to `timeout`, then fabricates an orphan
/// Turn so the children can be written without losing their context.
#[derive(Debug)]
pub struct OutOfOrderBuffer {
    buffered: Mutex<VecDeque<(EnrichedEvent, Instant)>>,
    seen_turns: Mutex<BTreeSet<String>>,
    timeout: Duration,
}

impl OutOfOrderBuffer {
    /// Build a buffer with the given timeout.
    pub fn new(timeout: Duration) -> Self {
        Self {
            buffered: Mutex::new(VecDeque::new()),
            seen_turns: Mutex::new(BTreeSet::new()),
            timeout,
        }
    }

    /// Number of events currently held back.
    pub fn pending(&self) -> usize {
        self.buffered.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Whether the buffer has already seen a `turn.start` with the
    /// given id (mainly useful for tests).
    pub fn has_seen(&self, turn_id: &str) -> bool {
        self.seen_turns
            .lock()
            .map(|g| g.contains(turn_id))
            .unwrap_or(false)
    }

    /// Feed `events` through the buffer. Turn events are recorded as
    /// "seen"; events depending on an unseen Turn parent get held; the
    /// buffer is then swept for events whose parent has now arrived or
    /// whose timeout has elapsed.
    pub fn ingest(&self, events: Vec<EnrichedEvent>) -> ReadyBatch {
        // 1. Record incoming Turn events so dependent children flush.
        if let Ok(mut seen) = self.seen_turns.lock() {
            for e in &events {
                if matches!(e.kind, Kind::Turn) {
                    seen.insert(e.event_id.clone());
                }
            }
        }

        // 2. Split incoming into ready vs needs-buffering.
        let mut ready = Vec::new();
        let mut to_buffer = Vec::new();
        let now = Instant::now();
        for e in events {
            if self.is_dependency_satisfied(&e) {
                ready.push(e);
            } else {
                to_buffer.push((e, now));
            }
        }

        // 3. Append to the buffer, then sweep — events whose parent has
        //    arrived flush; events past the timeout flush with an
        //    orphan-Turn injection.
        let mut orphan_turn_ids: Vec<String> = Vec::new();
        if let Ok(mut buf) = self.buffered.lock() {
            buf.extend(to_buffer);
            let mut still_buffered: VecDeque<(EnrichedEvent, Instant)> = VecDeque::new();
            while let Some((event, queued_at)) = buf.pop_front() {
                if self.is_dependency_satisfied(&event) {
                    ready.push(event);
                } else if now.duration_since(queued_at) >= self.timeout {
                    if let Some(parent) = event.parent_event_id.clone() {
                        orphan_turn_ids.push(parent);
                    }
                    ready.push(event);
                } else {
                    still_buffered.push_back((event, queued_at));
                }
            }
            *buf = still_buffered;
        }

        // 4. Coalesce orphan ids — the same parent referenced N times
        //    yields one orphan Turn upsert.
        orphan_turn_ids.sort();
        orphan_turn_ids.dedup();

        ReadyBatch {
            events: ready,
            orphan_turn_ids,
        }
    }

    /// Force an immediate sweep with the given `now`. Used by tests that
    /// want to assert orphan emission without waiting wall-clock 30 s.
    #[doc(hidden)]
    pub fn sweep_at(&self, now: Instant) -> ReadyBatch {
        let mut ready = Vec::new();
        let mut orphan_turn_ids: Vec<String> = Vec::new();
        if let Ok(mut buf) = self.buffered.lock() {
            let mut still_buffered: VecDeque<(EnrichedEvent, Instant)> = VecDeque::new();
            while let Some((event, queued_at)) = buf.pop_front() {
                if self.is_dependency_satisfied(&event) {
                    ready.push(event);
                } else if now.duration_since(queued_at) >= self.timeout {
                    if let Some(parent) = event.parent_event_id.clone() {
                        orphan_turn_ids.push(parent);
                    }
                    ready.push(event);
                } else {
                    still_buffered.push_back((event, queued_at));
                }
            }
            *buf = still_buffered;
        }
        orphan_turn_ids.sort();
        orphan_turn_ids.dedup();
        ReadyBatch {
            events: ready,
            orphan_turn_ids,
        }
    }

    fn is_dependency_satisfied(&self, event: &EnrichedEvent) -> bool {
        // Only `tool_call.*` and `agent_call.*` carry a parent Turn we
        // need to wait on — everything else flushes immediately.
        let needs_parent = matches!(event.kind, Kind::ToolCall | Kind::AgentCall);
        if !needs_parent {
            return true;
        }
        match &event.parent_event_id {
            None => true, // No parent ref ⇒ no dependency to satisfy.
            Some(pid) => self
                .seen_turns
                .lock()
                .map(|g| g.contains(pid))
                .unwrap_or(true),
        }
    }
}

/// Build the synthetic patch that materialises an orphan Turn node.
///
/// Exposed as `pub` so the integration test suite can assert the
/// orphan-flag prop without re-deriving the helper.
pub fn orphan_turn_patch(turn_id: &str) -> GraphPatch {
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(turn_id.to_string()));
    props.insert("orphan".to_string(), Value::Bool(true));
    GraphPatch {
        nodes: vec![NodeOp {
            label: "Turn".to_string(),
            natural_key: turn_id.to_string(),
            props,
        }],
        edges: vec![],
    }
}

// ---------- Worker ------------------------------------------------------

/// Graph-writer worker that drives the Synap → mapper → coalescer →
/// Nexus → Synap pipeline.
pub struct Worker {
    config: GraphConfig,
    writer: Arc<dyn GraphWriter>,
    consumer: Arc<dyn SynapConsumer>,
    publisher: Arc<dyn SynapPublisher>,
    /// Shared metrics registry. Public so the binary entrypoint
    /// can read the counters from its admin `/healthz` listener
    /// (phase8a).
    pub metrics: Arc<Metrics>,
    backpressure: Arc<BackpressureState>,
    buffer: Arc<OutOfOrderBuffer>,
    processed: Mutex<BTreeSet<String>>,
}

impl Worker {
    /// Build a new worker.
    pub fn new(
        config: GraphConfig,
        writer: Arc<dyn GraphWriter>,
        consumer: Arc<dyn SynapConsumer>,
        publisher: Arc<dyn SynapPublisher>,
        metrics: Arc<Metrics>,
    ) -> Self {
        let timeout = Duration::from_secs(config.out_of_order_buffer_secs.max(1));
        Self {
            config,
            writer,
            consumer,
            publisher,
            metrics,
            backpressure: Arc::new(BackpressureState::new()),
            buffer: Arc::new(OutOfOrderBuffer::new(timeout)),
            processed: Mutex::new(BTreeSet::new()),
        }
    }

    /// Borrow the runtime configuration.
    pub fn config(&self) -> &GraphConfig {
        &self.config
    }

    /// Borrow the metrics registry.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Borrow the backpressure gauge.
    pub fn backpressure(&self) -> &Arc<BackpressureState> {
        &self.backpressure
    }

    /// Borrow the out-of-order buffer.
    pub fn buffer(&self) -> &Arc<OutOfOrderBuffer> {
        &self.buffer
    }

    /// Stream name this worker consumes from.
    pub fn enriched_stream(&self) -> &'static str {
        STREAM_ENRICHED
    }

    /// Single iteration of the run loop — fetches one batch, processes
    /// it, publishes outcomes. Exposed so tests can drive the loop
    /// without spinning an infinite task.
    ///
    /// Returns the number of messages handled in this iteration.
    pub async fn run_once(&self) -> Result<usize> {
        if self.backpressure.is_paused() {
            self.metrics.set_backpressure(true);
            return Ok(0);
        }
        self.metrics.set_backpressure(self.backpressure.is_active());

        let batch_size = self.config.patch_batch.max(1);
        let msgs = self
            .consumer
            .next_batch(STREAM_ENRICHED, batch_size)
            .await?;
        let received = msgs.len();
        self.handle_batch(msgs).await?;
        Ok(received)
    }

    /// Long-running loop. Polls `run_once`, idling briefly when the
    /// room is empty, and honouring backpressure pauses. Exits cleanly
    /// when `shutdown.load(Relaxed)` becomes true.
    pub async fn run_forever(&self, shutdown: Arc<AtomicBool>) -> Result<()> {
        tracing::info!(
            workers = self.config.workers,
            patch_batch = self.config.patch_batch,
            flush_ms = self.config.flush_ms,
            nexus = %self.config.nexus_url,
            synap = %self.config.synap_url,
            "cortex-graph worker started"
        );
        let idle = Duration::from_millis(self.config.flush_ms.max(50));
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
                    self.metrics.incr_errors("other");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            // Phase8b — bump the per-stage activity counters.
            self.metrics.record_jobs_processed(handled as u64);
            if handled == 0 {
                tokio::time::sleep(idle).await;
            }
        }
        tracing::info!("cortex-graph worker stopped");
        Ok(())
    }

    /// Spawn `config.workers` copies of the loop onto the current Tokio
    /// runtime and join them. Every copy shares writer + consumer +
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

    /// Process a freshly-pulled batch of messages: deserialize, dedupe,
    /// run them through the out-of-order buffer, write the resulting
    /// patch, and publish reports / dead-letters.
    ///
    /// Empty pulls still flow through the buffer sweep so child events
    /// past their timeout get flushed with an orphan-Turn injection
    /// even when no new traffic is arriving.
    async fn handle_batch(&self, msgs: Vec<ConsumedMessage>) -> Result<()> {
        // 1. Deserialize + classify each message.
        let mut events: Vec<EnrichedEvent> = Vec::with_capacity(msgs.len());
        let mut to_ack: Vec<u64> = Vec::with_capacity(msgs.len());
        for msg in &msgs {
            match serde_json::from_value::<EnrichedEvent>(msg.payload.clone()) {
                Ok(event) => {
                    let is_replay = {
                        match self.processed.lock() {
                            Ok(mut seen) => !seen.insert(event.event_id.clone()),
                            Err(_) => false,
                        }
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

        // 2. Out-of-order buffer: hold child events whose parent Turn
        //    has not been seen yet.
        let ready = self.buffer.ingest(events);
        if ready.events.is_empty() && ready.orphan_turn_ids.is_empty() {
            for offset in &to_ack {
                self.ack_offset(*offset).await;
            }
            return Ok(());
        }

        // 3. Materialise the patch list — one per event plus one per
        //    orphan Turn id — and hand it to the writer.
        let mut patches: Vec<GraphPatch> = ready
            .events
            .iter()
            .map(super::map_event_to_patch)
            .collect();
        for turn_id in &ready.orphan_turn_ids {
            self.metrics.incr_orphans("Turn");
            patches.push(orphan_turn_patch(turn_id));
        }

        let event_count = ready.events.len();
        let orphan_count = ready.orphan_turn_ids.len();
        match self.writer.write_patches(patches).await {
            Ok(report) => {
                // Per-batch span — spec 07 §Observability calls for
                // nodes upserted, edges upserted, dedup hits, tx
                // latency. Emitted as a structured `info` event so any
                // tracing subscriber (Prometheus exporter, OpenTelemetry
                // sink, JSON log shipper) can pick them up without
                // bespoke wiring.
                tracing::info!(
                    events = event_count,
                    orphan_turns = orphan_count,
                    nodes_upserted = report.nodes_upserted,
                    edges_upserted = report.edges_upserted,
                    nodes_deduped = report.nodes_deduped,
                    edges_deduped = report.edges_deduped,
                    latency_ms = report.latency_ms,
                    outcome = "ok",
                    "graph batch flushed"
                );
                self.publish_report(&ready.events, &ready.orphan_turn_ids, &report)
                    .await;
                self.backpressure.record_success();
                for offset in &to_ack {
                    self.ack_offset(*offset).await;
                }
            }
            Err(GraphClientError::TransientError(detail)) => {
                tracing::warn!(
                    events = event_count,
                    orphan_turns = orphan_count,
                    outcome = "transient",
                    detail = %detail,
                    "transient nexus error; engaging backpressure"
                );
                self.backpressure.record_transient();
                self.metrics.set_backpressure(true);
                self.metrics.incr_errors("transient");
                // Roll back the dedup + offset: the next pull must
                // redeliver these messages once Nexus recovers.
                self.untrack(&ready.events, &to_ack);
            }
            Err(GraphClientError::ConstraintViolation { detail }) => {
                tracing::warn!(
                    events = event_count,
                    orphan_turns = orphan_count,
                    outcome = "constraint_violation",
                    detail = %detail,
                    "constraint violation; routing batch to invalid"
                );
                self.metrics.incr_errors("constraint");
                for evt in &ready.events {
                    self.publish_invalid(&evt.event_id, "constraint_violation", &detail)
                        .await;
                }
                for offset in &to_ack {
                    self.ack_offset(*offset).await;
                }
            }
            Err(other) => {
                tracing::warn!(
                    events = event_count,
                    orphan_turns = orphan_count,
                    outcome = "error",
                    error = %other,
                    "write_patches failed"
                );
                self.metrics.incr_errors("other");
                self.untrack(&ready.events, &to_ack);
            }
        }

        Ok(())
    }

    async fn publish_report(
        &self,
        events: &[EnrichedEvent],
        orphan_turn_ids: &[String],
        report: &super::patch::GraphWriteReport,
    ) {
        let event_ids: Vec<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
        let envelope = json!({
            "kind": "graphed",
            "event_ids": event_ids,
            "orphan_turn_ids": orphan_turn_ids,
            "nodes_upserted": report.nodes_upserted,
            "edges_upserted": report.edges_upserted,
            "nodes_deduped": report.nodes_deduped,
            "edges_deduped": report.edges_deduped,
            "by_label": report.by_label,
            "latency_ms": report.latency_ms,
        });
        if let Err(e) = self.publisher.publish(STREAM_GRAPHED, &envelope).await {
            tracing::warn!(error = %e, "failed to publish graphed envelope");
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

    /// Roll back the in-memory dedup set so transient failures get
    /// retried on the next pull. Offsets are intentionally NOT acked
    /// in this path — the consumer cursor stays where it was.
    fn untrack(&self, events: &[EnrichedEvent], _offsets: &[u64]) {
        if let Ok(mut seen) = self.processed.lock() {
            for e in events {
                seen.remove(&e.event_id);
            }
        }
    }
}
