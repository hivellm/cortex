//! Minimal Prometheus-text metrics.
//!
//! Intentionally dep-free (no `prometheus` crate): we only need counters +
//! a histogram-ish accumulator for `/metrics`. When the full governance
//! story lands we can swap for a real registry without breaking callers.

use std::sync::atomic::{AtomicU64, Ordering};

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
}

impl Metrics {
    /// Render metrics in Prometheus text format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE cortex_events_received counter\n");
        out.push_str(&format!(
            "cortex_events_received {}\n",
            self.events_received.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE cortex_events_rejected counter\n");
        out.push_str(&format!(
            "cortex_events_rejected {}\n",
            self.events_rejected.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE cortex_events_routed counter\n");
        out.push_str(&format!(
            "cortex_events_routed{{stream=\"raw\"}} {}\n",
            self.events_routed_raw.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "cortex_events_routed{{stream=\"bootstrap\"}} {}\n",
            self.events_routed_bootstrap.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE cortex_redaction_hits counter\n");
        out.push_str(&format!(
            "cortex_redaction_hits {}\n",
            self.redaction_hits.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE cortex_publisher_errors counter\n");
        out.push_str(&format!(
            "cortex_publisher_errors {}\n",
            self.publisher_errors.load(Ordering::Relaxed)
        ));
        out.push_str("# TYPE cortex_archive_errors counter\n");
        out.push_str(&format!(
            "cortex_archive_errors {}\n",
            self.archive_errors.load(Ordering::Relaxed)
        ));
        out
    }
}

