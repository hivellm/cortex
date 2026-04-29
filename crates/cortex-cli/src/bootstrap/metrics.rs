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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_registry_is_zeroed() {
        let m = Metrics::new();
        assert!(m.files_walked.lock().unwrap().is_empty());
        assert!(m.files_dropped.lock().unwrap().is_empty());
        assert!(m.events_emitted.lock().unwrap().is_empty());
        assert!(m.bytes_processed.lock().unwrap().is_empty());
        assert!(m.commits_walked.lock().unwrap().is_empty());
        assert!(m.repo_duration_s.lock().unwrap().is_empty());
        assert!(m.errors.lock().unwrap().is_empty());
        assert!(m.publish_latency_ms.lock().unwrap().is_empty());
        assert_eq!(m.redactions.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn incr_files_walked_per_repo() {
        let m = Metrics::new();
        m.incr_files_walked("alpha");
        m.incr_files_walked("alpha");
        m.incr_files_walked("beta");
        let map = m.files_walked.lock().unwrap();
        assert_eq!(map.get("alpha"), Some(&2));
        assert_eq!(map.get("beta"), Some(&1));
    }

    #[test]
    fn incr_files_dropped_per_repo_reason_pair() {
        let m = Metrics::new();
        m.incr_files_dropped("alpha", "oversize");
        m.incr_files_dropped("alpha", "oversize");
        m.incr_files_dropped("alpha", "binary");
        let map = m.files_dropped.lock().unwrap();
        assert_eq!(
            map.get(&("alpha".to_string(), "oversize".to_string())),
            Some(&2)
        );
        assert_eq!(
            map.get(&("alpha".to_string(), "binary".to_string())),
            Some(&1)
        );
    }

    #[test]
    fn incr_events_emitted_and_total_for_repo() {
        let m = Metrics::new();
        m.incr_events_emitted("alpha", "artifact.code");
        m.incr_events_emitted("alpha", "artifact.code");
        m.incr_events_emitted("alpha", "artifact.doc");
        m.incr_events_emitted("beta", "artifact.code");
        assert_eq!(m.events_total_for("alpha"), 3);
        assert_eq!(m.events_total_for("beta"), 1);
        assert_eq!(m.events_total_for("gamma"), 0);
    }

    #[test]
    fn incr_bytes_processed_accumulates() {
        let m = Metrics::new();
        m.incr_bytes_processed("alpha", 100);
        m.incr_bytes_processed("alpha", 250);
        assert_eq!(
            m.bytes_processed.lock().unwrap().get("alpha"),
            Some(&350)
        );
    }

    #[test]
    fn incr_commits_walked_per_repo() {
        let m = Metrics::new();
        m.incr_commits_walked("alpha");
        m.incr_commits_walked("alpha");
        assert_eq!(m.commits_walked.lock().unwrap().get("alpha"), Some(&2));
    }

    #[test]
    fn observe_repo_duration_overwrites_per_repo() {
        let m = Metrics::new();
        m.observe_repo_duration("alpha", 1.5);
        m.observe_repo_duration("alpha", 3.2);
        assert_eq!(m.repo_duration_s.lock().unwrap().get("alpha"), Some(&3.2));
    }

    #[test]
    fn incr_errors_keyed_per_repo_stage() {
        let m = Metrics::new();
        m.incr_errors("alpha", "publish");
        m.incr_errors("alpha", "publish");
        m.incr_errors("alpha", "walk");
        assert_eq!(
            m.errors
                .lock()
                .unwrap()
                .get(&("alpha".to_string(), "publish".to_string())),
            Some(&2)
        );
    }

    #[test]
    fn observe_publish_latency_appends() {
        let m = Metrics::new();
        m.observe_publish_latency(11);
        m.observe_publish_latency(22);
        assert_eq!(
            m.publish_latency_ms.lock().unwrap().as_slice(),
            &[11, 22]
        );
    }

    #[test]
    fn incr_redactions_atomic() {
        let m = Metrics::new();
        m.incr_redactions(5);
        m.incr_redactions(7);
        assert_eq!(m.redactions.load(Ordering::Relaxed), 12);
    }
}
