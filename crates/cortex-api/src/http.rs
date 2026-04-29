//! Axum HTTP router. Wraps [`QueryService`] in the `POST /v1/query`
//! endpoint and threads the spec-11 status codes (`200`, `400`,
//! `403`, `429`). Also exposes a small `GET /v1/status` health
//! snapshot consumed by the spec-18 `cortex.status` MCP tool.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::service::{ErrorBody, QueryService, ServiceOutcome};
use crate::types::{QueryRequest, QueryResponse};

/// Header used to identify the caller. Spec 11 §Rate limiting +
/// §Security / privacy: per-caller ACL + token bucket.
pub const CALLER_HEADER: &str = "x-cortex-caller";

/// Shared state for the router — the query service plus the
/// process-start instant so `/v1/status` can report uptime.
#[derive(Clone)]
pub struct ApiState {
    /// The query service the `/v1/query` route dispatches to.
    pub service: Arc<QueryService>,
    /// Start instant — surfaced as `uptime_ms` on the status endpoint.
    pub started_at: Arc<Instant>,
}

/// `/v1/status` response body — consumed by the spec-18
/// `cortex.status` MCP tool. Field set is intentionally small so the
/// shape is forward-compatible with future daemons (queue depth /
/// publisher errors land here later).
#[derive(Debug, Clone, Serialize)]
pub struct StatusBody {
    /// Always `"cortex-api"`.
    pub service: &'static str,
    /// Crate version baked at compile time.
    pub version: &'static str,
    /// Process pid.
    pub pid: u32,
    /// Wall-clock since service boot.
    pub uptime_ms: u64,
    /// Sorted list of repo slugs the daemon currently has signal for
    /// (derived from the keyword-lane snapshot the dashboard uses).
    /// Empty when the daemon was started without an indexed-repos
    /// lane wired through. Callers use this list to detect "this
    /// repo was never indexed" before issuing a query — see issue
    /// hivellm/cortex#1.
    pub indexed_repos: Vec<String>,
}

/// Build the router. The state Arc is cheap to clone per request.
pub fn build_router(service: Arc<QueryService>) -> Router {
    build_router_with(service, None)
}

/// `build_router` with an optional dashboard mount. Threads the
/// shared `MemoryKeywordLane` (the one the archive_loader seeds) so
/// `/v1/dashboard/*` and the `/dashboard/*` static asset route mount
/// alongside the spec-11 routes.
pub fn build_router_with(
    service: Arc<QueryService>,
    dashboard: Option<crate::dashboard::DashboardState>,
) -> Router {
    let state = ApiState {
        service,
        started_at: Arc::new(Instant::now()),
    };
    let mut router = Router::new()
        .route("/v1/query", post(handle_query))
        .route("/v1/status", get(handle_status))
        .route("/healthz", get(handle_healthz))
        .route("/v1/health", get(handle_v1_health))
        .with_state(state);
    if let Some(dash) = dashboard {
        // Phase8b — mount /v1/health/freshness + /v1/health/divergence
        // alongside the dashboard routes. Both endpoints share the
        // dashboard's `loader_metrics` Arc and a fresh aggregator
        // history so their probes stay consistent across calls.
        let health_state = crate::health::HealthState {
            aggregator: Arc::new(crate::health::HealthAggregatorState::new()),
            loader_metrics: dash.loader_metrics.clone(),
        };
        let health_router = Router::new()
            .route(
                "/v1/health/freshness",
                get(crate::health::freshness_handler),
            )
            .route(
                "/v1/health/divergence",
                get(crate::health::divergence_handler),
            )
            .with_state(health_state);
        // Phase8b — Prometheus-text `/metrics` endpoint exposing the
        // LoaderMetrics counters. The freshness aggregator already
        // surfaces the same numbers in JSON, but a parallel Prom
        // endpoint keeps cortex-api consistent with cortex-ingestion
        // (which already exposes one) so an external scraper picks
        // up every stage uniformly.
        let metrics_state = dash.loader_metrics.clone();
        let metrics_router = Router::new()
            .route(
                "/metrics",
                get({
                    let m = metrics_state.clone();
                    move || async move {
                        (
                            StatusCode::OK,
                            [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                            m.render_prom(),
                        )
                    }
                }),
            );
        router = router.merge(crate::dashboard::build_dashboard_router(dash));
        router = router.merge(health_router);
        router = router.merge(metrics_router);
    }
    router
}

async fn handle_healthz(State(state): State<ApiState>) -> Response {
    use cortex_health::{HealthState, SubsystemStatus};
    let started_at = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::milliseconds(
            i64::try_from(state.started_at.elapsed().as_millis()).unwrap_or(0),
        ))
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let mut status = SubsystemStatus::ok(
        "cortex-api",
        env!("CARGO_PKG_VERSION"),
        started_at,
    );
    let indexed_repos = state
        .service
        .indexed_repos
        .as_ref()
        .map(|lane| lane.indexed_repos())
        .unwrap_or_default();
    status.extras.insert(
        "indexed_repos".into(),
        serde_json::Value::Array(
            indexed_repos
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );
    status
        .extras
        .insert("uptime_ms".into(), serde_json::json!(state.started_at.elapsed().as_millis() as u64));
    // Phase8a §2.2 — `degraded` when the keyword-lane snapshot is
    // unavailable. The lane is the canonical source for repo
    // coverage; without it the query path returns degraded results
    // but the daemon itself is still answering.
    if state.service.indexed_repos.is_none() {
        status.state = HealthState::Degraded;
        status.last_error = Some("keyword-lane snapshot not wired (no indexed_repos source)".into());
    }
    let http_status = match status.state {
        HealthState::Ok | HealthState::Degraded => StatusCode::OK,
        HealthState::Down => StatusCode::SERVICE_UNAVAILABLE,
    };
    (http_status, Json(status)).into_response()
}

async fn handle_v1_health(State(state): State<ApiState>) -> Response {
    use cortex_health::client::{aggregate, build_client, AggregatorConfig, ProbeTarget};
    use cortex_health::SubsystemStatus;

    // Discover probe targets from env. Empty values fall back to
    // the localhost defaults so the operator gets a useful report
    // out of the box on a single-host install.
    let mut targets: Vec<ProbeTarget> = Vec::new();
    let candidates: &[(&'static str, &str, &str)] = &[
        (
            "cortex-adapter",
            "CORTEX_ADAPTER_ADMIN_URL",
            "http://127.0.0.1:17011/healthz",
        ),
        (
            "cortex-ingestion",
            "CORTEX_INGESTION_URL",
            "http://127.0.0.1:17010/v1/healthz",
        ),
        (
            "cortex-classifier-worker",
            "CORTEX_CLASSIFIER_WORKER_URL",
            "http://127.0.0.1:17021/healthz",
        ),
        (
            "cortex-embedder-worker",
            "CORTEX_EMBEDDER_WORKER_URL",
            "http://127.0.0.1:17022/healthz",
        ),
        (
            "cortex-fulltext-worker",
            "CORTEX_FULLTEXT_WORKER_URL",
            "http://127.0.0.1:17023/healthz",
        ),
        (
            "cortex-graph-worker",
            "CORTEX_GRAPH_WORKER_URL",
            "http://127.0.0.1:17024/healthz",
        ),
    ];
    for (name, key, default) in candidates {
        let url = std::env::var(key)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| (*default).to_string());
        targets.push(ProbeTarget {
            name,
            url,
        });
    }

    // Self-report — the aggregator is part of cortex-api, so we
    // emit a synthetic Ok row instead of fanning out to ourselves
    // and risking a deadlock on a busy worker pool.
    let mut self_report = SubsystemStatus::ok(
        "cortex-api",
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().to_rfc3339(),
    );
    self_report
        .extras
        .insert("uptime_ms".into(), serde_json::json!(state.started_at.elapsed().as_millis() as u64));

    let client = match build_client(&AggregatorConfig::default()) {
        Ok(c) => c,
        Err(err) => {
            // Couldn't build the HTTP client — emit a degraded
            // self-row so the operator sees the failure mode
            // explicitly.
            let report = cortex_health::client::unknown_targets_report(
                "cortex-api",
                format!("aggregator client init failed: {err}"),
            );
            return (StatusCode::OK, Json(report)).into_response();
        }
    };
    let mut report = aggregate(&client, &targets, &AggregatorConfig::default()).await;
    // Insert the self-row so the table is complete; aggregator
    // re-sorts after.
    report.subsystems.push(self_report);
    report.subsystems.sort_by(|a, b| a.name.cmp(&b.name));
    let recomputed = cortex_health::HealthReport::aggregate(report.subsystems);
    (StatusCode::OK, Json(recomputed)).into_response()
}

async fn handle_status(State(state): State<ApiState>) -> Response {
    let indexed_repos = state
        .service
        .indexed_repos
        .as_ref()
        .map(|lane| lane.indexed_repos())
        .unwrap_or_default();
    let body = StatusBody {
        service: "cortex-api",
        version: env!("CARGO_PKG_VERSION"),
        pid: std::process::id(),
        uptime_ms: u64::try_from(state.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        indexed_repos,
    };
    (StatusCode::OK, Json(body)).into_response()
}

async fn handle_query(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<QueryRequest>,
) -> Response {
    let caller = headers
        .get(CALLER_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    match state.service.handle_with_headers(&caller, req, &headers).await {
        ServiceOutcome::Ok(resp) => {
            (StatusCode::OK, Json::<QueryResponse>(*resp)).into_response()
        }
        ServiceOutcome::EmptyQuery => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                reason: "empty_query".into(),
            }),
        )
            .into_response(),
        ServiceOutcome::EmptyScope => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                reason: "scope_repo_required".into(),
            }),
        )
            .into_response(),
        ServiceOutcome::Denied => (
            StatusCode::FORBIDDEN,
            Json(ErrorBody {
                reason: "scope_forbidden".into(),
            }),
        )
            .into_response(),
        ServiceOutcome::RateLimited(retry_after) => {
            let mut hdrs = HeaderMap::new();
            hdrs.insert(
                "retry-after",
                HeaderValue::from_str(&format!("{}", retry_after.as_secs().max(1)))
                    .unwrap_or_else(|_| HeaderValue::from_static("1")),
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                hdrs,
                Json(ErrorBody {
                    reason: "rate_limited".into(),
                }),
            )
                .into_response()
        }
    }
}
