//! [`ProducerReport`] — per-run summary every [`EnvelopeProducer`]
//! emits, plus [`ProducerReportView`] — the dashboard projection
//! (ADR-014 / phase13f §2.4). The dashboard handler MUST call
//! [`ProducerReport::view`] rather than render `ProducerReport` fields
//! directly; this keeps the wire format stable across schema bumps
//! and prevents handler-side derivations (the failure mode ADR-014
//! locks against).
//!
//! Layout:
//!
//! - [`ProducerReport`] — domain shape, the producer fills in.
//!   `last_event_id` is a free-form string (empty when no work);
//!   `last_occurred_at` is `Some(_)` only when the producer emitted
//!   at least one envelope.
//! - [`ProducerReportView`] — handler shape. `last_event_id` becomes
//!   `Option<String>` (`None` instead of the empty-string sentinel)
//!   and a derived `had_work` boolean spells out the producer's
//!   in-flight vs idle state so the handler never re-derives.
//!
//! [`EnvelopeProducer`]: super::EnvelopeProducer

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::producer::ProducerCheckpoint;

/// Per-run producer summary the supervisor logs / persists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerReport {
    /// Producer name.
    pub producer_name: String,
    /// Total envelopes emitted across every batch in this run.
    pub envelopes_emitted: u64,
    /// Number of batches the producer wrote a checkpoint for.
    pub batches_emitted: u64,
    /// `event_id` of the last envelope emitted; empty when the
    /// producer had no work.
    pub last_event_id: String,
    /// `occurred_at` of the last envelope; `None` when the
    /// producer had no work.
    pub last_occurred_at: Option<DateTime<Utc>>,
}

impl ProducerReport {
    /// Project the report into the dashboard view. Pure (same input
    /// ⇒ same output). The dashboard handler MUST call this rather
    /// than recompute state on the handler side (ADR-014 / phase13f
    /// §3.4).
    pub fn view(&self) -> ProducerReportView {
        let had_work = self.envelopes_emitted > 0;
        let last_event_id = if self.last_event_id.is_empty() {
            None
        } else {
            Some(self.last_event_id.clone())
        };
        ProducerReportView {
            producer_name: self.producer_name.clone(),
            envelopes_emitted: self.envelopes_emitted,
            batches_emitted: self.batches_emitted,
            last_event_id,
            last_occurred_at: self.last_occurred_at,
            had_work,
        }
    }
}

/// Dashboard projection of [`ProducerReport`] (ADR-014 / phase13f
/// §2.4). The handler renders these without doing any additional
/// state inference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerReportView {
    /// Producer name.
    pub producer_name: String,
    /// Total envelopes emitted across every batch in this run.
    pub envelopes_emitted: u64,
    /// Number of batches the producer wrote a checkpoint for.
    pub batches_emitted: u64,
    /// `event_id` of the last envelope emitted. `None` when the
    /// producer had no work (rather than the empty-string sentinel
    /// the domain type carries) so handlers never need to check both
    /// `.is_empty()` and `.is_none()`.
    pub last_event_id: Option<String>,
    /// `occurred_at` of the last envelope; `None` when the
    /// producer had no work.
    pub last_occurred_at: Option<DateTime<Utc>>,
    /// `true` when the producer emitted at least one envelope in
    /// this run. Derived from `envelopes_emitted > 0`; carried on
    /// the view so handlers do not re-derive.
    pub had_work: bool,
}

/// Dashboard projection of one persisted [`ProducerCheckpoint`].
/// ADR-014 / phase13f §3.4. The dashboard's
/// `/v1/dashboard/producers` endpoint returns one of these per
/// `(producer_name, scope)` pair — checkpoints are the only durable
/// producer state today; per-run [`ProducerReport`] rows are
/// in-memory and disappear at process exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerCheckpointView {
    /// Producer name (`bootstrap`, `claude_archive`,
    /// `topic_cards`, `consolidator`, …).
    pub producer_name: String,
    /// Per-(producer, sub-source) scope.
    pub scope: String,
    /// `event_id` of the last envelope the prior batch emitted.
    /// `None` when the checkpoint row carries an empty marker
    /// (defensive; the writer always stamps a non-empty id today).
    pub last_event_id: Option<String>,
    /// `occurred_at` of the last envelope.
    pub last_occurred_at: DateTime<Utc>,
    /// When the row landed in SQLite.
    pub accumulated_at: DateTime<Utc>,
    /// `true` when `last_event_id` is non-empty; derived so the
    /// handler does not re-check.
    pub has_progress: bool,
}

impl ProducerCheckpoint {
    /// Project the checkpoint into the dashboard view. Pure (same
    /// input ⇒ same output). The dashboard handler MUST call this
    /// rather than render fields directly (ADR-014 / phase13f
    /// §3.4).
    pub fn view(&self) -> ProducerCheckpointView {
        let has_progress = !self.last_event_id.is_empty();
        let last_event_id = if has_progress {
            Some(self.last_event_id.clone())
        } else {
            None
        };
        ProducerCheckpointView {
            producer_name: self.producer_name.clone(),
            scope: self.scope.clone(),
            last_event_id,
            last_occurred_at: self.last_occurred_at,
            accumulated_at: self.accumulated_at,
            has_progress,
        }
    }
}

/// Aggregate view returned by `/v1/dashboard/producers`. One row
/// per `(producer_name, scope)` pair carrying the most recent
/// checkpoint, sorted by `producer_name` then `scope`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerCheckpointsReportView {
    /// One [`ProducerCheckpointView`] per `(producer_name, scope)`
    /// pair, sorted by `producer_name` then `scope`.
    pub rows: Vec<ProducerCheckpointView>,
    /// Equal to `rows.len()`; carried on the view so consumers
    /// cannot drift.
    pub total: u64,
}

impl ProducerCheckpointsReportView {
    /// Build the aggregate from a row collection. The caller owns
    /// the sort order — this constructor does not re-sort.
    pub fn from_rows(rows: Vec<ProducerCheckpointView>) -> Self {
        let total = rows.len() as u64;
        Self { rows, total }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    #[test]
    fn report_round_trips_via_serde() {
        let r = ProducerReport {
            producer_name: "bootstrap".into(),
            envelopes_emitted: 42,
            batches_emitted: 3,
            last_event_id: "01LAST".into(),
            last_occurred_at: Some(ts("2026-05-19T12:00:00Z")),
        };
        let j = serde_json::to_string(&r).unwrap();
        let p: ProducerReport = serde_json::from_str(&j).unwrap();
        assert_eq!(p, r);
    }

    #[test]
    fn view_is_a_pure_projection_with_had_work_flag() {
        let r = ProducerReport {
            producer_name: "bootstrap".into(),
            envelopes_emitted: 42,
            batches_emitted: 3,
            last_event_id: "01LAST".into(),
            last_occurred_at: Some(ts("2026-05-19T12:00:00Z")),
        };
        let v = r.view();
        assert_eq!(v.producer_name, "bootstrap");
        assert_eq!(v.envelopes_emitted, 42);
        assert_eq!(v.last_event_id, Some("01LAST".to_string()));
        assert!(v.had_work);
        // Pure — same report ⇒ same view.
        assert_eq!(r.view(), v);
    }

    #[test]
    fn view_maps_empty_event_id_to_none_and_clears_had_work() {
        let r = ProducerReport {
            producer_name: "claude_archive".into(),
            envelopes_emitted: 0,
            batches_emitted: 0,
            last_event_id: String::new(),
            last_occurred_at: None,
        };
        let v = r.view();
        assert_eq!(v.last_event_id, None);
        assert!(!v.had_work);
    }

    #[test]
    fn view_round_trips_via_serde() {
        let r = ProducerReport {
            producer_name: "topic_cards".into(),
            envelopes_emitted: 7,
            batches_emitted: 1,
            last_event_id: "01TOP".into(),
            last_occurred_at: Some(ts("2026-05-19T12:00:00Z")),
        };
        let v = r.view();
        let j = serde_json::to_string(&v).unwrap();
        let p: ProducerReportView = serde_json::from_str(&j).unwrap();
        assert_eq!(p, v);
    }

    fn sample_checkpoint(producer: &str, scope: &str, last: &str) -> ProducerCheckpoint {
        ProducerCheckpoint {
            producer_name: producer.into(),
            scope: scope.into(),
            last_event_id: last.into(),
            last_occurred_at: ts("2026-05-19T12:00:00Z"),
            accumulated_at: ts("2026-05-19T12:00:01Z"),
        }
    }

    #[test]
    fn checkpoint_view_is_a_pure_projection_with_has_progress_flag() {
        let cp = sample_checkpoint("bootstrap", "cortex", "01LAST");
        let v = cp.view();
        assert_eq!(v.producer_name, "bootstrap");
        assert_eq!(v.scope, "cortex");
        assert_eq!(v.last_event_id, Some("01LAST".to_string()));
        assert!(v.has_progress);
        assert_eq!(cp.view(), v);
    }

    #[test]
    fn checkpoint_view_maps_empty_event_id_to_none_and_clears_has_progress() {
        let cp = sample_checkpoint("claude_archive", "project-foo", "");
        let v = cp.view();
        assert_eq!(v.last_event_id, None);
        assert!(!v.has_progress);
    }

    #[test]
    fn checkpoint_view_round_trips_via_serde() {
        let v = sample_checkpoint("topic_cards", "topic-slug", "01TOP").view();
        let j = serde_json::to_string(&v).unwrap();
        let p: ProducerCheckpointView = serde_json::from_str(&j).unwrap();
        assert_eq!(p, v);
    }

    #[test]
    fn checkpoints_report_view_carries_total_from_row_count() {
        let rows: Vec<ProducerCheckpointView> = ["bootstrap", "claude_archive", "topic_cards"]
            .iter()
            .map(|n| sample_checkpoint(n, "scope", "01X").view())
            .collect();
        let view = ProducerCheckpointsReportView::from_rows(rows.clone());
        assert_eq!(view.total, 3);
        assert_eq!(view.rows, rows);
    }

    #[test]
    fn checkpoints_report_view_round_trips_via_serde() {
        let v = ProducerCheckpointsReportView::from_rows(vec![
            sample_checkpoint("bootstrap", "cortex", "01A").view(),
            sample_checkpoint("claude_archive", "proj", "01B").view(),
        ]);
        let j = serde_json::to_string(&v).unwrap();
        let p: ProducerCheckpointsReportView = serde_json::from_str(&j).unwrap();
        assert_eq!(p, v);
    }
}
