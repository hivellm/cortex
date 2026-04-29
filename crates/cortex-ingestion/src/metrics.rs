//! Minimal Prometheus-text metrics.
//!
//! Intentionally dep-free (no `prometheus` crate): we only need counters +
//! a histogram-ish accumulator for `/metrics`. When the full governance
//! story lands we can swap for a real registry without breaking callers.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Shared metrics counters.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Events accepted (after validation + redaction).
    pub events_received: AtomicU64,
    /// Events rejected by validation.
    pub events_rejected: AtomicU64,
    /// Events routed to `cortex.events.raw`.
    pub events_routed_raw: AtomicU64,
    /// Events routed to `cortex.events.bootstrap`.
    pub events_routed_bootstrap: AtomicU64,
    /// Redaction token matches emitted.
    pub redaction_hits: AtomicU64,
    /// Publisher errors (durable queue will absorb).
    pub publisher_errors: AtomicU64,
    /// Archive write errors.
    pub archive_errors: AtomicU64,
    /// Phase8a — Unix-epoch milliseconds of the most recent
    /// successfully accepted batch. `0` until the first ingest
    /// completes; `/healthz` reads this to detect "no batch in last
    /// 60s" and downgrade to `degraded`.
    pub last_batch_accepted_ts_ms: AtomicU64,
    /// Phase8b — `cortex_ingestion_events_received_total{kind}`.
    /// Bumped from `process_event` after the envelope passes
    /// validation, before archive write.
    pub events_received_by_kind: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `cortex_ingestion_events_archived_total{kind}`.
    /// Bumped right after the archive writer accepts an envelope —
    /// same point as the durability-counter bump.
    pub events_archived_by_kind: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `cortex_ingestion_events_rejected_total{reason}`.
    /// Bumped per validation/archive/publisher failure path.
    pub events_rejected_by_reason: Mutex<BTreeMap<String, u64>>,
    /// Phase8b — `last_archive_write_ts_ms{kind}` — Unix-epoch ms of
    /// the most recent successful archive write per canonical kind.
    /// Source of truth for the freshness aggregator's
    /// `ingestion.archived.<kind>` rows.
    pub last_archive_write_ts_ms: Mutex<BTreeMap<String, u64>>,
}

impl Metrics {
    /// Render metrics in Prometheus text format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE cortex_events_received counter\n");
        let _ = writeln!(
            out,
            "cortex_events_received {}",
            self.events_received.load(Ordering::Relaxed)
        );
        out.push_str("# TYPE cortex_events_rejected counter\n");
        let _ = writeln!(
            out,
            "cortex_events_rejected {}",
            self.events_rejected.load(Ordering::Relaxed)
        );
        out.push_str("# TYPE cortex_events_routed counter\n");
        let _ = writeln!(
            out,
            "cortex_events_routed{{stream=\"raw\"}} {}",
            self.events_routed_raw.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            out,
            "cortex_events_routed{{stream=\"bootstrap\"}} {}",
            self.events_routed_bootstrap.load(Ordering::Relaxed)
        );
        out.push_str("# TYPE cortex_redaction_hits counter\n");
        let _ = writeln!(
            out,
            "cortex_redaction_hits {}",
            self.redaction_hits.load(Ordering::Relaxed)
        );
        out.push_str("# TYPE cortex_publisher_errors counter\n");
        let _ = writeln!(
            out,
            "cortex_publisher_errors {}",
            self.publisher_errors.load(Ordering::Relaxed)
        );
        out.push_str("# TYPE cortex_archive_errors counter\n");
        let _ = writeln!(
            out,
            "cortex_archive_errors {}",
            self.archive_errors.load(Ordering::Relaxed)
        );
        out.push_str("# TYPE cortex_last_batch_accepted_ts_ms gauge\n");
        let _ = writeln!(
            out,
            "cortex_last_batch_accepted_ts_ms {}",
            self.last_batch_accepted_ts_ms.load(Ordering::Relaxed)
        );
        // Phase8b — per-kind throughput.
        out.push_str("# TYPE cortex_ingestion_events_received_total counter\n");
        for (kind, n) in self.events_received_snapshot() {
            let _ = writeln!(
                out,
                "cortex_ingestion_events_received_total{{kind=\"{kind}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_ingestion_events_archived_total counter\n");
        for (kind, n) in self.events_archived_snapshot() {
            let _ = writeln!(
                out,
                "cortex_ingestion_events_archived_total{{kind=\"{kind}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_ingestion_events_rejected_total counter\n");
        for (reason, n) in self.events_rejected_snapshot() {
            let _ = writeln!(
                out,
                "cortex_ingestion_events_rejected_total{{reason=\"{reason}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_ingestion_last_archive_write_ts_ms gauge\n");
        for (kind, ms) in self.last_archive_write_ts_snapshot() {
            let _ = writeln!(
                out,
                "cortex_ingestion_last_archive_write_ts_ms{{kind=\"{kind}\"}} {ms}"
            );
        }
        out
    }

    fn now_ms() -> u64 {
        chrono::Utc::now().timestamp_millis().max(0) as u64
    }

    /// Phase8b — bump `events_received_total{kind}`. Called from the
    /// router right after validation + redaction, before archive
    /// write.
    pub fn incr_events_received_kind(&self, kind: &str) {
        if let Ok(mut m) = self.events_received_by_kind.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    /// Phase8b — bump `events_archived_total{kind}` and stamp
    /// `last_archive_write_ts_ms{kind}` with the current wall-clock
    /// ms. Called from the router right after the archive writer
    /// accepts the envelope.
    pub fn incr_events_archived_kind(&self, kind: &str) {
        if let Ok(mut m) = self.events_archived_by_kind.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut m) = self.last_archive_write_ts_ms.lock() {
            m.insert(kind.to_string(), Self::now_ms());
        }
    }
    /// Phase8b — bump `events_rejected_total{reason}`. Reason buckets
    /// are coarse (`invalid_json`, `archive_failure`,
    /// `publisher_failure`, `validation`) so cardinality stays bounded.
    pub fn incr_events_rejected_reason(&self, reason: &str) {
        if let Ok(mut m) = self.events_rejected_by_reason.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }
    /// Phase8b — snapshot `events_received_by_kind`.
    pub fn events_received_snapshot(&self) -> BTreeMap<String, u64> {
        self.events_received_by_kind
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot `events_archived_by_kind`.
    pub fn events_archived_snapshot(&self) -> BTreeMap<String, u64> {
        self.events_archived_by_kind
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot `events_rejected_by_reason`.
    pub fn events_rejected_snapshot(&self) -> BTreeMap<String, u64> {
        self.events_rejected_by_reason
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Phase8b — snapshot `last_archive_write_ts_ms`.
    pub fn last_archive_write_ts_snapshot(&self) -> BTreeMap<String, u64> {
        self.last_archive_write_ts_ms
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Stamp the current Unix-epoch ms onto
    /// `last_batch_accepted_ts_ms`. Called from the ingest path
    /// after a successful publish.
    pub fn record_batch_accepted_now(&self) {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.last_batch_accepted_ts_ms.store(now_ms, Ordering::Relaxed);
    }

    /// Read the most recent accepted-batch timestamp in
    /// Unix-epoch ms. `0` means no batch has been accepted yet.
    pub fn last_batch_accepted_ts_ms(&self) -> u64 {
        self.last_batch_accepted_ts_ms.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_registry_per_kind_is_empty() {
        let m = Metrics::default();
        assert!(m.events_received_snapshot().is_empty());
        assert!(m.events_archived_snapshot().is_empty());
        assert!(m.events_rejected_snapshot().is_empty());
        assert!(m.last_archive_write_ts_snapshot().is_empty());
    }

    #[test]
    fn incr_events_received_kind_pivots_per_kind() {
        let m = Metrics::default();
        m.incr_events_received_kind("turn");
        m.incr_events_received_kind("turn");
        m.incr_events_received_kind("tool_call");
        let snap = m.events_received_snapshot();
        assert_eq!(snap.get("turn"), Some(&2));
        assert_eq!(snap.get("tool_call"), Some(&1));
    }

    #[test]
    fn incr_events_archived_kind_stamps_last_archive_write() {
        let m = Metrics::default();
        let before = chrono::Utc::now().timestamp_millis().max(0) as u64;
        m.incr_events_archived_kind("tool_call");
        let after = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let snap = m.events_archived_snapshot();
        assert_eq!(snap.get("tool_call"), Some(&1));
        let ts_snap = m.last_archive_write_ts_snapshot();
        let ts = *ts_snap.get("tool_call").expect("kind key present");
        assert!(ts >= before && ts <= after);
    }

    #[test]
    fn incr_events_rejected_reason_pivots_per_reason() {
        let m = Metrics::default();
        m.incr_events_rejected_reason("invalid_json");
        m.incr_events_rejected_reason("invalid_json");
        m.incr_events_rejected_reason("validation");
        let snap = m.events_rejected_snapshot();
        assert_eq!(snap.get("invalid_json"), Some(&2));
        assert_eq!(snap.get("validation"), Some(&1));
    }

    #[test]
    fn render_includes_per_kind_rows() {
        let m = Metrics::default();
        m.incr_events_received_kind("tool_call");
        m.incr_events_archived_kind("tool_call");
        m.incr_events_rejected_reason("validation");
        let txt = m.render();
        assert!(txt.contains(
            "cortex_ingestion_events_received_total{kind=\"tool_call\"} 1"
        ));
        assert!(txt.contains(
            "cortex_ingestion_events_archived_total{kind=\"tool_call\"} 1"
        ));
        assert!(txt.contains(
            "cortex_ingestion_events_rejected_total{reason=\"validation\"} 1"
        ));
        assert!(txt.contains("cortex_ingestion_last_archive_write_ts_ms{kind=\"tool_call\"}"));
    }
}

