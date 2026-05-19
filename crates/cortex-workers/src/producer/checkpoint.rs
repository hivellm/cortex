//! [`ProducerCheckpoint`] — the per-(producer, scope) cursor that
//! survives `kill -9` and drives resume on next boot.
//!
//! Layout per ADR-010 §2.3:
//!
//! - `producer_name` — matches [`EnvelopeProducer::name`].
//! - `scope` — per-(producer, sub-source) string. Bootstrap uses
//!   the repo slug; consolidator uses the grain
//!   (`session`/`topic`/`decision`); claude-archive uses the
//!   project path; topic-cards uses the topic slug.
//! - `last_event_id` — the `event_id` of the last envelope the
//!   prior batch emitted.
//! - `last_occurred_at` — the `occurred_at` of the last envelope.
//! - `accumulated_at` — when the row landed in SQLite.
//!
//! The SQLite-side row shape lives in
//! `cortex_storage::ProducerCheckpointRow`. This module mirrors it
//! at the worker layer so callers do not need a `cortex_storage`
//! import to handle a checkpoint.
//!
//! [`ProducerEnvelopeProducer::name`]: super::EnvelopeProducer::name

use chrono::{DateTime, Utc};
use cortex_storage::ProducerCheckpointRow;
use serde::{Deserialize, Serialize};

/// Per-(producer, scope) cursor row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerCheckpoint {
    /// Producer name (`bootstrap`, `claude_archive`, ...).
    pub producer_name: String,
    /// Per-(producer, sub-source) scope.
    pub scope: String,
    /// `event_id` of the last envelope the prior batch emitted.
    pub last_event_id: String,
    /// `occurred_at` of the last envelope.
    pub last_occurred_at: DateTime<Utc>,
    /// When the row landed in SQLite.
    pub accumulated_at: DateTime<Utc>,
}

impl ProducerCheckpoint {
    /// Translate the SQLite row into the worker-facing shape.
    /// Parses the RFC-3339 timestamps; falls back to `Utc::now()`
    /// on parse failure (defensive — the writer always stamps
    /// `to_rfc3339()`).
    pub fn from_row(row: ProducerCheckpointRow) -> Self {
        let last_occurred_at = DateTime::parse_from_rfc3339(&row.last_occurred_at)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let accumulated_at = DateTime::parse_from_rfc3339(&row.accumulated_at)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        Self {
            producer_name: row.producer_name,
            scope: row.scope,
            last_event_id: row.last_event_id,
            last_occurred_at,
            accumulated_at,
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    #[test]
    fn checkpoint_round_trips_via_serde() {
        let cp = ProducerCheckpoint {
            producer_name: "bootstrap".into(),
            scope: "cortex".into(),
            last_event_id: "01ENV".into(),
            last_occurred_at: ts("2026-05-19T12:00:00Z"),
            accumulated_at: ts("2026-05-19T12:00:01Z"),
        };
        let j = serde_json::to_string(&cp).unwrap();
        let p: ProducerCheckpoint = serde_json::from_str(&j).unwrap();
        assert_eq!(p, cp);
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
    fn from_row_parses_rfc3339_timestamps() {
        let row = ProducerCheckpointRow {
            producer_name: "bootstrap".into(),
            scope: "cortex".into(),
            last_event_id: "01ENV".into(),
            last_occurred_at: "2026-05-19T12:00:00+00:00".into(),
            accumulated_at: "2026-05-19T12:00:01+00:00".into(),
        };
        let cp = ProducerCheckpoint::from_row(row);
        assert_eq!(cp.producer_name, "bootstrap");
        assert_eq!(cp.last_event_id, "01ENV");
    }
}
