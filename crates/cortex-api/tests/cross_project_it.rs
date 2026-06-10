//! Phase18 §5.2 — cross-project propagation integration test.
//!
//! Drives the full `Orchestrator::run` pipeline with a synthetic
//! graph lane seeded on the `cross_project_ref` template. When the
//! §5.3 `CrossProjectConfig` is enabled and the request names sibling
//! `projects`, the orchestrator walks the active project's
//! `CROSS_PROJECT_REF` edges, classifies each discovered reference by
//! the edge's temporal window, and fuses the survivors in with
//! `source_project` provenance. These tests pin:
//!
//! - disabled (default) ⇒ no propagation,
//! - enabled + in-window edge ⇒ the sibling reference is propagated,
//! - enabled + stale edge (`valid_to` < `as_of`) ⇒ classifier drops it,
//! - enabled + edge into an un-requested sibling ⇒ filtered out.
//!
//! The graph hits are seeded directly into `MemoryGraphLane` (they do
//! NOT pass through `NexusGraphLane::project_row`), so the test builds
//! the edge `LaneHit` shape — `source_project` / `valid_from` /
//! `valid_to` in `extras` — by hand. RFC-3339 date literals avoid a
//! `chrono` dependency in the test crate.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, LaneSource, MemoryGraphLane, Overlay,
    VectorLane, VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, Props, QueryRequest, Scope};
use cortex_config::CrossProjectConfig;
use serde_json::json;

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct EmptyKeywordLane;

#[async_trait]
impl KeywordLane for EmptyKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

/// Build a synthetic `CROSS_PROJECT_REF` edge hit as the graph lane
/// would surface it for the `cross_project_ref` template — the
/// orchestrator reads `source_project` / `valid_from` / `valid_to`
/// off `extras`.
fn cross_edge_hit(source_project: &str, valid_from: &str, valid_to: Option<&str>) -> LaneHit {
    let mut props = Props::new();
    props.insert("source".into(), json!("graph"));
    props.insert("source_project".into(), json!(source_project));
    props.insert("version_constraint".into(), json!("2.1"));
    props.insert("valid_from".into(), json!(valid_from));
    if let Some(vt) = valid_to {
        props.insert("valid_to".into(), json!(vt));
    }
    LaneHit {
        doc_id: format!("graph|cross_project_ref|cortex:main|{source_project}:main"),
        text: format!("{source_project} cross-project reference"),
        repo: Some(source_project.to_string()),
        path: Some(format!("docs/{source_project}_ref.md")),
        symbol: Some("CROSS_PROJECT_REF".into()),
        content_hash: None,
        score: 1.0,
        ts: 0,
        severity: None,
        extras: props,
        overlay: Overlay {
            source: LaneSource::Graph,
            ..Overlay::default()
        },
    }
}

/// Extract the synthetic id from a `docs/<id>.md` fixture path.
fn extract_doc_id(path: &str) -> String {
    path.strip_prefix("docs/")
        .and_then(|s| s.strip_suffix(".md"))
        .unwrap_or(path)
        .to_string()
}

fn req(projects: Option<Vec<String>>, as_of: Option<String>) -> QueryRequest {
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
        as_of,
        branch: None,
        projects,
        include_history: None,
        include_future: None,
        include_branches: None,
    }
}

/// Build an orchestrator whose graph lane is seeded with one
/// `cross_project_ref` edge. `enabled` flips the §5.3 config.
fn orch_with_edge(edge: LaneHit, enabled: bool) -> Orchestrator {
    let graph = MemoryGraphLane::default();
    graph.seed("cross_project_ref", vec![edge]);
    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(EmptyKeywordLane),
        Arc::new(graph),
    );
    orch.with_cross_project(CrossProjectConfig {
        enabled,
        max_hops: 1,
    })
}

async fn run_doc_ids(orch: &Orchestrator, request: &QueryRequest) -> Vec<String> {
    let (resp, _) = orch.run(request).await;
    resp.results
        .snippets
        .iter()
        .filter_map(|s| s.path.as_deref().map(extract_doc_id))
        .collect()
}

#[tokio::test]
async fn cross_project_disabled_yields_no_propagation() {
    // Open-ended valid window so the only reason it could be absent is
    // the disabled gate.
    let edge = cross_edge_hit(
        "nexus",
        "2020-01-01T00:00:00Z",
        Some("2099-01-01T00:00:00Z"),
    );
    let orch = orch_with_edge(edge, /* enabled */ false);
    let doc_ids = run_doc_ids(&orch, &req(Some(vec!["nexus".into()]), None)).await;
    assert!(
        !doc_ids.iter().any(|id| id == "nexus_ref"),
        "cross_project disabled ⇒ no propagation; got {doc_ids:?}"
    );
}

#[tokio::test]
async fn cross_project_enabled_propagates_with_provenance() {
    let edge = cross_edge_hit(
        "nexus",
        "2020-01-01T00:00:00Z",
        Some("2099-01-01T00:00:00Z"),
    );
    let orch = orch_with_edge(edge, /* enabled */ true);
    let doc_ids = run_doc_ids(&orch, &req(Some(vec!["nexus".into()]), None)).await;
    assert!(
        doc_ids.iter().any(|id| id == "nexus_ref"),
        "in-window cross-project reference must be propagated; got {doc_ids:?}"
    );
}

#[tokio::test]
async fn cross_project_stale_constraint_dropped() {
    // `valid_to` well in the past ⇒ the classifier marks it EXPIRED
    // and the propagation path drops it.
    let edge = cross_edge_hit(
        "nexus",
        "2019-01-01T00:00:00Z",
        Some("2020-01-01T00:00:00Z"),
    );
    let orch = orch_with_edge(edge, /* enabled */ true);
    let doc_ids = run_doc_ids(&orch, &req(Some(vec!["nexus".into()]), None)).await;
    assert!(
        !doc_ids.iter().any(|id| id == "nexus_ref"),
        "stale (expired) cross-project edge must drop; got {doc_ids:?}"
    );
}

#[tokio::test]
async fn cross_project_unrequested_sibling_filtered() {
    // Edge points into `nexus`, but the request only asks for
    // `vectorizer` ⇒ the nexus reference is filtered before classify.
    let edge = cross_edge_hit(
        "nexus",
        "2020-01-01T00:00:00Z",
        Some("2099-01-01T00:00:00Z"),
    );
    let orch = orch_with_edge(edge, /* enabled */ true);
    let doc_ids = run_doc_ids(&orch, &req(Some(vec!["vectorizer".into()]), None)).await;
    assert!(
        !doc_ids.iter().any(|id| id == "nexus_ref"),
        "un-requested sibling must be filtered; got {doc_ids:?}"
    );
}
