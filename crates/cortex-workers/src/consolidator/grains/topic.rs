//! Phase14a §2.2 — [`TopicGrain`] consolidator.
//!
//! Nightly clustering of a repo's turn corpus into HDBSCAN clusters.
//! Dispatched by the daemon on every [`Trigger::NightlyTopic`]. The
//! grain delegates the parquet walk + HDBSCAN pass to a
//! [`TopicClusterFetcher`] so tests can drive the trigger path
//! without spinning up an archive. Each returned cluster is fed
//! through [`Orchestrator::run_topic`]; failed clusters are logged
//! and skipped so a single under-size / hallucinated payload does
//! not lose the rest of the nightly batch.
//!
//! Composition with [`EnvelopeProducer`] mirrors
//! [`super::session::SessionGrain`]: the daemon owns the per-trigger
//! checkpoint write, so the batch path returns a zero-row report.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use cortex_core::events::ConsolidationGrain;

use crate::consolidator::consolidator_trait::{
    ConsolidationReport, Consolidator, ConsolidatorCtx, ConsolidatorError, TriggerLabel,
};
use crate::consolidator::orchestrator::{Orchestrator, Trigger};
use crate::consolidator::producer::topic::TopicCluster;
use crate::consolidator::source::{LiveTopicSource, SourceError};
use crate::consolidator::summariser::SummariserKind;
use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

/// Stable producer name for the topic grain.
pub const TOPIC_GRAIN_PRODUCER_NAME: &str = "consolidator.topic";

/// Default look-back window for the nightly topic pass. Matches the
/// `cortex-consolidator nightly` bin's 7-day window so the daemon
/// and the bin converge on the same corpus.
pub const TOPIC_DEFAULT_WINDOW: Duration = Duration::days(7);

/// Async-trait wrapper around the live HDBSCAN pass so tests can
/// supply an in-memory cluster slice without touching the archive.
#[async_trait]
pub trait TopicClusterFetcher: Send + Sync {
    /// Cluster the repo's turn corpus at `now` and return the
    /// resulting [`TopicCluster`] list. Empty result is `Ok(vec![])`.
    async fn fetch(
        &self,
        repo: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<TopicCluster>, SourceError>;
}

/// Production fetcher backed by [`LiveTopicSource`]. The underlying
/// scan + HDBSCAN pass is sync; the wrapper hops onto a blocking
/// task so a multi-thousand-envelope walk does not stall the
/// daemon's tokio executor.
pub struct LiveTopicClusterFetcher {
    inner: LiveTopicSource,
    window: Duration,
}

impl LiveTopicClusterFetcher {
    /// Build a live fetcher with the default 7-day window.
    pub fn new(source: LiveTopicSource) -> Self {
        Self {
            inner: source,
            window: TOPIC_DEFAULT_WINDOW,
        }
    }

    /// Override the default look-back window.
    pub fn with_window(mut self, window: Duration) -> Self {
        self.window = window;
        self
    }
}

#[async_trait]
impl TopicClusterFetcher for LiveTopicClusterFetcher {
    async fn fetch(
        &self,
        repo: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<TopicCluster>, SourceError> {
        let source = self.inner.clone();
        let repo = repo.to_string();
        let now_ms = now.timestamp_millis();
        let since_ms = (now - self.window).timestamp_millis();
        tokio::task::spawn_blocking(move || source.fetch(&repo, since_ms, now_ms))
            .await
            .map_err(|e| SourceError::Storage(format!("topic fetch task: {e}")))?
    }
}

/// Per-grain consolidator dispatched by the daemon on every
/// [`Trigger::NightlyTopic`].
pub struct TopicGrain {
    orchestrator: Arc<Orchestrator>,
    fetcher: Arc<dyn TopicClusterFetcher>,
}

impl TopicGrain {
    /// Build a topic grain that runs through `orchestrator` and
    /// hydrates clusters via `fetcher`.
    pub fn new(orchestrator: Arc<Orchestrator>, fetcher: Arc<dyn TopicClusterFetcher>) -> Self {
        Self {
            orchestrator,
            fetcher,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for TopicGrain {
    fn name(&self) -> &'static str {
        TOPIC_GRAIN_PRODUCER_NAME
    }

    /// Trigger-driven grain — see [`super::session::SessionGrain`]
    /// docs. Returns a zero-row report; the daemon writes the
    /// per-trigger checkpoint.
    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        Ok(ProducerReport {
            producer_name: TOPIC_GRAIN_PRODUCER_NAME.to_string(),
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
        let row = store.latest_producer_checkpoint(TOPIC_GRAIN_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[async_trait]
impl Consolidator for TopicGrain {
    fn grain(&self) -> ConsolidationGrain {
        ConsolidationGrain::Topic
    }

    async fn on_trigger(
        &self,
        trigger: &Trigger,
        ctx: &ConsolidatorCtx,
    ) -> Result<ConsolidationReport, ConsolidatorError> {
        let repo = match trigger {
            Trigger::NightlyTopic { repo } => repo.as_str(),
            Trigger::SessionEnd { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "session_end",
                    expected: "nightly_topic",
                })
            }
            Trigger::DecisionLanded { .. } => {
                return Err(ConsolidatorError::TriggerMismatch {
                    got: "decision_landed",
                    expected: "nightly_topic",
                })
            }
        };

        let started = Instant::now();
        let clusters = self
            .fetcher
            .fetch(repo, ctx.now)
            .await
            .map_err(|e| ConsolidatorError::Other(format!("topic fetch: {e}")))?;

        let mut envelopes_emitted: u64 = 0;
        let mut cost_cents: u64 = 0;
        let mut source_event_count: u64 = 0;
        let mut summariser_seen: Option<SummariserKind> = None;

        for cluster in &clusters {
            match self.orchestrator.run_topic(cluster).await {
                Ok(produced) => {
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
                        cluster = %cluster.label,
                        "topic grain: cluster run failed",
                    );
                }
            }
        }

        Ok(ConsolidationReport {
            grain: ConsolidationGrain::Topic,
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
    use crate::consolidator::producer::topic::{ClusterSession, MIN_CLUSTER_SIZE};
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserRequest, SummariserResult,
    };
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    fn cluster_session(id: &str, start: i64, end: i64) -> ClusterSession {
        let mut outcome = BTreeMap::new();
        outcome.insert("success".to_string(), 1);
        ClusterSession {
            session_id: id.into(),
            start_ms: start,
            end_ms: end,
            outcome_distribution: outcome,
            one_line_digest: format!("digest for {id} — tuned HNSW ef_search"),
        }
    }

    fn ok_topic_summary() -> String {
        serde_json::to_string(&serde_json::json!({
            "title": "HNSW tuning",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["raise ef_search to 128"],
        }))
        .unwrap()
    }

    fn make_cluster(label: &str) -> TopicCluster {
        TopicCluster {
            label: label.into(),
            repo: "cortex".into(),
            sessions: (0..MIN_CLUSTER_SIZE)
                .map(|i| {
                    cluster_session(
                        &format!("01HXSESS{label}{i}"),
                        1_600_000_000_000 + i as i64 * 1_000,
                        1_600_000_000_000 + (i as i64 + 1) * 1_000,
                    )
                })
                .collect(),
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

    struct InMemoryClusterFetcher {
        clusters: Mutex<Vec<TopicCluster>>,
    }

    impl InMemoryClusterFetcher {
        fn with(clusters: Vec<TopicCluster>) -> Self {
            Self {
                clusters: Mutex::new(clusters),
            }
        }
    }

    #[async_trait]
    impl TopicClusterFetcher for InMemoryClusterFetcher {
        async fn fetch(
            &self,
            _repo: &str,
            _now: DateTime<Utc>,
        ) -> Result<Vec<TopicCluster>, SourceError> {
            Ok(self.clusters.lock().unwrap().clone())
        }
    }

    fn build_grain(clusters: Vec<TopicCluster>, cost: u32) -> TopicGrain {
        let haiku = Arc::new(CannedSummariser {
            text: ok_topic_summary(),
            kind: SummariserKind::Haiku45,
            cost,
        });
        let opus = Arc::new(CannedSummariser {
            text: ok_topic_summary(),
            kind: SummariserKind::Opus47,
            cost: 5_000,
        });
        let orchestrator = Arc::new(Orchestrator::new(haiku, opus));
        let fetcher: Arc<dyn TopicClusterFetcher> =
            Arc::new(InMemoryClusterFetcher::with(clusters));
        TopicGrain::new(orchestrator, fetcher)
    }

    #[test]
    fn topic_grain_reports_topic_grain_label() {
        let grain = build_grain(Vec::new(), 80);
        assert_eq!(grain.grain(), ConsolidationGrain::Topic);
        assert_eq!(grain.name(), "consolidator.topic");
    }

    #[tokio::test]
    async fn topic_grain_sums_cost_and_source_count_across_clusters() {
        let clusters = vec![make_cluster("alpha"), make_cluster("beta")];
        let grain = build_grain(clusters, 70);
        let trigger = Trigger::NightlyTopic {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");

        assert_eq!(report.grain, ConsolidationGrain::Topic);
        assert_eq!(
            report.trigger,
            TriggerLabel::NightlyTopic {
                repo: "cortex".into(),
            }
        );
        assert_eq!(report.envelopes_emitted, 2);
        assert_eq!(report.cost_cents, 140);
        assert_eq!(report.source_event_count, (MIN_CLUSTER_SIZE as u64) * 2);
        assert_eq!(report.finished_at, ts("2026-05-25T12:00:00Z"));
        assert_eq!(report.summariser, SummariserKind::Haiku45);
    }

    #[tokio::test]
    async fn topic_grain_skips_under_size_clusters_and_keeps_going() {
        let mut under = make_cluster("under");
        under.sessions.truncate(MIN_CLUSTER_SIZE - 1);
        let clusters = vec![under, make_cluster("ok")];
        let grain = build_grain(clusters, 80);
        let trigger = Trigger::NightlyTopic {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");
        assert_eq!(report.envelopes_emitted, 1);
        assert_eq!(report.cost_cents, 80);
        assert_eq!(report.source_event_count, MIN_CLUSTER_SIZE as u64);
    }

    #[tokio::test]
    async fn topic_grain_empty_corpus_emits_zero_envelopes() {
        let grain = build_grain(Vec::new(), 80);
        let trigger = Trigger::NightlyTopic {
            repo: "cortex".into(),
        };
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        let report = grain.on_trigger(&trigger, &ctx).await.expect("on_trigger");
        assert_eq!(report.envelopes_emitted, 0);
        assert_eq!(report.cost_cents, 0);
        assert_eq!(report.source_event_count, 0);
        assert_eq!(report.summariser, SummariserKind::Haiku45);
    }

    #[tokio::test]
    async fn topic_grain_rejects_mismatched_trigger() {
        let grain = build_grain(Vec::new(), 80);
        let ctx = ConsolidatorCtx::at(ts("2026-05-25T12:00:00Z"));
        for (bad, got) in [
            (
                Trigger::SessionEnd {
                    session_id: "sid".into(),
                },
                "session_end",
            ),
            (
                Trigger::DecisionLanded {
                    decision_id: "DEC".into(),
                    force_deep: false,
                },
                "decision_landed",
            ),
        ] {
            let err = grain
                .on_trigger(&bad, &ctx)
                .await
                .expect_err("must reject");
            match err {
                ConsolidatorError::TriggerMismatch { got: g, expected } => {
                    assert_eq!(g, got);
                    assert_eq!(expected, "nightly_topic");
                }
                other => panic!("wrong error: {other}"),
            }
        }
    }

    #[tokio::test]
    async fn topic_grain_produce_returns_zero_row_report() {
        use crate::producer::{ProducerCtx, ProducerMetadataHandle};
        use cortex_storage::MetadataStore;
        use tokio::sync::Mutex as TokioMutex;

        let store = MetadataStore::open_in_memory().unwrap();
        let handle: ProducerMetadataHandle = Arc::new(TokioMutex::new(store));
        let pctx = ProducerCtx::new(handle, "cortex.test").with_now(ts("2026-05-25T12:00:00Z"));
        let grain = build_grain(Vec::new(), 80);
        let report = grain.produce(&pctx).await.unwrap();
        assert_eq!(report.envelopes_emitted, 0);
        assert_eq!(report.batches_emitted, 0);
        assert_eq!(report.producer_name, "consolidator.topic");
    }
}
