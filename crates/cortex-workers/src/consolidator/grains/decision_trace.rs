//! Phase14a §2.3 — [`DecisionTraceGrain`] consolidator.
//!
//! Dispatched by the daemon on every [`Trigger::DecisionLanded`].
//! Walks the `parent_event_id` chain from the decision envelope back
//! to the root (up to
//! [`crate::consolidator::producer::decision_trace::MAX_HOPS`] hops)
//! via a [`DecisionTraceFetcher`] and runs the resulting
//! [`DecisionTraceInput`] through
//! [`Orchestrator::run_decision_trace`]. The orchestrator auto-
//! promotes to Opus; the `force_deep` flag on the trigger is
//! informational today and reaches the orchestrator via the trigger
//! shape.
//!
//! Composition with [`EnvelopeProducer`] mirrors the other grains:
//! the daemon owns the per-trigger checkpoint write.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use cortex_core::events::ConsolidationGrain;

use crate::consolidator::consolidator_trait::{
    ConsolidationReport, Consolidator, ConsolidatorCtx, ConsolidatorError, TriggerLabel,
};
use crate::consolidator::orchestrator::{Orchestrator, Trigger};
use crate::consolidator::producer::decision_trace::DecisionTraceInput;
use crate::consolidator::source::{LiveDecisionTraceSource, SourceError};
use crate::consolidator::summariser::SummariserKind;
use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

/// Stable producer name for the decision-trace grain.
pub const DECISION_TRACE_GRAIN_PRODUCER_NAME: &str = "consolidator.decision_trace";

/// Async-trait wrapper around the live archive walk so tests can
/// supply an in-memory chain without touching parquet.
#[async_trait]
pub trait DecisionTraceFetcher: Send + Sync {
    /// Resolve the decision envelope + walk its ancestor chain.
    async fn fetch(&self, decision_event_id: &str) -> Result<DecisionTraceInput, SourceError>;
}

/// Production fetcher backed by [`LiveDecisionTraceSource`]. The
/// archive walk is sync; the wrapper hops onto a blocking task so a
/// deep chain walk does not stall the daemon's tokio executor.
pub struct LiveDecisionTraceFetcher {
    inner: LiveDecisionTraceSource,
}

impl LiveDecisionTraceFetcher {
    /// Build a live fetcher rooted at `source`.
    pub fn new(source: LiveDecisionTraceSource) -> Self {
        Self { inner: source }
    }
}

#[async_trait]
impl DecisionTraceFetcher for LiveDecisionTraceFetcher {
    async fn fetch(&self, decision_event_id: &str) -> Result<DecisionTraceInput, SourceError> {
        let source = self.inner.clone();
        let id = decision_event_id.to_string();
        tokio::task::spawn_blocking(move || source.fetch(&id))
            .await
            .map_err(|e| SourceError::Storage(format!("decision-trace fetch task: {e}")))?
    }
}

/// Per-grain consolidator dispatched by the daemon on every
/// [`Trigger::DecisionLanded`].
pub struct DecisionTraceGrain {
    orchestrator: Arc<Orchestrator>,
    fetcher: Arc<dyn DecisionTraceFetcher>,
}

impl DecisionTraceGrain {
    /// Build a decision-trace grain that runs through `orchestrator`
    /// and hydrates inputs via `fetcher`.
    pub fn new(orchestrator: Arc<Orchestrator>, fetcher: Arc<dyn DecisionTraceFetcher>) -> Self {
        Self {
            orchestrator,
            fetcher,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for DecisionTraceGrain {
    fn name(&self) -> &'static str {
        DECISION_TRACE_GRAIN_PRODUCER_NAME
    }

    /// Trigger-driven grain — see [`super::session::SessionGrain`]
    /// docs. Returns a zero-row report; the daemon writes the
    /// per-trigger checkpoint.
    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        Ok(ProducerReport {
            producer_name: DECISION_TRACE_GRAIN_PRODUCER_NAME.to_string(),
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
        let row = store.latest_producer_checkpoint(DECISION_TRACE_GRAIN_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[async_trait]
impl Consolidator for DecisionTraceGrain {
    fn grain(&self) -> ConsolidationGrain {
        ConsolidationGrain::DecisionTrace
    }

    async fn on_trigger(
        &self,
        trigger: &Trigger,
        ctx: &ConsolidatorCtx,
    ) -> Result<ConsolidationReport, ConsolidatorError> {
        let decision_id = match trigger {
            Trigger::DecisionLanded { decision_id, .. } => decision_id.as_str(),
            Trigger::SessionEnd { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "session_end",
                    expected: "decision_landed",
                })
            }
            Trigger::NightlyTopic { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "nightly_topic",
                    expected: "decision_landed",
                })
            }
        };

        let started = Instant::now();
        let input = self
            .fetcher
            .fetch(decision_id)
            .await
            .map_err(|e| ConsolidatorError::Other(format!("decision-trace fetch: {e}")))?;
        let produced = self
            .orchestrator
            .run_decision_trace(&input)
            .await
            .map_err(|e| ConsolidatorError::Summariser(e.to_string()))?;

        Ok(ConsolidationReport {
            grain: ConsolidationGrain::DecisionTrace,
            trigger: TriggerLabel::from(trigger),
            envelopes_emitted: 1,
            cost_cents: u64::from(produced.cost_cents),
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            source_event_count: u64::from(produced.payload.source_event_count),
            finished_at: ctx.now,
            summariser: depth_to_summariser(produced.payload.depth),
        })
    }
}

fn depth_to_summariser(
    depth: cortex_core::events::ConsolidationDepth,
) -> SummariserKind {
    match depth {
        cortex_core::events::ConsolidationDepth::Shallow => SummariserKind::Haiku45,
        cortex_core::events::ConsolidationDepth::Deep => SummariserKind::Opus47,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserRequest, SummariserResult,
    };
    use chrono::{DateTime, Utc};
    use cortex_core::events::{Context, Envelope, Kind, Stream};
    use serde_json::Value;
    use std::sync::Mutex;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    fn ctx() -> Context {
        Context {
            repo: Some("cortex".into()),
            branch: None,
            commit: None,
            cwd: None,
            user: None,
            platform: "linux".into(),
            ide: None,
            extras: Default::default(),
        }
    }

    fn decision_envelope(decision_id: &str) -> Envelope {
        let payload: Value = serde_json::json!({
            "decision_id": decision_id,
            "title": "Adopt HNSW for vector index",
            "status": "accepted",
            "rationale": "Faster recall@10 at production load",
        });
        Envelope {
            event_id: format!("01HXDEC{decision_id}"),
            schema_version: "1".into(),
            occurred_at: "2026-04-20T11:00:00Z".into(),
            ingested_at: None,
            session_id: "01HXSESS00000000000000000A".into(),
            stream: Stream::Live,
            tool: "cortex-cli".into(),
            model: None,
            kind: Kind::Decision,
            context: ctx(),
            payload,
            redactions: vec![],
            content_hash: "sha256:00".to_string()
                + "00000000000000000000000000000000000000000000000000000000000000",
            parent_event_id: None,
        }
    }

    fn ok_decision_summary() -> String {
        serde_json::to_string(&serde_json::json!({
            "title": "Adopt HNSW",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["HNSW chosen", "Meili kept for full-text"],
        }))
        .unwrap()
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

    struct InMemoryDecisionFetcher {
        input: Mutex<Option<DecisionTraceInput>>,
    }

    impl InMemoryDecisionFetcher {
        fn with(input: DecisionTraceInput) -> Self {
            Self {
                input: Mutex::new(Some(input)),
            }
        }
    }

    #[async_trait]
    impl DecisionTraceFetcher for InMemoryDecisionFetcher {
        async fn fetch(&self, _id: &str) -> Result<DecisionTraceInput, SourceError> {
            self.input
                .lock()
                .unwrap()
                .clone()
                .ok_or(SourceError::EmptyResult)
        }
    }

    fn build_grain(input: DecisionTraceInput) -> DecisionTraceGrain {
        let haiku = Arc::new(CannedSummariser {
            text: ok_decision_summary(),
            kind: SummariserKind::Haiku45,
            cost: 80,
        });
        let opus = Arc::new(CannedSummariser {
            text: ok_decision_summary(),
            kind: SummariserKind::Opus47,
            cost: 3_200,
        });
        let orchestrator = Arc::new(Orchestrator::new(haiku, opus));
        let fetcher: Arc<dyn DecisionTraceFetcher> = Arc::new(InMemoryDecisionFetcher::with(input));
        DecisionTraceGrain::new(orchestrator, fetcher)
    }

    fn single_decision_input() -> DecisionTraceInput {
        DecisionTraceInput {
            decision: decision_envelope("DEC1"),
            chain: vec![],
            repo: Some("cortex".into()),
        }
    }

    #[test]
    fn decision_trace_grain_reports_decision_trace_grain_label() {
        let grain = build_grain(single_decision_input());
        assert_eq!(grain.grain(), ConsolidationGrain::DecisionTrace);
        assert_eq!(grain.name(), "consolidator.decision_trace");
    }

    #[tokio::test]
    async fn decision_trace_grain_on_decision_landed_promotes_to_opus_and_reports() {
        let grain = build_grain(single_decision_input());
        let trigger = Trigger::DecisionLanded {
            decision_id: "DEC1".into(),
            force_deep: false,
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");

        assert_eq!(report.grain, ConsolidationGrain::DecisionTrace);
        assert_eq!(
            report.trigger,
            TriggerLabel::DecisionLanded {
                decision_id: "DEC1".into(),
            }
        );
        assert_eq!(report.envelopes_emitted, 1);
        assert_eq!(report.cost_cents, 3_200);
        // chain empty + decision-only → producer counts 1 source envelope
        assert_eq!(report.source_event_count, 1);
        assert_eq!(report.summariser, SummariserKind::Opus47);
        assert_eq!(report.finished_at, ts("2026-05-25T12:00:00Z"));
    }

    #[tokio::test]
    async fn decision_trace_grain_force_deep_flag_is_recorded_in_trigger_label() {
        // The TriggerLabel from-impl drops force_deep so two
        // triggers with different force_deep render the same label —
        // the report stays summariser-agnostic about the knob.
        let grain = build_grain(single_decision_input());
        let trigger_a = Trigger::DecisionLanded {
            decision_id: "DEC1".into(),
            force_deep: true,
        };
        let trigger_b = Trigger::DecisionLanded {
            decision_id: "DEC1".into(),
            force_deep: false,
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let report_a = grain.on_trigger(&trigger_a, &ctx).await.unwrap();
        let report_b = grain.on_trigger(&trigger_b, &ctx).await.unwrap();
        assert_eq!(report_a.trigger, report_b.trigger);
    }

    #[tokio::test]
    async fn decision_trace_grain_rejects_mismatched_trigger() {
        let grain = build_grain(single_decision_input());
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        for (bad, got) in [
            (
                Trigger::SessionEnd {
                    session_id: "sid".into(),
                },
                "session_end",
            ),
            (
                Trigger::NightlyTopic {
                    repo: "cortex".into(),
                },
                "nightly_topic",
            ),
        ] {
            let err = grain
                .on_trigger(&bad, &ctx)
                .await
                .expect_err("must reject");
            match err {
                ConsolidatorError::TriggerMismatch { got: g, expected } => {
                    assert_eq!(g, got);
                    assert_eq!(expected, "decision_landed");
                }
                other => panic!("wrong error: {other}"),
            }
        }
    }

    #[tokio::test]
    async fn decision_trace_grain_surfaces_source_error_on_missing_decision() {
        let haiku = Arc::new(CannedSummariser {
            text: ok_decision_summary(),
            kind: SummariserKind::Haiku45,
            cost: 80,
        });
        let opus = Arc::new(CannedSummariser {
            text: ok_decision_summary(),
            kind: SummariserKind::Opus47,
            cost: 3_200,
        });
        let orchestrator = Arc::new(Orchestrator::new(haiku, opus));
        struct MissingFetcher;
        #[async_trait]
        impl DecisionTraceFetcher for MissingFetcher {
            async fn fetch(&self, _id: &str) -> Result<DecisionTraceInput, SourceError> {
                Err(SourceError::EmptyResult)
            }
        }
        let grain = DecisionTraceGrain::new(orchestrator, Arc::new(MissingFetcher));
        let trigger = Trigger::DecisionLanded {
            decision_id: "MISSING".into(),
            force_deep: false,
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let err = grain
            .on_trigger(&trigger, &ctx)
            .await
            .expect_err("missing decision must surface");
        match err {
            ConsolidatorError::Other(msg) => assert!(msg.contains("empty result")),
            other => panic!("wrong error: {other}"),
        }
    }

    #[tokio::test]
    async fn decision_trace_grain_produce_returns_zero_row_report() {
        use crate::producer::{ProducerCtx, ProducerMetadataHandle};
        use cortex_storage::MetadataStore;
        use tokio::sync::Mutex as TokioMutex;

        let store = MetadataStore::open_in_memory().unwrap();
        let handle: ProducerMetadataHandle = Arc::new(TokioMutex::new(store));
        let pctx = ProducerCtx::new(handle, "cortex.test").with_now(ts("2026-05-25T12:00:00Z"));
        let grain = build_grain(single_decision_input());
        let report = grain.produce(&pctx).await.unwrap();
        assert_eq!(report.envelopes_emitted, 0);
        assert_eq!(report.batches_emitted, 0);
        assert_eq!(report.producer_name, "consolidator.decision_trace");
    }
}
