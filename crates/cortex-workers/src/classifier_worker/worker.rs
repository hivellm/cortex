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
//! [`crate::embedder::EnrichedEvent`], and publishes it. Acks happen
//! after a successful publish so an at-least-once retry is the worst
//! case.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::classifier::{
    merge_sensitivity, Classifier, ClassifierOutput, ClassifierSource, ClassifierStack,
    EnrichmentInput, PiiRisk, PricingTable, SensitivityOutput, Severity,
};
use crate::embedder::EnrichedEvent;
use anyhow::Result;
use async_trait::async_trait;
use cortex_core::events::{Envelope, Kind};
use cortex_storage::{hour_bucket_rfc3339, MetadataStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use synap_sdk::stream::StreamManager;
use synap_sdk::types::Event;
use synap_sdk::{SynapClient, SynapConfig};

use super::config::ClassifierWorkerConfig;
use super::kinds::kind_from_bootstrap;

/// Phase11s §1.2 — default supervisor threshold for consecutive
/// `run_once` failures before [`Worker::run_forever`] returns an
/// error so docker `restart: unless-stopped` recovers the container.
pub const DEFAULT_MAX_CONSUME_ERRORS: u64 = 5;

/// Phase11s §1.2 — read the supervisor threshold from
/// `CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS`. Falls back to
/// [`DEFAULT_MAX_CONSUME_ERRORS`] when unset / unparseable. A value
/// of `0` disables the supervisor (kept dormant for the original
/// pre-§1.2 behaviour when an operator wants it off).
pub fn max_consume_errors_from_env() -> u64 {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.classifier.max_consume_errors)
        .unwrap_or(DEFAULT_MAX_CONSUME_ERRORS)
}

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
                    return Err(anyhow::anyhow!("synap consume {room}: {e}"));
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
        match self
            .handle
            .streams()
            .publish(room, kind, envelope.clone())
            .await
        {
            Ok(_offset) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("Room") {
                    if let Err(create_err) = self.handle.streams().create_room(room, None).await {
                        tracing::debug!(
                            room,
                            error = %create_err,
                            "stream.create failed; retrying publish anyway"
                        );
                    }
                    self.handle
                        .streams()
                        .publish(room, kind, envelope.clone())
                        .await
                        .map(|_| ())
                        .map_err(|e| anyhow::anyhow!("synap publish {room} (post-create): {e}"))
                } else {
                    Err(anyhow::anyhow!("synap publish {room}: {e}"))
                }
            }
        }
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
    /// Owning session id. From the canonical envelope's top-level
    /// `session_id`; bootstrap events fall back to whatever they
    /// stamped at synthesis time.
    session_id: Option<String>,
    /// Phase11i §2.2 — the originating tool tag (`claude-code`,
    /// `openai-codex`, `bootstrap`, etc.). Carried through so the
    /// post-classify hook can stamp a `tool:<name>` topic without
    /// re-deserialising the envelope.
    #[serde(default)]
    tool: Option<String>,
    /// Phase20 §5.2 — envelope occurrence time (epoch ms). Parsed
    /// from `Envelope.occurred_at` (RFC3339) on the canonical path,
    /// stamped from `chrono::Utc::now()` on the bootstrap path.
    /// Plumbed into `EnrichedEvent.occurred_at_ms` so the fulltext
    /// worker can project `Document.ts` instead of leaving it 0.
    #[serde(default)]
    occurred_at_ms: i64,
    /// Phase21 §3.4 — declared classification floor from config path rules
    /// (bootstrap path) or from the canonical `Envelope.class_level` (live path).
    /// Merged with `classifier.sensitivity` in `into_enriched` via
    /// `merge_sensitivity`; the result is the `EnrichedEvent.class_level`.
    #[serde(default)]
    class_level: Option<u8>,
    /// Phase21 §3.4 — declared compartment floor; unioned with detected
    /// compartments in `into_enriched`.
    #[serde(default)]
    class_compartments: Option<Vec<String>>,
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
        let session_id = if env.session_id.is_empty() {
            None
        } else {
            Some(env.session_id.clone())
        };
        let tool = if env.tool.is_empty() {
            None
        } else {
            Some(env.tool.clone())
        };
        let occurred_at_ms = chrono::DateTime::parse_from_rfc3339(&env.occurred_at)
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0);
        Ok(Self {
            event_id: env.event_id,
            kind: env.kind,
            content_hash: env.content_hash,
            redacted_payload: env.payload,
            context_repo: env.context.repo,
            context_path,
            parent_event_id: env.parent_event_id,
            session_id,
            tool,
            occurred_at_ms,
            class_level: env.class_level,
            class_compartments: env.class_compartments,
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
        // Bootstrap events emit a top-level `session_id` (one per
        // bootstrap run, shared across every emitted artifact event)
        // — pick it up so the graph writer groups them under a real
        // Session node instead of one synthetic session per event.
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // Phase11i §2.2 — bootstrap events that came from
        // `cortex-claude-archive` carry a top-level `tool` field
        // (`claude-code` / `openai-codex`); preserve it so the
        // post-classify hook can stamp a matching topic.
        let tool = payload
            .get("tool")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // Bootstrap envelopes may carry a top-level `occurred_at`
        // (synthesized at bootstrap time). Parse when present; fall
        // back to "now" so the projected `ts` is non-zero even on
        // synthetic events from the bootstrap walker.
        let occurred_at_ms = payload
            .get("occurred_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        // Phase21 §3.4 — declared classification floor from the bootstrap emitter
        // (stamped by config path rules in the walker). u8 is not a serde native
        // number tag, so deserialize as u64 then narrow.
        let class_level = payload
            .get("class_level")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(255) as u8);
        let class_compartments = payload
            .get("class_compartments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());
        Ok(Self {
            event_id,
            kind,
            content_hash,
            redacted_payload,
            context_repo,
            context_path,
            parent_event_id: None,
            session_id,
            tool,
            occurred_at_ms,
            class_level,
            class_compartments,
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
        // `tool` lives on `NormalisedEvent` — it does not flow
        // into the classifier (kept tool-agnostic) but the
        // post-classify hook in `process_one` reads it from the
        // normalised value to stamp a `tool:<name>` topic.
    }

    fn into_enriched(self, classifier: ClassifierOutput) -> EnrichedEvent {
        // Phase21 §3.4 — merge declared floor (from config path rules or live
        // envelope) with content-detected sensitivity (classifier.sensitivity).
        // Escalate-only: level = max(declared, detected), compartments = union.
        let declared = SensitivityOutput {
            level: self.class_level.unwrap_or(0),
            compartments: self.class_compartments.clone().unwrap_or_default(),
        };
        let detected = classifier.sensitivity.clone();
        let merged = merge_sensitivity(declared, detected);
        // Keep None when result is public/empty — preserves backward-compat
        // (callers treat None as "unclassified / public" per ADR-032).
        let (class_level, class_compartments) =
            if merged.level > 0 || !merged.compartments.is_empty() {
                (
                    Some(merged.level),
                    if merged.compartments.is_empty() {
                        None
                    } else {
                        Some(merged.compartments)
                    },
                )
            } else {
                (None, None)
            };
        EnrichedEvent {
            event_id: self.event_id,
            kind: self.kind,
            content_hash: self.content_hash,
            redacted_payload: self.redacted_payload,
            classifier,
            context_repo: self.context_repo,
            context_path: self.context_path,
            parent_event_id: self.parent_event_id,
            session_id: self.session_id,
            occurred_at_ms: self.occurred_at_ms,
            class_level,
            class_compartments,
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
    /// Optional metadata store. When `Some`, every successful classify
    /// publishes a row into `classifier_spend_hourly` (+ `classifier_spend`)
    /// so the dashboard's `series.classifier_cost_usd_today` ribbon
    /// has real data. `None` keeps tests and cold-stack dev clean —
    /// the worker still classifies, it just doesn't book the spend.
    ///
    /// Wrapped in a [`Mutex`] because `rusqlite::Connection` is `Send`
    /// but not `Sync`, and the worker is shared across the tokio
    /// pool via `Arc<Worker>`.
    pub metadata: Option<Arc<Mutex<MetadataStore>>>,
    /// Pricing table the cost roll-up uses. Defaults to
    /// [`PricingTable::HAIKU_4_5`]; override via [`Self::with_pricing`]
    /// if the worker is wired to a different model.
    pub pricing: PricingTable,
    /// In-memory de-dup of already-classified event ids (at-least-once delivery guard).
    processed: Mutex<BTreeSet<String>>,
    /// phase24 §1.2 — last-seen wall-clock ms per `session_id`, used by
    /// the idle-window session-end detector. Shared across the tokio
    /// pool via `Arc<Worker>`; the critical section is sync (no await
    /// held under the lock).
    session_last_seen: Mutex<BTreeMap<String, i64>>,
    /// phase24 §1.3 — per-repo event count since the last `nightly_topic`
    /// trigger was published. When the count reaches
    /// [`crate::topic_cards::trigger::TRIGGER_EVENTS_THRESHOLD`], a
    /// `nightly_topic` trigger is published and the counter resets.
    /// Lock discipline mirrors `session_last_seen`: sync critical section,
    /// no await held while locked.
    repo_event_counts: Mutex<BTreeMap<String, u32>>,
    /// Phase8b — total Synap messages the worker has processed end
    /// to end. Bumped after each `run_once` returns a non-zero count.
    pub jobs_processed_total: AtomicU64,
    /// Phase8b — Unix-epoch ms of the most recent successful job.
    pub last_job_ts_ms: AtomicU64,
    /// Phase11s §1.3 — Unix-epoch ms of the most recent successful
    /// `run_once` return (regardless of whether it handled events).
    /// Distinct from [`last_job_ts_ms`] which only bumps when ≥ 1
    /// event was handled — the consume-loop liveness probe must
    /// see "we touched Synap" not just "we processed work".
    pub last_consume_ts_ms: AtomicU64,
    /// Phase11s §1.2 — consecutive `run_once` failures. Reset to 0
    /// on every successful return. Drives the supervisor exit when
    /// the count crosses [`Self::max_consume_errors`].
    pub consume_errors_consecutive: AtomicU64,
    /// Phase11s §1.2 — supervisor threshold. After N consecutive
    /// `run_once` failures, [`Self::run_forever`] returns an error
    /// so the bin's main propagates the non-zero exit and docker's
    /// `restart: unless-stopped` recovers the container fresh.
    /// Sourced from `CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS` (default 5).
    pub max_consume_errors: u64,
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
            metadata: None,
            pricing: PricingTable::HAIKU_4_5,
            processed: Mutex::new(BTreeSet::new()),
            session_last_seen: Mutex::new(BTreeMap::new()),
            repo_event_counts: Mutex::new(BTreeMap::new()),
            jobs_processed_total: AtomicU64::new(0),
            last_job_ts_ms: AtomicU64::new(0),
            last_consume_ts_ms: AtomicU64::new(0),
            consume_errors_consecutive: AtomicU64::new(0),
            max_consume_errors: max_consume_errors_from_env(),
        }
    }

    /// Phase8b — record `n` jobs processed (called after each
    /// `run_once`). `n == 0` is a no-op so idle polls don't refresh
    /// the freshness signal.
    pub fn record_jobs_processed(&self, n: u64) {
        if n == 0 {
            return;
        }
        self.jobs_processed_total.fetch_add(n, Ordering::Relaxed);
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.last_job_ts_ms.store(now_ms, Ordering::Relaxed);
    }
    /// Phase8b — read the cumulative jobs-processed counter.
    pub fn jobs_processed_total(&self) -> u64 {
        self.jobs_processed_total.load(Ordering::Relaxed)
    }
    /// Phase8b — read the most-recent-job timestamp.
    pub fn last_job_ts_ms(&self) -> u64 {
        self.last_job_ts_ms.load(Ordering::Relaxed)
    }
    /// Phase11s §1.3 — read the most-recent successful `run_once`
    /// timestamp. Liveness probe — distinct from `last_job_ts_ms`
    /// which only bumps on non-zero handled batches.
    pub fn last_consume_ts_ms(&self) -> u64 {
        self.last_consume_ts_ms.load(Ordering::Relaxed)
    }
    /// Phase11s §1.2 — read the consecutive-error counter.
    pub fn consume_errors_consecutive(&self) -> u64 {
        self.consume_errors_consecutive.load(Ordering::Relaxed)
    }
    /// Phase11s §1.2 — override the supervisor threshold. Tests
    /// use this to verify the exit-on-threshold contract without
    /// touching the `CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS` env var.
    pub fn with_max_consume_errors(mut self, max: u64) -> Self {
        self.max_consume_errors = max;
        self
    }
    /// Phase8b — render the classifier counters in Prometheus text format.
    pub fn render_prom(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        out.push_str("# TYPE cortex_classifier_jobs_processed_total counter\n");
        let _ = writeln!(
            out,
            "cortex_classifier_jobs_processed_total {}",
            self.jobs_processed_total()
        );
        out.push_str("# TYPE cortex_classifier_last_job_ts_ms gauge\n");
        let _ = writeln!(
            out,
            "cortex_classifier_last_job_ts_ms {}",
            self.last_job_ts_ms()
        );
        // Phase11s §1.3 — `last_consume_ts_ms` powers the operator
        // staleness probe. Distinct from `last_job_ts_ms` because
        // the consume loop must report forward progress even when
        // a poll returns zero events.
        out.push_str("# TYPE cortex_classifier_last_consume_ts_ms gauge\n");
        let _ = writeln!(
            out,
            "cortex_classifier_last_consume_ts_ms {}",
            self.last_consume_ts_ms()
        );
        out.push_str("# TYPE cortex_classifier_consume_errors_consecutive gauge\n");
        let _ = writeln!(
            out,
            "cortex_classifier_consume_errors_consecutive {}",
            self.consume_errors_consecutive()
        );
        out
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

    /// Attach a metadata store so per-event spend rolls up into
    /// `classifier_spend_hourly`. The dashboard reads that table to
    /// render `series.classifier_cost_usd_today`.
    pub fn with_metadata(mut self, store: Arc<Mutex<MetadataStore>>) -> Self {
        self.metadata = Some(store);
        self
    }

    /// Override the pricing table used for the cost roll-up.
    pub fn with_pricing(mut self, pricing: PricingTable) -> Self {
        self.pricing = pricing;
        self
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

    /// Phase14h — delegate to the shared `synap_worker` runtime.
    /// The trait impl below carries the classifier-specific
    /// supervisor (`max_consume_errors`) + the
    /// `last_consume_ts_ms` liveness probe wiring. See
    /// [`crate::synap_worker::run_pool`].
    pub async fn run_pool(self: Arc<Self>, shutdown: Arc<AtomicBool>) -> Result<()> {
        crate::synap_worker::run_pool(self, shutdown)
            .await
            .map_err(anyhow::Error::from)
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
        let mut classifier_output = outputs
            .into_iter()
            .next()
            .unwrap_or_else(|| static_fallback_output(&normalised, &self.config.prompt_version));

        // Phase11i §2.2 — stamp a `tool:<name>` topic so the
        // retrieval layer can filter on the originating AI agent
        // (`claude-code` / `openai-codex` / etc.). The classifier
        // proper does not see the envelope's `tool` field, so we
        // patch it here from the normalised event. Idempotent —
        // duplicate topic strings get deduped a few lines below
        // when the classifier-side dedupe runs (`topics.dedup()`
        // in StaticClassifier::classify_batch). Only stamp when
        // the tool tag is non-empty and not already present.
        if let Some(tool) = normalised.tool.as_deref() {
            if !tool.is_empty() {
                let topic = format!("tool:{tool}");
                if !classifier_output.topics.iter().any(|t| t == &topic) {
                    classifier_output.topics.push(topic);
                }
            }
        }

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

        // 5. Book per-hour spend so the dashboard's
        // `classifier_cost_usd_today` ribbon has real data.
        // Static-fallback hits land here with `tokens_in/out = 0`,
        // so the recorded spend is honestly zero — the call counter
        // still ticks, which keeps "we ran something" observable.
        self.record_spend(&enriched.classifier);

        // 6. phase24 §1.4 — fan out a consolidator `decision_landed`
        // trigger when this enriched event is a `Kind::Decision`. Gated
        // behind a default-off config flag: the decision-trace grain
        // auto-promotes to Opus, so this is opt-in spend. Non-fatal — a
        // trigger publish failure must not block enrichment or the ack
        // (re-triggers are idempotent via the daemon's
        // `decision:<id>` producer checkpoint).
        if self.config.consolidator_trigger_enabled {
            if let Some(trigger) =
                crate::consolidator::trigger_producer::decision_landed_trigger(&enriched)
            {
                if let Err(err) = self
                    .publisher
                    .publish(crate::consolidator::daemon::TRIGGER_STREAM, &trigger)
                    .await
                {
                    tracing::warn!(
                        error = %err,
                        event_id = %enriched.event_id,
                        "consolidator decision_landed trigger publish failed"
                    );
                }
            }

            // phase24 §1.2 — idle-window session-end detection. Stamp
            // this event's session, then fan out a `session_end` trigger
            // for any OTHER session quiet past `SESSION_IDLE_MS`. The lock
            // is released before any await (publish happens outside it).
            let ended = {
                let mut seen = self
                    .session_last_seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                crate::consolidator::trigger_producer::evaluate_idle_sessions(
                    &mut seen,
                    enriched.session_id.as_deref(),
                    enriched.occurred_at_ms,
                    crate::consolidator::trigger_producer::SESSION_IDLE_MS,
                )
            };
            for session_id in ended {
                if let Some(trigger) =
                    crate::consolidator::trigger_producer::session_end_trigger(&session_id)
                {
                    if let Err(err) = self
                        .publisher
                        .publish(crate::consolidator::daemon::TRIGGER_STREAM, &trigger)
                        .await
                    {
                        tracing::warn!(
                            error = %err,
                            session_id = %session_id,
                            "consolidator session_end trigger publish failed"
                        );
                    }
                }
            }

            // phase24 §1.3 — per-repo event-count topic-grain threshold.
            // The classifier worker has no Meili client, so we can't read
            // the current TopicCardPayload to feed `evaluate()`. We use the
            // card=None first-emit path: `topic_threshold_trigger` with
            // `card=None` always returns Some, so we gate it with our own
            // in-process counter instead. When the count reaches
            // TRIGGER_EVENTS_THRESHOLD the nightly_topic trigger is
            // published and the counter resets.
            if let Some(repo) = enriched.context_repo.as_deref() {
                let threshold_reached = {
                    let mut counts = self
                        .repo_event_counts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let count = counts.entry(repo.to_string()).or_insert(0);
                    *count += 1;
                    if *count >= crate::topic_cards::trigger::TRIGGER_EVENTS_THRESHOLD {
                        *count = 0;
                        true
                    } else {
                        false
                    }
                };
                if threshold_reached {
                    if let Some(trigger) =
                        crate::consolidator::trigger_producer::topic_threshold_trigger(
                            repo,
                            None,
                            enriched.kind,
                            &|| 1.0,
                            0,
                        )
                    {
                        if let Err(err) = self
                            .publisher
                            .publish(crate::consolidator::daemon::TRIGGER_STREAM, &trigger)
                            .await
                        {
                            tracing::warn!(
                                error = %err,
                                repo = %repo,
                                "consolidator nightly_topic trigger publish failed"
                            );
                        }
                    }
                }
            }
        }

        self.consumer.ack(stream, offset).await?;
        Ok(())
    }

    fn record_spend(&self, output: &ClassifierOutput) {
        let Some(store) = self.metadata.as_ref() else {
            return;
        };
        let now = chrono::Utc::now();
        let hour = hour_bucket_rfc3339(now);
        let day = now.format("%Y-%m-%d").to_string();
        let cents = self
            .pricing
            .spend_cents(output.tokens_in as u64, output.tokens_out as u64);
        let guard = match store.lock() {
            Ok(g) => g,
            // Another worker poisoned the lock — recover the inner
            // value rather than dropping the spend record.
            Err(p) => p.into_inner(),
        };
        if let Err(err) = guard.record_classifier_spend_hourly(
            &hour,
            1,
            output.tokens_in as u64,
            output.tokens_out as u64,
            cents,
        ) {
            tracing::warn!(error = %err, hour = %hour, "record_classifier_spend_hourly failed");
            return;
        }
        if let Err(err) = guard.record_classifier_spend(
            &day,
            1,
            output.tokens_in as u64,
            output.tokens_out as u64,
            cents,
        ) {
            tracing::warn!(error = %err, day = %day, "record_classifier_spend failed");
        }
    }
}

#[async_trait]
impl crate::synap_worker::SynapWorker for Worker {
    fn worker_name(&self) -> &'static str {
        "classifier"
    }

    fn pool_size(&self) -> usize {
        self.config.workers.max(1)
    }

    fn idle_duration(&self) -> Duration {
        Duration::from_millis(200)
    }

    fn max_consume_errors(&self) -> u32 {
        u32::try_from(self.max_consume_errors).unwrap_or(u32::MAX)
    }

    async fn run_once(&self) -> anyhow::Result<usize> {
        let handled = Worker::run_once(self).await?;
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.last_consume_ts_ms.store(now_ms, Ordering::Relaxed);
        Ok(handled)
    }

    fn on_run_once_ok(&self, handled: usize) {
        self.record_jobs_processed(handled as u64);
    }

    fn on_run_once_success_reset(&self) {
        self.consume_errors_consecutive.store(0, Ordering::Relaxed);
    }

    fn on_run_once_err(&self, _err: &anyhow::Error, consecutive: u32) {
        self.consume_errors_consecutive
            .store(consecutive as u64, Ordering::Relaxed);
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
        entities: Vec::new(),
        relations: Vec::new(),
        sensitivity: Default::default(),
        source: ClassifierSource::StaticFallback,
        prompt_version: prompt_version.to_string(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Phase11i §2.3 — bootstrap envelopes from cortex-claude-archive
    /// carry a top-level `tool` field. NormalisedEvent must
    /// preserve it so the post-classify hook can stamp the
    /// `tool:<name>` topic.
    #[test]
    fn bootstrap_envelope_with_tool_field_normalises_with_tool() {
        let payload = json!({
            "event_id": "01TEST0000000000000000000A",
            "kind": "turn",
            "content_hash": "sha256:abc",
            "redacted_payload": {"user_message": "hi", "assistant_message": "hello"},
            "session_id": "sess-1",
            "tool": "claude-code",
            "source": {"repo": "Cortex"}
        });
        let norm = NormalisedEvent::from_bootstrap_event(&payload).expect("normalise");
        assert_eq!(norm.tool.as_deref(), Some("claude-code"));
        assert_eq!(norm.kind, Kind::Turn);
        assert_eq!(norm.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn bootstrap_envelope_without_tool_field_normalises_with_none() {
        let payload = json!({
            "event_id": "01TEST0000000000000000000B",
            "kind": "artifact.code",
            "content_hash": "sha256:def",
            "redacted_payload": {},
            "source": {"repo": "Cortex"}
        });
        let norm = NormalisedEvent::from_bootstrap_event(&payload).expect("normalise");
        assert!(norm.tool.is_none());
    }

    #[test]
    fn dotted_tool_kind_resolves_via_kind_from_bootstrap() {
        // §2.1 cross-check: the dotted `turn.claude-code` shape
        // round-trips through normalisation as Kind::Turn.
        let payload = json!({
            "event_id": "01TEST0000000000000000000C",
            "kind": "turn.claude-code",
            "content_hash": "sha256:xyz",
            "redacted_payload": {},
            "tool": "claude-code"
        });
        let norm = NormalisedEvent::from_bootstrap_event(&payload).expect("normalise");
        assert_eq!(norm.kind, Kind::Turn);
        assert_eq!(norm.tool.as_deref(), Some("claude-code"));
    }

    #[test]
    fn canonical_envelope_with_tool_field_preserves_it() {
        // The full Envelope path (STREAM_RAW) carries `tool` as a
        // typed top-level field; verify the normaliser keeps it.
        // Serde tags: Stream = lowercase, Kind = snake_case.
        let env_json = json!({
            "event_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "schema_version": "1",
            "occurred_at": "2026-04-20T17:48:02.667Z",
            "session_id": "sess-x",
            "stream": "live",
            "tool": "claude-code",
            "kind": "turn",
            "context": {"repo": "cortex", "platform": "claude-code"},
            "payload": {"user_message": "hi", "assistant_message": "hello", "tokens": null},
            "content_hash": "sha256:abc"
        });
        let norm = NormalisedEvent::from_canonical_envelope(&env_json).expect("normalise");
        assert_eq!(norm.tool.as_deref(), Some("claude-code"));
        assert_eq!(norm.kind, Kind::Turn);
    }

    // -----------------------------------------------------------------
    // phase21 §3.4 — merge semantics (declared floor + detected sensitivity)
    // -----------------------------------------------------------------

    fn make_classifier_with_sensitivity(
        event_id: &str,
        level: u8,
        compartments: Vec<&'static str>,
    ) -> ClassifierOutput {
        ClassifierOutput {
            event_id: event_id.into(),
            kind_refinement: None,
            topics: vec![],
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: vec![],
            summary: None,
            entities: vec![],
            relations: vec![],
            sensitivity: SensitivityOutput {
                level,
                compartments: compartments.iter().map(|s| s.to_string()).collect(),
            },
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn bootstrap_norm_with_class(
        class_level: Option<u8>,
        class_compartments: Option<Vec<&'static str>>,
    ) -> NormalisedEvent {
        NormalisedEvent {
            event_id: "evt-merge-test".into(),
            kind: Kind::Turn,
            content_hash: "sha256:test".into(),
            redacted_payload: json!({}),
            context_repo: Some("cortex".into()),
            context_path: None,
            parent_event_id: None,
            session_id: None,
            tool: None,
            occurred_at_ms: 0,
            class_level,
            class_compartments: class_compartments
                .map(|v| v.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn into_enriched_both_public_leaves_class_fields_none() {
        let norm = bootstrap_norm_with_class(None, None);
        let clf = make_classifier_with_sensitivity("evt-merge-test", 0, vec![]);
        let enriched = norm.into_enriched(clf);
        assert!(
            enriched.class_level.is_none(),
            "public+public → class_level None"
        );
        assert!(
            enriched.class_compartments.is_none(),
            "public+public → class_compartments None"
        );
    }

    #[test]
    fn into_enriched_declared_only_propagates_to_enriched() {
        let norm = bootstrap_norm_with_class(Some(2), Some(vec!["financial"]));
        let clf = make_classifier_with_sensitivity("evt-merge-test", 0, vec![]);
        let enriched = norm.into_enriched(clf);
        assert_eq!(enriched.class_level, Some(2), "declared level propagates");
        assert_eq!(
            enriched.class_compartments.as_deref(),
            Some(vec!["financial".to_string()].as_slice()),
            "declared compartments propagate"
        );
    }

    #[test]
    fn into_enriched_detected_only_propagates_to_enriched() {
        let norm = bootstrap_norm_with_class(None, None);
        let clf = make_classifier_with_sensitivity("evt-merge-test", 3, vec!["security"]);
        let enriched = norm.into_enriched(clf);
        assert_eq!(enriched.class_level, Some(3), "detected level propagates");
        assert_eq!(
            enriched.class_compartments.as_deref(),
            Some(vec!["security".to_string()].as_slice()),
            "detected compartments propagate"
        );
    }

    #[test]
    fn into_enriched_detected_escalates_over_declared() {
        // declared=confidential(2), detected=restricted(3) → result=restricted(3)
        let norm = bootstrap_norm_with_class(Some(2), Some(vec!["financial"]));
        let clf = make_classifier_with_sensitivity("evt-merge-test", 3, vec!["security"]);
        let enriched = norm.into_enriched(clf);
        assert_eq!(enriched.class_level, Some(3), "max(2,3)=3");
        let mut comps = enriched.class_compartments.expect("compartments set");
        comps.sort();
        assert_eq!(
            comps,
            vec!["financial", "security"],
            "union of compartments"
        );
    }

    #[test]
    fn into_enriched_declared_wins_when_higher_than_detected() {
        // declared=restricted(3), detected=internal(1) → result=restricted(3)
        let norm = bootstrap_norm_with_class(Some(3), Some(vec!["security"]));
        let clf = make_classifier_with_sensitivity("evt-merge-test", 1, vec!["customer_pii"]);
        let enriched = norm.into_enriched(clf);
        assert_eq!(enriched.class_level, Some(3), "max(3,1)=3");
        let mut comps = enriched.class_compartments.expect("compartments set");
        comps.sort();
        assert_eq!(
            comps,
            vec!["customer_pii", "security"],
            "union of compartments"
        );
    }

    #[test]
    fn into_enriched_compartments_deduplicated_in_union() {
        let norm = bootstrap_norm_with_class(Some(1), Some(vec!["security", "financial"]));
        let clf = make_classifier_with_sensitivity("evt-merge-test", 1, vec!["security", "hr"]);
        let enriched = norm.into_enriched(clf);
        assert_eq!(enriched.class_level, Some(1));
        let mut comps = enriched.class_compartments.expect("compartments set");
        comps.sort();
        assert_eq!(
            comps.iter().filter(|c| *c == "security").count(),
            1,
            "security must appear exactly once after union"
        );
        assert!(comps.contains(&"financial".to_string()));
        assert!(comps.contains(&"hr".to_string()));
    }

    #[test]
    fn bootstrap_event_with_class_fields_is_parsed_correctly() {
        let payload = json!({
            "event_id": "01TEST0000000000000000000D",
            "kind": "artifact.code",
            "content_hash": "sha256:abc",
            "redacted_payload": {},
            "source": {"repo": "Cortex"},
            "class_level": 2,
            "class_compartments": ["financial", "hr"]
        });
        let norm = NormalisedEvent::from_bootstrap_event(&payload).expect("normalise");
        assert_eq!(norm.class_level, Some(2));
        assert_eq!(
            norm.class_compartments,
            Some(vec!["financial".to_string(), "hr".to_string()])
        );
    }

    #[test]
    fn bootstrap_event_without_class_fields_yields_none() {
        let payload = json!({
            "event_id": "01TEST0000000000000000000E",
            "kind": "turn",
            "content_hash": "sha256:def",
            "redacted_payload": {},
            "source": {"repo": "Cortex"}
        });
        let norm = NormalisedEvent::from_bootstrap_event(&payload).expect("normalise");
        assert!(norm.class_level.is_none());
        assert!(norm.class_compartments.is_none());
    }

    // -----------------------------------------------------------------
    // phase24 §1.4 — consolidator decision_landed trigger hook.
    // -----------------------------------------------------------------

    /// Publisher that records every (room, envelope) it is handed.
    #[derive(Default)]
    struct RecordingPublisher {
        calls: std::sync::Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl SynapPublisher for RecordingPublisher {
        async fn publish(&self, room: &str, envelope: &Value) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push((room.to_string(), envelope.clone()));
            Ok(())
        }
    }

    /// Consumer whose `ack` succeeds; `next_batch` is never called (the
    /// tests drive `handle_message` directly).
    struct AckOnlyConsumer;

    #[async_trait]
    impl SynapConsumer for AckOnlyConsumer {
        async fn next_batch(&self, _room: &str, _max: usize) -> Result<Vec<ConsumedMessage>> {
            Ok(Vec::new())
        }
        async fn ack(&self, _room: &str, _offset: u64) -> Result<()> {
            Ok(())
        }
    }

    fn decision_message(offset: u64) -> ConsumedMessage {
        ConsumedMessage {
            offset,
            kind: "decision".into(),
            payload: json!({
                "event_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "schema_version": "1",
                "occurred_at": "2026-06-06T12:00:00.000Z",
                "session_id": "sess-d",
                "stream": "live",
                "tool": "claude-code",
                "kind": "decision",
                "context": {"repo": "cortex", "platform": "claude-code"},
                "payload": {
                    "decision_id": "ADR-0042",
                    "title": "Adopt Meili",
                    "status": "accepted",
                    "body": "We pick Meili.",
                    "tags": []
                },
                "content_hash": "sha256:dec"
            }),
            event_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".into()),
        }
    }

    fn worker_with_trigger(enabled: bool, publisher: Arc<RecordingPublisher>) -> Worker {
        let config = ClassifierWorkerConfig {
            consolidator_trigger_enabled: enabled,
            ..ClassifierWorkerConfig::default()
        };
        Worker::new(
            config,
            Arc::new(NoopClassifier),
            Arc::new(AckOnlyConsumer),
            publisher,
        )
    }

    #[tokio::test]
    async fn decision_event_publishes_trigger_when_enabled() {
        let pub_ = Arc::new(RecordingPublisher::default());
        let worker = worker_with_trigger(true, pub_.clone());
        worker
            .handle_message(STREAM_RAW, decision_message(1))
            .await
            .expect("handle decision");
        let calls = pub_.calls.lock().unwrap();
        // One enriched publish + one trigger publish.
        let trigger = calls
            .iter()
            .find(|(room, _)| room == crate::consolidator::daemon::TRIGGER_STREAM)
            .expect("a decision_landed trigger must be published");
        assert_eq!(trigger.1["kind"], "decision_landed");
        assert_eq!(trigger.1["decision_id"], "ADR-0042");
    }

    #[tokio::test]
    async fn decision_event_publishes_no_trigger_when_disabled() {
        let pub_ = Arc::new(RecordingPublisher::default());
        let worker = worker_with_trigger(false, pub_.clone());
        worker
            .handle_message(STREAM_RAW, decision_message(2))
            .await
            .expect("handle decision");
        let calls = pub_.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|(room, _)| room != crate::consolidator::daemon::TRIGGER_STREAM),
            "no trigger must be published when the flag is off"
        );
    }

    #[tokio::test]
    async fn turn_event_never_publishes_trigger_even_when_enabled() {
        let pub_ = Arc::new(RecordingPublisher::default());
        let worker = worker_with_trigger(true, pub_.clone());
        let msg = ConsumedMessage {
            offset: 3,
            kind: "turn".into(),
            payload: json!({
                "event_id": "01ARZ3NDEKTSV4RRFFQ69G5FBT",
                "schema_version": "1",
                "occurred_at": "2026-06-06T12:00:00.000Z",
                "session_id": "sess-t",
                "stream": "live",
                "tool": "claude-code",
                "kind": "turn",
                "context": {"repo": "cortex", "platform": "claude-code"},
                "payload": {"user_message": "hi", "assistant_message": "yo", "tokens": null},
                "content_hash": "sha256:turn"
            }),
            event_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FBT".into()),
        };
        worker
            .handle_message(STREAM_RAW, msg)
            .await
            .expect("handle turn");
        let calls = pub_.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|(room, _)| room != crate::consolidator::daemon::TRIGGER_STREAM),
            "a turn must never trigger consolidation"
        );
    }

    // -----------------------------------------------------------------
    // Phase24 §1.3 — per-repo topic-grain threshold tests.
    // -----------------------------------------------------------------

    fn turn_message_with_repo(offset: u64, repo: &str) -> ConsumedMessage {
        ConsumedMessage {
            offset,
            kind: "turn".into(),
            payload: json!({
                "event_id": format!("01ARZ3NDEKTSV4RRFFQ69G5F{:02}", offset),
                "schema_version": "1",
                "occurred_at": "2026-06-06T12:00:00.000Z",
                "session_id": format!("sess-{offset}"),
                "stream": "live",
                "tool": "claude-code",
                "kind": "turn",
                "context": { "repo": repo, "platform": "claude-code" },
                "payload": {"user_message": "hi", "assistant_message": "yo", "tokens": null},
                "content_hash": format!("sha256:turn{offset}")
            }),
            event_id: Some(format!("01ARZ3NDEKTSV4RRFFQ69G5F{:02}", offset)),
        }
    }

    #[tokio::test]
    async fn topic_trigger_fires_after_threshold_events_from_same_repo() {
        let pub_ = Arc::new(RecordingPublisher::default());
        let worker = worker_with_trigger(true, pub_.clone());
        let threshold = crate::topic_cards::trigger::TRIGGER_EVENTS_THRESHOLD;
        // Send exactly `threshold` events for the same repo.
        for i in 0..threshold {
            worker
                .handle_message(STREAM_RAW, turn_message_with_repo(u64::from(i) + 100, "cortex"))
                .await
                .expect("handle turn");
        }
        let calls = pub_.calls.lock().unwrap();
        let nightly = calls
            .iter()
            .find(|(room, body)| {
                room == crate::consolidator::daemon::TRIGGER_STREAM
                    && body.get("kind").and_then(|v| v.as_str()) == Some("nightly_topic")
            });
        assert!(
            nightly.is_some(),
            "a nightly_topic trigger must fire after {threshold} events for the same repo"
        );
        if let Some((_, body)) = nightly {
            assert_eq!(body["repo"], "cortex");
        }
    }

    #[tokio::test]
    async fn topic_trigger_does_not_fire_before_threshold() {
        let pub_ = Arc::new(RecordingPublisher::default());
        let worker = worker_with_trigger(true, pub_.clone());
        let threshold = crate::topic_cards::trigger::TRIGGER_EVENTS_THRESHOLD;
        // Send one fewer than the threshold.
        for i in 0..(threshold - 1) {
            worker
                .handle_message(STREAM_RAW, turn_message_with_repo(u64::from(i) + 200, "nexus"))
                .await
                .expect("handle turn");
        }
        let calls = pub_.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|(room, body)| {
                    !(room == crate::consolidator::daemon::TRIGGER_STREAM
                        && body.get("kind").and_then(|v| v.as_str()) == Some("nightly_topic"))
                }),
            "nightly_topic must NOT fire before threshold events"
        );
    }

    // -----------------------------------------------------------------
    // Phase11s §1 — supervisor + liveness probe regression tests.
    // -----------------------------------------------------------------

    /// Always-failing consumer used to drive the supervisor through
    /// the consecutive-error threshold without touching live Synap.
    struct AlwaysFailingConsumer;

    #[async_trait]
    impl SynapConsumer for AlwaysFailingConsumer {
        async fn next_batch(&self, _room: &str, _max: usize) -> Result<Vec<ConsumedMessage>> {
            Err(anyhow::anyhow!("synap consume: simulated transport error"))
        }
        async fn ack(&self, _room: &str, _offset: u64) -> Result<()> {
            Ok(())
        }
    }

    /// No-op publisher — the failing consumer short-circuits before
    /// publish runs, so the supervisor tests never reach this code.
    struct NoopPublisher;

    #[async_trait]
    impl SynapPublisher for NoopPublisher {
        async fn publish(&self, _room: &str, _envelope: &Value) -> Result<()> {
            Ok(())
        }
    }

    /// No-op classifier — never reached in supervisor tests because
    /// the always-failing consumer aborts the batch upstream.
    struct NoopClassifier;

    #[async_trait]
    impl Classifier for NoopClassifier {
        async fn classify_batch(
            &self,
            _events: &[EnrichmentInput],
        ) -> Result<Vec<ClassifierOutput>, crate::classifier::ClassifierError> {
            Ok(Vec::new())
        }
    }

    fn supervisor_worker(threshold: u64) -> Worker {
        Worker::new(
            ClassifierWorkerConfig::default(),
            Arc::new(NoopClassifier),
            Arc::new(AlwaysFailingConsumer),
            Arc::new(NoopPublisher),
        )
        .with_max_consume_errors(threshold)
        .with_input_streams(vec![STREAM_RAW.to_string()])
    }

    #[tokio::test]
    async fn supervisor_returns_err_when_consecutive_errors_hit_threshold() {
        // Phase11s §1.5 — the supervisor MUST return Err after N
        // consecutive consume errors so docker `restart:
        // unless-stopped` recovers a stuck container. Pin the
        // contract: 3 consecutive errors at threshold=3 trip the
        // exit, and the consume-errors-consecutive counter
        // surfaces the exact count for /healthz.
        let worker = Arc::new(supervisor_worker(3));
        let shutdown = Arc::new(AtomicBool::new(false));
        let result = crate::synap_worker::run_forever(worker.clone(), shutdown).await;
        let err = result.expect_err("supervisor must trip");
        let msg = err.to_string();
        assert!(
            msg.contains("consume loop stuck"),
            "error must surface the supervisor exit reason: {msg}"
        );
        assert!(
            msg.contains("threshold 3"),
            "error must echo the configured threshold: {msg}"
        );
        // Counter reflects the consecutive failures up to the
        // moment the supervisor tripped — at threshold N the
        // counter is exactly N (incremented before the threshold
        // check fires).
        assert_eq!(worker.consume_errors_consecutive(), 3);
        // last_consume_ts_ms stays 0 because every consume failed.
        assert_eq!(worker.last_consume_ts_ms(), 0);
    }

    #[tokio::test]
    async fn supervisor_threshold_zero_disables_exit() {
        // Phase11s §1.2 — threshold = 0 keeps the pre-§1.2
        // dormant-loop behaviour (operators can opt out). The
        // worker logs the warning forever; we drive it for a
        // bounded number of iterations via the shutdown flag and
        // assert it never returned Err.
        let worker = Arc::new(supervisor_worker(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_kill = shutdown.clone();
        let kill = tokio::spawn(async move {
            // Real time — let the loop fail a handful of times
            // (each iteration sleeps 500ms in run_forever's
            // back-off branch) then signal shutdown.
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            shutdown_for_kill.store(true, Ordering::Relaxed);
        });
        let result = crate::synap_worker::run_forever(worker.clone(), shutdown).await;
        let _ = kill.await;
        result.expect("threshold=0 must keep the loop alive until shutdown");
    }

    /// Consumer that fails N times then succeeds — exercises the
    /// reset-on-success contract.
    struct FlakyThenStableConsumer {
        remaining_failures: AtomicU64,
    }

    #[async_trait]
    impl SynapConsumer for FlakyThenStableConsumer {
        async fn next_batch(&self, _room: &str, _max: usize) -> Result<Vec<ConsumedMessage>> {
            // Saturating-at-zero decrement — the naive
            // `fetch_sub > 0` check wraps around u64 and reverts
            // to perpetual failures, which masks the
            // counter-reset contract under test.
            let prior = self
                .remaining_failures
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                    Some(v.saturating_sub(1))
                })
                .unwrap_or(0);
            if prior > 0 {
                Err(anyhow::anyhow!("transient blip"))
            } else {
                Ok(Vec::new())
            }
        }
        async fn ack(&self, _room: &str, _offset: u64) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn consume_error_counter_resets_on_successful_return() {
        // Phase11s §1.2 — a transient blip (2 failures, then
        // recovery) MUST not trip the supervisor when the
        // threshold is 5. The counter resets to 0 on the first
        // successful return, and `last_consume_ts_ms` lands.
        let worker = Arc::new(
            Worker::new(
                ClassifierWorkerConfig::default(),
                Arc::new(NoopClassifier),
                Arc::new(FlakyThenStableConsumer {
                    remaining_failures: AtomicU64::new(2),
                }),
                Arc::new(NoopPublisher),
            )
            .with_max_consume_errors(5)
            .with_input_streams(vec![STREAM_RAW.to_string()]),
        );
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_kill = shutdown.clone();
        let kill = tokio::spawn(async move {
            // Two failures (500ms back-off each) + at least one
            // success cycle (200ms idle when handled=0) before
            // signalling shutdown.
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            shutdown_for_kill.store(true, Ordering::Relaxed);
        });
        let result = crate::synap_worker::run_forever(worker.clone(), shutdown).await;
        let _ = kill.await;
        result.expect("transient blip must not trip supervisor");
        assert_eq!(worker.consume_errors_consecutive(), 0);
        assert!(
            worker.last_consume_ts_ms() > 0,
            "last_consume_ts_ms must land after the first successful return"
        );
    }

    #[test]
    fn max_consume_errors_default_is_five() {
        // Phase11s §1.2 — pin the default constant so a future
        // bump surfaces here before the operator runbook gets
        // out of sync.
        assert_eq!(DEFAULT_MAX_CONSUME_ERRORS, 5);
    }
}
