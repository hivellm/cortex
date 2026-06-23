//! Phase18 §7.2 — temporal metrics integration test.
//!
//! Drives `Orchestrator::run` with a metrics handle attached and a
//! seeded keyword lane, then asserts the Prometheus render reflects
//! the classifier wedge: one query counted + a non-zero per-state
//! classified counter.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, Props, QueryRequest, Scope};
use cortex_api::TemporalMetrics;
use serde_json::json;

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

struct StampedKeywordLane {
    hits: Vec<LaneHit>,
}

#[async_trait]
impl KeywordLane for StampedKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(self.hits.clone())
    }
}

fn now_unix() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn stamped_hit(doc_id: &str, score: f64, extras: &[(&str, serde_json::Value)]) -> LaneHit {
    let mut props = Props::new();
    for (k, v) in extras {
        props.insert((*k).to_string(), v.clone());
    }
    LaneHit {
        doc_id: doc_id.to_string(),
        text: format!("body-{doc_id}"),
        repo: Some("cortex".into()),
        path: Some(format!("docs/{doc_id}.md")),
        symbol: Some(format!("sym_{doc_id}")),
        content_hash: None,
        score,
        ts: 0,
        severity: None,
        extras: props,
        overlay: Overlay {
            source: cortex_api::lanes::LaneSource::Keyword,
            ..Overlay::default()
        },
    }
}

fn req() -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("cortex".into()),
            ..Scope::default()
        },
        query: "anything".into(),
        limit: 20,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 500,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,

        principal: None,
    }
}

#[tokio::test]
async fn query_bumps_temporal_prometheus_counters() {
    let now = now_unix();
    let past = now - 86_400;
    let hits = vec![
        stamped_hit("valid", 1.0, &[]),
        stamped_hit("superseded", 1.5, &[("superseded_at_unix", json!(past))]),
    ];

    let metrics = Arc::new(TemporalMetrics::new());
    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StampedKeywordLane { hits }),
        Arc::new(MemoryGraphLane::default()),
    )
    .with_temporal_metrics(metrics.clone());

    let _ = orch.run(&req()).await;

    let prom = metrics.render_prom();
    assert!(
        prom.contains("cortex_temporal_queries_total 1"),
        "expected one counted query; got:\n{prom}"
    );
    // The SUPERSEDED hit must have been classified — its state counter
    // is non-zero. (branch_resolution also fired → main branch.)
    assert!(
        prom.contains("cortex_temporal_classified_total{state=\"superseded\"}"),
        "expected a superseded state sample; got:\n{prom}"
    );
    assert!(
        prom.contains("cortex_branch_queries_total{kind=\"main\"}"),
        "expected a main-branch sample; got:\n{prom}"
    );
}
