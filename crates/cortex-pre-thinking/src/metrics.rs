//! `cortex.prethink.*` counters / histograms (spec 12 §Observability).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::budget::TrimStep;

/// Pre-thinking metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.prethink.calls.total{intent}`.
    pub calls_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex.prethink.bundle.bytes` histogram.
    pub bundle_bytes: Mutex<Vec<u32>>,
    /// `cortex.prethink.sections.count{section}` histogram.
    pub section_count: Mutex<BTreeMap<String, Vec<u32>>>,
    /// `cortex.prethink.truncation.applied{step}`.
    pub truncation_applied: Mutex<BTreeMap<String, u64>>,
    /// `cortex.prethink.latency_ms` histogram.
    pub latency_ms: Mutex<Vec<u64>>,
    /// `cortex.prethink.empty_bundle`.
    pub empty_bundle: AtomicU64,
    /// `cortex.prethink.timeouts`.
    pub timeouts: AtomicU64,
    /// Phase14e — `cortex_pre_thinking_fail_open_total{reason}`.
    /// Reasons: `timeout`, `network`, `unauthorised`, `internal`,
    /// `breaker_open`. Read by the doctor + the
    /// `/v1/health/pre-thinking` endpoint to surface outage
    /// counts to the operator.
    pub fail_open_total: Mutex<BTreeMap<String, u64>>,
    /// Phase14f — `cortex_pre_thinking_bundle_bytes_per_intent`
    /// histogram. Per-intent bundle-size sample so the dashboard
    /// can render p50/p95/p99 per intent.
    pub bundle_bytes_per_intent: Mutex<BTreeMap<String, Vec<u32>>>,
    /// Phase14f — `cortex_pre_thinking_helpful_total{intent,helpful}`.
    /// Driven by feedback POSTs; the dashboard derives
    /// `helpful_rate = helpful / (helpful + unhelpful)` per intent.
    pub helpful_total: Mutex<BTreeMap<(String, bool), u64>>,
}

impl Metrics {
    /// Fresh registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Increment the call counter for `intent`.
    pub fn incr_calls(&self, intent: &str) {
        if let Ok(mut m) = self.calls_total.lock() {
            *m.entry(intent.to_string()).or_insert(0) += 1;
        }
    }
    /// Record a bundle size observation.
    pub fn observe_bundle_bytes(&self, n: u32) {
        if let Ok(mut g) = self.bundle_bytes.lock() {
            g.push(n);
        }
    }
    /// Record a section-count observation.
    pub fn observe_section_count(&self, section: &str, n: u32) {
        if let Ok(mut m) = self.section_count.lock() {
            m.entry(section.to_string()).or_default().push(n);
        }
    }
    /// Record a truncation step.
    pub fn incr_truncation_step(&self, step: TrimStep) {
        if let Ok(mut m) = self.truncation_applied.lock() {
            *m.entry(format!("{step:?}")).or_insert(0) += 1;
        }
    }
    /// Record a latency observation.
    pub fn observe_latency_ms(&self, ms: u64) {
        if let Ok(mut g) = self.latency_ms.lock() {
            g.push(ms);
        }
    }
    /// Increment the empty-bundle counter.
    pub fn incr_empty_bundle(&self) {
        self.empty_bundle.fetch_add(1, Ordering::Relaxed);
    }
    /// Increment the timeout counter.
    pub fn incr_timeouts(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    /// Phase14e — bump the per-reason fail-open counter. Called
    /// from the pipeline on every fail-open dispatch (real
    /// upstream failure OR breaker-open short-circuit).
    pub fn incr_fail_open(&self, reason: &str) {
        if let Ok(mut m) = self.fail_open_total.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }

    /// Phase14e — snapshot the per-reason fail-open counters.
    /// Used by the doctor + the `/v1/health/pre-thinking`
    /// endpoint.
    pub fn fail_open_snapshot(&self) -> BTreeMap<String, u64> {
        self.fail_open_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Phase14f — record a per-intent bundle-byte sample.
    /// Histogram values are buffered in-memory; the
    /// dashboard projection (`bundle_bytes_quantiles_per_intent`)
    /// summarises them to p50/p95/p99.
    pub fn observe_bundle_bytes_per_intent(&self, intent: &str, n: u32) {
        if let Ok(mut m) = self.bundle_bytes_per_intent.lock() {
            m.entry(intent.to_string()).or_default().push(n);
        }
    }

    /// Phase14f — bump the helpful counter for an
    /// `(intent, helpful)` pair.
    pub fn incr_helpful(&self, intent: &str, helpful: bool) {
        if let Ok(mut m) = self.helpful_total.lock() {
            *m.entry((intent.to_string(), helpful)).or_insert(0) += 1;
        }
    }

    /// Phase14f — snapshot per-intent bundle-byte quantiles. Each
    /// inner map carries `{ count, p50, p95, p99 }`. Empty intents
    /// are omitted.
    pub fn bundle_bytes_quantiles_per_intent(&self) -> BTreeMap<String, IntentByteQuantiles> {
        let mut out: BTreeMap<String, IntentByteQuantiles> = BTreeMap::new();
        let map = match self.bundle_bytes_per_intent.lock() {
            Ok(g) => g,
            Err(_) => return out,
        };
        for (intent, samples) in map.iter() {
            if samples.is_empty() {
                continue;
            }
            let mut sorted = samples.clone();
            sorted.sort_unstable();
            let q = IntentByteQuantiles {
                count: sorted.len() as u64,
                p50: percentile(&sorted, 0.50),
                p95: percentile(&sorted, 0.95),
                p99: percentile(&sorted, 0.99),
            };
            out.insert(intent.clone(), q);
        }
        out
    }

    /// Phase14f — snapshot per-intent `(helpful, unhelpful, rate)`
    /// triples driven by feedback POSTs. `rate = helpful /
    /// (helpful + unhelpful)`; `None` when both are zero.
    pub fn helpful_rate_per_intent(&self) -> BTreeMap<String, IntentHelpfulRate> {
        let mut out: BTreeMap<String, IntentHelpfulRate> = BTreeMap::new();
        let map = match self.helpful_total.lock() {
            Ok(g) => g,
            Err(_) => return out,
        };
        // Build temporary aggregate so a single pass covers both
        // halves of the (intent, helpful) pair.
        let mut tmp: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for ((intent, helpful), n) in map.iter() {
            let e = tmp.entry(intent.clone()).or_insert((0, 0));
            if *helpful {
                e.0 += n;
            } else {
                e.1 += n;
            }
        }
        for (intent, (helpful, unhelpful)) in tmp {
            let total = helpful + unhelpful;
            let rate = if total == 0 {
                None
            } else {
                Some(helpful as f64 / total as f64)
            };
            out.insert(
                intent,
                IntentHelpfulRate {
                    helpful,
                    unhelpful,
                    rate,
                },
            );
        }
        out
    }
}

/// Phase14f — per-intent bundle-byte quantile row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentByteQuantiles {
    /// Sample count.
    pub count: u64,
    /// 50th percentile (median).
    pub p50: u32,
    /// 95th percentile.
    pub p95: u32,
    /// 99th percentile.
    pub p99: u32,
}

/// Phase14f — per-intent helpful-rate row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentHelpfulRate {
    /// Helpful=true count.
    pub helpful: u64,
    /// Helpful=false count.
    pub unhelpful: u64,
    /// `helpful / (helpful + unhelpful)`; `None` when both zero.
    pub rate: Option<f64>,
}

fn percentile(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_bytes_quantiles_per_intent_picks_p50_p95_p99() {
        let m = Metrics::new();
        for n in 1u32..=100 {
            m.observe_bundle_bytes_per_intent("explain", n);
        }
        let q = m.bundle_bytes_quantiles_per_intent();
        let row = q.get("explain").unwrap();
        assert_eq!(row.count, 100);
        // round((100-1)*0.50) = 50 → sorted[50] = 51
        assert_eq!(row.p50, 51);
        // round((100-1)*0.95) = 94 → sorted[94] = 95
        assert_eq!(row.p95, 95);
        // round((100-1)*0.99) = 98 → sorted[98] = 99
        assert_eq!(row.p99, 99);
    }

    #[test]
    fn helpful_rate_per_intent_computes_ratio() {
        let m = Metrics::new();
        for _ in 0..3 {
            m.incr_helpful("explain", true);
        }
        m.incr_helpful("explain", false);
        m.incr_helpful("law_check", true);
        let r = m.helpful_rate_per_intent();
        let e = r.get("explain").unwrap();
        assert_eq!(e.helpful, 3);
        assert_eq!(e.unhelpful, 1);
        assert!((e.rate.unwrap() - 0.75).abs() < 1e-9);
        let lc = r.get("law_check").unwrap();
        assert_eq!(lc.rate, Some(1.0));
    }

    #[test]
    fn empty_intent_returns_zero_quantile_row_only_when_present() {
        let m = Metrics::new();
        let q = m.bundle_bytes_quantiles_per_intent();
        assert!(q.is_empty());
    }
}
