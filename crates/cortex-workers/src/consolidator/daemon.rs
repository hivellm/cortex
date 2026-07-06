//! Phase14a §3.1 + §3.2 — consolidator daemon main loop.
//!
//! The daemon pulls [`Trigger`] envelopes from a [`TriggerSource`]
//! (Synap-backed in production, in-memory in tests), dispatches each
//! to the matching grain via [`Consolidator::on_trigger`], and acks
//! the offset back to the source. Concurrency is deliberately one
//! grain at a time — consolidation is not throughput-sensitive and
//! the cost ledger plus producer-checkpoint write are easier to
//! reason about when runs are serial.
//!
//! The library half (this file) ships the loop + the source
//! abstraction so tests can drive the dispatcher without spinning up
//! Synap. The Synap wiring + raw-envelope → [`Trigger`] parse lives
//! in the `cortex-consolidator` bin (§3.4).
//!
//! Shutdown (§3.3) is handled via [`ConsolidatorDaemon::run_forever`]
//! which selects between the trigger loop and an external shutdown
//! future — the daemon finishes the in-flight grain run, writes the
//! producer-checkpoint, then returns.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::time::sleep;

use crate::consolidator::consolidator_trait::{
    ConsolidationReport, Consolidator, ConsolidatorCtx, ConsolidatorError,
};
use crate::consolidator::grains::{CommunityGrain, DecisionTraceGrain, SessionGrain, TopicGrain};
use crate::consolidator::orchestrator::Trigger;
use crate::producer::{EnvelopeProducer, ProducerMetadataHandle};

/// Canonical Synap stream the daemon subscribes to. The supervisor
/// publishes a JSON envelope per fired trigger onto this stream;
/// the bin layer parses each event back into a [`Trigger`] before
/// handing it to the daemon.
pub const TRIGGER_STREAM: &str = "cortex.consolidator.triggers";

/// Default idle wait when the trigger source returns nothing.
/// Keeps the loop from spinning at 100% CPU when the supervisor
/// publishes triggers infrequently.
pub const DEFAULT_IDLE_POLL: Duration = Duration::from_millis(250);

/// One pending trigger pulled from a [`TriggerSource`]. Carries the
/// stream offset so the daemon can ack after the grain run lands.
#[derive(Debug, Clone)]
pub struct PendingTrigger {
    /// Source-side offset (Synap stream cursor). Acked after the
    /// grain run completes.
    pub offset: u64,
    /// Parsed trigger handed to the dispatcher.
    pub trigger: Trigger,
}

/// Source of triggers for the daemon loop. The production impl wraps
/// the Synap consumer; tests build an in-memory implementation that
/// returns a fixed sequence.
#[async_trait]
pub trait TriggerSource: Send + Sync {
    /// Return the next pending trigger, or `Ok(None)` when the queue
    /// is empty.
    async fn next_trigger(&self) -> anyhow::Result<Option<PendingTrigger>>;

    /// Ack a successfully-processed trigger by its source offset.
    /// The Synap impl advances its offset cursor; in-memory impls
    /// remove the corresponding entry.
    async fn ack(&self, offset: u64) -> anyhow::Result<()>;
}

/// One iteration outcome from [`ConsolidatorDaemon::run_once`].
#[derive(Debug)]
pub enum IterationOutcome {
    /// Source had no pending trigger.
    Idle,
    /// Dispatched a trigger and got a successful report.
    Dispatched {
        /// Stream offset that was acked.
        offset: u64,
        /// The report the grain returned.
        report: ConsolidationReport,
    },
    /// Dispatcher ran but the grain raised [`ConsolidatorError`].
    /// The offset is acked so the daemon does not infinitely retry a
    /// poisoned trigger; the error is surfaced for the caller's logs.
    Failed {
        /// Stream offset that was acked.
        offset: u64,
        /// Error the grain returned.
        error: ConsolidatorError,
    },
}

/// Phase14a §3.1 + §3.2 — daemon main loop + grain dispatcher.
///
/// One instance owns the three grains, the shared cost-ledger ctx,
/// and the trigger source. The dispatcher runs one trigger at a time
/// so the producer-checkpoint table and the cost ledger see runs
/// land in source order.
pub struct ConsolidatorDaemon {
    session: Arc<SessionGrain>,
    topic: Arc<TopicGrain>,
    decision: Arc<DecisionTraceGrain>,
    /// Phase27c §1 — optional: the community grain needs a live
    /// graph client, which not every deployment configures. A
    /// `CommunityDetected` trigger arriving with no grain wired
    /// is surfaced as a failed (acked) run, never a queue wedge.
    community: Option<Arc<CommunityGrain>>,
    source: Arc<dyn TriggerSource>,
    ctx: ConsolidatorCtx,
    metadata: ProducerMetadataHandle,
    idle_poll: Duration,
}

impl ConsolidatorDaemon {
    /// Build a daemon with the supplied grains, trigger source,
    /// ctx, and metadata handle. The ctx owns the shared cost
    /// ledger every grain records against; the metadata handle
    /// receives one `producer_checkpoints` row per successful
    /// grain run (phase14a §3.3 — checkpoint write is the
    /// daemon's responsibility, NOT the grain's).
    pub fn new(
        session: Arc<SessionGrain>,
        topic: Arc<TopicGrain>,
        decision: Arc<DecisionTraceGrain>,
        source: Arc<dyn TriggerSource>,
        ctx: ConsolidatorCtx,
        metadata: ProducerMetadataHandle,
    ) -> Self {
        Self {
            session,
            topic,
            decision,
            community: None,
            source,
            ctx,
            metadata,
            idle_poll: DEFAULT_IDLE_POLL,
        }
    }

    /// Builder shim — wire the phase27c community grain. Deployments
    /// without a graph client skip this; `CommunityDetected`
    /// triggers then fail-and-ack instead of wedging the queue.
    #[must_use]
    pub fn with_community(mut self, community: Arc<CommunityGrain>) -> Self {
        self.community = Some(community);
        self
    }

    /// Builder shim — override the idle poll interval.
    #[must_use]
    pub fn with_idle_poll(mut self, dt: Duration) -> Self {
        self.idle_poll = dt;
        self
    }

    /// Borrow the shared ctx (the health endpoint reads the ledger
    /// through this handle).
    pub fn ctx(&self) -> &ConsolidatorCtx {
        &self.ctx
    }

    /// Pull one trigger and dispatch it. Returns [`IterationOutcome`]
    /// for the supervisor / tests to observe. The grain dispatch
    /// always acks the offset — both success and failure — so a
    /// poisoned trigger does not block the queue forever.
    pub async fn run_once(&self) -> anyhow::Result<IterationOutcome> {
        let Some(pending) = self.source.next_trigger().await? else {
            return Ok(IterationOutcome::Idle);
        };

        // Per-trigger ctx anchored on the current wall clock but
        // sharing the daemon's cost ledger so every run lands in
        // one ledger.
        let run_ctx = ConsolidatorCtx::with_ledger(Utc::now(), self.ctx.cost.clone());

        let (result, producer_name, scope) = match &pending.trigger {
            Trigger::SessionEnd { session_id } => (
                self.session.on_trigger(&pending.trigger, &run_ctx).await,
                self.session.name(),
                session_id.clone(),
            ),
            Trigger::NightlyTopic { repo } => (
                self.topic.on_trigger(&pending.trigger, &run_ctx).await,
                self.topic.name(),
                format!("topic:{repo}"),
            ),
            Trigger::DecisionLanded { decision_id, .. } => (
                self.decision.on_trigger(&pending.trigger, &run_ctx).await,
                self.decision.name(),
                format!("decision:{decision_id}"),
            ),
            Trigger::CommunityDetected { repo } => match &self.community {
                Some(community) => (
                    community.on_trigger(&pending.trigger, &run_ctx).await,
                    community.name(),
                    format!("community:{repo}"),
                ),
                None => (
                    Err(ConsolidatorError::Other(
                        "community grain not configured (daemon built without a graph client)"
                            .into(),
                    )),
                    "consolidator.community",
                    format!("community:{repo}"),
                ),
            },
        };

        // Write the producer-checkpoint row BEFORE acking the
        // trigger source — a successful checkpoint write is the
        // proof of completion the supervisor uses on resume. The
        // failure path skips the checkpoint write so the supervisor
        // does not see a poisoned run as completed.
        if let Ok(report) = &result {
            let last_event_id = format!(
                "{}::{}::{}",
                producer_name,
                scope,
                report.finished_at.to_rfc3339()
            );
            let store = self.metadata.lock().await;
            store.record_producer_checkpoint(
                producer_name,
                &scope,
                &last_event_id,
                run_ctx.now,
                Utc::now(),
            )?;
        }

        self.source.ack(pending.offset).await?;

        Ok(match result {
            Ok(report) => IterationOutcome::Dispatched {
                offset: pending.offset,
                report,
            },
            Err(error) => IterationOutcome::Failed {
                offset: pending.offset,
                error,
            },
        })
    }

    /// Run the daemon until `shutdown` completes. Each iteration
    /// either dispatches a trigger or idles for [`Self::idle_poll`]
    /// when the source is empty. The shutdown future is awaited
    /// AFTER the in-flight iteration finishes so the producer
    /// checkpoint write always completes cleanly (§3.3).
    pub async fn run_forever(
        &self,
        shutdown: impl std::future::Future<Output = ()> + Send,
    ) -> anyhow::Result<DaemonRunReport> {
        let mut report = DaemonRunReport::default();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => break,
                outcome = self.run_once() => {
                    match outcome? {
                        IterationOutcome::Idle => {
                            report.idle_polls += 1;
                            tokio::select! {
                                biased;
                                _ = &mut shutdown => break,
                                _ = sleep(self.idle_poll) => {}
                            }
                        }
                        IterationOutcome::Dispatched { .. } => {
                            report.dispatched += 1;
                        }
                        IterationOutcome::Failed { .. } => {
                            report.failed += 1;
                        }
                    }
                }
            }
        }
        Ok(report)
    }
}

/// Per-run summary the supervisor / tests observe after
/// [`ConsolidatorDaemon::run_forever`] returns. Counts are monotonic.
#[derive(Debug, Default, Clone)]
pub struct DaemonRunReport {
    /// Triggers dispatched without error.
    pub dispatched: u64,
    /// Triggers that hit a grain error (still acked).
    pub failed: u64,
    /// Idle polls — the source returned nothing.
    pub idle_polls: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::consolidator_trait::TriggerLabel;
    use crate::consolidator::cost_telemetry::CostLedger;
    use crate::consolidator::grains::decision_trace::DecisionTraceFetcher;
    use crate::consolidator::grains::session::SessionInputFetcher;
    use crate::consolidator::grains::topic::TopicClusterFetcher;
    use crate::consolidator::orchestrator::Orchestrator;
    use crate::consolidator::producer::decision_trace::DecisionTraceInput;
    use crate::consolidator::producer::session::SessionInput;
    use crate::consolidator::producer::topic::{ClusterSession, TopicCluster, MIN_CLUSTER_SIZE};
    use crate::consolidator::source::SourceError;
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
    };
    use cortex_core::events::{ConsolidationGrain, Context, Envelope, Kind, Stream};
    use cortex_storage::MetadataStore;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use tokio::sync::{Mutex as TokioMutex, Notify};

    fn ctx_field() -> Context {
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

    fn turn(idx: u8, occurred_at: &str) -> Envelope {
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
            context: ctx_field(),
            payload: serde_json::json!({
                "user_message": format!("user {idx} investigating ef_search recall regression"),
                "assistant_message": format!("reply {idx} — raised ef_search and reran"),
                "outcome": "success",
            }),
            redactions: vec![],
            content_hash: "sha256:00".to_string()
                + "00000000000000000000000000000000000000000000000000000000000000",
            parent_event_id: None,
            class_level: None,
            class_compartments: None,
        }
    }

    fn decision_envelope() -> Envelope {
        Envelope {
            event_id: "01HXDEC1".into(),
            schema_version: "1".into(),
            occurred_at: "2026-04-21T09:30:00Z".into(),
            ingested_at: None,
            session_id: "01HXSESS00000000000000000A".into(),
            stream: Stream::Live,
            tool: "cortex-cli".into(),
            model: None,
            kind: Kind::Decision,
            context: ctx_field(),
            payload: serde_json::json!({
                "decision_id": "DEC1",
                "title": "Adopt HNSW",
                "status": "accepted",
                "rationale": "Hit recall@10 0.92 at 2M vectors",
            }),
            redactions: vec![],
            content_hash: "sha256:00".to_string()
                + "00000000000000000000000000000000000000000000000000000000000000",
            parent_event_id: None,
            class_level: None,
            class_compartments: None,
        }
    }

    const SUMMARY: &str = r#"{
        "title": "ef_search tuning",
        "summary_markdown": "The session walked the recall@10 benchmark for HNSW with ef_search candidates {64,96,128,160}. Recall held above 0.92 from 128 upward while p99 stayed under 12ms across the 2M-vector index. The decision landed at 128, the override sits in relevance.toml, and the rerun confirmed metrics. The session ended with the decision accepted and zero errors.",
        "takeaways": ["ef_search=128", "p99 under 12ms"]
    }"#;

    struct CannedSummariser {
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
            let text = if req
                .prompt
                .contains("relevance judge for the Cortex consolidator")
            {
                "{\"relevant\": true, \"reason\": \"substantive\"}".to_string()
            } else {
                SUMMARY.to_string()
            };
            Ok(SummariserResult {
                text,
                cost_cents: self.cost,
                kind: self.kind,
                input_tokens: 100,
                output_tokens: 50,
            })
        }
    }

    struct CannedSessionFetcher;
    #[async_trait]
    impl SessionInputFetcher for CannedSessionFetcher {
        async fn fetch(&self, sid: &str) -> Result<SessionInput, SourceError> {
            Ok(SessionInput {
                session_id: sid.to_string(),
                repo: Some("cortex".into()),
                envelopes: vec![
                    turn(1, "2026-04-20T10:00:00Z"),
                    turn(2, "2026-04-20T10:01:00Z"),
                ],
            })
        }
    }

    struct CannedTopicFetcher;
    #[async_trait]
    impl TopicClusterFetcher for CannedTopicFetcher {
        async fn fetch(
            &self,
            repo: &str,
            _now: chrono::DateTime<chrono::Utc>,
        ) -> Result<Vec<TopicCluster>, SourceError> {
            let mut outcome = BTreeMap::new();
            outcome.insert("success".to_string(), 1);
            Ok(vec![TopicCluster {
                label: "hnsw".into(),
                repo: repo.to_string(),
                sessions: (0..MIN_CLUSTER_SIZE)
                    .map(|i| ClusterSession {
                        session_id: format!("01HXSESS00000000000000000{i}"),
                        start_ms: 1_600_000_000_000 + i as i64 * 1_000,
                        end_ms: 1_600_000_000_000 + (i as i64 + 1) * 1_000,
                        outcome_distribution: outcome.clone(),
                        one_line_digest: format!("digest {i}"),
                    })
                    .collect(),
            }])
        }
    }

    struct CannedDecisionFetcher;
    #[async_trait]
    impl DecisionTraceFetcher for CannedDecisionFetcher {
        async fn fetch(&self, _id: &str) -> Result<DecisionTraceInput, SourceError> {
            Ok(DecisionTraceInput {
                decision: decision_envelope(),
                chain: vec![],
                repo: Some("cortex".into()),
            })
        }
    }

    /// In-memory trigger source with a FIFO queue + acked-offset set.
    struct InMemoryTriggerQueue {
        pending: Mutex<Vec<PendingTrigger>>,
        acked: Mutex<Vec<u64>>,
    }

    impl InMemoryTriggerQueue {
        fn with(triggers: Vec<Trigger>) -> Self {
            let pending = triggers
                .into_iter()
                .enumerate()
                .map(|(i, t)| PendingTrigger {
                    offset: i as u64,
                    trigger: t,
                })
                .collect();
            Self {
                pending: Mutex::new(pending),
                acked: Mutex::new(Vec::new()),
            }
        }

        fn acked(&self) -> Vec<u64> {
            self.acked.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TriggerSource for InMemoryTriggerQueue {
        async fn next_trigger(&self) -> anyhow::Result<Option<PendingTrigger>> {
            let mut p = self.pending.lock().unwrap();
            if p.is_empty() {
                Ok(None)
            } else {
                Ok(Some(p.remove(0)))
            }
        }

        async fn ack(&self, offset: u64) -> anyhow::Result<()> {
            self.acked.lock().unwrap().push(offset);
            Ok(())
        }
    }

    fn build_daemon(
        source: Arc<dyn TriggerSource>,
    ) -> (ConsolidatorDaemon, ProducerMetadataHandle) {
        let haiku: Arc<dyn Summariser> = Arc::new(CannedSummariser {
            kind: SummariserKind::Haiku45,
            cost: 80,
        });
        let opus: Arc<dyn Summariser> = Arc::new(CannedSummariser {
            kind: SummariserKind::Opus47,
            cost: 3_200,
        });
        let session = Arc::new(SessionGrain::new(
            Arc::new(Orchestrator::new(haiku.clone(), opus.clone())),
            Arc::new(CannedSessionFetcher),
        ));
        let topic = Arc::new(TopicGrain::new(
            Arc::new(Orchestrator::new(haiku.clone(), opus.clone())),
            Arc::new(CannedTopicFetcher),
        ));
        let decision = Arc::new(DecisionTraceGrain::new(
            Arc::new(Orchestrator::new(haiku, opus)),
            Arc::new(CannedDecisionFetcher),
        ));
        let ctx = ConsolidatorCtx::with_ledger(
            Utc::now(),
            Arc::new(std::sync::Mutex::new(CostLedger::default())),
        );
        let metadata: ProducerMetadataHandle =
            Arc::new(TokioMutex::new(MetadataStore::open_in_memory().unwrap()));
        let daemon =
            ConsolidatorDaemon::new(session, topic, decision, source, ctx, metadata.clone())
                .with_idle_poll(Duration::from_millis(1));
        (daemon, metadata)
    }

    #[tokio::test]
    async fn run_once_empty_queue_returns_idle() {
        let source = Arc::new(InMemoryTriggerQueue::with(Vec::new()));
        let (daemon, _meta) = build_daemon(source);
        let outcome = daemon.run_once().await.unwrap();
        assert!(matches!(outcome, IterationOutcome::Idle));
    }

    #[tokio::test]
    async fn run_once_session_end_dispatches_to_session_grain_and_acks() {
        let source = Arc::new(InMemoryTriggerQueue::with(vec![Trigger::SessionEnd {
            session_id: "01HXSESS00000000000000000A".into(),
        }]));
        let (daemon, meta) = build_daemon(source.clone());
        let outcome = daemon.run_once().await.unwrap();
        match outcome {
            IterationOutcome::Dispatched { offset, report } => {
                assert_eq!(offset, 0);
                assert_eq!(report.grain, ConsolidationGrain::Session);
                assert_eq!(
                    report.trigger,
                    TriggerLabel::SessionEnd {
                        session_id: "01HXSESS00000000000000000A".into()
                    }
                );
                assert_eq!(report.cost_cents, 80);
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
        assert_eq!(source.acked(), vec![0]);
        let rows = meta
            .lock()
            .await
            .list_producer_checkpoints_for("consolidator.session", 50)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scope, "01HXSESS00000000000000000A");
    }

    #[tokio::test]
    async fn run_once_nightly_topic_dispatches_to_topic_grain() {
        let source = Arc::new(InMemoryTriggerQueue::with(vec![Trigger::NightlyTopic {
            repo: "cortex".into(),
        }]));
        let (daemon, meta) = build_daemon(source.clone());
        let outcome = daemon.run_once().await.unwrap();
        match outcome {
            IterationOutcome::Dispatched { report, .. } => {
                assert_eq!(report.grain, ConsolidationGrain::Topic);
                assert_eq!(report.envelopes_emitted, 1);
                assert_eq!(report.summariser, SummariserKind::Haiku45);
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
        assert_eq!(source.acked(), vec![0]);
        let rows = meta
            .lock()
            .await
            .list_producer_checkpoints_for("consolidator.topic", 50)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scope, "topic:cortex");
    }

    #[tokio::test]
    async fn run_once_decision_landed_dispatches_to_decision_grain() {
        let source = Arc::new(InMemoryTriggerQueue::with(vec![Trigger::DecisionLanded {
            decision_id: "DEC1".into(),
            force_deep: false,
        }]));
        let (daemon, meta) = build_daemon(source.clone());
        let outcome = daemon.run_once().await.unwrap();
        match outcome {
            IterationOutcome::Dispatched { report, .. } => {
                assert_eq!(report.grain, ConsolidationGrain::DecisionTrace);
                assert_eq!(report.summariser, SummariserKind::Opus47);
                assert_eq!(report.cost_cents, 3_200);
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
        assert_eq!(source.acked(), vec![0]);
        let rows = meta
            .lock()
            .await
            .list_producer_checkpoints_for("consolidator.decision_trace", 50)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scope, "decision:DEC1");
    }

    #[tokio::test]
    async fn run_forever_drains_queue_in_order_then_idles_until_shutdown() {
        let triggers = vec![
            Trigger::SessionEnd {
                session_id: "01HXSESS00000000000000000A".into(),
            },
            Trigger::NightlyTopic {
                repo: "cortex".into(),
            },
            Trigger::DecisionLanded {
                decision_id: "DEC1".into(),
                force_deep: false,
            },
        ];
        let source = Arc::new(InMemoryTriggerQueue::with(triggers));
        let (daemon, meta) = build_daemon(source.clone());

        let shutdown = Arc::new(Notify::new());
        let signal = shutdown.clone();
        // Fire shutdown after a short delay so the loop drains the
        // queue, idles once, then exits cleanly.
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            signal.notify_one();
        });
        let report = daemon
            .run_forever(async move { shutdown.notified().await })
            .await
            .unwrap();
        assert_eq!(report.dispatched, 3);
        assert_eq!(report.failed, 0);
        assert!(report.idle_polls >= 1, "loop must idle at least once");
        assert_eq!(source.acked(), vec![0, 1, 2]);

        // Cost ledger reflects all three runs.
        {
            let ledger = daemon.ctx().cost.lock().unwrap();
            assert_eq!(ledger.per_grain.len(), 3);
            assert_eq!(ledger.total_cents, 80 + 80 + 3_200);
        }

        // One producer-checkpoint row per grain.
        let meta = meta.lock().await;
        assert_eq!(
            meta.list_producer_checkpoints_for("consolidator.session", 50)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            meta.list_producer_checkpoints_for("consolidator.topic", 50)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            meta.list_producer_checkpoints_for("consolidator.decision_trace", 50)
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn run_once_acks_offset_even_when_grain_returns_error() {
        // A trigger whose fetcher cannot resolve the session
        // surfaces as ConsolidatorError::Other; the daemon still
        // acks so the queue does not block on a poisoned id.
        struct EmptyFetcher;
        #[async_trait]
        impl SessionInputFetcher for EmptyFetcher {
            async fn fetch(&self, _sid: &str) -> Result<SessionInput, SourceError> {
                Err(SourceError::EmptyResult)
            }
        }
        let haiku: Arc<dyn Summariser> = Arc::new(CannedSummariser {
            kind: SummariserKind::Haiku45,
            cost: 80,
        });
        let opus: Arc<dyn Summariser> = Arc::new(CannedSummariser {
            kind: SummariserKind::Opus47,
            cost: 3_200,
        });
        let session = Arc::new(SessionGrain::new(
            Arc::new(Orchestrator::new(haiku.clone(), opus.clone())),
            Arc::new(EmptyFetcher),
        ));
        let topic = Arc::new(TopicGrain::new(
            Arc::new(Orchestrator::new(haiku.clone(), opus.clone())),
            Arc::new(CannedTopicFetcher),
        ));
        let decision = Arc::new(DecisionTraceGrain::new(
            Arc::new(Orchestrator::new(haiku, opus)),
            Arc::new(CannedDecisionFetcher),
        ));
        let source = Arc::new(InMemoryTriggerQueue::with(vec![Trigger::SessionEnd {
            session_id: "01MISSING".into(),
        }]));
        let ctx = ConsolidatorCtx::with_ledger(
            Utc::now(),
            Arc::new(std::sync::Mutex::new(CostLedger::default())),
        );
        let metadata: ProducerMetadataHandle =
            Arc::new(TokioMutex::new(MetadataStore::open_in_memory().unwrap()));
        let daemon = ConsolidatorDaemon::new(
            session,
            topic,
            decision,
            source.clone(),
            ctx,
            metadata.clone(),
        );
        let outcome = daemon.run_once().await.unwrap();
        match outcome {
            IterationOutcome::Failed { offset, error } => {
                assert_eq!(offset, 0);
                assert!(matches!(error, ConsolidatorError::Other(_)));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(source.acked(), vec![0]);
        // Failure path SHALL NOT write a producer checkpoint —
        // otherwise the supervisor would treat the poisoned run as
        // completed on resume.
        let rows = metadata
            .lock()
            .await
            .list_producer_checkpoints_for("consolidator.session", 50)
            .unwrap();
        assert!(rows.is_empty(), "failure path must skip checkpoint write");
    }
}
