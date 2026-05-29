//! Integration tests for the live `NexusGraphLane`. Drives the
//! lane indirectly through `MemoryGraphLane` for the orchestrator
//! end-to-end shape (the SDK's HTTP transport doesn't expose the
//! same wiremock-friendly surface as Meili / Vectorizer — the
//! Nexus probe path goes through a typed RPC), and validates the
//! NexusGraphLane's row projection + template whitelist directly.

use std::sync::Arc;

use cortex_api::types::{IncludeField, Intent, Scope};
use cortex_api::{
    GraphLane, GraphRequest, KeywordLane, LaneError, LaneHit, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryRequest,
};

fn graph_request(template: &str, query: &str) -> GraphRequest {
    GraphRequest {
        template: template.into(),
        params: serde_json::json!({ "query": query }),
        max_hops: 2,
        scope: Scope::default(),
    }
}

#[tokio::test]
async fn graph_overlay_populates_when_lane_returns_hits() {
    // The 2026-04-27 audit showed graph_neighbors=0 across every
    // probed query because the orchestrator wired MemoryGraphLane
    // (empty). When the lane DOES return hits — the live
    // NexusGraphLane reading from a populated graph — the orchestrator
    // must surface them in `results.graph_neighbors`.
    let v = Arc::new(MemoryVectorLane::new());
    let mut keyword_extras = std::collections::BTreeMap::new();
    keyword_extras.insert(
        "source".to_string(),
        serde_json::Value::String("keyword".to_string()),
    );
    let k = Arc::new(MemoryKeywordLane::new());
    k.seed(
        "cortex-unknown-code",
        vec![LaneHit {
            doc_id: "kk".into(),
            text: "anchor hit".into(),
            repo: None,
            path: None,
            symbol: None,
            content_hash: None,
            score: 0.5,
            ts: 1,
            severity: None,
            extras: keyword_extras,
            overlay: cortex_api::lanes::Overlay {
                source: cortex_api::lanes::LaneSource::Keyword,
                ..Default::default()
            },
        }],
    );

    // Stand-in for the live Nexus lane — same `extras` shape the
    // NexusGraphLane stamps. The orchestrator's
    // `derive_graph_neighbors` reads these keys; if the contract
    // here drifts from `nexus_graph_lane::project_row`, the
    // overlay surfaces empty even when Nexus returned rows.
    let mut graph_extras = std::collections::BTreeMap::new();
    graph_extras.insert(
        "source".to_string(),
        serde_json::Value::String("graph".to_string()),
    );
    graph_extras.insert(
        "edge_from".to_string(),
        serde_json::Value::String("DEC-0042".into()),
    );
    graph_extras.insert(
        "edge_to".to_string(),
        serde_json::Value::String("DEC-0001".into()),
    );
    graph_extras.insert(
        "edge_type".to_string(),
        serde_json::Value::String("SUPERSEDES".into()),
    );
    graph_extras.insert("hops".to_string(), serde_json::Value::Number(1.into()));

    let g = Arc::new(MemoryGraphLane::new());
    g.seed(
        "edge_artifact_touched_neighbours",
        vec![LaneHit {
            doc_id: "graph|edge_artifact_touched_neighbours|DEC-0042|DEC-0001".into(),
            text: "Adopt RRF fusion".into(),
            repo: None,
            path: None,
            symbol: Some("SUPERSEDES".into()),
            content_hash: None,
            score: 1.0,
            ts: 0,
            severity: None,
            extras: graph_extras,
            overlay: cortex_api::lanes::Overlay {
                source: cortex_api::lanes::LaneSource::Graph,
                edge_from: Some("DEC-0042".to_string()),
                edge_to: Some("DEC-0001".to_string()),
                hops: Some(1),
                ..Default::default()
            },
        }],
    );

    let orch = Orchestrator::new(v, k, g);
    let req = QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope::default(),
        query: "RRF fusion".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets, IncludeField::GraphNeighbors],
        budget_ms: 1000,
        budget_bytes: None,
        as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
    };
    let (resp, _rewritten) = orch.run(&req).await;
    assert_eq!(resp.results.graph_neighbors.len(), 1);
    let neigh = &resp.results.graph_neighbors[0];
    assert_eq!(neigh.from, "DEC-0042");
    assert_eq!(neigh.to, "DEC-0001");
    assert_eq!(neigh.relation, "SUPERSEDES");
    assert_eq!(neigh.hops, 1);
    // `debug.lanes.graph_ms` populated — the regression had it
    // missing because the lane never executed.
    assert!(resp.debug.lanes.graph_ms.is_some());
}

#[tokio::test]
async fn unknown_template_is_rejected_at_lane_boundary() {
    // The lane must refuse arbitrary Cypher. Spec scenario:
    // "unknown template rejected" — drop_database / random strings
    // never reach Nexus.
    use cortex_api::nexus_graph_lane::NexusGraphLane;
    use std::sync::Arc as StdArc;

    // Build a lane wrapped around a Nexus client that points at a
    // dead address. We never let the lane execute Cypher because
    // the template check runs first; if the whitelist were broken,
    // the test would surface a Transport error (proving Cypher was
    // dispatched) instead of the Rejected error we want.
    let cfg = nexus_sdk::ClientConfig {
        base_url: "http://127.0.0.1:1".into(),
        api_key: None,
        ..Default::default()
    };
    let client = nexus_sdk::NexusClient::with_config(cfg).expect("nexus client built");
    let lane = NexusGraphLane::new(StdArc::new(client));

    let err = lane
        .query(&graph_request("drop_database", "anything"))
        .await
        .expect_err("unknown template must error");
    match err {
        LaneError::Rejected(msg) => {
            assert!(
                msg.contains("unknown graph template") && msg.contains("whitelist:"),
                "rejection message names the offending template + lists the whitelist: {msg}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn fail_open_when_nexus_unreachable_through_orchestrator() {
    // Spec scenario "Nexus down → fail-open". The lane bound to a
    // dead address must surface as `debug.errors["graph"]` and the
    // orchestrator continues with whatever the other lanes
    // produced.
    use cortex_api::nexus_graph_lane::NexusGraphLane;
    use std::sync::Arc as StdArc;

    let cfg = nexus_sdk::ClientConfig {
        base_url: "http://127.0.0.1:1".into(),
        api_key: None,
        ..Default::default()
    };
    let client = nexus_sdk::NexusClient::with_config(cfg).expect("nexus client built");
    let dead_lane: StdArc<dyn GraphLane> = StdArc::new(NexusGraphLane::new(StdArc::new(client)));

    let v = Arc::new(MemoryVectorLane::new());
    let k: Arc<dyn KeywordLane> = Arc::new(MemoryKeywordLane::new());
    let orch = Orchestrator::new(v, k, dead_lane);

    let req = QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope::default(),
        query: "anything".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets, IncludeField::GraphNeighbors],
        budget_ms: 1000,
        budget_bytes: None,
        as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
    };
    let (resp, _rewritten) = orch.run(&req).await;
    assert!(resp.results.graph_neighbors.is_empty());
    assert!(
        resp.debug.errors.contains_key("graph"),
        "graph lane error must surface in debug.errors",
    );
}
