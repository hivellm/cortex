//! In-process counters / histograms backing the
//! `cortex.bootstrap.*` metric family in
//! `docs/specs/09-bootstrap-cli.md` §Progress & telemetry.
//!
//! Light-weight atomic-counter implementation matching the embedder /
//! graph / full-text style. Any Prometheus / OpenTelemetry exporter
//! can read these values directly.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Bootstrap-CLI metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.bootstrap.files.walked` — files accepted by the walker
    /// (`reason=oversize` etc. drop into `files_dropped`).
    pub files_walked: Mutex<BTreeMap<String, u64>>,
    /// `cortex.bootstrap.files.skipped` — files dropped, keyed by
    /// `(repo, reason)`. `reason` is `oversize`, `extension`,
    /// `path_excluded`, `binary`, `not_a_file`.
    pub files_dropped: Mutex<BTreeMap<(String, String), u64>>,
    /// `cortex.bootstrap.events.emitted` — events emitted, keyed by
    /// `(repo, kind)`.
    pub events_emitted: Mutex<BTreeMap<(String, String), u64>>,
    /// `cortex.bootstrap.bytes.processed`.
    pub bytes_processed: Mutex<BTreeMap<String, u64>>,
    /// `cortex.bootstrap.commits.walked`.
    pub commits_walked: Mutex<BTreeMap<String, u64>>,
    /// `cortex.bootstrap.repo.duration_s` — wall-clock per repo.
    pub repo_duration_s: Mutex<BTreeMap<String, f64>>,
    /// `cortex.bootstrap.errors` — keyed by `(repo, stage)`.
    pub errors: Mutex<BTreeMap<(String, String), u64>>,
    /// `cortex.bootstrap.publish.latency_ms` — histogram-style buckets.
    pub publish_latency_ms: Mutex<Vec<u32>>,
    /// Total redactions applied across the run.
    pub redactions: AtomicU64,
}

impl Metrics {
    /// Fresh registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment `files_walked{repo}` by 1.
    pub fn incr_files_walked(&self, repo: &str) {
        if let Ok(mut m) = self.files_walked.lock() {
            *m.entry(repo.to_string()).or_insert(0) += 1;
        }
    }
    /// Increment `files_dropped{repo, reason}` by 1.
    pub fn incr_files_dropped(&self, repo: &str, reason: &str) {
        if let Ok(mut m) = self.files_dropped.lock() {
            *m.entry((repo.to_string(), reason.to_string()))
                .or_insert(0) += 1;
        }
    }
    /// Increment `events_emitted{repo, kind}` by 1.
    pub fn incr_events_emitted(&self, repo: &str, kind: &str) {
        if let Ok(mut m) = self.events_emitted.lock() {
            *m.entry((repo.to_string(), kind.to_string())).or_insert(0) += 1;
        }
    }
    /// Add `n` bytes to `bytes_processed{repo}`.
    pub fn incr_bytes_processed(&self, repo: &str, n: u64) {
        if let Ok(mut m) = self.bytes_processed.lock() {
            *m.entry(repo.to_string()).or_insert(0) += n;
        }
    }
    /// Increment `commits_walked{repo}` by 1.
    pub fn incr_commits_walked(&self, repo: &str) {
        if let Ok(mut m) = self.commits_walked.lock() {
            *m.entry(repo.to_string()).or_insert(0) += 1;
        }
    }
    /// Set `repo_duration_s{repo}`.
    pub fn observe_repo_duration(&self, repo: &str, secs: f64) {
        if let Ok(mut m) = self.repo_duration_s.lock() {
            m.insert(repo.to_string(), secs);
        }
    }
    /// Increment `errors{repo, stage}` by 1.
    pub fn incr_errors(&self, repo: &str, stage: &str) {
        if let Ok(mut m) = self.errors.lock() {
            *m.entry((repo.to_string(), stage.to_string())).or_insert(0) += 1;
        }
    }
    /// Record a `publish_latency_ms` observation.
    pub fn observe_publish_latency(&self, ms: u32) {
        if let Ok(mut g) = self.publish_latency_ms.lock() {
            g.push(ms);
        }
    }
    /// Add `n` to the cumulative redaction counter.
    pub fn incr_redactions(&self, n: u64) {
        self.redactions.fetch_add(n, Ordering::Relaxed);
    }

    /// Sum all `events_emitted` for `repo` regardless of kind.
    pub fn events_total_for(&self, repo: &str) -> u64 {
        self.events_emitted
            .lock()
            .map(|m| {
                m.iter()
                    .filter(|((r, _), _)| r == repo)
                    .map(|(_, v)| *v)
                    .sum()
            })
            .unwrap_or(0)
    }
}
