//! Phase14a §2.5 — per-grain trigger → emit integration test.
//!
//! Drives all three [`cortex_workers::consolidator::grains`] grains
//! through their [`Consolidator::on_trigger`] entry against realistic
//! [`Envelope`] fixtures. Asserts:
//!
//! - Each trigger lands a [`ConsolidationReport`] whose `grain` /
//!   `trigger` / `summariser` agree with the dispatched trigger.
//! - The shared [`ConsolidatorCtx`] ledger accumulates one bucket
//!   per grain with the correct `cost_cents` + `input_tokens` +
//!   `output_tokens` + `models_used` contents.
//! - Triggers routed to the wrong grain raise
//!   [`ConsolidatorError::TriggerMismatch`] without polluting the
//!   ledger.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use cortex_core::events::{ConsolidationGrain, Context, Envelope, Kind, Stream};
use cortex_workers::consolidator::consolidator_trait::{
    Consolidator, ConsolidatorCtx, ConsolidatorError, TriggerLabel,
};
use cortex_workers::consolidator::cost_telemetry::CostLedger;
use cortex_workers::consolidator::grains::{DecisionTraceGrain, SessionGrain, TopicGrain};
use cortex_workers::consolidator::grains::decision_trace::DecisionTraceFetcher;
use cortex_workers::consolidator::grains::session::SessionInputFetcher;
use cortex_workers::consolidator::grains::topic::TopicClusterFetcher;
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

const SESSION_FIXTURE_ID: &str = "01HXSESS00000000000000000A";
const REPO_FIXTURE: &str = "cortex";
const DECISION_FIXTURE_ID: &str = "DEC-PHASE14A-1";
const FIXED_NOW: &str = "2026-05-25T12:00:00Z";

fn now() -> chrono::DateTime<chrono::Utc> {
    FIXED_NOW.parse().expect("rfc3339 fixture")
}

fn ctx_field() -> Context {
    Context {
        repo: Some(REPO_FIXTURE.into()),
        branch: None,
        commit: None,
        cwd: None,
        user: None,
        platform: "linux".into(),
        ide: None,
        extras: Default::default(),
    }
}

fn fixture_turn(idx: u8, occurred_at: &str) -> Envelope {
    Envelope {
        event_id: format!("01HXEVT{idx:019}"),
        schema_version: "1".into(),
        occurred_at: occurred_at.into(),
        ingested_at: None,
        session_id: SESSION_FIXTURE_ID.into(),
        stream: Stream::Live,
        tool: "claude-code".into(),
        model: Some("claude-haiku-4-5".into()),
        kind: Kind::Turn,
        context: ctx_field(),
        payload: serde_json::json!({
            "user_message": format!(
                "user prompt {idx} investigating ef_search recall regression"
            ),
            "assistant_message": format!(
                "assistant reply {idx} — raised ef_search to 128 and reran the benchmark"
            ),
            "outcome": "success",
        }),
        redactions: vec![],
        content_hash: "sha256:00".to_string()
            + "00000000000000000000000000000000000000000000000000000000000000",
        parent_event_id: None,
    }
}

fn fixture_decision() -> Envelope {
    Envelope {
        event_id: format!("01HXDEC{DECISION_FIXTURE_ID}"),
        schema_version: "1".into(),
        occurred_at: "2026-04-21T09:30:00Z".into(),
        ingested_at: None,
        session_id: SESSION_FIXTURE_ID.into(),
        stream: Stream::Live,
        tool: "cortex-cli".into(),
        model: None,
        kind: Kind::Decision,
        context: ctx_field(),
        payload: serde_json::json!({
            "decision_id": DECISION_FIXTURE_ID,
            "title": "Adopt HNSW ef_search = 128",
            "status": "accepted",
            "rationale": "Hit 0.92 recall@10 at 2M vectors under 12ms p99",
        }),
        redactions: vec![],
        content_hash: "sha256:00".to_string()
            + "00000000000000000000000000000000000000000000000000000000000000",
        parent_event_id: None,
    }
}

const SESSION_SUMMARY: &str = r#"{
    "title": "ef_search tuning landed at 128",
    "summary_markdown": "The session walked the recall@10 benchmark for HNSW with ef_search ∈ {64, 96, 128, 160}. Recall held above 0.92 from ef_search=128 upward while p99 latency stayed under 12ms across the 2M-vector index. The decision-bearing fragment chose 128, the configuration landed in `relevance.toml`, and the follow-up rerun confirmed the metrics. The session ended with the decision accepted and zero error outcomes recorded across the run.",
    "takeaways": [
        "ef_search=128 holds recall@10 >= 0.92 at 2M vectors",
        "p99 latency stays under 12ms at ef_search=128"
    ]
}"#;

const TOPIC_SUMMARY: &str = r#"{
    "title": "HNSW tuning across nightly sessions",
    "summary_markdown": "Three nightly sessions converged on the same HNSW retuning workflow: enumerate recall@10 across ef_search candidates, isolate the latency cliff, land the override in `relevance.toml`. Each session ended with a single accepted decision; no session surfaced an error outcome across the corpus. The clustering result is consistent enough to recommend pinning ef_search=128 as the project-wide default for HNSW indexes serving sub-3M vector corpora.",
    "takeaways": [
        "ef_search=128 is the cross-session default candidate",
        "relevance.toml is the canonical override surface"
    ]
}"#;

const DECISION_SUMMARY: &str = r#"{
    "title": "HNSW ef_search default accepted",
    "summary_markdown": "The decision trace walks back from the accepted ADR through the supporting benchmark runs. The chain spans the recall@10 baseline, the ef_search sweep, and the latency-budget check that gated the final value. The accepted status reflects the consensus from the three contributing sessions in the topic cluster above. No superseding decisions are queued; the trace converges cleanly on the 128 value with the documented justification.",
    "takeaways": [
        "Accepted on the strength of three converging session runs",
        "No superseding ADR queued"
    ]
}"#;

/// Canned summariser that returns the relevance-gate green light
/// then the supplied summary body.
struct CannedSummariser {
    body: String,
    kind: SummariserKind,
    cost: u32,
    input: u64,
    output: u64,
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
            self.body.clone()
        };
        Ok(SummariserResult {
            text,
            cost_cents: self.cost,
            kind: self.kind,
            input_tokens: self.input,
            output_tokens: self.output,
        })
    }
}

struct InMemorySessionFetcher(Mutex<SessionInput>);

#[async_trait]
impl SessionInputFetcher for InMemorySessionFetcher {
    async fn fetch(&self, _id: &str) -> Result<SessionInput, SourceError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

struct InMemoryTopicFetcher(Mutex<Vec<TopicCluster>>);

#[async_trait]
impl TopicClusterFetcher for InMemoryTopicFetcher {
    async fn fetch(
        &self,
        _repo: &str,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<TopicCluster>, SourceError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

struct InMemoryDecisionFetcher(Mutex<DecisionTraceInput>);

#[async_trait]
impl DecisionTraceFetcher for InMemoryDecisionFetcher {
    async fn fetch(&self, _id: &str) -> Result<DecisionTraceInput, SourceError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn fixture_session_input() -> SessionInput {
    SessionInput {
        session_id: SESSION_FIXTURE_ID.into(),
        repo: Some(REPO_FIXTURE.into()),
        envelopes: vec![
            fixture_turn(1, "2026-04-20T10:00:00Z"),
            fixture_turn(2, "2026-04-20T10:01:00Z"),
            fixture_turn(3, "2026-04-20T10:02:00Z"),
        ],
    }
}

fn fixture_topic_clusters() -> Vec<TopicCluster> {
    let mut outcome = BTreeMap::new();
    outcome.insert("success".to_string(), 1);
    let sessions: Vec<ClusterSession> = (0..MIN_CLUSTER_SIZE)
        .map(|i| ClusterSession {
            session_id: format!("01HXSESS00000000000000000{i}"),
            start_ms: 1_600_000_000_000 + i as i64 * 1_000,
            end_ms: 1_600_000_000_000 + (i as i64 + 1) * 1_000,
            outcome_distribution: outcome.clone(),
            one_line_digest: format!("digest {i} — HNSW ef_search tuning"),
        })
        .collect();
    vec![TopicCluster {
        label: "hnsw-tuning".into(),
        repo: REPO_FIXTURE.into(),
        sessions,
    }]
}

fn fixture_decision_input() -> DecisionTraceInput {
    DecisionTraceInput {
        decision: fixture_decision(),
        chain: vec![],
        repo: Some(REPO_FIXTURE.into()),
    }
}

fn build_grains(
    shared_ledger: Arc<Mutex<CostLedger>>,
) -> (SessionGrain, TopicGrain, DecisionTraceGrain) {
    let haiku: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        body: SESSION_SUMMARY.to_string(),
        kind: SummariserKind::Haiku45,
        cost: 80,
        input: 1_200,
        output: 320,
    });
    let opus: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        body: DECISION_SUMMARY.to_string(),
        kind: SummariserKind::Opus47,
        cost: 3_200,
        input: 2_800,
        output: 960,
    });
    let topic_haiku: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        body: TOPIC_SUMMARY.to_string(),
        kind: SummariserKind::Haiku45,
        cost: 70,
        input: 1_000,
        output: 240,
    });
    // Two orchestrators: one for session/decision (sharing Haiku+Opus
    // because the producer pipelines key on the model id baked into
    // the summary fixture) and one for topic (its summary body is
    // different). Both share the ledger so it accumulates across
    // grains.
    let _ = &shared_ledger; // ledger lives in ctx, orchestrator's internal copy is unrelated
    let orch_for_session = Arc::new(Orchestrator::new(haiku.clone(), opus.clone()));
    let orch_for_topic = Arc::new(Orchestrator::new(topic_haiku, opus.clone()));
    let orch_for_decision = Arc::new(Orchestrator::new(haiku, opus));
    let session_grain = SessionGrain::new(
        orch_for_session,
        Arc::new(InMemorySessionFetcher(Mutex::new(fixture_session_input()))),
    );
    let topic_grain = TopicGrain::new(
        orch_for_topic,
        Arc::new(InMemoryTopicFetcher(Mutex::new(fixture_topic_clusters()))),
    );
    let decision_grain = DecisionTraceGrain::new(
        orch_for_decision,
        Arc::new(InMemoryDecisionFetcher(Mutex::new(fixture_decision_input()))),
    );
    (session_grain, topic_grain, decision_grain)
}

#[tokio::test]
async fn session_grain_trigger_to_emit_against_fixture() {
    let ledger = Arc::new(Mutex::new(CostLedger::default()));
    let (session, _topic, _decision) = build_grains(Arc::clone(&ledger));
    let ctx = ConsolidatorCtx::with_ledger(now(), Arc::clone(&ledger));

    let report = session
        .on_trigger(
            &Trigger::SessionEnd {
                session_id: SESSION_FIXTURE_ID.into(),
            },
            &ctx,
        )
        .await
        .expect("session on_trigger");

    assert_eq!(report.grain, ConsolidationGrain::Session);
    assert_eq!(
        report.trigger,
        TriggerLabel::SessionEnd {
            session_id: SESSION_FIXTURE_ID.into()
        }
    );
    assert_eq!(report.envelopes_emitted, 1);
    assert_eq!(report.summariser, SummariserKind::Haiku45);
    assert_eq!(report.cost_cents, 80);
    assert_eq!(report.source_event_count, 3);

    let l = ledger.lock().unwrap();
    let bucket = l.per_grain.get("session").expect("session ledger bucket");
    assert_eq!(bucket.consolidations, 1);
    assert_eq!(bucket.cost_cents, 80);
    assert_eq!(bucket.input_tokens, 1_200);
    assert_eq!(bucket.output_tokens, 320);
    assert!(bucket.models_used.contains("claude-haiku-4-5"));
}

#[tokio::test]
async fn topic_grain_trigger_to_emit_against_fixture() {
    let ledger = Arc::new(Mutex::new(CostLedger::default()));
    let (_session, topic, _decision) = build_grains(Arc::clone(&ledger));
    let ctx = ConsolidatorCtx::with_ledger(now(), Arc::clone(&ledger));

    let report = topic
        .on_trigger(
            &Trigger::NightlyTopic {
                repo: REPO_FIXTURE.into(),
            },
            &ctx,
        )
        .await
        .expect("topic on_trigger");

    assert_eq!(report.grain, ConsolidationGrain::Topic);
    assert_eq!(
        report.trigger,
        TriggerLabel::NightlyTopic {
            repo: REPO_FIXTURE.into()
        }
    );
    assert_eq!(report.envelopes_emitted, 1);
    assert_eq!(report.summariser, SummariserKind::Haiku45);
    assert_eq!(report.cost_cents, 70);

    let l = ledger.lock().unwrap();
    let bucket = l.per_grain.get("topic").expect("topic ledger bucket");
    assert_eq!(bucket.consolidations, 1);
    assert_eq!(bucket.cost_cents, 70);
    assert_eq!(bucket.input_tokens, 1_000);
    assert_eq!(bucket.output_tokens, 240);
    assert!(bucket.models_used.contains("claude-haiku-4-5"));
}

#[tokio::test]
async fn decision_trace_grain_trigger_to_emit_against_fixture() {
    let ledger = Arc::new(Mutex::new(CostLedger::default()));
    let (_session, _topic, decision) = build_grains(Arc::clone(&ledger));
    let ctx = ConsolidatorCtx::with_ledger(now(), Arc::clone(&ledger));

    let report = decision
        .on_trigger(
            &Trigger::DecisionLanded {
                decision_id: DECISION_FIXTURE_ID.into(),
                force_deep: false,
            },
            &ctx,
        )
        .await
        .expect("decision on_trigger");

    assert_eq!(report.grain, ConsolidationGrain::DecisionTrace);
    assert_eq!(
        report.trigger,
        TriggerLabel::DecisionLanded {
            decision_id: DECISION_FIXTURE_ID.into()
        }
    );
    assert_eq!(report.envelopes_emitted, 1);
    assert_eq!(report.summariser, SummariserKind::Opus47);
    assert_eq!(report.cost_cents, 3_200);

    let l = ledger.lock().unwrap();
    let bucket = l
        .per_grain
        .get("decision_trace")
        .expect("decision_trace ledger bucket");
    assert_eq!(bucket.consolidations, 1);
    assert_eq!(bucket.cost_cents, 3_200);
    assert_eq!(bucket.input_tokens, 2_800);
    assert_eq!(bucket.output_tokens, 960);
    assert!(bucket.models_used.contains("claude-opus-4-7"));
}

#[tokio::test]
async fn all_three_grains_share_a_single_ctx_ledger() {
    let ledger = Arc::new(Mutex::new(CostLedger::default()));
    let (session, topic, decision) = build_grains(Arc::clone(&ledger));
    let ctx = ConsolidatorCtx::with_ledger(now(), Arc::clone(&ledger));

    session
        .on_trigger(
            &Trigger::SessionEnd {
                session_id: SESSION_FIXTURE_ID.into(),
            },
            &ctx,
        )
        .await
        .unwrap();
    topic
        .on_trigger(
            &Trigger::NightlyTopic {
                repo: REPO_FIXTURE.into(),
            },
            &ctx,
        )
        .await
        .unwrap();
    decision
        .on_trigger(
            &Trigger::DecisionLanded {
                decision_id: DECISION_FIXTURE_ID.into(),
                force_deep: false,
            },
            &ctx,
        )
        .await
        .unwrap();

    let l = ledger.lock().unwrap();
    assert_eq!(l.per_grain.len(), 3, "one bucket per grain");
    assert_eq!(l.total_cents, 80 + 70 + 3_200);
    assert_eq!(l.per_grain["session"].input_tokens, 1_200);
    assert_eq!(l.per_grain["topic"].input_tokens, 1_000);
    assert_eq!(l.per_grain["decision_trace"].input_tokens, 2_800);
}

#[tokio::test]
async fn mismatched_triggers_do_not_pollute_the_ctx_ledger() {
    let ledger = Arc::new(Mutex::new(CostLedger::default()));
    let (session, topic, decision) = build_grains(Arc::clone(&ledger));
    let ctx = ConsolidatorCtx::with_ledger(now(), Arc::clone(&ledger));

    let err = session
        .on_trigger(
            &Trigger::NightlyTopic {
                repo: REPO_FIXTURE.into(),
            },
            &ctx,
        )
        .await
        .expect_err("session must reject nightly trigger");
    assert!(matches!(err, ConsolidatorError::TriggerMismatch { .. }));

    let err = topic
        .on_trigger(
            &Trigger::DecisionLanded {
                decision_id: DECISION_FIXTURE_ID.into(),
                force_deep: false,
            },
            &ctx,
        )
        .await
        .expect_err("topic must reject decision trigger");
    assert!(matches!(err, ConsolidatorError::TriggerMismatch { .. }));

    let err = decision
        .on_trigger(
            &Trigger::SessionEnd {
                session_id: SESSION_FIXTURE_ID.into(),
            },
            &ctx,
        )
        .await
        .expect_err("decision must reject session trigger");
    assert!(matches!(err, ConsolidatorError::TriggerMismatch { .. }));

    let l = ledger.lock().unwrap();
    assert!(
        l.per_grain.is_empty(),
        "no successful run → ledger stays empty",
    );
    assert_eq!(l.total_cents, 0);
}
