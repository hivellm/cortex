//! In-process counters / histograms that back the `cortex.embedder.*` metric
//! family in `docs/specs/06-embedder.md` §Observability.
//!
//! This is a light-weight atomic-counter implementation. A Prometheus /
//! OpenTelemetry exporter can read these values directly or wrap them in a
//! registry — that wiring lives outside this crate.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::chunker::ChunkSource;

/// Embedder metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.embedder.chunks.total{source=code}`.
    pub chunks_total_code: AtomicU64,
    /// `cortex.embedder.chunks.total{source=doc}`.
    pub chunks_total_doc: AtomicU64,
    /// `cortex.embedder.chunks.total{source=summary}`.
    pub chunks_total_summary: AtomicU64,
    /// `cortex.embedder.chunks.total{source=fallback_window}`.
    pub chunks_total_fallback_window: AtomicU64,
    /// `cortex.embedder.chunks.total{source=raw_oversize}`.
    pub chunks_total_raw_oversize: AtomicU64,
    /// `cortex.embedder.chunks.bytes` — sum of chunk byte sizes seen.
    pub chunks_bytes_sum: AtomicU64,
    /// `cortex.embedder.chunks.bytes` — count of observations.
    pub chunks_bytes_count: AtomicU64,
    /// `cortex.embedder.upsert.latency_ms` — observations (ms samples).
    pub upsert_latency_ms: Mutex<Vec<u32>>,
    /// `cortex.embedder.upsert.batch_size` — observations.
    pub upsert_batch_size: Mutex<Vec<u32>>,
    /// `cortex.embedder.dedup.hits`.
    pub dedup_hits: AtomicU64,
    /// `cortex.embedder.vectorizer.errors` — per HTTP-status bucket.
    pub vectorizer_errors: AtomicU64,
    /// `cortex.embedder.backpressure.active` — 0 / 1.
    pub backpressure_active: AtomicU64,
    /// `cortex.embedder.oversize_without_summary`.
    pub oversize_without_summary: AtomicU64,
    /// Phase8b — total Synap messages the worker has processed end
    /// to end. Bumped after each `run_once` returns a non-zero count.
    pub jobs_processed_total: AtomicU64,
    /// Phase8b — Unix-epoch ms of the most recent successful job.
    /// `0` until the worker handles its first message; the freshness
    /// aggregator flags the row as `degraded` when the gap exceeds
    /// 60 s.
    pub last_job_ts_ms: AtomicU64,
}

impl Metrics {
    /// Create a fresh metrics registry with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record N chunks of the given source.
    pub fn incr_chunks(&self, source: ChunkSource, count: u64) {
        let counter = match source {
            ChunkSource::Code => &self.chunks_total_code,
            ChunkSource::Doc => &self.chunks_total_doc,
            ChunkSource::Summary => &self.chunks_total_summary,
            ChunkSource::FallbackWindow => &self.chunks_total_fallback_window,
            ChunkSource::RawOversize => &self.chunks_total_raw_oversize,
        };
        counter.fetch_add(count, Ordering::Relaxed);
    }

    /// Record a chunk byte-size observation.
    pub fn observe_chunk_bytes(&self, bytes: u64) {
        self.chunks_bytes_sum.fetch_add(bytes, Ordering::Relaxed);
        self.chunks_bytes_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an upsert latency in milliseconds.
    pub fn observe_upsert_latency(&self, ms: u32) {
        if let Ok(mut g) = self.upsert_latency_ms.lock() {
            g.push(ms);
        }
    }

    /// Record an upsert batch size.
    pub fn observe_upsert_batch(&self, size: u32) {
        if let Ok(mut g) = self.upsert_batch_size.lock() {
            g.push(size);
        }
    }

    /// Increment the dedup-hit counter.
    pub fn incr_dedup_hits(&self, n: u64) {
        self.dedup_hits.fetch_add(n, Ordering::Relaxed);
    }

    /// Increment the Vectorizer-error counter.
    pub fn incr_vectorizer_errors(&self, n: u64) {
        self.vectorizer_errors.fetch_add(n, Ordering::Relaxed);
    }

    /// Flip the backpressure-active gauge.
    pub fn set_backpressure(&self, active: bool) {
        self.backpressure_active
            .store(u64::from(active), Ordering::Relaxed);
    }

    /// Increment the oversize-without-summary counter.
    pub fn incr_oversize_without_summary(&self) {
        self.oversize_without_summary
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Phase8a — sum of every per-source `chunks_total_*` counter,
    /// surfaced in the `/healthz` extras as `chunks_written_total`.
    pub fn chunks_written_total(&self) -> u64 {
        self.chunks_total_code.load(Ordering::Relaxed)
            + self.chunks_total_doc.load(Ordering::Relaxed)
            + self.chunks_total_summary.load(Ordering::Relaxed)
            + self.chunks_total_fallback_window.load(Ordering::Relaxed)
            + self.chunks_total_raw_oversize.load(Ordering::Relaxed)
    }

    /// Phase8a — read the cumulative Vectorizer-error counter.
    pub fn vectorizer_errors_total(&self) -> u64 {
        self.vectorizer_errors.load(Ordering::Relaxed)
    }

    /// Phase8b — record `n` jobs processed, stamping the freshness
    /// timestamp at the same instant. `n == 0` is a no-op so an idle
    /// poll loop never bumps the freshness signal.
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
    /// Phase8b — read the most-recent-job timestamp (Unix-epoch ms).
    pub fn last_job_ts_ms(&self) -> u64 {
        self.last_job_ts_ms.load(Ordering::Relaxed)
    }

    /// Phase8b — render the embedder counters in Prometheus text
    /// format. Used by the admin `/metrics` endpoint.
    pub fn render_prom(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        out.push_str("# TYPE cortex_embedder_jobs_processed_total counter\n");
        let _ = writeln!(
            out,
            "cortex_embedder_jobs_processed_total {}",
            self.jobs_processed_total()
        );
        out.push_str("# TYPE cortex_embedder_last_job_ts_ms gauge\n");
        let _ = writeln!(
            out,
            "cortex_embedder_last_job_ts_ms {}",
            self.last_job_ts_ms()
        );
        out.push_str("# TYPE cortex_embedder_chunks_written_total counter\n");
        let _ = writeln!(
            out,
            "cortex_embedder_chunks_written_total {}",
            self.chunks_written_total()
        );
        out.push_str("# TYPE cortex_embedder_vectorizer_errors_total counter\n");
        let _ = writeln!(
            out,
            "cortex_embedder_vectorizer_errors_total {}",
            self.vectorizer_errors_total()
        );
        out.push_str("# TYPE cortex_embedder_dedup_hits_total counter\n");
        let _ = writeln!(
            out,
            "cortex_embedder_dedup_hits_total {}",
            self.dedup_hits.load(Ordering::Relaxed)
        );
        out
    }
}
