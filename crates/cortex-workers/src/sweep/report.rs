//! [`SweepReport`] — the uniform per-invocation outcome every
//! [`Sweep::run`](super::Sweep::run) returns. The scheduler persists
//! this into `retention_sweeps` (one row per invocation, ADR-009
//! §4.2); the dashboard reads only [`SweepReportView`], the
//! projection.
//!
//! Field layout per the proposal (`docs/analysis/rework/04-
//! architecture.md` §A.1 + task §2.3):
//!
//! - `name` — the sweep's stable identifier
//!   ([`super::Sweep::name`]).
//! - `started_at` / `finished_at` — bracket the invocation. The
//!   scheduler stamps `started_at` before calling
//!   [`super::Sweep::run`]; the sweep itself stamps `finished_at`
//!   via `finish_*`.
//! - `status` — [`SweepStatus`]; mirrors the existing
//!   `retention_sweeps.status` taxonomy.
//! - `bytes_reclaimed` — best-effort byte counter. Sweeps that
//!   cannot measure (e.g., Meili partial-update) report 0.
//! - `rows_processed` — invocation-level total (= demoted +
//!   pruned + reaped). The dashboard uses this for the per-sweep
//!   bar chart.
//! - `tier_transitions` — the per-pair counters the tier sweep
//!   already populates; other sweeps populate the empty map.
//! - `error_message` — `Some(msg)` when `status =
//!   SweepStatus::Failed`; truncated to 256 chars to fit the
//!   eventual `retention_sweeps.error_message` column (Phase B
//!   schema migration).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Maximum bytes preserved in `SweepReport::error_message`. Matches
/// the cap the cron scheduler already enforces on its
/// `last_error` field (`scheduler.rs::STREAM_CAP_BYTES` is for
/// stdout / stderr; this is the user-facing single-line error).
pub const ERROR_MESSAGE_CAP_BYTES: usize = 256;

/// Canonical sweep status. Values match the strings the existing
/// `retention_sweeps.status` column already stores so the
/// scheduler-side persistence layer does not change in Phase13a.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SweepStatus {
    /// Row inserted; the sweep is mid-flight.
    Running,
    /// Sweep returned `Ok(_)` and finalised within the error-rate
    /// ceiling.
    Success,
    /// Sweep returned `Err(_)` or exceeded its error-rate ceiling.
    Failed,
    /// Previous `running` row exceeded the abandon-grace window;
    /// scheduler stamped this as terminal so a new invocation may
    /// proceed.
    Abandoned,
}

impl SweepStatus {
    /// Stable string identifier — matches the `retention_sweeps.status`
    /// column values today.
    pub fn as_str(self) -> &'static str {
        match self {
            SweepStatus::Running => "running",
            SweepStatus::Success => "success",
            SweepStatus::Failed => "failed",
            SweepStatus::Abandoned => "abandoned",
        }
    }
    /// `true` when the status is terminal (sweep no longer holds
    /// the advisory lock). Phase B schedulers use this to decide
    /// whether to recompute `cron_jobs.next_run_at`.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SweepStatus::Success | SweepStatus::Failed | SweepStatus::Abandoned
        )
    }
}

/// Uniform per-invocation outcome — what every sweep returns and
/// what the scheduler persists to `retention_sweeps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepReport {
    /// Stable `'static` identifier of the sweep that produced this
    /// report.
    pub name: String,
    /// When the sweep started.
    pub started_at: DateTime<Utc>,
    /// When the sweep finished (or transitioned to `Abandoned`).
    /// `None` while the row is still `Running`.
    pub finished_at: Option<DateTime<Utc>>,
    /// Terminal status — see [`SweepStatus`].
    pub status: SweepStatus,
    /// Best-effort byte count reclaimed. Sweeps that cannot
    /// measure report 0.
    pub bytes_reclaimed: u64,
    /// Invocation-level row total (demoted + pruned + reaped).
    pub rows_processed: u64,
    /// Per-pair tier-transition counters (`<kind>:<from>-><to>`).
    /// Populated by the tier sweep; other sweeps leave empty.
    pub tier_transitions: BTreeMap<String, u64>,
    /// Truncated to [`ERROR_MESSAGE_CAP_BYTES`]. `Some(_)` only when
    /// `status == SweepStatus::Failed`.
    pub error_message: Option<String>,
}

impl SweepReport {
    /// Build the initial `Running` row. The scheduler stamps the
    /// `started_at` clock and inserts the SQLite row before the
    /// sweep itself proceeds.
    pub fn started(name: &str, started_at: DateTime<Utc>) -> Self {
        Self {
            name: name.to_string(),
            started_at,
            finished_at: None,
            status: SweepStatus::Running,
            bytes_reclaimed: 0,
            rows_processed: 0,
            tier_transitions: BTreeMap::new(),
            error_message: None,
        }
    }

    /// Mark the report `Success`, set the finish clock and the
    /// counters. Returns `self` so callers can chain at the end of
    /// `run`.
    #[must_use]
    pub fn finish_success(
        mut self,
        finished_at: DateTime<Utc>,
        rows_processed: u64,
        bytes_reclaimed: u64,
    ) -> Self {
        self.finished_at = Some(finished_at);
        self.status = SweepStatus::Success;
        self.rows_processed = rows_processed;
        self.bytes_reclaimed = bytes_reclaimed;
        self.error_message = None;
        self
    }

    /// Mark the report `Failed`, stamp the truncated error message.
    #[must_use]
    pub fn finish_failed(
        mut self,
        finished_at: DateTime<Utc>,
        rows_processed: u64,
        bytes_reclaimed: u64,
        error_message: impl Into<String>,
    ) -> Self {
        self.finished_at = Some(finished_at);
        self.status = SweepStatus::Failed;
        self.rows_processed = rows_processed;
        self.bytes_reclaimed = bytes_reclaimed;
        self.error_message = Some(truncate_error(error_message.into()));
        self
    }

    /// Builder shim — bump a per-pair tier-transition counter by
    /// `delta`. Used by the tier sweep during migration (§3.1).
    #[must_use]
    pub fn with_tier_transition(mut self, key: &str, delta: u64) -> Self {
        *self.tier_transitions.entry(key.to_string()).or_insert(0) += delta;
        self
    }

    /// Project the report into the dashboard view. Pure (same input
    /// ⇒ same output). The dashboard handler MUST call this rather
    /// than recompute state on the handler side (ADR-014 / §4.3).
    pub fn view(&self) -> SweepReportView {
        SweepReportView {
            name: self.name.clone(),
            status: self.status,
            started_at: self.started_at,
            finished_at: self.finished_at,
            duration_secs: self
                .finished_at
                .map(|f| (f - self.started_at).num_seconds().max(0) as u64),
            rows_processed: self.rows_processed,
            bytes_reclaimed: self.bytes_reclaimed,
            tier_transitions: self.tier_transitions.clone(),
            error_message: self.error_message.clone(),
        }
    }
}

/// Dashboard projection. The handler renders these without doing
/// any additional state inference — see ADR-014 / task §4.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepReportView {
    /// Stable identifier of the sweep.
    pub name: String,
    /// Terminal status (or `Running` while in flight).
    pub status: SweepStatus,
    /// Started clock.
    pub started_at: DateTime<Utc>,
    /// Finished clock (`None` while running).
    pub finished_at: Option<DateTime<Utc>>,
    /// `finished_at - started_at` in seconds, `None` while running.
    pub duration_secs: Option<u64>,
    /// Invocation-level row total.
    pub rows_processed: u64,
    /// Bytes reclaimed (best-effort).
    pub bytes_reclaimed: u64,
    /// Per-pair tier-transition counters.
    pub tier_transitions: BTreeMap<String, u64>,
    /// Truncated error message — `Some(_)` only on `Failed`.
    pub error_message: Option<String>,
}

/// Truncate `s` so the UTF-8 byte length fits
/// [`ERROR_MESSAGE_CAP_BYTES`]. Splits on a char boundary so the
/// resulting `String` stays valid UTF-8.
fn truncate_error(mut s: String) -> String {
    if s.len() <= ERROR_MESSAGE_CAP_BYTES {
        return s;
    }
    // Find the largest char boundary ≤ ERROR_MESSAGE_CAP_BYTES.
    let mut cap = ERROR_MESSAGE_CAP_BYTES;
    while cap > 0 && !s.is_char_boundary(cap) {
        cap -= 1;
    }
    s.truncate(cap);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    #[test]
    fn status_as_str_round_trips() {
        for s in [
            SweepStatus::Running,
            SweepStatus::Success,
            SweepStatus::Failed,
            SweepStatus::Abandoned,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            assert_eq!(j.trim_matches('"'), s.as_str());
            let p: SweepStatus = serde_json::from_str(&j).unwrap();
            assert_eq!(p, s);
        }
    }

    #[test]
    fn status_is_terminal_classifies_running_as_non_terminal() {
        assert!(!SweepStatus::Running.is_terminal());
        assert!(SweepStatus::Success.is_terminal());
        assert!(SweepStatus::Failed.is_terminal());
        assert!(SweepStatus::Abandoned.is_terminal());
    }

    #[test]
    fn report_started_carries_running_status_and_no_finish() {
        let r = SweepReport::started("tier_sweep", ts("2026-05-19T12:00:00Z"));
        assert_eq!(r.name, "tier_sweep");
        assert_eq!(r.status, SweepStatus::Running);
        assert!(r.finished_at.is_none());
        assert_eq!(r.rows_processed, 0);
        assert!(r.error_message.is_none());
    }

    #[test]
    fn finish_success_transitions_status_and_stamps_counters() {
        let start = ts("2026-05-19T12:00:00Z");
        let end = ts("2026-05-19T12:00:30Z");
        let r = SweepReport::started("tier_sweep", start).finish_success(end, 42, 1024);
        assert_eq!(r.status, SweepStatus::Success);
        assert_eq!(r.finished_at, Some(end));
        assert_eq!(r.rows_processed, 42);
        assert_eq!(r.bytes_reclaimed, 1024);
        assert!(r.error_message.is_none());
    }

    #[test]
    fn finish_failed_truncates_long_error_message_at_char_boundary() {
        let start = ts("2026-05-19T12:00:00Z");
        let end = ts("2026-05-19T12:00:30Z");
        // 600-byte payload — 4-byte chars to make the boundary test
        // meaningful. `🌑` is 4 bytes.
        let payload: String = "🌑".repeat(150);
        assert!(payload.len() > ERROR_MESSAGE_CAP_BYTES);
        let r = SweepReport::started("pii_enforce", start).finish_failed(end, 0, 0, payload);
        assert_eq!(r.status, SweepStatus::Failed);
        let msg = r.error_message.unwrap();
        assert!(msg.len() <= ERROR_MESSAGE_CAP_BYTES);
        // Must still be valid UTF-8 — pull-out parse is the proof.
        let _: &str = msg.as_str();
        // Cap landed on a char boundary, so length is a multiple of 4.
        assert_eq!(msg.len() % 4, 0);
    }

    #[test]
    fn with_tier_transition_accumulates_keys() {
        let r = SweepReport::started("tier_sweep", ts("2026-05-19T12:00:00Z"))
            .with_tier_transition("turn:fp32->pq", 4)
            .with_tier_transition("turn:fp32->pq", 2)
            .with_tier_transition("tool_call:pq->binary", 1);
        assert_eq!(*r.tier_transitions.get("turn:fp32->pq").unwrap(), 6);
        assert_eq!(*r.tier_transitions.get("tool_call:pq->binary").unwrap(), 1);
    }

    #[test]
    fn view_is_a_pure_projection_with_duration_in_seconds() {
        let start = ts("2026-05-19T12:00:00Z");
        let end = ts("2026-05-19T12:01:30Z");
        let r = SweepReport::started("tier_sweep", start)
            .with_tier_transition("turn:fp32->pq", 4)
            .finish_success(end, 4, 1024);
        let v = r.view();
        assert_eq!(v.name, "tier_sweep");
        assert_eq!(v.status, SweepStatus::Success);
        assert_eq!(v.duration_secs, Some(90));
        assert_eq!(v.rows_processed, 4);
        assert_eq!(*v.tier_transitions.get("turn:fp32->pq").unwrap(), 4);
        // Pure — same report ⇒ same view.
        assert_eq!(r.view(), v);
    }

    #[test]
    fn report_round_trips_via_serde_with_failed_status() {
        let start = ts("2026-05-19T12:00:00Z");
        let end = ts("2026-05-19T12:00:05Z");
        let r = SweepReport::started("meili_prune", start).finish_failed(end, 7, 128, "boom");
        let j = serde_json::to_string(&r).unwrap();
        let p: SweepReport = serde_json::from_str(&j).unwrap();
        assert_eq!(p, r);
    }
}
