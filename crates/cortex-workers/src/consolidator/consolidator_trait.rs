//! phase14a §1 — `Consolidator` trait + per-trigger report shape.
//!
//! The consolidator daemon (phase14a §3) dispatches a [`Trigger`]
//! to a `Consolidator` implementation chosen by [`Consolidator::grain`].
//! Each grain runs its own clustering + summarisation pipeline and
//! returns a [`ConsolidationReport`] the daemon persists alongside
//! the producer-checkpoint shipped by phase13b.
//!
//! ADR-014 pure-reader contract surfaces in two places:
//!
//! - The dashboard reads [`ConsolidationReport`] rows verbatim
//!   (phase13f §3.2 already wraps them in `ConsolidationReportView`).
//! - The health endpoint (phase14a §4.1) projects the latest report
//!   per grain into the typed `/v1/health/consolidator` payload —
//!   no handler-side derivations.
//!
//! [`Trigger`]: super::orchestrator::Trigger

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cortex_core::events::ConsolidationGrain;
use serde::{Deserialize, Serialize};

use super::orchestrator::Trigger;
use super::summariser::SummariserKind;
use crate::producer::EnvelopeProducer;

/// Per-run report returned by every grain. Mirrors the
/// [`crate::producer::ProducerReport`] shape (envelope counters +
/// last-event ids) plus consolidation-specific fields the daemon /
/// health endpoint surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationReport {
    /// Grain that produced the report.
    pub grain: ConsolidationGrain,
    /// Trigger that kicked the run (echoed for audit).
    pub trigger: TriggerLabel,
    /// Number of consolidation envelopes the run emitted.
    pub envelopes_emitted: u64,
    /// Summariser cost in cents (integer cents — fractional kept by
    /// the cost ledger; this field is the daemon-facing rollup).
    pub cost_cents: u64,
    /// Wall-clock latency of the run in milliseconds.
    pub latency_ms: u64,
    /// Total source-envelope count folded into the consolidations
    /// this run emitted. Sums across every envelope in the batch.
    pub source_event_count: u64,
    /// When the run finished. Stamped by the daemon at report time.
    pub finished_at: DateTime<Utc>,
    /// Summariser used (`haiku-4-5` / `opus-4-7`). Informational —
    /// the cost ledger carries the authoritative attribution.
    pub summariser: SummariserKind,
}

/// Serialisable shape of [`Trigger`] for [`ConsolidationReport`].
/// The runtime `Trigger` enum carries `force_deep: bool` on
/// `DecisionLanded` which is informational only; the report shape
/// flattens it into a label so the dashboard / health endpoint
/// renders a stable string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TriggerLabel {
    /// Stop hook on a session.
    SessionEnd {
        /// Session id that triggered the run.
        session_id: String,
    },
    /// Nightly cron over a repo's sessions.
    NightlyTopic {
        /// Repo slug the run scoped to.
        repo: String,
    },
    /// New decision envelope landed.
    DecisionLanded {
        /// Decision id that triggered the run.
        decision_id: String,
    },
}

impl From<&Trigger> for TriggerLabel {
    fn from(t: &Trigger) -> Self {
        match t {
            Trigger::SessionEnd { session_id } => Self::SessionEnd {
                session_id: session_id.clone(),
            },
            Trigger::NightlyTopic { repo } => Self::NightlyTopic { repo: repo.clone() },
            Trigger::DecisionLanded { decision_id, .. } => Self::DecisionLanded {
                decision_id: decision_id.clone(),
            },
        }
    }
}

impl TriggerLabel {
    /// Stable snake-case kind string for telemetry + log filters.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SessionEnd { .. } => "session_end",
            Self::NightlyTopic { .. } => "nightly_topic",
            Self::DecisionLanded { .. } => "decision_landed",
        }
    }
}

/// Contract every consolidator grain implements. Composes with
/// [`EnvelopeProducer`] (phase13b) — the daemon resolves the
/// producer's name + checkpoint table via the producer trait, then
/// dispatches each trigger through [`Consolidator::on_trigger`].
#[async_trait]
pub trait Consolidator: EnvelopeProducer {
    /// Grain this consolidator emits. The daemon uses this to map
    /// incoming triggers onto the correct implementation.
    fn grain(&self) -> ConsolidationGrain;

    /// Run the consolidation pipeline for `trigger`. The daemon
    /// passes a [`ConsolidatorCtx`] carrying the producer ctx, the
    /// cost ledger handle, and a clock so tests can pin the
    /// finished-at stamp without touching the system clock.
    async fn on_trigger(
        &self,
        trigger: &Trigger,
        ctx: &ConsolidatorCtx,
    ) -> Result<ConsolidationReport, ConsolidatorError>;
}

/// Daemon-facing error shape every grain returns. Mapped onto
/// metrics + log lines by the daemon.
#[derive(Debug, thiserror::Error)]
pub enum ConsolidatorError {
    /// Trigger does not match the grain's expected variant.
    #[error("trigger {got} does not match grain {expected}")]
    TriggerMismatch {
        /// Trigger kind the caller passed.
        got: &'static str,
        /// Trigger kind the grain expects (e.g. `session_end` for
        /// the session grain).
        expected: &'static str,
    },
    /// Summariser-side failure — surfaced verbatim so the daemon
    /// can log it without losing the upstream context.
    #[error("summariser: {0}")]
    Summariser(String),
    /// Bus / producer-side failure.
    #[error("producer: {0}")]
    Producer(String),
    /// Any other consolidator-internal error.
    #[error("consolidator: {0}")]
    Other(String),
}

/// Per-run context the daemon hands to each grain.
#[derive(Debug, Clone)]
pub struct ConsolidatorCtx {
    /// Reference clock — the grain stamps its `finished_at` field
    /// from this. Tests pin this to a fixed value.
    pub now: DateTime<Utc>,
}

impl ConsolidatorCtx {
    /// Build a context that uses the supplied reference clock.
    pub fn at(now: DateTime<Utc>) -> Self {
        Self { now }
    }

    /// Build a context anchored on the current wall clock.
    pub fn now() -> Self {
        Self { now: Utc::now() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    #[test]
    fn trigger_label_from_session_end_carries_id() {
        let t = Trigger::SessionEnd {
            session_id: "01HSESS001".into(),
        };
        let label = TriggerLabel::from(&t);
        assert_eq!(
            label,
            TriggerLabel::SessionEnd {
                session_id: "01HSESS001".into()
            }
        );
        assert_eq!(label.kind(), "session_end");
    }

    #[test]
    fn trigger_label_from_decision_landed_drops_force_deep_flag() {
        // Two triggers with different `force_deep` produce the same
        // serialisable label — the daemon report is summariser-
        // agnostic about that knob.
        let t_a = Trigger::DecisionLanded {
            decision_id: "01HDEC".into(),
            force_deep: true,
        };
        let t_b = Trigger::DecisionLanded {
            decision_id: "01HDEC".into(),
            force_deep: false,
        };
        assert_eq!(TriggerLabel::from(&t_a), TriggerLabel::from(&t_b));
    }

    #[test]
    fn trigger_label_serialises_with_snake_case_kind_tag() {
        let label = TriggerLabel::NightlyTopic {
            repo: "cortex".into(),
        };
        let j = serde_json::to_value(&label).unwrap();
        assert_eq!(j["kind"], "nightly_topic");
        assert_eq!(j["repo"], "cortex");
    }

    #[test]
    fn consolidation_report_round_trips_via_serde() {
        let report = ConsolidationReport {
            grain: ConsolidationGrain::Session,
            trigger: TriggerLabel::SessionEnd {
                session_id: "01HSESS001".into(),
            },
            envelopes_emitted: 4,
            cost_cents: 12,
            latency_ms: 1_500,
            source_event_count: 27,
            finished_at: ts("2026-05-25T12:00:00Z"),
            summariser: SummariserKind::Haiku45,
        };
        let j = serde_json::to_string(&report).unwrap();
        let parsed: ConsolidationReport = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed, report);
    }

    #[test]
    fn consolidator_ctx_at_pins_clock_for_tests() {
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        assert_eq!(ctx.now, ts("2026-05-25T12:00:00Z"));
    }

    #[test]
    fn consolidator_error_renders_trigger_mismatch_message() {
        let err = ConsolidatorError::TriggerMismatch {
            got: "decision_landed",
            expected: "session_end",
        };
        let msg = format!("{err}");
        assert!(msg.contains("decision_landed"));
        assert!(msg.contains("session_end"));
    }
}
