//! Phase14a §5.2 — `/v1/health/consolidator` route-mount IT.
//!
//! Exercises the endpoint through the same `build_router_with`
//! entry point the daemon binary mounts. The default state carries
//! `UnwiredConsolidatorHealthSource` so a cold stack answers with
//! the all-empty `ConsolidatorHealthReport` shape — the IT pins:
//!
//! 1. The route is mounted at `GET /v1/health/consolidator`.
//! 2. The response is HTTP 200 + valid JSON.
//! 3. The body has the three documented grain keys
//!    (`session_grain`, `topic_grain`, `decision_trace_grain`) each
//!    carrying the four documented fields with default values when
//!    no run has landed (omitted optional fields, zero counters).
//! 4. The handler returns a custom source's snapshot verbatim when
//!    one is wired (proves the ADR-014 pure-reader contract end-to-
//!    end through the router stack, not just the handler unit).

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use cortex_api::health::consolidator::{
    build_router as build_health_router, ConsolidatorHealthReport, ConsolidatorHealthSource,
    ConsolidatorHealthState, GrainHealth,
};
use cortex_api::{
    build_router_with, DashboardState, LoaderMetrics, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryService, TaskLoader,
};
use serde_json::Value;
use tower::ServiceExt;

fn build_test_router() -> Router {
    let lane = Arc::new(MemoryKeywordLane::new());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(vector, lane.clone(), graph);
    let service =
        Arc::new(QueryService::with_memory_defaults(orchestrator).with_indexed_repos(lane.clone()));
    let dashboard = DashboardState {
        lane,
        nexus: None,
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

async fn get_json(router: Router, uri: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, parsed)
}

#[tokio::test]
async fn consolidator_endpoint_is_mounted_with_default_unwired_source() {
    let router = build_test_router();
    let (status, body) = get_json(router, "/v1/health/consolidator").await;
    assert_eq!(status, StatusCode::OK, "endpoint must answer 200");
    assert!(body.is_object(), "response is a JSON object");

    for key in ["session_grain", "topic_grain", "decision_trace_grain"] {
        let grain = body.get(key).unwrap_or_else(|| panic!("missing {key}"));
        // last_run / last_status are skip_serializing_if=None per
        // ConsolidatorHealthReport. The two counters always render.
        assert!(grain.get("last_run").is_none(), "{key} last_run absent");
        assert!(
            grain.get("last_status").is_none(),
            "{key} last_status absent"
        );
        assert_eq!(
            grain.get("envelopes_emitted").and_then(|v| v.as_u64()),
            Some(0),
            "{key} envelopes_emitted defaults to 0"
        );
        assert_eq!(
            grain.get("latency_ms").and_then(|v| v.as_u64()),
            Some(0),
            "{key} latency_ms defaults to 0"
        );
    }
}

struct FixedSource(ConsolidatorHealthReport);

#[async_trait]
impl ConsolidatorHealthSource for FixedSource {
    async fn snapshot(&self) -> ConsolidatorHealthReport {
        self.0.clone()
    }
}

#[tokio::test]
async fn consolidator_endpoint_returns_custom_source_snapshot_verbatim() {
    // Mount only the consolidator sub-router with a fixture source —
    // this exercises the ADR-014 pure-reader contract end-to-end
    // through tower + axum, not just the handler unit.
    let fixture = ConsolidatorHealthReport {
        session_grain: GrainHealth {
            last_run: Some("2026-05-25T12:00:00Z".parse().expect("rfc3339 fixture")),
            last_status: Some("success".into()),
            envelopes_emitted: 4,
            latency_ms: 1_500,
        },
        topic_grain: GrainHealth {
            last_run: Some("2026-05-24T03:00:00Z".parse().expect("rfc3339 fixture")),
            last_status: Some("failed".into()),
            envelopes_emitted: 0,
            latency_ms: 200,
        },
        ..ConsolidatorHealthReport::default()
    };
    let state = ConsolidatorHealthState {
        source: Arc::new(FixedSource(fixture.clone())),
    };
    let router = build_health_router(state);
    let (status, body) = get_json(router, "/v1/health/consolidator").await;
    assert_eq!(status, StatusCode::OK);

    let session = body.get("session_grain").expect("session_grain present");
    assert_eq!(session["last_run"], "2026-05-25T12:00:00Z");
    assert_eq!(session["last_status"], "success");
    assert_eq!(session["envelopes_emitted"], 4);
    assert_eq!(session["latency_ms"], 1_500);

    let topic = body.get("topic_grain").expect("topic_grain present");
    assert_eq!(topic["last_run"], "2026-05-24T03:00:00Z");
    assert_eq!(topic["last_status"], "failed");

    let dt = body
        .get("decision_trace_grain")
        .expect("decision_trace_grain present");
    assert!(dt.get("last_run").is_none(), "default grain skips last_run");
    assert!(
        dt.get("last_status").is_none(),
        "default grain skips last_status"
    );
}
