//! Phase11j §2.8 — per-grain cost telemetry.
//!
//! The orchestrator records every summariser charge against a
//! [`CostLedger`]. The ledger surfaces under
//! `/v1/health/coverage` (phase11i §5.2 wiring) so operators can
//! catch a cost regression before the monthly budget burns.
//!
//! Default monthly cap is $1 000 (100 000 cents). The
//! [`CostBudget::remaining_for_grain`] helper lets the orchestrator
//! decide whether to keep emitting or to stop and report
//! `buckets_pending` so the next nightly run resumes cleanly.

use std::collections::BTreeMap;

/// Cost figures per grain (USD cents, monotonically increasing).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GrainCost {
    /// Number of consolidations produced for this grain in the
    /// current accounting window.
    pub consolidations: u32,
    /// Total cents charged.
    pub cost_cents: u32,
}

impl GrainCost {
    /// Mean cost per consolidation in cents. Returns `0` when no
    /// consolidations have been produced yet.
    pub fn mean_cost_cents(&self) -> u32 {
        if self.consolidations == 0 {
            0
        } else {
            self.cost_cents / self.consolidations
        }
    }
}

/// Running ledger keyed by grain label
/// (`"session"`/`"topic"`/`"decision_trace"`).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CostLedger {
    /// Per-grain running totals.
    pub per_grain: BTreeMap<String, GrainCost>,
    /// Total spend (cents) since the ledger was created.
    pub total_cents: u32,
}

impl CostLedger {
    /// Record a single charge against the ledger. The orchestrator
    /// calls this for every summariser response.
    pub fn record(&mut self, grain_label: &str, cost_cents: u32) {
        let bucket = self.per_grain.entry(grain_label.to_string()).or_default();
        bucket.consolidations = bucket.consolidations.saturating_add(1);
        bucket.cost_cents = bucket.cost_cents.saturating_add(cost_cents);
        self.total_cents = self.total_cents.saturating_add(cost_cents);
    }

    /// Reset the ledger — the cron caller invokes this at the start
    /// of every accounting window (typically monthly).
    pub fn reset(&mut self) {
        self.per_grain.clear();
        self.total_cents = 0;
    }
}

/// Operator-tunable budget. Default: $1 000/month.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CostBudget {
    /// Hard cap on total cents per accounting window.
    pub monthly_cents_cap: u32,
}

impl Default for CostBudget {
    fn default() -> Self {
        Self {
            monthly_cents_cap: 100_000,
        }
    }
}

impl CostBudget {
    /// Cents the orchestrator may still spend in this window.
    /// Returns 0 when the cap is exhausted.
    pub fn remaining_cents(&self, ledger: &CostLedger) -> u32 {
        self.monthly_cents_cap.saturating_sub(ledger.total_cents)
    }

    /// Quick gate the orchestrator uses before invoking a
    /// summariser: returns false when adding `est_cents` would
    /// breach the cap.
    pub fn can_afford(&self, ledger: &CostLedger, est_cents: u32) -> bool {
        ledger.total_cents.saturating_add(est_cents) <= self.monthly_cents_cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_accumulates_per_grain_and_total_cents() {
        let mut l = CostLedger::default();
        l.record("session", 80);
        l.record("session", 80);
        l.record("decision_trace", 5_000);
        assert_eq!(l.total_cents, 5_160);
        assert_eq!(l.per_grain["session"].consolidations, 2);
        assert_eq!(l.per_grain["session"].cost_cents, 160);
        assert_eq!(l.per_grain["decision_trace"].cost_cents, 5_000);
        assert_eq!(l.per_grain["session"].mean_cost_cents(), 80);
    }

    #[test]
    fn reset_clears_every_bucket() {
        let mut l = CostLedger::default();
        l.record("session", 80);
        l.reset();
        assert_eq!(l.total_cents, 0);
        assert!(l.per_grain.is_empty());
    }

    #[test]
    fn budget_remaining_and_can_afford_track_the_cap() {
        let mut l = CostLedger::default();
        l.record("session", 99_999);
        let budget = CostBudget::default();
        assert_eq!(budget.remaining_cents(&l), 1);
        assert!(budget.can_afford(&l, 1));
        assert!(!budget.can_afford(&l, 2));
        l.record("session", 1);
        assert_eq!(budget.remaining_cents(&l), 0);
    }
}
