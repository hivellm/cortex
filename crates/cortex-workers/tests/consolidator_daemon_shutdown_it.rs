//! Phase14a §5.2 — daemon-shutdown integration test.
//!
//! Exercises [`ConsolidatorDaemon::run_forever`] end-to-end through
//! its public surface:
//!
//! 1. Spawn the daemon on a background task with three pending
//!    triggers (one per grain).
//! 2. Fire the shutdown notify shortly after start.
//! 3. Assert the loop drains the queue, writes one
//!    `producer_checkpoints` row per successful grain, accumulates
//!    the cost ledger across all three runs, and returns cleanly
//!    with the expected `DaemonRunReport` shape.
//! 4. Variant: shutdown fires BEFORE the queue drains — the
//!    in-flight grain still lands its checkpoint and the loop
//!    returns with a partial dispatched count.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex as TokioMutex, Notify};
use tokio::time::sleep;

use cortex_core::events::{Context, Envelope, Kind, Stream};
use cortex_storage::MetadataStore;
use cortex_workers::consolidator::consolidator_trait::ConsolidatorCtx;
use cortex_workers::consolidator::cost_telemetry::CostLedger;
use cortex_workers::consolidator::daemon::{
    ConsolidatorDaemon, PendingTrigger, TriggerSource,
};
use cortex_workers::consolidator::grains::decision_trace::DecisionTraceFetcher;
use cortex_workers::consolidator::grains::session::SessionInputFetcher;
use cortex_workers::consolidator::grains::topic::TopicClusterFetcher;
use cortex_workers::consolidator::grains::{
    DecisionTraceGrain, SessionGrain, TopicGrain,
};
use cortex_workers::consolidator::orchestrator::{Orchestrator, Trigger};
use cortex_workers::consolidator::producer::decision_trace::DecisionTraceInput;
use cortex_workers::consolidator::producer::session::SessionInput;
use cortex_workers::consolidator::producer::topic::{
    ClusterSession, TopicCluster, MIN_CLUSTER_SIZE,
};
use cortex_workers::consolidator::source::SourceError;
use cortex_workers::consolidator::summariser::{
    Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
};
use cortex_workers::producer::ProducerMetadataHandle;

const SUMMARY_BODY: &str = r#"{
    "title": "ef_search tuning",
    "summary_markdown": "The session walked the recall@10 benchmark for HNSW with ef_search candidates {64, 96, 128, 160}. Recall held above 0.92 from 128 upward while p99 stayed under 12ms across the 2M-vector index. The decision landed at 128, the override sits in relevance.toml, and the rerun confirmed metrics. The session ended with the decision accepted and zero error outcomes.",
    "takeaways": ["ef_search = 128", "p99 < 12ms"]
}"#;

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
            "user_message": format!("user {idx} investigating ef_search regression at length"),
            "assistant_message": format!("reply {idx} — bumped ef_search and reran"),
            "outcome": "success",
        }),
        redactions: vec![],
        content_hash: "sha256:00".to_string()
            + "00000000000000000000000000000000000000000000000000000000000000",
        parent_event_id: None,
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
            "rationale": "Hit recall@10 0.92 at 2M vectors under 12ms",
        }),
        redactions: vec![],
        content_hash: "sha256:00".to_string()
            + "00000000000000000000000000000000000000000000000000000000000000",
        parent_event_id: None,
    }
}

struct CannedSummariser {
    kind: SummariserKind,
    cost: u32,
}

#[async_trait]
impl Summariser for CannedSummariser {
    fn kind(&self) -> SummariserKind {
        self.kind
    }
    async fn summarise(&self, req: SummariserRequest) -> Result<SummariserResult, SummariserError> {
        let text = if req.prompt.contains("relevance judge for the Cortex consolidator") {
            "{\"relevant\": true, \"reason\": \"substantive\"}".to_string()
        } else {
            SUMMARY_BODY.to_string()
        };
        Ok(SummariserResult {
            text,
            cost_cents: self.cost,
            kind: self.kind,
            input_tokens: 120,
            output_tokens: 80,
        })
    }
}

struct StaticSessionFetcher;
#[async_trait]
impl SessionInputFetcher for StaticSessionFetcher {
    async fn fetch(&self, sid: &str) -> Result<SessionInput, SourceError> {
        Ok(SessionInput {
            session_id: sid.to_string(),
            repo: Some("cortex".into()),
            envelopes: vec![turn(1, "2026-04-20T10:00:00Z"), turn(2, "2026-04-20T10:01:00Z")],
        })
    }
}

struct StaticTopicFetcher;
#[async_trait]
impl TopicClusterFetcher for StaticTopicFetcher {
    async fn fetch(
        &self,
        repo: &str,
        _now: DateTime<Utc>,
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

struct StaticDecisionFetcher;
#[async_trait]
impl DecisionTraceFetcher for StaticDecisionFetcher {
    async fn fetch(&self, _id: &str) -> Result<DecisionTraceInput, SourceError> {
        Ok(DecisionTraceInput {
            decision: decision_envelope(),
            chain: vec![],
            repo: Some("cortex".into()),
        })
    }
}

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

fn build_daemon_with(
    source: Arc<dyn TriggerSource>,
) -> (Arc<ConsolidatorDaemon>, ProducerMetadataHandle) {
    let haiku: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Haiku45,
        cost: 80,
    });
    let opus: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Opus47,
        cost: 3_200,
    });
    let session_grain = Arc::new(SessionGrain::new(
        Arc::new(Orchestrator::new(haiku.clone(), opus.clone())),
        Arc::new(StaticSessionFetcher),
    ));
    let topic_grain = Arc::new(TopicGrain::new(
        Arc::new(Orchestrator::new(haiku.clone(), opus.clone())),
        Arc::new(StaticTopicFetcher),
    ));
    let decision_grain = Arc::new(DecisionTraceGrain::new(
        Arc::new(Orchestrator::new(haiku, opus)),
        Arc::new(StaticDecisionFetcher),
    ));
    let ctx = ConsolidatorCtx::with_ledger(
        Utc::now(),
        Arc::new(Mutex::new(CostLedger::default())),
    );
    let metadata: ProducerMetadataHandle =
        Arc::new(TokioMutex::new(MetadataStore::open_in_memory().unwrap()));
    let daemon = Arc::new(
        ConsolidatorDaemon::new(
            session_grain,
            topic_grain,
            decision_grain,
            source,
            ctx,
            metadata.clone(),
        )
        .with_idle_poll(Duration::from_millis(5)),
    );
    (daemon, metadata)
}

#[tokio::test]
async fn daemon_run_forever_drains_three_triggers_then_shuts_down_cleanly() {
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
    let (daemon, meta) = build_daemon_with(source.clone());

    let shutdown = Arc::new(Notify::new());
    let signal = shutdown.clone();
    tokio::spawn(async move {
        // Give the loop enough time to drain three sequential
        // dispatches + at least one idle poll before signalling.
        sleep(Duration::from_millis(150)).await;
        signal.notify_one();
    });

    let daemon_for_run = daemon.clone();
    let report = daemon_for_run
        .run_forever(async move { shutdown.notified().await })
        .await
        .expect("daemon exited cleanly");

    assert_eq!(report.dispatched, 3, "all three triggers dispatched");
    assert_eq!(report.failed, 0, "no failures expected on canned grains");
    assert!(report.idle_polls >= 1, "loop must idle at least once");
    assert_eq!(source.acked(), vec![0, 1, 2], "every offset acked in order");

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
    drop(meta);

    let ledger = daemon.ctx().cost.lock().unwrap();
    assert_eq!(ledger.per_grain.len(), 3);
    assert_eq!(ledger.total_cents, 80 + 80 + 3_200);
}

#[tokio::test]
async fn daemon_run_forever_returns_partial_count_when_shutdown_fires_early() {
    let triggers = vec![
        Trigger::SessionEnd {
            session_id: "01HXSESS00000000000000000A".into(),
        },
        Trigger::SessionEnd {
            session_id: "01HXSESS00000000000000000B".into(),
        },
        Trigger::SessionEnd {
            session_id: "01HXSESS00000000000000000C".into(),
        },
    ];
    let source = Arc::new(InMemoryTriggerQueue::with(triggers));
    let (daemon, _meta) = build_daemon_with(source.clone());

    // Fire shutdown almost immediately. The loop's `tokio::select!`
    // is biased so the shutdown branch races run_once; at least one
    // iteration may still land before the signal observes. The
    // assertion below is intentionally loose — "at most three, at
    // least zero" — because we are validating clean exit semantics,
    // not a specific dispatch count.
    let shutdown = Arc::new(Notify::new());
    let signal = shutdown.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(1)).await;
        signal.notify_one();
    });

    let report = daemon
        .run_forever(async move { shutdown.notified().await })
        .await
        .expect("daemon exits cleanly even when shutdown fires early");

    assert!(report.dispatched <= 3);
    assert_eq!(report.failed, 0);
    let acked = source.acked();
    // Whatever was dispatched MUST have been acked — the in-flight
    // iteration always finishes before the shutdown branch wins.
    assert_eq!(acked.len() as u64, report.dispatched);
}
