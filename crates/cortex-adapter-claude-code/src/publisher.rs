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

use crate::events::ClaudeEvent;
use crate::metrics::Metrics;
use crate::wal::OverflowWal;

/// Publisher trait so tests use a recording fake.
#[async_trait]
pub trait Publisher: Send + Sync {
    /// Push an event onto the publisher. Returns the queue depth
    /// after enqueue. Implementations may drop oldest under pressure.
    async fn publish(&self, event: ClaudeEvent) -> usize;

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
    queue: Arc<tokio::sync::Mutex<std::collections::VecDeque<ClaudeEvent>>>,
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
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client builder");
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
    pub async fn replay_wal(&self) -> usize {
        let entries = match self.wal.drain() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "wal drain at startup failed");
                return 0;
            }
        };
        let mut count = 0usize;
        for value in entries {
            if let Ok(event) = serde_json::from_value::<ClaudeEvent>(value) {
                self.enqueue_internal(event).await;
                count += 1;
            }
        }
        count
    }

    /// Enqueue without metric noise; used by replay.
    async fn enqueue_internal(&self, event: ClaudeEvent) {
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

    async fn flush_locked(
        &self,
        q: &mut std::collections::VecDeque<ClaudeEvent>,
    ) -> usize {
        if q.is_empty() {
            return 0;
        }
        let take = q.len().min(self.batch_size);
        let mut batch: Vec<ClaudeEvent> = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(e) = q.pop_front() {
                batch.push(e);
            }
        }
        let n = batch.len();
        match self.post_batch(&batch).await {
            Ok(()) => {
                for e in &batch {
                    self.metrics.incr_events_total(&e.kind);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "publisher batch failed; spilling to WAL");
                self.metrics.incr_publisher_error("network");
                for evt in batch {
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

    async fn post_batch(&self, batch: &[ClaudeEvent]) -> Result<(), reqwest::Error> {
        let body: Value = json!({
            "events": batch
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                .collect::<Vec<_>>()
        });
        let url = format!(
            "{}/v1/events/batch",
            self.endpoint.trim_end_matches('/')
        );
        let mut attempt = 0u32;
        let attempts = self.max_retry.max(1);
        loop {
            let resp = self.client.post(&url).json(&body).send().await;
            match resp {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 202 => {
                    return Ok(());
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

#[async_trait]
impl Publisher for HttpPublisher {
    async fn publish(&self, event: ClaudeEvent) -> usize {
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
        self.queue
            .try_lock()
            .map(|g| g.len())
            .unwrap_or(0)
    }
}

/// In-memory publisher for tests — never touches HTTP.
#[derive(Default)]
pub struct MemoryPublisher {
    /// Recorded events in arrival order.
    pub events: Arc<tokio::sync::Mutex<Vec<ClaudeEvent>>>,
}

impl MemoryPublisher {
    /// Fresh recorder.
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot of recorded events.
    pub async fn snapshot(&self) -> Vec<ClaudeEvent> {
        self.events.lock().await.clone()
    }
    /// Total events recorded.
    pub async fn count(&self) -> usize {
        self.events.lock().await.len()
    }
}

#[async_trait]
impl Publisher for MemoryPublisher {
    async fn publish(&self, event: ClaudeEvent) -> usize {
        let mut g = self.events.lock().await;
        g.push(event);
        g.len()
    }
    async fn flush(&self) -> usize {
        0
    }
    fn queue_depth(&self) -> usize {
        self.events
            .try_lock()
            .map(|g| g.len())
            .unwrap_or(0)
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
