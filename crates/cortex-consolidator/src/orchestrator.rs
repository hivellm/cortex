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

/// Phase11j §2.7 — orchestrator handle. Holds the two summariser
/// handles (Haiku for shallow / Opus for deep), the cost ledger,
/// and the operator-tunable budget. Producer dispatch (`run_*`)
/// picks the summariser via the auto-promotion rule, gates against
/// the budget, runs the producer, and records the realised cost.
pub struct Orchestrator {
    haiku: std::sync::Arc<dyn crate::summariser::Summariser>,
    opus: std::sync::Arc<dyn crate::summariser::Summariser>,
    cost: std::sync::Arc<std::sync::Mutex<crate::cost_telemetry::CostLedger>>,
    budget: crate::cost_telemetry::CostBudget,
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
            budget: crate::cost_telemetry::CostBudget::default(),
        }
    }

    /// Override the default $1 000/month budget. Builder method so
    /// existing callers keep the default.
    pub fn with_budget(mut self, budget: crate::cost_telemetry::CostBudget) -> Self {
        self.budget = budget;
        self
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

    /// Run the Session producer against `input`. Picks Haiku via
    /// the trigger rule, gates against the budget, records the
    /// realised cost. Returns the produced consolidation OR a
    /// `CostCeiling` error when the budget would be breached even
    /// before the producer runs.
    pub async fn run_session(
        &self,
        input: &crate::producer::session::SessionInput,
    ) -> Result<crate::producer::ProducedConsolidation, crate::producer::ProducerError> {
        let trigger = Trigger::SessionEnd {
            session_id: input.session_id.clone(),
        };
        let sel = ProducerSelection::for_trigger(&trigger);
        self.gate_budget(sel.summariser, "session")?;
        let summariser = self.summariser_for(&sel);
        let produced = crate::producer::session::produce(input, summariser.as_ref()).await?;
        self.record_cost(sel.grain_label(), produced.cost_cents);
        Ok(produced)
    }

    /// Run the Topic producer against `cluster`. Picks Haiku.
    pub async fn run_topic(
        &self,
        cluster: &crate::producer::topic::TopicCluster,
    ) -> Result<crate::producer::ProducedConsolidation, crate::producer::ProducerError> {
        let trigger = Trigger::NightlyTopic {
            repo: cluster.repo.clone(),
        };
        let sel = ProducerSelection::for_trigger(&trigger);
        self.gate_budget(sel.summariser, "topic")?;
        let summariser = self.summariser_for(&sel);
        let produced = crate::producer::topic::produce(cluster, summariser.as_ref()).await?;
        self.record_cost(sel.grain_label(), produced.cost_cents);
        Ok(produced)
    }

    /// Run the DecisionTrace producer against `input`. Always
    /// promotes to Opus (deeper grain → higher fidelity).
    pub async fn run_decision_trace(
        &self,
        input: &crate::producer::decision_trace::DecisionTraceInput,
    ) -> Result<crate::producer::ProducedConsolidation, crate::producer::ProducerError> {
        let trigger = Trigger::DecisionLanded {
            decision_id: input.decision_id().to_string(),
            force_deep: false,
        };
        let sel = ProducerSelection::for_trigger(&trigger);
        self.gate_budget(sel.summariser, "decision_trace")?;
        let summariser = self.summariser_for(&sel);
        let produced = crate::producer::decision_trace::produce(input, summariser.as_ref()).await?;
        self.record_cost(sel.grain_label(), produced.cost_cents);
        Ok(produced)
    }

    /// Estimate the upcoming charge against the budget. The
    /// estimate uses a per-grain conservative ceiling (4 000 cents
    /// for Opus DecisionTrace, 100 cents for Haiku). When the
    /// estimate would breach the cap, the call returns
    /// `SummariserError::CostCeiling` so the orchestrator never
    /// emits a charge that breaks the operator's budget.
    fn gate_budget(
        &self,
        kind: SummariserKind,
        grain_label: &str,
    ) -> Result<(), crate::producer::ProducerError> {
        let _ = grain_label;
        let estimated = match kind {
            SummariserKind::Haiku45 => 100,
            SummariserKind::Opus47 => 4_000,
        };
        let ledger = self.cost.lock().expect("cost ledger lock poisoned");
        if !self.budget.can_afford(&ledger, estimated) {
            return Err(crate::producer::ProducerError::Summariser(
                crate::summariser::SummariserError::CostCeiling(ledger.total_cents),
            ));
        }
        Ok(())
    }

    /// Record the realised cost against the ledger.
    fn record_cost(&self, grain_label: &str, cost_cents: u32) {
        let mut ledger = self.cost.lock().expect("cost ledger lock poisoned");
        ledger.record(grain_label, cost_cents);
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

    use crate::summariser::{
        Summariser as Sum, SummariserError, SummariserKind as SK, SummariserRequest as Req,
        SummariserResult,
    };

    struct StubSummariser {
        kind: SK,
        text: String,
        cost: u32,
    }
    #[async_trait::async_trait]
    impl Sum for StubSummariser {
        fn kind(&self) -> SK {
            self.kind
        }
        async fn summarise(&self, _r: Req) -> Result<SummariserResult, SummariserError> {
            Ok(SummariserResult {
                text: self.text.clone(),
                cost_cents: self.cost,
                kind: self.kind,
                input_tokens: 1,
                output_tokens: 1,
            })
        }
    }

    fn ok_session_payload() -> String {
        serde_json::to_string(&serde_json::json!({
            "title": "tune ef_search",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["raise ef_search to 128"]
        }))
        .unwrap()
    }

    fn turn_envelope() -> cortex_core::events::Envelope {
        cortex_core::events::Envelope {
            event_id: "01HXEVT0000000000000000001".into(),
            schema_version: "1".into(),
            occurred_at: "2026-04-20T10:00:00Z".into(),
            ingested_at: None,
            session_id: "01HXSESS00000000000000000A".into(),
            stream: cortex_core::events::Stream::Live,
            tool: "claude-code".into(),
            model: None,
            kind: cortex_core::events::Kind::Turn,
            context: cortex_core::events::Context {
                repo: Some("cortex".into()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".into(),
                ide: None,
                extras: Default::default(),
            },
            payload: serde_json::json!({"user_message": "hi", "assistant_message": "ok"}),
            redactions: vec![],
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .into(),
            parent_event_id: None,
        }
    }

    #[tokio::test]
    async fn run_session_records_cost_under_session_grain_label() {
        use std::sync::Arc;
        let haiku = Arc::new(StubSummariser {
            kind: SK::Haiku45,
            text: ok_session_payload(),
            cost: 80,
        });
        let opus = Arc::new(StubSummariser {
            kind: SK::Opus47,
            text: ok_session_payload(),
            cost: 5_000,
        });
        let orch = Orchestrator::new(haiku, opus);
        let input = crate::producer::session::SessionInput {
            session_id: "01HXSESS00000000000000000A".into(),
            repo: Some("cortex".into()),
            envelopes: vec![turn_envelope()],
        };
        let produced = orch.run_session(&input).await.expect("run_session");
        assert_eq!(produced.cost_cents, 80);
        let ledger = orch.cost_ledger();
        let g = ledger.lock().unwrap();
        assert_eq!(g.per_grain["session"].consolidations, 1);
        assert_eq!(g.per_grain["session"].cost_cents, 80);
        assert_eq!(g.total_cents, 80);
    }

    #[tokio::test]
    async fn run_session_refuses_when_budget_is_exhausted() {
        use std::sync::Arc;
        let haiku = Arc::new(StubSummariser {
            kind: SK::Haiku45,
            text: ok_session_payload(),
            cost: 80,
        });
        let opus = Arc::new(StubSummariser {
            kind: SK::Opus47,
            text: ok_session_payload(),
            cost: 5_000,
        });
        // Tight budget: enough for one shallow run, not enough for
        // a second once the first lands. The gate compares *the
        // estimated charge* against the cap, so a 100-cent estimate
        // against a 99-cent ledger remainder must trip.
        let orch = Orchestrator::new(haiku, opus).with_budget(crate::cost_telemetry::CostBudget {
            monthly_cents_cap: 100,
        });
        let input = crate::producer::session::SessionInput {
            session_id: "01HXSESS00000000000000000A".into(),
            repo: Some("cortex".into()),
            envelopes: vec![turn_envelope()],
        };
        // First run lands (estimated 100, ledger 0, fits exactly).
        orch.run_session(&input).await.expect("first run");
        // Second run trips the gate.
        let err = orch.run_session(&input).await.expect_err("budget gate");
        assert!(format!("{err}").contains("cost ceiling"));
    }
}
