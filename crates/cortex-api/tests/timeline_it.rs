//! Phase18 §4.7 — in-process integration tests for the timeline +
//! entity-history + supersession HTTP surfaces.
//!
//! All three handlers (`handle_timeline`, `handle_entity_history`,
//! `handle_entity_supersession`) check that `ApiState.nexus` is
//! `Some(_)` before doing any further work.  When the router is built
//! via `cortex_api::build_router(svc)` the nexus field is `None`, so
//! every request returns 503 with `reason = "nexus_unconfigured"`.
//!
//! These tests therefore assert:
//!   1. The route is correctly wired (path params parse, query strings
//!      are accepted without a 400/404).
//!   2. The nexus-unconfigured guard fires and returns 503.
//!
//! Real row-level assertions (actual timeline entries, supersession
//! chains, etc.) require a live Nexus and belong to the gated
//! live-stack variant controlled by `CORTEX_TIMELINE_IT=1`.  That
//! path is stubbed here as an early-return placeholder, mirroring how
//! `decision_lookup_it.rs` gates its live path behind
//! `CORTEX_GOVERNANCE_IT`.

use std::env;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::{
    build_router, AclStore, AuditStore, InMemoryCache, MemoryAuditPublisher, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryService, RateConfig, RateLimiter,
};
use serde_json::Value;
use tower::ServiceExt;

fn build_service() -> Arc<QueryService> {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(v, k, g);
    let audit = Arc::new(MemoryAuditPublisher::new());
    let svc = QueryService {
        orchestrator,
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit,
        audit_store: Arc::new(AuditStore::new()),
        indexed_repos: None,
        coverage_snapshot: None,

        principal_store: None,

        api_key_store: None,
    };
    Arc::new(svc)
}

async fn get(uri: &str) -> (StatusCode, Value) {
    let app = build_router(build_service());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

#[tokio::test]
async fn timeline_nexus_unconfigured() {
    let (status, json) = get("/v1/timeline/cortex").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={json}");
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

#[tokio::test]
async fn timeline_with_filters_still_routes_to_handler() {
    // Verifies that the full query-string surface (branch, kind, from,
    // to, as_of, limit) is accepted and reaches the handler without a
    // 400/404.  With a live Nexus this would exercise every filter.
    let (status, json) = get("/v1/timeline/cortex\
         ?branch=feature-x\
         &kind=adr\
         &from=2026-01-01\
         &to=2026-06-01\
         &as_of=2026-03-01\
         &limit=10")
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={json}");
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

#[tokio::test]
async fn entity_history_nexus_unconfigured() {
    let (status, json) = get("/v1/entity/ADR-0042/history").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={json}");
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

#[tokio::test]
async fn entity_history_with_as_of_routes() {
    // Verifies that the optional `as_of` and `limit` query params are
    // accepted and the request reaches the handler.
    let (status, json) = get("/v1/entity/ADR-0042/history?as_of=2026-03-01&limit=5").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={json}");
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

#[tokio::test]
async fn entity_supersession_nexus_unconfigured() {
    let (status, json) = get("/v1/entity/ADR-0042/supersession").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={json}");
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

#[tokio::test]
async fn live_stack_gate_is_a_noop_without_env() {
    if env::var("CORTEX_TIMELINE_IT").ok().as_deref() == Some("1") {
        eprintln!("CORTEX_TIMELINE_IT=1 — live-stack path not implemented in this IT");
        return;
    }
    // When the env var is absent the live gate is a no-op; assert the
    // route surface still answers in-process so future implementors
    // know where to hook the live-stack assertions in.
    let (status, json) = get("/v1/timeline/cortex").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={json}");
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}
