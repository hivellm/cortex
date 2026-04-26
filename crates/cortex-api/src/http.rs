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
        .with_state(state);
    if let Some(dash) = dashboard {
        router = router.merge(crate::dashboard::build_dashboard_router(dash));
    }
    router
}

async fn handle_status(State(state): State<ApiState>) -> Response {
    let body = StatusBody {
        service: "cortex-api",
        version: env!("CARGO_PKG_VERSION"),
        pid: std::process::id(),
        uptime_ms: u64::try_from(state.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
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
    match state.service.handle(&caller, req).await {
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
