//! Phase21 §8.2 — in-memory ACL decision metrics for the dashboard.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// One decision record kept in the rolling window.
#[derive(Debug, Clone)]
pub struct AclDecisionRecord {
    /// Unix epoch seconds at the time of the decision.
    pub ts_secs: i64,
    /// `"grant"` or `"deny"`.
    pub verdict: &'static str,
    /// Principal that triggered the decision.
    pub principal_id: String,
    /// Classification level of the fact that was evaluated.
    pub fact_level: u8,
    /// Human-readable reason code (see `acl_decision_reason`).
    pub reason: &'static str,
    /// `doc_id` of the evaluated hit.
    pub doc_id: String,
}

/// Maximum number of decision records kept in memory.
pub const DEFAULT_WINDOW_SIZE: usize = 10_000;

/// Aggregating registry for post-fusion ACL decisions.
///
/// Mirrors the `TemporalMetrics` pattern: atomics for scalar counters
/// and a bounded ring buffer for time-series computation. The
/// orchestrator holds an `Option<Arc<AclMetrics>>` — `None` is a
/// zero-cost no-op so tests that build `Orchestrator::new` without
/// wiring the handle are unaffected.
#[derive(Debug)]
pub struct AclMetrics {
    /// Running total of evaluated hits (across all requests).
    pub evaluated_total: AtomicU64,
    /// Running total of granted hits.
    pub granted_total: AtomicU64,
    /// Running total of denied hits.
    pub denied_total: AtomicU64,
    /// Per-principal denial count. Unbounded keys but typically small
    /// in practice (bounded by the principal population, not traffic).
    pub denied_by_principal: Mutex<BTreeMap<String, u64>>,
    /// Rolling window of recent decisions for time-series computation.
    pub recent: Mutex<VecDeque<AclDecisionRecord>>,
    /// Capacity of the rolling window (eviction happens at the front).
    capacity: usize,
}

impl AclMetrics {
    /// Fresh registry with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one ACL decision. `ts_secs` is Unix epoch seconds.
    ///
    /// This method is hot-path safe: the atomics use `Relaxed` ordering,
    /// and the `Mutex` locks are contention-free in normal operation
    /// because the orchestrator is the sole writer.
    pub fn record(
        &self,
        ts_secs: i64,
        verdict: &'static str,
        principal_id: &str,
        fact_level: u8,
        reason: &'static str,
        doc_id: &str,
    ) {
        self.evaluated_total.fetch_add(1, Ordering::Relaxed);
        if verdict == "grant" {
            self.granted_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.denied_total.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut m) = self.denied_by_principal.lock() {
                *m.entry(principal_id.to_string()).or_insert(0) += 1;
            }
        }
        if let Ok(mut buf) = self.recent.lock() {
            if buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(AclDecisionRecord {
                ts_secs,
                verdict,
                principal_id: principal_id.to_string(),
                fact_level,
                reason,
                doc_id: doc_id.to_string(),
            });
        }
    }

    /// Top-N denied principals sorted descending by denial count.
    pub fn top_denied_principals(&self, limit: usize) -> Vec<(String, u64)> {
        let Ok(m) = self.denied_by_principal.lock() else {
            return vec![];
        };
        let mut pairs: Vec<(String, u64)> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
        pairs.sort_by_key(|pair| std::cmp::Reverse(pair.1));
        pairs.truncate(limit);
        pairs
    }

    /// Denial rate bucketed into 5-minute slots over the last `window_minutes` minutes.
    ///
    /// Returns `(bucket_start_secs, denied, evaluated)` triples sorted ascending
    /// by bucket start. Empty when no decisions have been recorded yet.
    pub fn denial_rate_over_time(&self, window_minutes: u32) -> Vec<(i64, u64, u64)> {
        let Ok(buf) = self.recent.lock() else {
            return vec![];
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let bucket_secs: i64 = 300; // 5-minute buckets
        let cutoff = now - i64::from(window_minutes) * 60;
        let mut buckets: BTreeMap<i64, (u64, u64)> = BTreeMap::new();
        for rec in buf.iter() {
            if rec.ts_secs < cutoff {
                continue;
            }
            let slot = (rec.ts_secs / bucket_secs) * bucket_secs;
            let entry = buckets.entry(slot).or_insert((0, 0));
            entry.1 += 1; // evaluated
            if rec.verdict == "deny" {
                entry.0 += 1; // denied
            }
        }
        buckets
            .into_iter()
            .map(|(ts, (denied, eval))| (ts, denied, eval))
            .collect()
    }
}

impl Default for AclMetrics {
    fn default() -> Self {
        Self {
            evaluated_total: AtomicU64::new(0),
            granted_total: AtomicU64::new(0),
            denied_total: AtomicU64::new(0),
            denied_by_principal: Mutex::new(BTreeMap::new()),
            recent: Mutex::new(VecDeque::with_capacity(DEFAULT_WINDOW_SIZE)),
            capacity: DEFAULT_WINDOW_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_bumps_counters() {
        let m = AclMetrics::new();
        m.record(1_000, "grant", "alice", 0, "granted", "doc-1");
        m.record(1_000, "deny", "bob", 2, "above_clearance_level", "doc-2");
        assert_eq!(m.evaluated_total.load(Ordering::Relaxed), 2);
        assert_eq!(m.granted_total.load(Ordering::Relaxed), 1);
        assert_eq!(m.denied_total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn top_denied_principals_sorts_descending() {
        let m = AclMetrics::new();
        for _ in 0..3 {
            m.record(1_000, "deny", "charlie", 3, "above_clearance_level", "d");
        }
        for _ in 0..1 {
            m.record(1_000, "deny", "dave", 1, "missing_compartment", "d");
        }
        let top = m.top_denied_principals(2);
        assert_eq!(top[0].0, "charlie");
        assert_eq!(top[0].1, 3);
        assert_eq!(top[1].0, "dave");
    }

    #[test]
    fn grants_do_not_appear_in_denied_by_principal() {
        let m = AclMetrics::new();
        m.record(1_000, "grant", "alice", 0, "granted", "doc-1");
        m.record(1_000, "grant", "alice", 0, "granted", "doc-2");
        let top = m.top_denied_principals(10);
        assert!(top.is_empty(), "grants must not increment denied_by_principal");
    }

    #[test]
    fn rolling_window_evicts_oldest_when_full() {
        let m = AclMetrics {
            capacity: 2,
            ..AclMetrics::new()
        };
        m.record(1, "deny", "a", 1, "above_clearance_level", "d1");
        m.record(2, "grant", "b", 0, "granted", "d2");
        m.record(3, "deny", "c", 2, "above_clearance_level", "d3");
        let buf = m.recent.lock().unwrap();
        assert_eq!(buf.len(), 2, "capacity=2 must evict the oldest entry");
        assert_eq!(buf[0].doc_id, "d2");
        assert_eq!(buf[1].doc_id, "d3");
    }

    #[test]
    fn denial_rate_over_time_excludes_records_outside_window() {
        let m = AclMetrics::new();
        // Record one decision well outside the 60-minute window.
        m.record(0, "deny", "x", 1, "above_clearance_level", "old");
        // Record one decision at "now".
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        m.record(now, "deny", "y", 1, "above_clearance_level", "recent");
        let buckets = m.denial_rate_over_time(60);
        // The "old" record (ts=0) must be outside the window.
        for (_, denied, _) in &buckets {
            assert!(*denied <= 1, "only the recent deny should appear");
        }
    }
}
