//! Classifier-worker run loop.
//!
//! Two Synap input streams (`cortex.events.raw` and
//! `cortex.events.bootstrap`) feed the worker; one output stream
//! (`cortex.events.enriched`) carries the classified records the
//! embedder, graph writer, and full-text indexer all consume.
//!
//! The loop pulls a batch from each input stream, deserialises the
//! envelope (canonical [`cortex_core::events::Envelope`] for the live
//! stream; the bootstrap-event shape from `cortex-bootstrap` for the
//! backfill stream), runs the classifier, builds a
//! [`cortex_embedder::EnrichedEvent`], and publishes it. Acks happen
//! after a successful publish so an at-least-once retry is the worst
//! case.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use cortex_classifier::{
    Classifier, ClassifierOutput, ClassifierSource, ClassifierStack, EnrichmentInput, PiiRisk,
    Severity,
};
use cortex_core::events::{Envelope, Kind};
use cortex_embedder::EnrichedEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use synap_sdk::stream::StreamManager;
use synap_sdk::types::Event;
use synap_sdk::{SynapClient, SynapConfig};

use crate::config::ClassifierWorkerConfig;
use crate::kinds::kind_from_bootstrap;

/// Synap stream — live ingestion writes raw envelopes here.
pub const STREAM_RAW: &str = "cortex.events.raw";
/// Synap stream — `cortex-bootstrap` writes synthetic envelopes here.
pub const STREAM_BOOTSTRAP: &str = "cortex.events.bootstrap";
/// Synap stream — the worker publishes [`EnrichedEvent`]s here.
pub const STREAM_ENRICHED: &str = "cortex.events.enriched";

// ---------------------------------------------------------------- Consumer / Publisher

/// One message delivered by a [`SynapConsumer`].
#[derive(Debug, Clone)]
pub struct ConsumedMessage {
    /// Stream offset — monotonically increasing per room.
    pub offset: u64,
    /// `event` label Synap stored alongside the data.
    pub kind: String,
    /// Raw envelope JSON.
    pub payload: Value,
    /// Pre-extracted event id when the payload already carried one.
    pub event_id: Option<String>,
}

/// Synap-consumer abstraction. Mirrors the trait `cortex-embedder`
/// uses so callers can swap in a memory consumer for tests.
#[async_trait]
pub trait SynapConsumer: Send + Sync + 'static {
    /// Fetch up to `max` un-processed messages from `room`.
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>>;
    /// Mark a message as processed.
    async fn ack(&self, room: &str, offset: u64) -> Result<()>;
}

/// Synap-publisher abstraction.
#[async_trait]
pub trait SynapPublisher: Send + Sync + 'static {
    /// Publish `envelope` onto `room`.
    async fn publish(&self, room: &str, envelope: &Value) -> Result<()>;
}

/// Per-room offset cursor used by [`LiveSynapConsumer`].
#[derive(Debug, Default)]
pub struct OffsetTracker {
    next: AtomicU64,
}

impl OffsetTracker {
    /// Tracker starting at offset 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current cursor.
    pub fn current(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }

    /// Advance past `offset`, monotonic.
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

/// Synap client handle (one TCP connection shared by consumer + publisher).
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

    /// Borrow the inner stream manager.
    pub fn streams(&self) -> &StreamManager {
        &self.streams
    }
}

/// Live Synap consumer using the 0.11 pull API. Tracks one offset per
/// room (the SDK call is stateless).
pub struct LiveSynapConsumer {
    handle: Arc<SynapHandle>,
    raw_tracker: Arc<OffsetTracker>,
    bootstrap_tracker: Arc<OffsetTracker>,
}

impl LiveSynapConsumer {
    /// Build a new live consumer.
    pub fn new(handle: Arc<SynapHandle>) -> Self {
        Self {
            handle,
            raw_tracker: Arc::new(OffsetTracker::new()),
            bootstrap_tracker: Arc::new(OffsetTracker::new()),
        }
    }

    fn tracker_for(&self, room: &str) -> Arc<OffsetTracker> {
        if room == STREAM_BOOTSTRAP {
            self.bootstrap_tracker.clone()
        } else {
            self.raw_tracker.clone()
        }
    }
}

#[async_trait]
impl SynapConsumer for LiveSynapConsumer {
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>> {
        let tracker = self.tracker_for(room);
        let offset = tracker.current();
        let events: Vec<Event> = self
            .handle
            .streams()
            .consume(room, Some(offset), Some(max))
            .await
            .map_err(|e| anyhow::anyhow!("synap consume {room}: {e}"))?;

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

    async fn ack(&self, room: &str, offset: u64) -> Result<()> {
        self.tracker_for(room).advance_past(offset);
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
            .map_err(|e| anyhow::anyhow!("synap publish {room}: {e}"))
    }
}

/// In-memory consumer used by tests.
#[derive(Default)]
pub struct MemorySynapConsumer {
    queues: Mutex<std::collections::HashMap<String, VecDeque<ConsumedMessage>>>,
}

impl MemorySynapConsumer {
    /// Empty consumer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a message under `room`.
    pub fn enqueue(&self, room: &str, msg: ConsumedMessage) {
        if let Ok(mut g) = self.queues.lock() {
            g.entry(room.to_string()).or_default().push_back(msg);
        }
    }

    /// Remaining messages on `room`.
    pub fn remaining(&self, room: &str) -> usize {
        self.queues
            .lock()
            .ok()
            .and_then(|g| g.get(room).map(|q| q.len()))
            .unwrap_or(0)
    }
}

#[async_trait]
impl SynapConsumer for MemorySynapConsumer {
    async fn next_batch(&self, room: &str, max: usize) -> Result<Vec<ConsumedMessage>> {
        let mut out = Vec::with_capacity(max);
        if let Ok(mut g) = self.queues.lock() {
            if let Some(q) = g.get_mut(room) {
                while out.len() < max {
                    match q.pop_front() {
                        Some(m) => out.push(m),
                        None => break,
                    }
                }
            }
        }
        Ok(out)
    }

    async fn ack(&self, _room: &str, _offset: u64) -> Result<()> {
        Ok(())
    }
}

/// In-memory publisher used by tests.
#[derive(Default)]
pub struct MemorySynapPublisher {
    /// `(room, envelope)` pairs in arrival order.
    pub calls: Mutex<Vec<(String, Value)>>,
}

impl MemorySynapPublisher {
    /// Empty publisher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot recorded calls.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Filter recorded calls by room.
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

// ---------------------------------------------------------------- Worker

/// One ingested envelope already normalised to a canonical shape so the
/// classifier and the [`EnrichedEvent`] composition can ignore which
/// stream the message came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NormalisedEvent {
    event_id: String,
    kind: Kind,
    content_hash: String,
    redacted_payload: Value,
    context_repo: Option<String>,
    context_path: Option<String>,
    parent_event_id: Option<String>,
}

impl NormalisedEvent {
    fn from_message(msg: &ConsumedMessage, source_stream: &str) -> Result<Self> {
        match source_stream {
            STREAM_RAW => Self::from_canonical_envelope(&msg.payload),
            STREAM_BOOTSTRAP => Self::from_bootstrap_event(&msg.payload),
            other => Err(anyhow::anyhow!(
                "unsupported source stream for classifier worker: {other}"
            )),
        }
    }

    fn from_canonical_envelope(payload: &Value) -> Result<Self> {
        let env: Envelope = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("deserialize canonical envelope: {e}"))?;
        let context_path = env
            .context
            .extras
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(Self {
            event_id: env.event_id,
            kind: env.kind,
            content_hash: env.content_hash,
            redacted_payload: env.payload,
            context_repo: env.context.repo,
            context_path,
            parent_event_id: env.parent_event_id,
        })
    }

    fn from_bootstrap_event(payload: &Value) -> Result<Self> {
        let event_id = payload
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("bootstrap event missing event_id"))?
            .to_string();
        let kind_str = payload
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("bootstrap event missing kind"))?;
        let kind = kind_from_bootstrap(kind_str)
            .map_err(|e| anyhow::anyhow!("bootstrap kind map: {e}"))?;
        let content_hash = payload
            .get("content_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("bootstrap event missing content_hash"))?
            .to_string();
        let redacted_payload = payload
            .get("redacted_payload")
            .cloned()
            .unwrap_or(Value::Null);
        let context_repo = payload
            .get("source")
            .and_then(|s| s.get("repo"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let context_path = payload
            .get("source")
            .and_then(|s| s.get("path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(Self {
            event_id,
            kind,
            content_hash,
            redacted_payload,
            context_repo,
            context_path,
            parent_event_id: None,
        })
    }

    fn to_enrichment_input(&self) -> EnrichmentInput {
        EnrichmentInput {
            event_id: self.event_id.clone(),
            kind: self.kind,
            content_hash: self.content_hash.clone(),
            redacted_payload: self.redacted_payload.clone(),
            context_repo: self.context_repo.clone(),
        }
    }

    fn into_enriched(self, classifier: ClassifierOutput) -> EnrichedEvent {
        EnrichedEvent {
            event_id: self.event_id,
            kind: self.kind,
            content_hash: self.content_hash,
            redacted_payload: self.redacted_payload,
            classifier,
            context_repo: self.context_repo,
            context_path: self.context_path,
            parent_event_id: self.parent_event_id,
        }
    }
}

/// Worker that drives the Synap → classify → Synap pipeline.
pub struct Worker {
    /// Worker configuration.
    pub config: ClassifierWorkerConfig,
    /// Classifier stack (typically `BudgetedClassifier<CachedClassifier<...>>`).
    pub classifier: Arc<dyn Classifier>,
    /// Synap consumer.
    pub consumer: Arc<dyn SynapConsumer>,
    /// Synap publisher.
    pub publisher: Arc<dyn SynapPublisher>,
    /// Source streams to drain (typically `[STREAM_RAW, STREAM_BOOTSTRAP]`).
    pub input_streams: Vec<String>,
    /// In-memory de-dup of already-classified event ids (at-least-once delivery guard).
    processed: Mutex<BTreeSet<String>>,
}

impl Worker {
    /// Build a worker that drains both `cortex.events.raw` and
    /// `cortex.events.bootstrap`.
    pub fn new(
        config: ClassifierWorkerConfig,
        classifier: Arc<dyn Classifier>,
        consumer: Arc<dyn SynapConsumer>,
        publisher: Arc<dyn SynapPublisher>,
    ) -> Self {
        Self {
            config,
            classifier,
            consumer,
            publisher,
            input_streams: vec![STREAM_RAW.to_string(), STREAM_BOOTSTRAP.to_string()],
            processed: Mutex::new(BTreeSet::new()),
        }
    }

    /// Build a worker bound to a [`ClassifierStack`]. The stack is the
    /// production composition (`Budgeted ← Cached ← backend`); for tests
    /// pass a simpler `Arc<dyn Classifier>` via [`Self::new`].
    pub fn with_stack(
        config: ClassifierWorkerConfig,
        stack: ClassifierStack,
        consumer: Arc<dyn SynapConsumer>,
        publisher: Arc<dyn SynapPublisher>,
    ) -> Self {
        Self::new(config, Arc::new(stack), consumer, publisher)
    }

    /// Convenience for tests / single-stream drivers.
    pub fn with_input_streams(mut self, streams: Vec<String>) -> Self {
        self.input_streams = streams;
        self
    }

    /// Process one batch from each input stream. Returns the total
    /// number of messages handled (across streams).
    pub async fn run_once(&self) -> Result<usize> {
        let mut handled = 0;
        for stream in self.input_streams.clone() {
            handled += self.drain_one(&stream).await?;
        }
        Ok(handled)
    }

    /// Long-running loop until `shutdown` flips.
    pub async fn run_forever(&self, shutdown: Arc<AtomicBool>) -> Result<()> {
        tracing::info!(
            workers = self.config.workers,
            batch = self.config.batch_size,
            mode = ?self.config.mode,
            synap = %self.config.synap_url,
            "cortex-classifier-worker started"
        );
        while !shutdown.load(Ordering::Relaxed) {
            let handled = match self.run_once().await {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "run_once failed; backing off");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            if handled == 0 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        tracing::info!("cortex-classifier-worker stopped");
        Ok(())
    }

    /// Spawn `config.workers` copies of [`Self::run_forever`].
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

    async fn drain_one(&self, stream: &str) -> Result<usize> {
        let batch = self
            .consumer
            .next_batch(stream, self.config.batch_size)
            .await?;
        if batch.is_empty() {
            return Ok(0);
        }
        let count = batch.len();
        for msg in batch {
            self.handle_message(stream, msg).await?;
        }
        Ok(count)
    }

    async fn handle_message(&self, stream: &str, msg: ConsumedMessage) -> Result<()> {
        let offset = msg.offset;

        // 1. Normalise.
        let normalised = match NormalisedEvent::from_message(&msg, stream) {
            Ok(n) => n,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    stream = stream,
                    offset = offset,
                    "skipping malformed envelope"
                );
                self.consumer.ack(stream, offset).await?;
                return Ok(());
            }
        };

        // 2. In-memory dedup. Drop the lock before any await.
        let is_replay = {
            let mut seen = match self.processed.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            !seen.insert(normalised.event_id.clone())
        };
        if is_replay {
            tracing::debug!(
                event_id = %normalised.event_id,
                "skipping already-classified event"
            );
            self.consumer.ack(stream, offset).await?;
            return Ok(());
        }

        // 3. Classify.
        let input = normalised.to_enrichment_input();
        let outputs = match self
            .classifier
            .classify_batch(std::slice::from_ref(&input))
            .await
        {
            Ok(out) => out,
            Err(err) => {
                tracing::warn!(error = %err, event_id = %normalised.event_id, "classifier error; using static fallback record");
                vec![static_fallback_output(
                    &normalised,
                    &self.config.prompt_version,
                )]
            }
        };
        let classifier_output = outputs
            .into_iter()
            .next()
            .unwrap_or_else(|| static_fallback_output(&normalised, &self.config.prompt_version));

        // 4. Publish enriched.
        let enriched = normalised.into_enriched(classifier_output);
        let envelope = serde_json::to_value(&enriched)
            .map_err(|e| anyhow::anyhow!("serialize enriched: {e}"))?;
        if let Err(err) = self.publisher.publish(STREAM_ENRICHED, &envelope).await {
            tracing::warn!(error = %err, event_id = %enriched.event_id, "publish enriched failed");
            // Drop dedup so the redelivery is reprocessed.
            if let Ok(mut seen) = self.processed.lock() {
                seen.remove(&enriched.event_id);
            }
            return Err(err);
        }
        self.consumer.ack(stream, offset).await?;
        Ok(())
    }
}

fn static_fallback_output(n: &NormalisedEvent, prompt_version: &str) -> ClassifierOutput {
    ClassifierOutput {
        event_id: n.event_id.clone(),
        kind_refinement: None,
        topics: vec![],
        severity: Severity::Info,
        pii_risk: PiiRisk::Low,
        redaction_suggestions: vec![],
        summary: None,
        source: ClassifierSource::StaticFallback,
        prompt_version: prompt_version.to_string(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}
