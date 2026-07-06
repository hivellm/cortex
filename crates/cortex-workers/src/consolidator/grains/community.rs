//! Phase27c §1 — [`CommunityGrain`] consolidator.
//!
//! Dispatched by the daemon on every [`Trigger::CommunityDetected`]
//! (the phase27b writeback landed a fresh partition). The grain
//! delegates the Nexus snapshot to a [`CommunityInputFetcher`] so
//! tests can drive the trigger path without a live graph. Each
//! returned input (one per `(community_id, level)` — §1.3
//! multi-resolution) is fed through `Orchestrator::run_community`;
//! failed communities are logged and skipped so a single under-size
//! partition does not lose the rest of the batch.
//!
//! Composition with [`EnvelopeProducer`] mirrors
//! [`super::topic::TopicGrain`]: the daemon owns the per-trigger
//! checkpoint write, so the batch path returns a zero-row report.
//!
//! Live reality (2026-07): the graph carries no `community_id`
//! properties until phase27b §2.5's writeback worker runs (gated on
//! the semantic projection — ADR-027), so the live fetcher returns
//! an empty vec and every trigger is a benign zero-envelope run.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use cortex_core::events::ConsolidationGrain;

use crate::consolidator::consolidator_trait::{
    ConsolidationReport, Consolidator, ConsolidatorCtx, ConsolidatorError, TriggerLabel,
};
use crate::consolidator::orchestrator::{Orchestrator, Trigger};
use crate::consolidator::producer::community::CommunityInput;
use crate::consolidator::source::{LiveCommunitySource, SourceError};
use crate::consolidator::summariser::SummariserKind;
use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

/// Stable producer name for the community grain.
pub const COMMUNITY_GRAIN_PRODUCER_NAME: &str = "consolidator.community";

/// Async-trait wrapper around the Nexus snapshot so tests can supply
/// in-memory inputs without touching a live graph.
#[async_trait]
pub trait CommunityInputFetcher: Send + Sync {
    /// Snapshot the current partition for `repo` at `snapshot_ms`.
    /// Empty result is `Ok(vec![])` — the realistic live state until
    /// the phase27b §2.5 writeback ships.
    async fn fetch(&self, repo: &str, snapshot_ms: i64)
        -> Result<Vec<CommunityInput>, SourceError>;
}

/// Production fetcher backed by [`LiveCommunitySource`].
pub struct LiveCommunityFetcher {
    inner: LiveCommunitySource,
}

impl LiveCommunityFetcher {
    /// Build a live fetcher.
    pub fn new(source: LiveCommunitySource) -> Self {
        Self { inner: source }
    }
}

#[async_trait]
impl CommunityInputFetcher for LiveCommunityFetcher {
    async fn fetch(
        &self,
        repo: &str,
        snapshot_ms: i64,
    ) -> Result<Vec<CommunityInput>, SourceError> {
        self.inner.fetch(repo, snapshot_ms).await
    }
}

/// Per-grain consolidator dispatched by the daemon on every
/// [`Trigger::CommunityDetected`].
pub struct CommunityGrain {
    orchestrator: Arc<Orchestrator>,
    fetcher: Arc<dyn CommunityInputFetcher>,
}

impl CommunityGrain {
    /// Build a community grain that runs through `orchestrator` and
    /// hydrates inputs via `fetcher`.
    pub fn new(orchestrator: Arc<Orchestrator>, fetcher: Arc<dyn CommunityInputFetcher>) -> Self {
        Self {
            orchestrator,
            fetcher,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for CommunityGrain {
    fn name(&self) -> &'static str {
        COMMUNITY_GRAIN_PRODUCER_NAME
    }

    /// Trigger-driven grain — returns a zero-row report; the daemon
    /// writes the per-trigger checkpoint.
    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        Ok(ProducerReport {
            producer_name: COMMUNITY_GRAIN_PRODUCER_NAME.to_string(),
            envelopes_emitted: 0,
            batches_emitted: 0,
            last_event_id: String::new(),
            last_occurred_at: Some(ctx.now),
        })
    }

    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> anyhow::Result<Option<ProducerCheckpoint>> {
        let store = ctx.metadata.lock().await;
        let row = store.latest_producer_checkpoint(COMMUNITY_GRAIN_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[async_trait]
impl Consolidator for CommunityGrain {
    fn grain(&self) -> ConsolidationGrain {
        ConsolidationGrain::Community
    }

    async fn on_trigger(
        &self,
        trigger: &Trigger,
        ctx: &ConsolidatorCtx,
    ) -> Result<ConsolidationReport, ConsolidatorError> {
        let repo = match trigger {
            Trigger::CommunityDetected { repo } => repo.as_str(),
            Trigger::SessionEnd { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "session_end",
                    expected: "community_detected",
                })
            }
            Trigger::NightlyTopic { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "nightly_topic",
                    expected: "community_detected",
                })
            }
            Trigger::DecisionLanded { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "decision_landed",
                    expected: "community_detected",
                })
            }
        };

        let started = Instant::now();
        let inputs = self
            .fetcher
            .fetch(repo, ctx.now.timestamp_millis())
            .await
            .map_err(|e| ConsolidatorError::Other(format!("community fetch: {e}")))?;

        let mut envelopes_emitted: u64 = 0;
        let mut cost_cents: u64 = 0;
        let mut source_event_count: u64 = 0;
        let mut summariser_seen: Option<SummariserKind> = None;

        for input in &inputs {
            match self.orchestrator.run_community(input).await {
                Ok(produced) => {
                    ctx.record_cost(
                        "community",
                        &produced.payload.model,
                        produced.cost_cents,
                        produced.input_tokens,
                        produced.output_tokens,
                    );
                    envelopes_emitted += 1;
                    cost_cents = cost_cents.saturating_add(u64::from(produced.cost_cents));
                    source_event_count = source_event_count
                        .saturating_add(u64::from(produced.payload.source_event_count));
                    summariser_seen = Some(depth_to_summariser(produced.payload.depth));
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        repo = %repo,
                        community_id = input.community_id,
                        level = input.level,
                        "community grain: input run failed",
                    );
                }
            }
        }

        Ok(ConsolidationReport {
            grain: ConsolidationGrain::Community,
            trigger: TriggerLabel::from(trigger),
            envelopes_emitted,
            cost_cents,
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            source_event_count,
            finished_at: ctx.now,
            summariser: summariser_seen.unwrap_or(SummariserKind::Haiku45),
        })
    }
}

fn depth_to_summariser(depth: cortex_core::events::ConsolidationDepth) -> SummariserKind {
    match depth {
        cortex_core::events::ConsolidationDepth::Shallow => SummariserKind::Haiku45,
        cortex_core::events::ConsolidationDepth::Deep => SummariserKind::Opus47,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::producer::community::{CommunityMember, MIN_COMMUNITY_SIZE};
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserRequest, SummariserResult,
    };
    use chrono::{DateTime, Utc};
    use std::sync::Mutex;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    fn ok_community_summary() -> String {
        serde_json::to_string(&serde_json::json!({
            "title": "Graph write pipeline",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["nexus_client anchors writes"],
        }))
        .unwrap()
    }

    fn make_input(community_id: u32, level: u32) -> CommunityInput {
        CommunityInput {
            community_id,
            level,
            repo: "cortex".into(),
            members: (0..MIN_COMMUNITY_SIZE)
                .map(|i| CommunityMember {
                    id: format!("n{community_id}-{i}"),
                    label: "Symbol".into(),
                    name: format!("sym{i}"),
                    is_god_node: i == 0,
                })
                .collect(),
            cross_edges: Vec::new(),
            snapshot_ms: 1_000,
        }
    }

    struct CannedSummariser {
        text: String,
        kind: SummariserKind,
        cost: u32,
    }

    #[async_trait]
    impl Summariser for CannedSummariser {
        fn kind(&self) -> SummariserKind {
            self.kind
        }
        async fn summarise(
            &self,
            _req: SummariserRequest,
        ) -> Result<SummariserResult, SummariserError> {
            Ok(SummariserResult {
                text: self.text.clone(),
                cost_cents: self.cost,
                kind: self.kind,
                input_tokens: 10,
                output_tokens: 200,
            })
        }
    }

    struct InMemoryFetcher {
        inputs: Mutex<Vec<CommunityInput>>,
    }

    impl InMemoryFetcher {
        fn with(inputs: Vec<CommunityInput>) -> Self {
            Self {
                inputs: Mutex::new(inputs),
            }
        }
    }

    #[async_trait]
    impl CommunityInputFetcher for InMemoryFetcher {
        async fn fetch(
            &self,
            _repo: &str,
            _snapshot_ms: i64,
        ) -> Result<Vec<CommunityInput>, SourceError> {
            Ok(self.inputs.lock().unwrap().clone())
        }
    }

    fn build_grain(inputs: Vec<CommunityInput>, cost: u32) -> CommunityGrain {
        let haiku = Arc::new(CannedSummariser {
            text: ok_community_summary(),
            kind: SummariserKind::Haiku45,
            cost,
        });
        let opus = Arc::new(CannedSummariser {
            text: ok_community_summary(),
            kind: SummariserKind::Opus47,
            cost: 5_000,
        });
        let orchestrator = Arc::new(Orchestrator::new(haiku, opus));
        let fetcher: Arc<dyn CommunityInputFetcher> = Arc::new(InMemoryFetcher::with(inputs));
        CommunityGrain::new(orchestrator, fetcher)
    }

    #[test]
    fn community_grain_reports_community_grain_label() {
        let grain = build_grain(Vec::new(), 80);
        assert_eq!(grain.grain(), ConsolidationGrain::Community);
        assert_eq!(grain.name(), "consolidator.community");
    }

    #[tokio::test]
    async fn community_grain_emits_one_envelope_per_community_per_level() {
        // §1.3 — two communities at level 0 plus one of them re-cut
        // at level 1 → three envelopes.
        let inputs = vec![make_input(1, 0), make_input(2, 0), make_input(1, 1)];
        let grain = build_grain(inputs, 70);
        let trigger = Trigger::CommunityDetected {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-07-06T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");

        assert_eq!(report.grain, ConsolidationGrain::Community);
        assert_eq!(
            report.trigger,
            TriggerLabel::CommunityDetected {
                repo: "cortex".into(),
            }
        );
        assert_eq!(report.envelopes_emitted, 3);
        assert_eq!(report.cost_cents, 210);
        assert_eq!(report.source_event_count, (MIN_COMMUNITY_SIZE as u64) * 3);
        assert_eq!(report.summariser, SummariserKind::Haiku45);
    }

    #[tokio::test]
    async fn community_grain_empty_partition_is_a_benign_zero_run() {
        // The realistic live state until phase27b §2.5 ships.
        let grain = build_grain(Vec::new(), 80);
        let trigger = Trigger::CommunityDetected {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-07-06T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");
        assert_eq!(report.envelopes_emitted, 0);
        assert_eq!(report.cost_cents, 0);
    }

    #[tokio::test]
    async fn community_grain_skips_under_size_communities_and_keeps_going() {
        let mut small = make_input(9, 0);
        small.members.truncate(MIN_COMMUNITY_SIZE - 1);
        let inputs = vec![small, make_input(1, 0)];
        let grain = build_grain(inputs, 80);
        let trigger = Trigger::CommunityDetected {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-07-06T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");
        assert_eq!(report.envelopes_emitted, 1);
        assert_eq!(report.cost_cents, 80);
    }

    #[tokio::test]
    async fn community_grain_rejects_mismatched_trigger() {
        let grain = build_grain(Vec::new(), 80);
        let ctx = ConsolidatorCtx::at(ts("2026-07-06T12:00:00Z"));
        let err = grain
            .on_trigger(
                &Trigger::SessionEnd {
                    session_id: "sid".into(),
                },
                &ctx,
            )
            .await
            .expect_err("must reject");
        match err {
            ConsolidatorError::TriggerMismatch { got, expected } => {
                assert_eq!(got, "session_end");
                assert_eq!(expected, "community_detected");
            }
            other => panic!("wrong error: {other}"),
        }
    }

    #[tokio::test]
    async fn community_grain_records_cost_into_ctx_ledger() {
        let inputs = vec![make_input(1, 0), make_input(2, 0)];
        let grain = build_grain(inputs, 70);
        let trigger = Trigger::CommunityDetected {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-07-06T12:00:00Z"));
        grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");

        let ledger = ctx.cost.lock().unwrap();
        let bucket = ledger.per_grain.get("community").expect("community bucket");
        assert_eq!(bucket.consolidations, 2);
        assert_eq!(bucket.cost_cents, 140);
    }
}
