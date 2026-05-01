//! Phase11j §2.4 — Session producer.
//!
//! Input: every envelope sharing a `session_id` (Turn / ToolCall /
//! AgentCall in occurred_at order). Output: one
//! `Kind::Consolidation` payload with `grain = Session`. The §2.1
//! skeleton ships the public surface + input shape so the §2.4
//! producer body has a stable contract to fill in.

use cortex_core::events::Envelope;

/// Input the session producer reads. The orchestrator hydrates this
/// from the archive_loader + Synap stream replay.
#[derive(Debug, Clone)]
pub struct SessionInput {
    /// Originating session id. Drives `scope = SessionId(_)`.
    pub session_id: String,
    /// Repo slug the session ran against (for `payload.repos`).
    pub repo: Option<String>,
    /// Envelopes ordered by `occurred_at` — Turn / ToolCall /
    /// AgentCall variants only.
    pub envelopes: Vec<Envelope>,
}

impl SessionInput {
    /// Quick sanity check the orchestrator runs before invoking the
    /// summariser. Producers reject empty inputs cleanly so the
    /// nightly back-fill never emits an empty payload.
    pub fn ensure_non_empty(&self) -> Result<(), super::ProducerError> {
        if self.envelopes.is_empty() {
            return Err(super::ProducerError::EmptyInput(format!(
                "session {} has zero envelopes",
                self.session_id
            )));
        }
        Ok(())
    }

    /// Earliest / latest `occurred_at` across the envelope set, in
    /// epoch ms. Drives `temporal_span` on the produced payload.
    pub fn temporal_bounds_ms(&self) -> Option<(i64, i64)> {
        let mut iter = self.envelopes.iter().filter_map(|e| {
            chrono::DateTime::parse_from_rfc3339(&e.occurred_at)
                .ok()
                .map(|d| d.timestamp_millis())
        });
        let first = iter.next()?;
        Some(iter.fold((first, first), |(lo, hi), ts| (lo.min(ts), hi.max(ts))))
    }
}
