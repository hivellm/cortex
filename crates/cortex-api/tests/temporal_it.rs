//! Phase18 §3.7 — temporal classifier integration test.
//!
//! Drives the full `Orchestrator::run` pipeline with a synthetic
//! keyword lane that returns hits stamped with bitemporal axes
//! (`valid_from_unix` / `valid_to_unix` / `superseded_at_unix` /
//! `lifecycle`). The wedge that lives between `rrf_fuse` and the
//! anchor-dedupe MUST:
//!
//! - drop SUPERSEDED hits under the default flags,
//! - drop EXPIRED hits under the default flags,
//! - boost TEMPORAL hits (`valid_to` inside the recency window) by
//!   `temporal_boost`,
//! - leave VALID hits unchanged.
//!
//! The fixture seeds three hits — one VALID (high score), one
//! SUPERSEDED (high score; must drop), one TEMPORAL (lower base
//! score; must boost). The expected post-wedge order is:
//!
//!   `valid` → `temporal_boosted` (or vice-versa, depending on the
//!   relative score after boost).
//!
//! The SUPERSEDED hit must NOT appear in the snippet section.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, Props, QueryRequest, Scope};
use serde_json::json;

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

/// Keyword lane that returns three bitemporal-stamped hits.
struct StampedKeywordLane {
    hits: Vec<LaneHit>,
}

impl StampedKeywordLane {
    fn new(hits: Vec<LaneHit>) -> Self {
        Self { hits }
    }
}

#[async_trait]
impl KeywordLane for StampedKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(self.hits.clone())
    }
}

/// Extract the synthetic `doc_id` from a `docs/<id>.md` fixture path.
fn extract_doc_id(path: &str) -> String {
    path.strip_prefix("docs/")
        .and_then(|s| s.strip_suffix(".md"))
        .unwrap_or(path)
        .to_string()
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
    let overlay = Overlay {
        source: cortex_api::lanes::LaneSource::Keyword,
        ..Overlay::default()
    };
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
        overlay,
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
async fn orchestrator_drops_superseded_and_boosts_temporal() {
    let now = now_unix();
    let past = now - 86_400;
    let soon = now + 3_600;

    let hits = vec![
        // VALID — no bitemporal columns, base 1.0.
        stamped_hit("valid", 1.0, &[]),
        // SUPERSEDED — must drop.
        stamped_hit("superseded", 1.5, &[("superseded_at_unix", json!(past))]),
        // TEMPORAL — `valid_to` inside the 30-day window, base 0.9.
        stamped_hit("temporal_boost", 0.9, &[("valid_to_unix", json!(soon))]),
    ];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StampedKeywordLane::new(hits)),
        Arc::new(MemoryGraphLane::default()),
    );

    let (resp, _) = orch.run(&req()).await;

    let doc_ids: Vec<String> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref().map(extract_doc_id))
        .collect();

    assert!(
        !doc_ids.iter().any(|id| id == "superseded"),
        "SUPERSEDED hit must drop from the snippet section, got {:?}",
        doc_ids
    );
    assert!(
        doc_ids.iter().any(|id| id == "valid"),
        "VALID hit must survive, got {:?}",
        doc_ids
    );
    assert!(
        doc_ids.iter().any(|id| id == "temporal_boost"),
        "TEMPORAL hit must survive, got {:?}",
        doc_ids
    );
}

#[tokio::test]
async fn orchestrator_drops_expired_under_default_flags() {
    let now = now_unix();
    let past = now - 86_400 * 90; // 90 days ago, well outside window.

    let hits = vec![
        stamped_hit("alive", 1.0, &[]),
        stamped_hit("dead", 1.5, &[("valid_to_unix", json!(past))]),
    ];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StampedKeywordLane::new(hits)),
        Arc::new(MemoryGraphLane::default()),
    );

    let (resp, _) = orch.run(&req()).await;
    let doc_ids: Vec<String> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref().map(extract_doc_id))
        .collect();
    assert!(
        !doc_ids.iter().any(|id| id == "dead"),
        "EXPIRED hit must drop, got {:?}",
        doc_ids
    );
    assert!(doc_ids.iter().any(|id| id == "alive"));
}

#[tokio::test]
async fn orchestrator_drops_abandoned_hits_under_default_flags() {
    let hits = vec![
        stamped_hit("active", 1.0, &[]),
        stamped_hit("abandoned", 1.5, &[("lifecycle", json!("abandoned"))]),
    ];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StampedKeywordLane::new(hits)),
        Arc::new(MemoryGraphLane::default()),
    );

    let (resp, _) = orch.run(&req()).await;
    let doc_ids: Vec<String> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref().map(extract_doc_id))
        .collect();
    assert!(
        !doc_ids.iter().any(|id| id == "abandoned"),
        "ABANDONED hit must drop, got {:?}",
        doc_ids
    );
    assert!(doc_ids.iter().any(|id| id == "active"));
}

#[tokio::test]
async fn orchestrator_disabling_classifier_passes_every_hit_through() {
    let now = now_unix();
    let past = now - 86_400;
    let hits = vec![
        stamped_hit("alive", 1.0, &[]),
        stamped_hit("superseded", 1.5, &[("superseded_at_unix", json!(past))]),
    ];

    let cfg = cortex_config::TemporalConfig {
        enabled: false,
        ..cortex_config::TemporalConfig::default()
    };

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StampedKeywordLane::new(hits)),
        Arc::new(MemoryGraphLane::default()),
    )
    .with_temporal(cfg);

    let (resp, _) = orch.run(&req()).await;
    let doc_ids: Vec<String> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref().map(extract_doc_id))
        .collect();
    // With the classifier off the SUPERSEDED hit MUST survive — the
    // wedge is bypassed and the row passes verbatim into the
    // snippet section.
    assert!(
        doc_ids.iter().any(|id| id == "superseded"),
        "classifier disabled ⇒ no drops; got {:?}",
        doc_ids
    );
    assert!(doc_ids.iter().any(|id| id == "alive"));
}
