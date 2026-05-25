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
}
