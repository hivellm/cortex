//! Phase17 §2.6 — cross-encoder reranker integration tests.
//!
//! Drives the full `Orchestrator::run` pipeline with a mock `Reranker`
//! implementation that avoids needing a live TEI service. Pins three
//! contracts:
//!
//! - **success path** — reranker returns scores, fused hits are re-sorted
//!   by the returned cross-encoder scores before dedupe/truncate.
//! - **timeout fallback** — on `RerankerError::Timeout`, the orchestrator
//!   logs a warning and keeps the pre-rerank fusion order (fail-open §2.5).
//! - **disabled-flag passthrough** — when `RerankerConfig { enabled: false
//!   }` the reranker impl is never invoked; hits arrive in fusion order.

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, LaneSource, MemoryGraphLane, Overlay,
    VectorLane, VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, QueryRequest, Scope};
use cortex_config::RerankerConfig;
use cortex_workers::rerank::{Candidate, Reranker, RerankerError};

// ── Lane stubs ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(vec![])
    }
}

/// Keyword lane that always returns two hits with scores 1.0 and 0.5 so
/// the pre-rerank fusion order is `hit-a` before `hit-b`.
struct TwoHitKeywordLane;

#[async_trait]
impl KeywordLane for TwoHitKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(vec![hit("hit-a", 1.0), hit("hit-b", 0.5)])
    }
}

fn hit(id: &str, score: f64) -> LaneHit {
    LaneHit {
        doc_id: id.to_string(),
        text: format!("text for {id}"),
        repo: Some("cortex".into()),
        path: Some(format!("{id}.rs")),
        symbol: Some(id.to_string()),
        content_hash: None,
        score,
        ts: 0,
        severity: None,
        extras: Default::default(),
        overlay: Overlay {
            source: LaneSource::Keyword,
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
        query: "test query".into(),
        limit: 10,
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

// ── Mock rerankers ──────────────────────────────────────────────────────────

/// Reverses the input order — gives the second candidate a higher score so
/// we can assert the orchestrator re-sorted the hits.
struct ReversingReranker {
    called: AtomicBool,
}

impl ReversingReranker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            called: AtomicBool::new(false),
        })
    }
}

#[async_trait]
impl Reranker for ReversingReranker {
    async fn score(
        &self,
        _query: &str,
        candidates: &[Candidate],
    ) -> Result<Vec<f32>, RerankerError> {
        self.called.store(true, Ordering::SeqCst);
        // Return scores in reverse order so the second candidate wins.
        let n = candidates.len();
        let scores: Vec<f32> = (0..n).map(|i| (n - i) as f32 * 0.1).collect();
        // Reverse: first candidate gets the lowest score.
        let reversed: Vec<f32> = scores.into_iter().rev().collect();
        Ok(reversed)
    }
}

/// Always returns a timeout error — simulates a slow TEI service.
struct TimeoutReranker {
    call_count: AtomicUsize,
}

impl TimeoutReranker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            call_count: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl Reranker for TimeoutReranker {
    async fn score(
        &self,
        _query: &str,
        _candidates: &[Candidate],
    ) -> Result<Vec<f32>, RerankerError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Err(RerankerError::Timeout { ms: 500 })
    }
}

/// Should never be called — asserts disabled-flag passthrough.
struct PanicReranker;

#[async_trait]
impl Reranker for PanicReranker {
    async fn score(
        &self,
        _query: &str,
        _candidates: &[Candidate],
    ) -> Result<Vec<f32>, RerankerError> {
        panic!("PanicReranker: reranker must not be called when disabled");
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

fn base_orch() -> Orchestrator {
    Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(TwoHitKeywordLane),
        Arc::new(MemoryGraphLane::default()),
    )
}

#[tokio::test]
async fn reranker_success_path_reorders_hits() {
    let reranker = ReversingReranker::new();
    let cfg = RerankerConfig {
        enabled: true,
        endpoint: Some("http://localhost:8080".into()),
        top_k_input: 100,
        timeout_ms: 500,
    };
    let orch = base_orch().with_reranker(reranker.clone(), cfg);
    let (resp, _) = orch.run(&req()).await;

    assert!(
        reranker.called.load(Ordering::SeqCst),
        "reranker must be called on the success path"
    );

    let ids: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref())
        .collect();

    // ReversingReranker gives hit-b a higher score than hit-a, so hit-b
    // must appear first after re-sort.
    assert!(
        ids.first().copied() == Some("hit-b.rs"),
        "after reranking the higher-scored hit-b must sort first; got {ids:?}"
    );
}

#[tokio::test]
async fn reranker_timeout_falls_back_to_fusion_order() {
    let reranker = TimeoutReranker::new();
    let cfg = RerankerConfig {
        enabled: true,
        endpoint: Some("http://localhost:8080".into()),
        top_k_input: 100,
        timeout_ms: 500,
    };
    let orch = base_orch().with_reranker(reranker.clone(), cfg);
    let (resp, _) = orch.run(&req()).await;

    assert_eq!(
        reranker.call_count.load(Ordering::SeqCst),
        1,
        "reranker must be called exactly once"
    );

    let ids: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref())
        .collect();

    // Fusion order is hit-a (score 1.0) before hit-b (score 0.5).
    assert!(
        ids.first().copied() == Some("hit-a.rs"),
        "on timeout fallback the fusion order (hit-a first) must be preserved; got {ids:?}"
    );
}

#[tokio::test]
async fn reranker_disabled_flag_bypasses_reranker() {
    let cfg = RerankerConfig {
        enabled: false,
        endpoint: Some("http://localhost:8080".into()),
        top_k_input: 100,
        timeout_ms: 500,
    };
    let orch = base_orch().with_reranker(Arc::new(PanicReranker), cfg);
    // This must not panic — PanicReranker::score must never be called.
    let (resp, _) = orch.run(&req()).await;

    let ids: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref())
        .collect();

    // Without reranking, fusion order (hit-a first) is preserved.
    assert!(
        ids.first().copied() == Some("hit-a.rs"),
        "disabled reranker ⇒ fusion order preserved; got {ids:?}"
    );
}
