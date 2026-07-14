//! Phase27b §3.1/§3.2 — integration coverage for
//! `GET /v1/dashboard/graph/communities`.
//!
//! The community writeback worker (§2.5) is blocked on the semantic
//! graph projection (ADR-027), so the realistic state today is: no
//! node in the live graph carries `community_id`. These tests pin
//! two contracts:
//!
//! 1. The endpoint answers an honest empty payload (never an error)
//!    when no Nexus client is configured, and when Nexus is
//!    configured but returns zero rows for both queries.
//! 2. When synthetic community data IS present, the grouping /
//!    god-node / cross-community-edge logic groups correctly.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use cortex_api::{
    build_router_with, DashboardState, LoaderMetrics, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryService, TaskLoader,
};
use serde_json::Value;
use tower::ServiceExt;

fn build_test_router(nexus: Option<Arc<nexus_sdk::NexusClient>>) -> Router {
    let lane = Arc::new(MemoryKeywordLane::new());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(vector, lane.clone(), graph);
    let service =
        Arc::new(QueryService::with_memory_defaults(orchestrator).with_indexed_repos(lane.clone()));
    let dashboard = DashboardState {
        lane,
        nexus,
        analyzer: Arc::new(cortex_api::analyzer::Analyzer::from_env()),
        tasks: Arc::new(cortex_api::MultiTaskLoader::new(vec![TaskLoader::new(
            std::path::PathBuf::from("__tests_no_rulebook__"),
        )])),
        metadata: None,
        loader_metrics: Arc::new(LoaderMetrics::new()),
        temporal_metrics: Arc::new(cortex_api::TemporalMetrics::new()),
        events_bus: cortex_api::dashboard_watcher::DashboardEventBus::new(),
        acl_metrics: None,
    };
    build_router_with(service, Some(dashboard))
}

fn nexus_client_for(server_uri: &str) -> Arc<nexus_sdk::NexusClient> {
    let cfg = nexus_sdk::ClientConfig {
        base_url: server_uri.to_string(),
        api_key: None,
        ..Default::default()
    };
    Arc::new(nexus_sdk::NexusClient::with_config(cfg).expect("nexus client built"))
}

async fn get_json(router: Router, uri: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, parsed)
}

/// Empty Cypher reply envelope — zero rows for either query.
fn empty_cypher_body() -> serde_json::Value {
    serde_json::json!({
        "columns": [],
        "rows": [],
        "execution_time_ms": 1
    })
}

#[tokio::test]
async fn communities_endpoint_returns_empty_when_no_nexus_client() {
    // No Nexus client wired at all — the realistic dev/cold-stack
    // path. Must answer 200 with empty arrays, never an error.
    let router = build_test_router(None);
    let (status, body) = get_json(router, "/v1/dashboard/graph/communities").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["communities"].as_array().unwrap().len(), 0);
    assert_eq!(body["cross_community_edges"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn communities_endpoint_returns_empty_when_no_nodes_carry_community_id() {
    // Nexus IS configured but no node carries `community_id` yet —
    // the exact live state today because §2.5 (the writeback worker)
    // is blocked on the semantic graph projection (ADR-027).
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_cypher_body()))
        .mount(&server)
        .await;

    let router = build_test_router(Some(nexus_client_for(&server.uri())));
    let (status, body) = get_json(router, "/v1/dashboard/graph/communities").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["communities"].as_array().unwrap().len(), 0);
    assert_eq!(body["cross_community_edges"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn communities_endpoint_groups_members_god_nodes_and_cross_community_edges() {
    // Synthetic community data: two communities of two members each,
    // one god node per community, and two cross-community edges.
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    // Members query — RETURNs n.community_level AS level, a substring
    // that never appears in the cross-community-edges query below.
    let members_body = serde_json::json!({
        "columns": ["id", "name", "community_id", "level", "is_god_node"],
        "rows": [
            ["sym-a", "Auth Service", 1, 0, true],
            ["sym-b", "Auth Helper", 1, 0, false],
            ["sym-c", "Billing Gateway", 2, 0, true],
            ["sym-d", "Billing Service", 2, 0, false]
        ],
        "execution_time_ms": 2
    });
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("n.community_level"))
        .respond_with(ResponseTemplate::new(200).set_body_json(members_body))
        .mount(&server)
        .await;

    // Cross-community edges query — RETURNs type(r) AS rel_type, a
    // substring that never appears in the members query above.
    let edges_body = serde_json::json!({
        "columns": ["from_id", "from_community", "to_id", "to_community", "rel_type"],
        "rows": [
            ["sym-a", 1, "sym-c", 2, "CALLS"],
            ["sym-b", 1, "sym-d", 2, "IMPORTS"]
        ],
        "execution_time_ms": 2
    });
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("type(r)"))
        .respond_with(ResponseTemplate::new(200).set_body_json(edges_body))
        .mount(&server)
        .await;

    let router = build_test_router(Some(nexus_client_for(&server.uri())));
    let (status, body) = get_json(router, "/v1/dashboard/graph/communities").await;
    assert_eq!(status, StatusCode::OK);

    let communities = body["communities"].as_array().unwrap();
    assert_eq!(communities.len(), 2, "two distinct communities");

    // BTreeMap keeps community_id ascending — community 1 first.
    let c1 = &communities[0];
    assert_eq!(c1["community_id"], 1);
    assert_eq!(c1["member_count"], 2);
    let c1_god = c1["god_nodes"].as_array().unwrap();
    assert_eq!(
        c1_god.len(),
        1,
        "only sym-a is flagged is_god_node in community 1"
    );
    assert_eq!(c1_god[0]["id"], "sym-a");
    assert_eq!(c1_god[0]["name"], "Auth Service");

    let c2 = &communities[1];
    assert_eq!(c2["community_id"], 2);
    assert_eq!(c2["member_count"], 2);
    let c2_god = c2["god_nodes"].as_array().unwrap();
    assert_eq!(
        c2_god.len(),
        1,
        "only sym-c is flagged is_god_node in community 2"
    );
    assert_eq!(c2_god[0]["id"], "sym-c");
    assert_eq!(c2_god[0]["name"], "Billing Gateway");

    let edges = body["cross_community_edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2, "both cross-community edges surfaced");
    // Sorted by (from, to, relation) — "sym-a" < "sym-b" lexically.
    assert_eq!(edges[0]["from"], "sym-a");
    assert_eq!(edges[0]["from_community"], 1);
    assert_eq!(edges[0]["to"], "sym-c");
    assert_eq!(edges[0]["to_community"], 2);
    assert_eq!(edges[0]["relation"], "CALLS");
    assert_eq!(edges[1]["from"], "sym-b");
    assert_eq!(edges[1]["to"], "sym-d");
    assert_eq!(edges[1]["relation"], "IMPORTS");
}

// ── Phase27e §2 — graph query primitives: path + compare ──────────────────

/// One-row endpoint-resolution reply: `[id, name, [label]]`.
fn resolve_body(id: &str, name: &str, label: &str) -> serde_json::Value {
    serde_json::json!({
        "columns": ["id", "name", "labels"],
        "rows": [[id, name, [label]]],
        "execution_time_ms": 1
    })
}

/// Neighbour-expansion reply: rows of `[id, name, [label], rel]`.
fn neighbors_body(rows: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "columns": ["id", "name", "labels", "rel"],
        "rows": rows,
        "execution_time_ms": 1
    })
}

#[tokio::test]
async fn path_endpoint_returns_not_found_when_no_nexus_client() {
    let router = build_test_router(None);
    let (status, body) = get_json(router, "/v1/dashboard/graph/path?from=a&to=b").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], false);
    assert_eq!(body["hops"], 0);
    assert!(body["path"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn path_endpoint_finds_two_hop_path_across_synthetic_graph() {
    // node-a --DEFINES--> node-b --CALLS--> node-c
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    // Endpoint resolution (query carries `LIMIT 1`).
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("LIMIT 1"))
        .and(body_string_contains("node-a"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(resolve_body("node-a", "A", "Artifact")),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("LIMIT 1"))
        .and(body_string_contains("node-c"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(resolve_body("node-c", "C", "Symbol")),
        )
        .mount(&server)
        .await;

    // Frontier expansions (query shape `(a)-[r]-(b)`).
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("-[r]-(b)"))
        .and(body_string_contains("a._id = 'node-a'"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(neighbors_body(serde_json::json!([[
                "node-b",
                "B",
                ["Symbol"],
                "DEFINES"
            ]]))),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("-[r]-(b)"))
        .and(body_string_contains("a._id = 'node-b'"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(neighbors_body(serde_json::json!([
                ["node-a", "A", ["Artifact"], "DEFINES"],
                ["node-c", "C", ["Symbol"], "CALLS"]
            ]))),
        )
        .mount(&server)
        .await;

    let router = build_test_router(Some(nexus_client_for(&server.uri())));
    let (status, body) = get_json(router, "/v1/dashboard/graph/path?from=node-a&to=node-c").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], true, "path must be found; body={body}");
    assert_eq!(body["hops"], 2);
    let ids: Vec<&str> = body["path"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["node-a", "node-b", "node-c"]);
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["relation"], "DEFINES");
    assert_eq!(edges[1]["relation"], "CALLS");
}

#[tokio::test]
async fn path_endpoint_unresolvable_endpoint_is_found_false_not_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    // Every Cypher (incl. resolution) returns zero rows.
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_cypher_body()))
        .mount(&server)
        .await;
    let router = build_test_router(Some(nexus_client_for(&server.uri())));
    let (status, body) = get_json(router, "/v1/dashboard/graph/path?from=ghost&to=phantom").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], false);
}

#[tokio::test]
async fn compare_endpoint_returns_shared_and_divergent_sets() {
    // X neighbours: {S, OA}; Y neighbours: {S, OB}
    // → shared = [S], only_a = [OA], only_b = [OB].
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("LIMIT 1"))
        .and(body_string_contains("node-x"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(resolve_body("node-x", "X", "Symbol")),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("LIMIT 1"))
        .and(body_string_contains("node-y"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(resolve_body("node-y", "Y", "Symbol")),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("-[r]-(b)"))
        .and(body_string_contains("a._id = 'node-x'"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(neighbors_body(serde_json::json!([
                ["node-s", "Shared", ["Symbol"], "CALLS"],
                ["node-oa", "OnlyA", ["Symbol"], "CALLS"]
            ]))),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/cypher"))
        .and(body_string_contains("-[r]-(b)"))
        .and(body_string_contains("a._id = 'node-y'"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(neighbors_body(serde_json::json!([
                ["node-s", "Shared", ["Symbol"], "CALLS"],
                ["node-ob", "OnlyB", ["Symbol"], "IMPORTS"]
            ]))),
        )
        .mount(&server)
        .await;

    let router = build_test_router(Some(nexus_client_for(&server.uri())));
    let (status, body) = get_json(router, "/v1/dashboard/graph/compare?a=node-x&b=node-y").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found_a"], true);
    assert_eq!(body["found_b"], true);
    assert_eq!(body["shared_count"], 1);
    assert_eq!(body["only_a_count"], 1);
    assert_eq!(body["only_b_count"], 1);
    assert_eq!(body["shared"][0]["id"], "node-s");
    assert_eq!(body["only_a"][0]["id"], "node-oa");
    assert_eq!(body["only_b"][0]["id"], "node-ob");
}

#[tokio::test]
async fn compare_endpoint_no_nexus_client_is_empty_not_error() {
    let router = build_test_router(None);
    let (status, body) = get_json(router, "/v1/dashboard/graph/compare?a=x&b=y").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found_a"], false);
    assert_eq!(body["found_b"], false);
    assert_eq!(body["shared_count"], 0);
}
