//! Async publisher — drains the in-memory queue to
//! `cortex-core /v1/events/batch`. Spec 10 §Asynchronous publisher.
//!
//! Behaviour:
//! - Bounded queue (`queue_bounded`, default 2 048).
//! - Drained in batches of 32 with at most 200 ms between flushes.
//! - HTTP 5xx → up to 3 retries with exponential backoff.
//! - Persistent failure → spill to the overflow WAL.
//! - Queue-full → drop-oldest with a metric bump (and the dropped
//!   event is mirrored to the WAL).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::metrics::Metrics;
use crate::wal::OverflowWal;
use cortex_core::events::Envelope;

/// Publisher trait so tests use a recording fake.
#[async_trait]
pub trait Publisher: Send + Sync {
    /// Push an event onto the publisher. Returns the queue depth
    /// after enqueue. Implementations may drop oldest under pressure.
    async fn publish(&self, event: Envelope) -> usize;

    /// Force-drain whatever is queued. Returns the number of events
    /// the drain attempted to write. Used at shutdown / tests.
    async fn flush(&self) -> usize;

    /// Current queue depth (best-effort; not strongly consistent).
    fn queue_depth(&self) -> usize;
}

/// Live HTTP publisher.
pub struct HttpPublisher {
    client: Client,
    endpoint: String,
    queue: Arc<tokio::sync::Mutex<std::collections::VecDeque<Envelope>>>,
    queue_bounded: usize,
    batch_size: usize,
    max_retry: u32,
    wal: Arc<OverflowWal>,
    metrics: Arc<Metrics>,
}

impl HttpPublisher {
    /// Build a new publisher bound to `endpoint`.
    pub fn new(
        endpoint: impl Into<String>,
        queue_bounded: usize,
        timeout: Duration,
        wal: Arc<OverflowWal>,
        metrics: Arc<Metrics>,
    ) -> Self {
        // Phase14i §1.2 — never panic on the daemon's boot path.
        // A reqwest client-build failure is exceptional (TLS init
        // catastrophe) but if it happens we fall back to the
        // default Client + WARN log so the rest of the daemon
        // (sync paths, dispatcher, IPC) keeps serving instead of
        // taking the whole user-session capture down.
        let client = match Client::builder().timeout(timeout).build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    timeout_ms = timeout.as_millis() as u64,
                    "reqwest client builder failed; falling back to default Client (per-call timeout unset)"
                );
                Client::new()
            }
        };
        Self {
            client,
            endpoint: endpoint.into(),
            queue: Arc::new(tokio::sync::Mutex::new(std::collections::VecDeque::new())),
            queue_bounded: queue_bounded.max(1),
            batch_size: 32,
            max_retry: 3,
            wal,
            metrics,
        }
    }

    /// Replay everything queued in the WAL. Spec 10 §Asynchronous
    /// publisher: WAL is drained at startup. Each replayed event goes
    /// straight into the publisher queue so the normal drain machinery
    /// handles the retry semantics.
    ///
    /// Lines that don't deserialize as a canonical [`Envelope`] are
    /// dropped — typically these are legacy `ClaudeEvent` lines from
    /// the pre-spec-04 build. The drop count is logged once at
    /// `WARN`.
    pub async fn replay_wal(&self) -> usize {
        let entries = match self.wal.drain() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "wal drain at startup failed");
                return 0;
            }
        };
        let mut count = 0usize;
        let mut dropped = 0usize;
        for value in entries {
            match serde_json::from_value::<Envelope>(value) {
                Ok(event) => {
                    self.enqueue_internal(event).await;
                    count += 1;
                }
                Err(_) => dropped += 1,
            }
        }
        if dropped > 0 {
            tracing::warn!(
                dropped,
                "dropped legacy WAL lines from pre-spec-04 build (no canonical envelope)"
            );
        }
        count
    }

    /// Enqueue without metric noise; used by replay.
    async fn enqueue_internal(&self, event: Envelope) {
        let mut q = self.queue.lock().await;
        if q.len() >= self.queue_bounded {
            // Drop oldest, mirror to WAL.
            if let Some(dropped) = q.pop_front() {
                self.metrics.incr_dropped("queue_full");
                if let Ok(v) = serde_json::to_value(&dropped) {
                    if let Err(e) = self.wal.append(&v) {
                        tracing::warn!(error = %e, "wal mirror on drop failed");
                    }
                }
            }
        }
        q.push_back(event);
    }

    async fn flush_locked(&self, q: &mut std::collections::VecDeque<Envelope>) -> usize {
        if q.is_empty() {
            return 0;
        }
        let take = q.len().min(self.batch_size);
        let mut batch: Vec<Envelope> = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(e) = q.pop_front() {
                batch.push(e);
            }
        }
        let n = batch.len();
        match self.post_batch(&batch).await {
            Ok(report) => {
                // The ingestion `/v1/events/batch` endpoint replies
                // with `{accepted, rejected, errors[]}` inside an
                // HTTP 202 (spec 04). Treating "any 2xx" as success
                // hides every per-envelope rejection — schema drift,
                // synap room missing, archive backpressure. Parse the
                // body so the failure surfaces in metrics + logs.
                self.metrics.add_publisher_accepted(report.accepted as u64);
                if report.accepted > 0 {
                    // Phase8a — stamp the most recent successful
                    // publish so /healthz can detect publisher stall.
                    self.metrics.record_publish_ok_now();
                }
                // Phase8b — accepted vs rejected per-envelope split.
                // BatchReport.errors carries indices the ingestion
                // side rejected; everything not in that index list
                // counts as `publish_ok{kind}`. That keeps the counter
                // truthful — without the split, all rejected envelopes
                // would still bump `publish_ok` and the divergence
                // signal disappears.
                let rejected_indices: std::collections::BTreeSet<usize> =
                    report.errors.iter().map(|err| err.index).collect();
                for (i, e) in batch.iter().enumerate() {
                    // events_total is "submitted to ingestion", not
                    // "accepted". The split shows up as a divergence
                    // between events_total and publisher_accepted in
                    // /metrics — that gap IS the silent loss.
                    self.metrics.incr_events_total(e.kind.schema_stem());
                    if rejected_indices.contains(&i) {
                        self.metrics
                            .incr_envelopes_publish_fail(e.kind.schema_stem());
                    } else {
                        self.metrics.incr_envelopes_publish_ok(e.kind.schema_stem());
                    }
                }
                if report.rejected > 0 {
                    let sample = report
                        .errors
                        .first()
                        .map(|err| {
                            format!(
                                "index={} event_id={:?} errors={:?}",
                                err.index, err.event_id, err.errors
                            )
                        })
                        .unwrap_or_else(|| "<no error sample>".to_string());
                    tracing::warn!(
                        batch_size = n,
                        accepted = report.accepted,
                        rejected = report.rejected,
                        sample = %sample,
                        "ingestion accepted batch but rejected envelopes inside the 202 body — events archived state depends on the failure class (see ingestion logs)"
                    );
                    for err in &report.errors {
                        let reason = classify_batch_error(&err.errors);
                        self.metrics.incr_publisher_rejected(reason);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "publisher batch failed; spilling to WAL");
                self.metrics.incr_publisher_error("network");
                for evt in batch {
                    // Phase8b — every envelope in a network-failed
                    // batch counts as `publish_fail{kind}`. The WAL
                    // replay path will re-deliver later; until then,
                    // built-vs-publish_ok diverges by exactly the
                    // batch size, which is the WAL-spill smoking gun.
                    self.metrics
                        .incr_envelopes_publish_fail(evt.kind.schema_stem());
                    if let Ok(v) = serde_json::to_value(&evt) {
                        if let Err(e) = self.wal.append(&v) {
                            tracing::warn!(error = %e, "wal append after publish failure failed");
                        }
                    }
                }
            }
        }
        n
    }

    async fn post_batch(&self, batch: &[Envelope]) -> Result<BatchReport, reqwest::Error> {
        let body: Value = json!({
            "events": batch
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                .collect::<Vec<_>>()
        });
        let url = format!("{}/v1/events/batch", self.endpoint.trim_end_matches('/'));
        let mut attempt = 0u32;
        let attempts = self.max_retry.max(1);
        loop {
            let resp = self.client.post(&url).json(&body).send().await;
            match resp {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 202 => {
                    let bytes = match r.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            // Body read failed after a 2xx — treat
                            // as a successful post with no per-event
                            // detail, since the server has already
                            // committed. Log so operators know they
                            // lost the diagnostic body.
                            tracing::warn!(
                                error = %e,
                                "ingestion 2xx but response body read failed; cannot tell accepted vs rejected"
                            );
                            return Ok(BatchReport::all_accepted(batch.len()));
                        }
                    };
                    let report = match serde_json::from_slice::<BatchReport>(&bytes) {
                        Ok(r) => r,
                        Err(_) => {
                            // Older ingestion builds (or /v1/events
                            // single-event path replays) might not
                            // ship the canonical envelope. Default
                            // to "all accepted" which preserves the
                            // historical caller contract.
                            BatchReport::all_accepted(batch.len())
                        }
                    };
                    return Ok(report);
                }
                Ok(r) if r.status().is_server_error() => {
                    attempt += 1;
                    if attempt >= attempts {
                        // Manufacture an error for the caller. The
                        // body content is irrelevant — we only need
                        // to signal "post failed permanently".
                        return Err(r.error_for_status().unwrap_err());
                    }
                    let delay_ms = 100u64 << (2 * (attempt - 1) as u64);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                Ok(r) => {
                    return Err(r.error_for_status().unwrap_err());
                }
                Err(e) => {
                    attempt += 1;
                    if attempt >= attempts || !e.is_timeout() && !e.is_connect() {
                        return Err(e);
                    }
                    let delay_ms = 100u64 << (2 * (attempt - 1) as u64);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }
}

/// Mirror of cortex-ingestion `BatchResponse` — kept independent so
/// the adapter doesn't depend on the ingestion crate.
#[derive(Debug, serde::Deserialize)]
struct BatchReport {
    #[serde(default)]
    accepted: usize,
    #[serde(default)]
    rejected: usize,
    #[serde(default)]
    errors: Vec<BatchErrorEntry>,
}

impl BatchReport {
    fn all_accepted(n: usize) -> Self {
        Self {
            accepted: n,
            rejected: 0,
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct BatchErrorEntry {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
}

/// Bucket a batch-error message list into a coarse reason label so
/// `cortex.adapter.publisher.rejected{reason}` stays low-cardinality.
/// Anything we don't recognise lands in `other`.
fn classify_batch_error(messages: &[String]) -> &'static str {
    if messages.is_empty() {
        return "empty";
    }
    let joined = messages.join(" | ").to_lowercase();
    if joined.contains("does not match") || joined.contains("required property") {
        "schema"
    } else if joined.contains("synap") || joined.contains("publisher:") {
        "publisher"
    } else if joined.contains("archive") {
        "archive"
    } else {
        "other"
    }
}

#[async_trait]
impl Publisher for HttpPublisher {
    async fn publish(&self, event: Envelope) -> usize {
        let mut q = self.queue.lock().await;
        if q.len() >= self.queue_bounded {
            if let Some(dropped) = q.pop_front() {
                self.metrics.incr_dropped("queue_full");
                if let Ok(v) = serde_json::to_value(&dropped) {
                    if let Err(e) = self.wal.append(&v) {
                        tracing::warn!(error = %e, "wal mirror on drop failed");
                    }
                }
            }
        }
        q.push_back(event);
        q.len()
    }

    async fn flush(&self) -> usize {
        let mut total = 0usize;
        loop {
            let mut q = self.queue.lock().await;
            let n = self.flush_locked(&mut q).await;
            if n == 0 {
                break;
            }
            total += n;
        }
        total
    }

    fn queue_depth(&self) -> usize {
        self.queue.try_lock().map(|g| g.len()).unwrap_or(0)
    }
}

/// In-memory publisher for tests — never touches HTTP.
#[derive(Default)]
pub struct MemoryPublisher {
    /// Recorded events in arrival order.
    pub events: Arc<tokio::sync::Mutex<Vec<Envelope>>>,
}

impl MemoryPublisher {
    /// Fresh recorder.
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot of recorded events.
    pub async fn snapshot(&self) -> Vec<Envelope> {
        self.events.lock().await.clone()
    }
    /// Total events recorded.
    pub async fn count(&self) -> usize {
        self.events.lock().await.len()
    }
}

#[async_trait]
impl Publisher for MemoryPublisher {
    async fn publish(&self, event: Envelope) -> usize {
        let mut g = self.events.lock().await;
        g.push(event);
        g.len()
    }
    async fn flush(&self) -> usize {
        0
    }
    fn queue_depth(&self) -> usize {
        self.events.try_lock().map(|g| g.len()).unwrap_or(0)
    }
}

/// Spawn a background flush task that wakes every `interval` and
/// drains the publisher. Returns a handle the caller drops to stop
/// the loop.
pub fn spawn_flusher(
    publisher: Arc<dyn Publisher>,
    interval: Duration,
    shutdown: Arc<tokio::sync::Notify>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    let _ = publisher.flush().await;
                }
                _ = shutdown.notified() => {
                    let _ = publisher.flush().await;
                    break;
                }
            }
        }
    })
}
