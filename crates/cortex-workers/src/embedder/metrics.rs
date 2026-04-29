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
}
