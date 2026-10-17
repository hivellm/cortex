//! Runtime capabilities and the daily profile's HTTP boundary.

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Observe local SQLite availability without probing optional services.
pub(crate) async fn capabilities(state: &ApiState) -> Value {
    let memory = match &state.daily_memory {
        Some(local) => {
            match local
                .run(|store| {
                    store
                        .daily_memory_revision(&cortex_storage::daily_memory::MemoryScope::Global)
                        .map(|_| ())
                })
                .await
            {
                Ok(()) => "ready",
                Err(_) => "unavailable",
            }
        }
        None => "unavailable",
    };
    let bulk = if state.cfg.is_daily() {
        "disabled_by_profile"
    } else {
        "configured_unverified"
    };
    json!({
        "daily_memory": {
            "state": memory,
            "probe": "sqlite_open_and_read",
            "checked_at": chrono::Utc::now().to_rfc3339(),
            "delivery_consumption": "not_observed"
        },
        "claude_adapter": {"state": "unprobed", "required_for": "claude_lifecycle_only"},
        "bulk_indexers": {"state": bulk},
        "bulk_monitoring": {"state": bulk},
        "scheduled_maintenance": {"state": bulk},
        "query_enrichment": {"state": bulk},
        "jev": {"state": "on_demand_unverified", "required": false},
        "archive": {
            "state": if state.cfg.ingestion.archive_incremental_enabled {
                "opted_in_unverified"
            } else {
                "disabled"
            },
            "details": "/v1/status#archive"
        }
    })
}

/// Daily mode exposes the local memory loop, local query, ACL and basic health.
/// Legacy bulk endpoints may reload environment configuration internally, so
/// enforce this boundary before their handlers or extractors can run.
pub(crate) async fn daily_routes(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let allowed = matches!(
        path,
        "/healthz"
            | "/metrics"
            | "/v1/status"
            | "/v1/health"
            | "/v1/query"
            | "/v1/ingest"
            | "/v1/memory/query"
            | "/v1/pre-thinking/feedback"
            | "/v1/feedback/list"
            | "/v1/dashboard/active-work"
    ) || path.starts_with("/v1/acl/")
        || path.starts_with("/v1/audit/");
    if allowed {
        next.run(request).await
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "reason": "disabled_by_profile",
                "profile": "daily",
                "detail": "This optional endpoint requires the full runtime profile."
            })),
        )
            .into_response()
    }
}
