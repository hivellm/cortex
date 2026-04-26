//! Axum HTTP router. Wraps [`QueryService`] in the `POST /v1/query`
//! endpoint and threads the spec-11 status codes (`200`, `400`,
//! `403`, `429`).

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};

use crate::service::{ErrorBody, QueryService, ServiceOutcome};
use crate::types::{QueryRequest, QueryResponse};

/// Header used to identify the caller. Spec 11 §Rate limiting +
/// §Security / privacy: per-caller ACL + token bucket.
pub const CALLER_HEADER: &str = "x-cortex-caller";

/// Build the router. The state Arc is cheap to clone per request.
pub fn build_router(service: Arc<QueryService>) -> Router {
    Router::new()
        .route("/v1/query", post(handle_query))
        .with_state(service)
}

async fn handle_query(
    State(service): State<Arc<QueryService>>,
    headers: HeaderMap,
    Json(req): Json<QueryRequest>,
) -> Response {
    let caller = headers
        .get(CALLER_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    match service.handle(&caller, req).await {
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
