//! Phase8b — per-loader stage counters exposed through `/healthz`
//! and the freshness aggregator.
//!
//! The two boot-time loaders (`archive_loader`, `meili_loader`) seed
//! the in-memory keyword lane and run on a refresh interval. Until
//! phase8b they had no observable signal beyond a `tracing::info!`
//! per pass; the freshness aggregator needs `last_refresh_ts` and
//! per-kind counters so the GUI can flag a stalled loader the same
//! way it flags a stalled adapter or worker.
//!
//! Both loaders share this registry; cortex-api's `main.rs` builds a
//! single `LoaderMetrics` and clones the `Arc` into each spawned
//! refresh task. The dashboard router (`DashboardState`) holds another
//! clone so the freshness handler can read without duplicating env
//! plumbing.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Combined registry for archive_loader + meili_loader.
#[derive(Debug, Default)]
pub struct LoaderMetrics {
    /// `cortex_archive_loader_last_refresh_ts_ms` — Unix-epoch ms of
    /// the most recent successful archive scan. `0` until the boot
    /// pass completes.
    pub archive_last_refresh_ts_ms: AtomicU64,
    /// `cortex_archive_loader_envelopes_seeded_total{kind}` — total
    /// envelopes the loader has surfaced to the keyword lane keyed
    /// by canonical kind. Each refresh writes the cumulative load
    /// (loader replaces the lane contents on each pass), so the
    /// counter tracks the high-water mark of envelopes seen.
    pub archive_envelopes_seeded: Mutex<BTreeMap<String, u64>>,
    /// `cortex_meili_loader_last_refresh_ts_ms` — Unix-epoch ms of
    /// the most recent successful meili scan.
    pub meili_last_refresh_ts_ms: AtomicU64,
    /// `cortex_meili_loader_docs_seeded_total{family}` — total
    /// docs surfaced from each Meili index family
    /// (`decisions`, `violations`, `memories`, `analyses`, `turns`).
    pub meili_docs_seeded: Mutex<BTreeMap<String, u64>>,
}

impl LoaderMetrics {
    /// Fresh registry with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    fn now_ms() -> u64 {
        chrono::Utc::now().timestamp_millis().max(0) as u64
    }

    /// Stamp the archive loader's `last_refresh_ts_ms` to now.
    pub fn record_archive_refresh_now(&self) {
        self.archive_last_refresh_ts_ms
            .store(Self::now_ms(), Ordering::Relaxed);
    }
    /// Read the archive loader's `last_refresh_ts_ms`.
    pub fn archive_last_refresh_ts_ms(&self) -> u64 {
        self.archive_last_refresh_ts_ms.load(Ordering::Relaxed)
    }
    /// Add `n` archive-seeded envelopes to the per-kind counter.
    pub fn add_archive_envelopes_seeded(&self, kind: &str, n: u64) {
        if n == 0 {
            return;
        }
        if let Ok(mut m) = self.archive_envelopes_seeded.lock() {
            *m.entry(kind.to_string()).or_insert(0) += n;
        }
    }
    /// Snapshot the archive seed counter.
    pub fn archive_envelopes_snapshot(&self) -> BTreeMap<String, u64> {
        self.archive_envelopes_seeded
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
    /// Stamp the meili loader's `last_refresh_ts_ms` to now.
    pub fn record_meili_refresh_now(&self) {
        self.meili_last_refresh_ts_ms
            .store(Self::now_ms(), Ordering::Relaxed);
    }
    /// Read the meili loader's `last_refresh_ts_ms`.
    pub fn meili_last_refresh_ts_ms(&self) -> u64 {
        self.meili_last_refresh_ts_ms.load(Ordering::Relaxed)
    }
    /// Add `n` meili-seeded docs to the per-family counter.
    pub fn add_meili_docs_seeded(&self, family: &str, n: u64) {
        if n == 0 {
            return;
        }
        if let Ok(mut m) = self.meili_docs_seeded.lock() {
            *m.entry(family.to_string()).or_insert(0) += n;
        }
    }
    /// Snapshot the meili seed counter.
    pub fn meili_docs_snapshot(&self) -> BTreeMap<String, u64> {
        self.meili_docs_seeded
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Render the loader metrics in Prometheus text format. The
    /// scraper picks up the same numbers `/healthz` exposes via
    /// extras, so an external Prometheus and the dashboard freshness
    /// aggregator stay aligned.
    pub fn render_prom(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE cortex_archive_loader_last_refresh_ts_ms gauge\n");
        let _ = writeln!(
            out,
            "cortex_archive_loader_last_refresh_ts_ms {}",
            self.archive_last_refresh_ts_ms()
        );
        out.push_str("# TYPE cortex_archive_loader_envelopes_seeded_total counter\n");
        for (kind, n) in self.archive_envelopes_snapshot() {
            let _ = writeln!(
                out,
                "cortex_archive_loader_envelopes_seeded_total{{kind=\"{kind}\"}} {n}"
            );
        }
        out.push_str("# TYPE cortex_meili_loader_last_refresh_ts_ms gauge\n");
        let _ = writeln!(
            out,
            "cortex_meili_loader_last_refresh_ts_ms {}",
            self.meili_last_refresh_ts_ms()
        );
        out.push_str("# TYPE cortex_meili_loader_docs_seeded_total counter\n");
        for (family, n) in self.meili_docs_snapshot() {
            let _ = writeln!(
                out,
                "cortex_meili_loader_docs_seeded_total{{family=\"{family}\"}} {n}"
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_registry_is_zeroed() {
        let m = LoaderMetrics::new();
        assert_eq!(m.archive_last_refresh_ts_ms(), 0);
        assert!(m.archive_envelopes_snapshot().is_empty());
        assert_eq!(m.meili_last_refresh_ts_ms(), 0);
        assert!(m.meili_docs_snapshot().is_empty());
    }

    #[test]
    fn add_archive_envelopes_accumulates_per_kind() {
        let m = LoaderMetrics::new();
        m.add_archive_envelopes_seeded("turn", 3);
        m.add_archive_envelopes_seeded("turn", 2);
        m.add_archive_envelopes_seeded("tool_call", 7);
        let snap = m.archive_envelopes_snapshot();
        assert_eq!(snap.get("turn"), Some(&5));
        assert_eq!(snap.get("tool_call"), Some(&7));
    }

    #[test]
    fn add_zero_does_not_create_entry() {
        let m = LoaderMetrics::new();
        m.add_archive_envelopes_seeded("turn", 0);
        m.add_meili_docs_seeded("decisions", 0);
        assert!(m.archive_envelopes_snapshot().is_empty());
        assert!(m.meili_docs_snapshot().is_empty());
    }

    #[test]
    fn record_refresh_stamps_recent_ms() {
        let m = LoaderMetrics::new();
        let before = chrono::Utc::now().timestamp_millis().max(0) as u64;
        m.record_archive_refresh_now();
        m.record_meili_refresh_now();
        let after = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let arch = m.archive_last_refresh_ts_ms();
        let meili = m.meili_last_refresh_ts_ms();
        assert!(arch >= before && arch <= after);
        assert!(meili >= before && meili <= after);
    }

    #[test]
    fn render_prom_includes_per_kind_rows() {
        let m = LoaderMetrics::new();
        m.add_archive_envelopes_seeded("turn", 4);
        m.add_meili_docs_seeded("decisions", 11);
        m.record_archive_refresh_now();
        let txt = m.render_prom();
        assert!(txt.contains("cortex_archive_loader_envelopes_seeded_total{kind=\"turn\"} 4"));
        assert!(txt.contains("cortex_meili_loader_docs_seeded_total{family=\"decisions\"} 11"));
        assert!(txt.contains("cortex_archive_loader_last_refresh_ts_ms"));
    }
}
