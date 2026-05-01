//! Phase11j §2.7 — orchestrator.
//!
//! Routes a [`Trigger`] to the matching producer + summariser kind.
//! Implements the auto-promotion rule: `grain = DecisionTrace` OR
//! `outcome = success + high-impact` runs against Opus instead of
//! Haiku. The full live wiring (summariser dispatch, payload
//! validation, envelope emission) drops in alongside §2.7; the
//! §2.1 skeleton ships the routing surface so the producer modules
//! compile against a real type.

use cortex_core::events::ConsolidationGrain;

use crate::summariser::SummariserKind;

/// What kicked off a consolidation run. The orchestrator reads the
/// variant to pick the producer + summariser.
#[derive(Debug, Clone)]
pub enum Trigger {
    /// Stop hook on a session — produces one Session consolidation.
    SessionEnd {
        /// The session that just ended.
        session_id: String,
    },
    /// Nightly cron over all sessions in a repo — produces one
    /// Topic consolidation per HDBSCAN cluster.
    NightlyTopic {
        /// Repo slug to cluster within.
        repo: String,
    },
    /// New `Kind::Decision` envelope landed — produces one
    /// DecisionTrace consolidation.
    DecisionLanded {
        /// The decision id that triggered the run.
        decision_id: String,
        /// Whether the orchestrator should auto-promote to Opus.
        /// Caller can force-promote by setting this; the default
        /// rule from [`ProducerSelection::for_trigger`] also kicks
        /// in at routing time.
        force_deep: bool,
    },
}

/// Phase11j §2.7 — routing decision the orchestrator emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerSelection {
    /// Grain the producer will emit.
    pub grain: ConsolidationGrain,
    /// Summariser the orchestrator will run the producer through.
    pub summariser: SummariserKind,
    /// Repo scope (when applicable). `None` for triggers that
    /// span repos (none today).
    pub repo: Option<String>,
}

impl ProducerSelection {
    /// Map a trigger onto a `(grain, summariser)` pair. Encodes the
    /// auto-promotion rule: DecisionTrace always runs against Opus.
    pub fn for_trigger(trigger: &Trigger) -> Self {
        match trigger {
            Trigger::SessionEnd { .. } => Self {
                grain: ConsolidationGrain::Session,
                summariser: SummariserKind::Haiku45,
                repo: None,
            },
            Trigger::NightlyTopic { repo } => Self {
                grain: ConsolidationGrain::Topic,
                summariser: SummariserKind::Haiku45,
                repo: Some(repo.clone()),
            },
            Trigger::DecisionLanded { force_deep, .. } => Self {
                grain: ConsolidationGrain::DecisionTrace,
                // DecisionTrace always promotes to Opus; the
                // `force_deep` knob is informational today (kept
                // for §2.7 wiring against high-impact session
                // outcomes).
                summariser: if *force_deep {
                    SummariserKind::Opus47
                } else {
                    SummariserKind::Opus47
                },
                repo: None,
            },
        }
    }

    /// Snake-case label for the chosen grain. Drives the
    /// `cost_telemetry::GrainCost` keys + the §6.2 fidelity IT
    /// per-grain bucketing.
    pub fn grain_label(&self) -> &'static str {
        match self.grain {
            ConsolidationGrain::Session => "session",
            ConsolidationGrain::Topic => "topic",
            ConsolidationGrain::DecisionTrace => "decision_trace",
        }
    }
}

/// Phase11j §2.7 — orchestrator handle. The §2.1 skeleton stores
/// the summariser handles + cost ledger; live producer dispatch
/// lands alongside §2.7.
pub struct Orchestrator {
    haiku: std::sync::Arc<dyn crate::summariser::Summariser>,
    opus: std::sync::Arc<dyn crate::summariser::Summariser>,
    cost: std::sync::Arc<std::sync::Mutex<crate::cost_telemetry::CostLedger>>,
}

impl Orchestrator {
    /// Construct an orchestrator with the two summariser handles
    /// the auto-promotion rule needs.
    pub fn new(
        haiku: std::sync::Arc<dyn crate::summariser::Summariser>,
        opus: std::sync::Arc<dyn crate::summariser::Summariser>,
    ) -> Self {
        Self {
            haiku,
            opus,
            cost: std::sync::Arc::new(std::sync::Mutex::new(
                crate::cost_telemetry::CostLedger::default(),
            )),
        }
    }

    /// Pick the summariser handle for a selection.
    pub fn summariser_for(
        &self,
        selection: &ProducerSelection,
    ) -> std::sync::Arc<dyn crate::summariser::Summariser> {
        match selection.summariser {
            SummariserKind::Haiku45 => self.haiku.clone(),
            SummariserKind::Opus47 => self.opus.clone(),
        }
    }

    /// Cheap clone of the cost ledger handle so callers can read
    /// the running spend without holding the orchestrator.
    pub fn cost_ledger(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<crate::cost_telemetry::CostLedger>> {
        self.cost.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_trigger_picks_session_grain_and_haiku() {
        let sel = ProducerSelection::for_trigger(&Trigger::SessionEnd {
            session_id: "sess".into(),
        });
        assert_eq!(sel.grain, ConsolidationGrain::Session);
        assert_eq!(sel.summariser, SummariserKind::Haiku45);
        assert_eq!(sel.grain_label(), "session");
    }

    #[test]
    fn nightly_topic_trigger_picks_topic_grain_and_haiku_with_repo() {
        let sel = ProducerSelection::for_trigger(&Trigger::NightlyTopic {
            repo: "cortex".into(),
        });
        assert_eq!(sel.grain, ConsolidationGrain::Topic);
        assert_eq!(sel.summariser, SummariserKind::Haiku45);
        assert_eq!(sel.repo.as_deref(), Some("cortex"));
    }

    #[test]
    fn decision_landed_always_promotes_to_opus() {
        let sel = ProducerSelection::for_trigger(&Trigger::DecisionLanded {
            decision_id: "DEC-1".into(),
            force_deep: false,
        });
        assert_eq!(sel.grain, ConsolidationGrain::DecisionTrace);
        assert_eq!(sel.summariser, SummariserKind::Opus47);
    }
}
