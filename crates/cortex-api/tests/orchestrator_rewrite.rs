//! Phase6f orchestrator integration test — proves the rewriter's
//! `vector_query` and `keyword_query` reach the per-lane request
//! builders (not just the strategy stamp on the audit envelope).
//!
//! Recording lanes capture the inbound `VectorRequest` /
//! `KeywordRequest` so the assertion is exact.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_api::lanes::{
    GraphLane, GraphRequest, KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane,
    VectorLane, VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::query_rewrite::{NounPhraseRewriter, QueryRewriter, RewriteError, RewrittenQuery};
use cortex_api::types::{IncludeField, Intent, QueryRequest, Scope};

#[derive(Default)]
struct RecordingVectorLane {
    seen: Mutex<Vec<VectorRequest>>,
}

#[async_trait]
impl VectorLane for RecordingVectorLane {
    async fn search(&self, req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        if let Ok(mut g) = self.seen.lock() {
            g.push(req.clone());
        }
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct RecordingKeywordLane {
    seen: Mutex<Vec<KeywordRequest>>,
}

#[async_trait]
impl KeywordLane for RecordingKeywordLane {
    async fn search(&self, req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        if let Ok(mut g) = self.seen.lock() {
            g.push(req.clone());
        }
        Ok(Vec::new())
    }
}

fn req(prompt: &str) -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("Cortex".into()),
            ..Scope::default()
        },
        query: prompt.into(),
        limit: 20,
        k: 50,
        include: vec![
            IncludeField::Snippets,
            IncludeField::Decisions,
            IncludeField::Violations,
            IncludeField::GraphNeighbors,
            IncludeField::SimilarTurns,
        ],
        budget_ms: 500,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
    }
}

#[tokio::test]
async fn rewriter_output_threads_into_each_lane_request() {
    let v = Arc::new(RecordingVectorLane::default());
    let k = Arc::new(RecordingKeywordLane::default());
    let g = Arc::new(MemoryGraphLane::default());
    let orch = Orchestrator::new(v.clone(), k.clone(), g)
        .with_rewriter(Arc::new(NounPhraseRewriter::new()));

    let prompt = "why is the meili fan-out worker offset broken should we rewrite it";
    let (_resp, rewritten) = orch.run(&req(prompt)).await;

    // The noun-phrase rewriter strips "why is the", so the
    // recorded query should NOT contain those framing words.
    assert_eq!(rewritten.strategy, "noun_phrase");
    assert!(!rewritten.vector_query.starts_with("why "));
    assert_eq!(rewritten.original, prompt);

    // Every recorded VectorRequest carries the rewritten query —
    // not the original prompt.
    let vec_seen = v.seen.lock().unwrap();
    assert!(!vec_seen.is_empty(), "vector lane was not invoked");
    for r in vec_seen.iter() {
        assert_eq!(
            r.query, rewritten.vector_query,
            "vector request must carry rewritten.vector_query"
        );
    }

    // Every recorded KeywordRequest carries the rewritten query.
    let kw_seen = k.seen.lock().unwrap();
    assert!(!kw_seen.is_empty(), "keyword lane was not invoked");
    for r in kw_seen.iter() {
        assert_eq!(
            r.query, rewritten.keyword_query,
            "keyword request must carry rewritten.keyword_query"
        );
    }
}

#[tokio::test]
async fn passthrough_default_leaves_lane_queries_verbatim() {
    let v = Arc::new(RecordingVectorLane::default());
    let k = Arc::new(RecordingKeywordLane::default());
    let g = Arc::new(MemoryGraphLane::default());
    // No `with_rewriter` call → the default PassthroughRewriter
    // reproduces today's behaviour.
    let orch = Orchestrator::new(v.clone(), k.clone(), g);

    let prompt = "why is meili broken";
    let (_resp, rewritten) = orch.run(&req(prompt)).await;
    assert_eq!(rewritten.strategy, "passthrough");

    let vec_seen = v.seen.lock().unwrap();
    for r in vec_seen.iter() {
        assert_eq!(r.query, prompt);
    }
    let kw_seen = k.seen.lock().unwrap();
    for r in kw_seen.iter() {
        assert_eq!(r.query, prompt);
    }
}

// A rewriter that always fails — proves the orchestrator falls
// back to the verbatim prompt rather than blowing up the request.
struct FailingRewriter;

#[async_trait]
impl QueryRewriter for FailingRewriter {
    async fn rewrite(
        &self,
        _prompt: &str,
        _intent: Intent,
    ) -> Result<RewrittenQuery, RewriteError> {
        Err(RewriteError::Upstream("forced failure".into()))
    }
}

#[tokio::test]
async fn rewriter_failure_falls_back_to_passthrough() {
    let v = Arc::new(RecordingVectorLane::default());
    let k = Arc::new(RecordingKeywordLane::default());
    let g = Arc::new(MemoryGraphLane::default());
    let orch = Orchestrator::new(v.clone(), k.clone(), g).with_rewriter(Arc::new(FailingRewriter));

    let prompt = "anything at all";
    let (_resp, rewritten) = orch.run(&req(prompt)).await;
    assert_eq!(rewritten.strategy, "passthrough");

    let vec_seen = v.seen.lock().unwrap();
    for r in vec_seen.iter() {
        assert_eq!(r.query, prompt);
    }
}

#[tokio::test]
async fn graph_request_params_carry_rewritten_query() {
    let v = Arc::new(RecordingVectorLane::default());
    let k = Arc::new(RecordingKeywordLane::default());

    // Recording graph lane — captures the inbound GraphRequest so
    // we can assert the patched params land.
    #[derive(Default)]
    struct RecordingGraphLane {
        seen: Mutex<Vec<GraphRequest>>,
    }
    #[async_trait]
    impl GraphLane for RecordingGraphLane {
        async fn query(&self, req: &GraphRequest) -> Result<Vec<LaneHit>, LaneError> {
            if let Ok(mut g) = self.seen.lock() {
                g.push(req.clone());
            }
            Ok(Vec::new())
        }
    }

    let g = Arc::new(RecordingGraphLane::default());
    let orch =
        Orchestrator::new(v, k, g.clone()).with_rewriter(Arc::new(NounPhraseRewriter::new()));

    let prompt = "how does meili route enriched events into per-repo indexes";
    let (_resp, rewritten) = orch.run(&req(prompt)).await;

    let seen = g.seen.lock().unwrap();
    assert!(!seen.is_empty(), "graph lane was not invoked");
    for gr in seen.iter() {
        let q = gr
            .params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(q, rewritten.vector_query);
    }
}
