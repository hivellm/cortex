//! Phase14a §2.1 — [`SessionGrain`] consolidator.
//!
//! Clusters every envelope sharing a `session_id` and runs them
//! through the existing
//! [`crate::consolidator::producer::session::produce`] pipeline via
//! the [`Orchestrator::run_session`] entry point. The daemon (§3)
//! dispatches a [`Trigger::SessionEnd`] to this grain on every Stop
//! hook + nightly back-fill row.
//!
//! Composition with [`EnvelopeProducer`] keeps the daemon's
//! per-trigger checkpoint write on the same `producer_checkpoints`
//! table the bootstrap / claude-archive / topic-cards producers use.
//! The `produce()` batch path is reserved for the daemon-side
//! supervisor loop (§3) and returns a zero-row report here because
//! every run flows through [`Consolidator::on_trigger`] driven by the
//! bus consumer; the supervisor records its own checkpoint per
//! trigger.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use cortex_core::events::ConsolidationGrain;

use crate::consolidator::consolidator_trait::{
    ConsolidationReport, Consolidator, ConsolidatorCtx, ConsolidatorError, TriggerLabel,
};
use crate::consolidator::orchestrator::{Orchestrator, Trigger};
use crate::consolidator::producer::session::SessionInput;
use crate::consolidator::source::{LiveSessionSource, SourceError};
use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

/// Stable producer name for the session grain — written into
/// `producer_checkpoints.producer_name` when the daemon records a
/// per-trigger checkpoint.
pub const SESSION_GRAIN_PRODUCER_NAME: &str = "consolidator.session";

/// Async-trait wrapper around the live [`LiveSessionSource`] fetch so
/// tests can supply an in-memory fixture without touching parquet.
#[async_trait]
pub trait SessionInputFetcher: Send + Sync {
    /// Hydrate the envelope set for `session_id` into the
    /// producer-facing [`SessionInput`].
    async fn fetch(&self, session_id: &str) -> Result<SessionInput, SourceError>;
}

/// Production fetcher backed by [`LiveSessionSource`]. The underlying
/// scan is sync; the wrapper hops onto a blocking task so a long
/// archive walk does not stall the daemon's tokio executor.
pub struct LiveSessionInputFetcher {
    inner: LiveSessionSource,
}

impl LiveSessionInputFetcher {
    /// Build a live fetcher rooted at `source`.
    pub fn new(source: LiveSessionSource) -> Self {
        Self { inner: source }
    }
}

#[async_trait]
impl SessionInputFetcher for LiveSessionInputFetcher {
    async fn fetch(&self, session_id: &str) -> Result<SessionInput, SourceError> {
        let source = self.inner.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || source.fetch(&sid))
            .await
            .map_err(|e| SourceError::Storage(format!("session fetch task: {e}")))?
    }
}

/// Per-grain consolidator dispatched by the daemon on every
/// [`Trigger::SessionEnd`].
pub struct SessionGrain {
    orchestrator: Arc<Orchestrator>,
    fetcher: Arc<dyn SessionInputFetcher>,
}

impl SessionGrain {
    /// Build a session grain that runs through `orchestrator` and
    /// hydrates inputs via `fetcher`.
    pub fn new(orchestrator: Arc<Orchestrator>, fetcher: Arc<dyn SessionInputFetcher>) -> Self {
        Self {
            orchestrator,
            fetcher,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for SessionGrain {
    fn name(&self) -> &'static str {
        SESSION_GRAIN_PRODUCER_NAME
    }

    /// The session grain is trigger-driven: every consolidation
    /// flows through [`Consolidator::on_trigger`] from the daemon's
    /// bus consumer, and the daemon writes the
    /// `producer_checkpoints` row itself. The batch entry point
    /// therefore reports zero envelopes — the per-trigger path is
    /// the production driver. Tests assert this contract so a
    /// future caller that wires the grain into a batch supervisor
    /// surfaces the contract mismatch up front.
    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        Ok(ProducerReport {
            producer_name: SESSION_GRAIN_PRODUCER_NAME.to_string(),
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
        let row = store.latest_producer_checkpoint(SESSION_GRAIN_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[async_trait]
impl Consolidator for SessionGrain {
    fn grain(&self) -> ConsolidationGrain {
        ConsolidationGrain::Session
    }

    async fn on_trigger(
        &self,
        trigger: &Trigger,
        ctx: &ConsolidatorCtx,
    ) -> Result<ConsolidationReport, ConsolidatorError> {
        let session_id = match trigger {
            Trigger::SessionEnd { session_id } => session_id.as_str(),
            Trigger::NightlyTopic { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "nightly_topic",
                    expected: "session_end",
                })
            }
            Trigger::DecisionLanded { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "decision_landed",
                    expected: "session_end",
                })
            }
        };

        let started = Instant::now();
        let input = self
            .fetcher
            .fetch(session_id)
            .await
            .map_err(|e| ConsolidatorError::Other(format!("session fetch: {e}")))?;
        let produced = self
            .orchestrator
            .run_session(&input)
            .await
            .map_err(|e| ConsolidatorError::Summariser(e.to_string()))?;

        Ok(ConsolidationReport {
            grain: ConsolidationGrain::Session,
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
) -> crate::consolidator::summariser::SummariserKind {
    match depth {
        cortex_core::events::ConsolidationDepth::Shallow => {
            crate::consolidator::summariser::SummariserKind::Haiku45
        }
        cortex_core::events::ConsolidationDepth::Deep => {
            crate::consolidator::summariser::SummariserKind::Opus47
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
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

    fn turn_envelope(idx: u8, occurred_at: &str) -> Envelope {
        let payload: Value = serde_json::json!({
            "user_message": format!(
                "user message {idx} investigating failing auth tests with detail"
            ),
            "assistant_message": format!(
                "reply {idx} — patched JWT cache and reran the suite"
            ),
            "outcome": "success",
        });
        Envelope {
            event_id: format!("01HXEVT{idx:019}"),
            schema_version: "1".into(),
            occurred_at: occurred_at.into(),
            ingested_at: None,
            session_id: "01HXSESS00000000000000000A".into(),
            stream: Stream::Live,
            tool: "claude-code".into(),
            model: Some("claude-haiku-4-5".into()),
            kind: Kind::Turn,
            context: ctx(),
            payload,
            redactions: vec![],
            content_hash: "sha256:00".to_string()
                + "00000000000000000000000000000000000000000000000000000000000000",
            parent_event_id: None,
        }
    }

    fn ok_session_summary() -> String {
        serde_json::to_string(&serde_json::json!({
            "title": "JWT cache rotation",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["bump invalidation TTL", "watch p95 latency"],
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
            req: SummariserRequest,
        ) -> Result<SummariserResult, SummariserError> {
            // Producer makes two LLM calls per session — relevance
            // gate first, then full summary. Match on the relevance
            // prompt anchor so the canned summariser returns a
            // green verdict before the real summary lands.
            let text = if req.prompt.contains("relevance judge for the Cortex consolidator") {
                "{\"relevant\": true, \"reason\": \"substantive\"}".to_string()
            } else {
                self.text.clone()
            };
            Ok(SummariserResult {
                text,
                cost_cents: self.cost,
                kind: self.kind,
                input_tokens: 10,
                output_tokens: 200,
            })
        }
    }

    struct InMemoryFetcher {
        input: Mutex<Option<SessionInput>>,
    }

    impl InMemoryFetcher {
        fn with(input: SessionInput) -> Self {
            Self {
                input: Mutex::new(Some(input)),
            }
        }
    }

    #[async_trait]
    impl SessionInputFetcher for InMemoryFetcher {
        async fn fetch(&self, _session_id: &str) -> Result<SessionInput, SourceError> {
            self.input
                .lock()
                .unwrap()
                .clone()
                .ok_or(SourceError::EmptyResult)
        }
    }

    fn build_grain(text: String, cost: u32, input: SessionInput) -> SessionGrain {
        let haiku = Arc::new(CannedSummariser {
            text,
            kind: SummariserKind::Haiku45,
            cost,
        });
        let opus = Arc::new(CannedSummariser {
            text: ok_session_summary(),
            kind: SummariserKind::Opus47,
            cost: 5_000,
        });
        let orchestrator = Arc::new(Orchestrator::new(haiku, opus));
        let fetcher: Arc<dyn SessionInputFetcher> = Arc::new(InMemoryFetcher::with(input));
        SessionGrain::new(orchestrator, fetcher)
    }

    fn two_turn_input() -> SessionInput {
        SessionInput {
            session_id: "01HXSESS00000000000000000A".into(),
            repo: Some("cortex".into()),
            envelopes: vec![
                turn_envelope(1, "2026-04-20T10:00:00Z"),
                turn_envelope(2, "2026-04-20T10:01:00Z"),
            ],
        }
    }

    #[test]
    fn session_grain_reports_session_grain_label() {
        let grain = build_grain(ok_session_summary(), 80, two_turn_input());
        assert_eq!(grain.grain(), ConsolidationGrain::Session);
        assert_eq!(grain.name(), "consolidator.session");
    }

    #[tokio::test]
    async fn session_grain_on_session_end_emits_one_consolidation_report() {
        let grain = build_grain(ok_session_summary(), 80, two_turn_input());
        let trigger = Trigger::SessionEnd {
            session_id: "01HXSESS00000000000000000A".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");

        assert_eq!(report.grain, ConsolidationGrain::Session);
        assert_eq!(
            report.trigger,
            TriggerLabel::SessionEnd {
                session_id: "01HXSESS00000000000000000A".into(),
            }
        );
        assert_eq!(report.envelopes_emitted, 1);
        assert_eq!(report.cost_cents, 80);
        assert_eq!(report.source_event_count, 2);
        assert_eq!(report.finished_at, ts("2026-05-25T12:00:00Z"));
        assert_eq!(report.summariser, SummariserKind::Haiku45);
    }

    #[tokio::test]
    async fn session_grain_rejects_mismatched_trigger() {
        let grain = build_grain(ok_session_summary(), 80, two_turn_input());
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let err = grain
            .on_trigger(
                &Trigger::NightlyTopic {
                    repo: "cortex".into(),
                },
                &ctx,
            )
            .await
            .expect_err("nightly trigger must be rejected");
        match err {
            ConsolidatorError::TriggerMismatch { got, expected } => {
                assert_eq!(got, "nightly_topic");
                assert_eq!(expected, "session_end");
            }
            other => panic!("wrong error: {other}"),
        }

        let err = grain
            .on_trigger(
                &Trigger::DecisionLanded {
                    decision_id: "DEC".into(),
                    force_deep: false,
                },
                &ctx,
            )
            .await
            .expect_err("decision trigger must be rejected");
        assert!(matches!(err, ConsolidatorError::TriggerMismatch { .. }));
    }

    #[tokio::test]
    async fn session_grain_surfaces_source_error_when_fetch_returns_empty() {
        let haiku = Arc::new(CannedSummariser {
            text: ok_session_summary(),
            kind: SummariserKind::Haiku45,
            cost: 80,
        });
        let opus = Arc::new(CannedSummariser {
            text: ok_session_summary(),
            kind: SummariserKind::Opus47,
            cost: 5_000,
        });
        let orchestrator = Arc::new(Orchestrator::new(haiku, opus));
        struct EmptyFetcher;
        #[async_trait]
        impl SessionInputFetcher for EmptyFetcher {
            async fn fetch(&self, _id: &str) -> Result<SessionInput, SourceError> {
                Err(SourceError::EmptyResult)
            }
        }
        let grain = SessionGrain::new(orchestrator, Arc::new(EmptyFetcher));
        let trigger = Trigger::SessionEnd {
            session_id: "01MISSING".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let err = grain
            .on_trigger(&trigger, &ctx)
            .await
            .expect_err("empty source must surface");
        match err {
            ConsolidatorError::Other(msg) => assert!(msg.contains("empty result")),
            other => panic!("wrong error: {other}"),
        }
    }

    #[tokio::test]
    async fn session_grain_produce_returns_zero_row_report() {
        use crate::producer::{ProducerCtx, ProducerMetadataHandle};
        use cortex_storage::MetadataStore;
        use tokio::sync::Mutex as TokioMutex;

        let store = MetadataStore::open_in_memory().unwrap();
        let handle: ProducerMetadataHandle = Arc::new(TokioMutex::new(store));
        let pctx = ProducerCtx::new(handle, "cortex.test").with_now(ts("2026-05-25T12:00:00Z"));
        let grain = build_grain(ok_session_summary(), 80, two_turn_input());
        let report = grain.produce(&pctx).await.unwrap();
        assert_eq!(report.envelopes_emitted, 0);
        assert_eq!(report.batches_emitted, 0);
        assert_eq!(report.producer_name, "consolidator.session");
    }
}
