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
    /// Whether hook envelopes should be built and published at all.
    fn enabled(&self) -> bool {
        true
    }

    /// Push an event onto the publisher. Returns the queue depth
    /// after enqueue. Implementations may drop oldest under pressure.
    async fn publish(&self, event: Envelope) -> usize;

    /// Force-drain whatever is queued. Returns the number of events
    /// the drain attempted to write. Used at shutdown / tests.
    async fn flush(&self) -> usize;

    /// Current queue depth (best-effort; not strongly consistent).
    fn queue_depth(&self) -> usize;
}

/// Disabled bulk capture: no queue, HTTP client, retry task, or WAL.
#[derive(Default)]
pub struct DisabledPublisher;

#[async_trait]
impl Publisher for DisabledPublisher {
    fn enabled(&self) -> bool {
        false
    }

    async fn publish(&self, _event: Envelope) -> usize {
        0
    }

    async fn flush(&self) -> usize {
        0
    }

    fn queue_depth(&self) -> usize {
        0
    }
}

/// Live HTTP publisher.
pub struct HttpPublisher {
    client: Client,
    endpoint: String,
    queue: Arc<tokio::sync::Mutex<std::collections::VecDeque<Envelope>>>,
    flush_lock: tokio::sync::Mutex<()>,
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
            flush_lock: tokio::sync::Mutex::new(()),
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

    async fn flush_batch(&self, batch: Vec<Envelope>) -> usize {
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
        // Serialize send/retry order without making prompt-side enqueue wait
        // for the ingestion service. At most one bounded batch is in flight.
        let _flush = self.flush_lock.lock().await;
        let mut total = 0usize;
        loop {
            let batch = {
                let mut queue = self.queue.lock().await;
                let count = queue.len().min(self.batch_size);
                queue.drain(..count).collect::<Vec<_>>()
            };
            if batch.is_empty() {
                break;
            }
            total += self.flush_batch(batch).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{build_event, HookFrame, HookKind};
    use crate::session::SessionManager;

    fn event(prompt: &str) -> Envelope {
        build_event(
            HookKind::UserPromptSubmit,
            &HookFrame {
                hook: "UserPromptSubmit".into(),
                session_id: Some("publisher-fixture".into()),
                cwd: None,
                payload: json!({"prompt":prompt}),
                client: Default::default(),
            },
            &SessionManager::new(),
            1,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn disabled_publisher_retains_no_events_or_queue() {
        let publisher = DisabledPublisher;
        assert!(!publisher.enabled());
        for _ in 0..16 {
            assert_eq!(publisher.publish(event("ignored bulk payload")).await, 0);
        }
        assert_eq!(publisher.flush().await, 0);
        assert_eq!(publisher.queue_depth(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stalled_http_flush_does_not_block_enqueue_and_concurrent_flushes_keep_fifo() {
        use axum::{http::StatusCode, routing::post, Json, Router};
        use tokio::sync::Semaphore;

        let directory = tempfile::tempdir().unwrap();
        let entered = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let received = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let app = Router::new().route(
            "/v1/events/batch",
            post({
                let entered = entered.clone();
                let release = release.clone();
                let received = received.clone();
                move |Json(body): Json<Value>| {
                    let entered = entered.clone();
                    let release = release.clone();
                    let received = received.clone();
                    async move {
                        let ids = body["events"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|event| event["event_id"].as_str().unwrap().to_owned())
                            .collect::<Vec<_>>();
                        let count = ids.len();
                        received.lock().unwrap().push(ids);
                        entered.add_permits(1);
                        release.acquire().await.unwrap().forget();
                        (
                            StatusCode::ACCEPTED,
                            Json(json!({"accepted":count,"rejected":0,"errors":[]})),
                        )
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
        let wal = Arc::new(OverflowWal::open(directory.path().join("overflow.wal")).unwrap());
        let publisher = Arc::new(HttpPublisher::new(
            endpoint,
            2,
            Duration::from_secs(5),
            wal.clone(),
            Arc::new(Metrics::new()),
        ));
        let first = event("first");
        let second = event("second");
        let third = event("third");
        let expected = [
            first.event_id.clone(),
            second.event_id.clone(),
            third.event_id.clone(),
        ];
        publisher.publish(first).await;
        let first_flush = tokio::spawn({
            let publisher = publisher.clone();
            async move { publisher.flush().await }
        });
        tokio::time::timeout(Duration::from_secs(3), entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();

        // The backend has received the first batch but deliberately withholds
        // its response. Prompt publication must still finish well below 600 ms.
        let depth = tokio::time::timeout(Duration::from_millis(250), async {
            publisher.publish(second).await;
            publisher.publish(third).await
        })
        .await
        .expect("prompt enqueue waited on an ingestion HTTP response");
        assert_eq!(depth, 2);
        assert_eq!(publisher.queue_depth(), 2);
        let second_flush = tokio::spawn({
            let publisher = publisher.clone();
            async move { publisher.flush().await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), entered.acquire())
                .await
                .is_err(),
            "another flush overtook the pending first batch"
        );
        release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(3), entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        assert_eq!(
            *received.lock().unwrap(),
            vec![
                vec![expected[0].clone()],
                vec![expected[1].clone(), expected[2].clone()]
            ]
        );
        release.add_permits(1);
        let (first_count, second_count) = tokio::time::timeout(Duration::from_secs(3), async {
            (first_flush.await.unwrap(), second_flush.await.unwrap())
        })
        .await
        .unwrap();
        assert_eq!(first_count + second_count, 3);
        assert_eq!(publisher.queue_depth(), 0);
        assert_eq!(wal.size_bytes(), 0);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn failed_detached_batch_remains_replayable_and_overflow_keeps_oldest_event() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let directory = tempfile::tempdir().unwrap();
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/events/batch"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&upstream)
            .await;
        let wal = Arc::new(OverflowWal::open(directory.path().join("overflow.wal")).unwrap());
        let mut publisher = HttpPublisher::new(
            upstream.uri(),
            2,
            Duration::from_secs(2),
            wal.clone(),
            Arc::new(Metrics::new()),
        );
        publisher.max_retry = 1;
        let first = event("overflow oldest");
        let second = event("failed batch first");
        let third = event("failed batch second");
        let expected = [
            first.event_id.clone(),
            second.event_id.clone(),
            third.event_id.clone(),
        ];
        publisher.publish(first).await;
        publisher.publish(second).await;
        assert_eq!(publisher.publish(third).await, 2);
        assert!(
            wal.size_bytes() > 0,
            "overflow must persist the oldest event"
        );
        assert_eq!(publisher.flush().await, 2);
        assert_eq!(publisher.queue_depth(), 0);
        upstream.verify().await;
        upstream.reset().await;
        Mock::given(method("POST"))
            .and(path("/v1/events/batch"))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "accepted":3,"rejected":0,"errors":[]
            })))
            .expect(1)
            .mount(&upstream)
            .await;
        let replay = HttpPublisher::new(
            upstream.uri(),
            3,
            Duration::from_secs(2),
            wal.clone(),
            Arc::new(Metrics::new()),
        );
        assert_eq!(replay.replay_wal().await, 3);
        assert_eq!(replay.flush().await, 3);
        let requests = upstream.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        let ids = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["event_id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, expected.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(wal.size_bytes(), 0);
        upstream.verify().await;
    }
}
