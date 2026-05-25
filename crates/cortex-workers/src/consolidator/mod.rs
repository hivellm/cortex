//! `cortex-consolidator` — phase11j.
//!
//! Distils raw Cortex envelopes into evergreen
//! [`cortex_core::events::ConsolidationPayload`] summaries. Three
//! producer modes match the three [`cortex_core::events::ConsolidationGrain`]
//! variants:
//!
//! - [`producer::session`] — fires on Stop hook + nightly back-fill.
//!   Input: every envelope sharing a `session_id`. Output: one
//!   `Kind::Consolidation` envelope with `grain = Session`.
//! - [`producer::topic`] — nightly cron. HDBSCAN over turn vectors
//!   per repo, `min_cluster_size = 3`. Output: one consolidation
//!   per cluster.
//! - [`producer::decision_trace`] — fires when a `Kind::Decision`
//!   envelope lands. Walks the `parent_event_id` chain up to N hops.
//!   Output: `grain = DecisionTrace`.
//!
//! Every producer goes through a [`summariser::Summariser`] trait
//! impl (Haiku for shallow / Opus for deep). The
//! [`orchestrator::Orchestrator`] picks the producer + auto-promotes
//! to Opus on `grain = DecisionTrace` or `outcome = success +
//! high-impact`.
//!
//! Module layout (full implementations land in §2.2..§2.10):
//!
//! - [`templates`] — Markdown prompt fixtures, one per grain.
//! - [`summariser`] — abstract trait + Haiku45 / Opus47 impls.
//! - [`producer`] — `session.rs`, `topic.rs`, `decision_trace.rs`.
//! - [`orchestrator`] — chooses producer + summariser per trigger.
//! - [`cost_telemetry`] — per-grain $/consolidation gauge that
//!   surfaces under `/v1/health/coverage` (phase11i §5.2 wiring).
//!
//! This skeleton (§2.1) ships the trait surface + module wiring so
//! the §3 routing + §4 query lane can land before the producer
//! bodies (§2.4..§2.7) are real. Re-exports under
//! [`prelude`] give downstream callers a stable import path.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[path = "consolidator_trait.rs"]
pub mod consolidator_trait;
pub mod cost_telemetry;
pub mod metrics;
pub mod orchestrator;
pub mod producer;
pub mod producer_trait;
pub mod source;
pub mod summariser;
pub mod summariser_cli;
pub mod templates;

pub use consolidator_trait::{
    ConsolidationReport, Consolidator, ConsolidatorCtx, ConsolidatorError, TriggerLabel,
};

pub use source::{LiveDecisionTraceSource, LiveSessionSource, LiveTopicSource, SourceError};

/// One-stop import set for callers wiring the consolidator into
/// the daemon or test harnesses.
pub mod prelude {
    pub use super::cost_telemetry::{CostBudget, CostLedger, GrainCost};
    pub use super::orchestrator::{Orchestrator, ProducerSelection, Trigger};
    pub use super::producer::ProducerError;
    pub use super::summariser::{Summariser, SummariserError, SummariserKind, SummariserResult};
}

#[cfg(test)]
mod skeleton_tests {
    //! Phase11j §2.1 — skeleton sanity check. The full producer
    //! / summariser tests land alongside §2.4..§2.10.

    use super::prelude::*;

    #[test]
    fn cost_budget_default_is_a_thousand_dollars_per_month() {
        let budget = CostBudget::default();
        assert_eq!(budget.monthly_cents_cap, 100_000);
    }

    #[test]
    fn trigger_session_end_routes_through_producer_selection() {
        let sel = ProducerSelection::for_trigger(&Trigger::SessionEnd {
            session_id: "01HXSESS01".into(),
        });
        assert_eq!(sel.grain_label(), "session");
    }
}
