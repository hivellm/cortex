//! Standalone `/healthz` listener — used by worker binaries that
//! don't already own an axum router. Mount with one call:
//!
//! ```ignore
//! tokio::spawn(cortex_health::server::serve_standalone(
//!     17012,
//!     "cortex-classifier-worker",
//!     env!("CARGO_PKG_VERSION"),
//!     started_at_rfc3339,
//!     extras_provider,
//! ));
//! ```
//!
//! Crates that own a router (cortex-api, cortex-ingestion) skip
//! this module and write a short axum handler that returns the
//! same [`SubsystemStatus`] JSON shape directly.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Map;

use crate::{HealthState, SubsystemStatus};

/// Live-extras snapshot the worker hands back per `/healthz` probe.
/// Producers populate `state` from their own freshness rules
/// (queue lag, last claimed job, IPC pipe alive, …) and stamp
/// `extras` with whatever runtime signals the dashboard later
/// reads.
#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    /// Current health state. Producers MUST compute this from their
    /// own rules — `Down` when the subsystem is hard-broken,
    /// `Degraded` when a soft signal trips, otherwise `Ok`.
    pub state: HealthState,
    /// One-line reason when `state != Ok`.
    pub last_error: Option<String>,
    /// Free-form telemetry the dashboard surfaces verbatim.
    pub extras: Map<String, serde_json::Value>,
}

impl Default for HealthSnapshot {
    fn default() -> Self {
        Self {
            state: HealthState::Ok,
            last_error: None,
            extras: Map::new(),
        }
    }
}

/// Closure type the listener calls per `/healthz` probe to compute
/// the live snapshot.
pub type SnapshotProvider = Arc<dyn Fn() -> HealthSnapshot + Send + Sync>;

#[derive(Clone)]
struct ServerState {
    name: &'static str,
    version: &'static str,
    since: String,
    provider: SnapshotProvider,
}

/// Build the `/healthz` router. Exposed so callers that already own
/// an axum app (cortex-api, cortex-ingestion) can `merge()` it
/// instead of opening a separate port.
pub fn router(
    name: &'static str,
    version: &'static str,
    since: String,
    provider: SnapshotProvider,
) -> Router {
    let state = ServerState {
        name,
        version,
        since,
        provider,
    };
    Router::new()
        .route("/healthz", get(handle_healthz))
        .with_state(state)
}

async fn handle_healthz(State(state): State<ServerState>) -> impl IntoResponse {
    let snap = (state.provider)();
    let mut status = SubsystemStatus::ok(state.name, state.version, state.since.clone());
    status.state = snap.state;
    status.last_error = snap.last_error;
    status.extras = snap.extras;
    let http_status = match status.state {
        HealthState::Ok | HealthState::Degraded => StatusCode::OK,
        // Spec 18 §health: a `Down` subsystem returns 503 so naive
        // load balancers can use it as a liveness probe without
        // parsing the body.
        HealthState::Down => StatusCode::SERVICE_UNAVAILABLE,
    };
    (http_status, Json(status))
}

/// Spawn an axum server bound to `0.0.0.0:port` serving `/healthz`.
/// Awaits indefinitely — caller wraps in `tokio::spawn`. On bind
/// failure returns the I/O error so the caller can decide whether
/// to crash the worker or log + continue with degraded
/// observability.
pub async fn serve_standalone(
    port: u16,
    name: &'static str,
    version: &'static str,
    since: String,
    provider: SnapshotProvider,
) -> std::io::Result<()> {
    let app = router(name, version, since, provider);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(port, name, "health endpoint listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixed_provider(state: HealthState, queue_depth: u64) -> SnapshotProvider {
        Arc::new(move || {
            let mut extras = Map::new();
            extras.insert(
                "queue_depth".to_string(),
                serde_json::json!(queue_depth),
            );
            HealthSnapshot {
                state,
                last_error: if state == HealthState::Ok {
                    None
                } else {
                    Some("stalled".into())
                },
                extras,
            }
        })
    }

    async fn invoke_router(router: Router) -> (axum::http::StatusCode, SubsystemStatus) {
        use tower::ServiceExt;
        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("invoke");
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let body: SubsystemStatus = serde_json::from_slice(&bytes).expect("parse body");
        (status, body)
    }

    #[tokio::test]
    async fn ok_provider_returns_200_with_extras() {
        let provider = fixed_provider(HealthState::Ok, 42);
        let r = router(
            "cortex-test",
            "0.1.0",
            "2026-04-29T00:00:00Z".into(),
            provider,
        );
        let (status, body) = invoke_router(r).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body.state, HealthState::Ok);
        assert_eq!(body.name, "cortex-test");
        assert_eq!(body.version, "0.1.0");
        assert_eq!(body.last_error, None);
        assert_eq!(body.extras.get("queue_depth"), Some(&serde_json::json!(42)));
    }

    #[tokio::test]
    async fn degraded_provider_still_returns_200() {
        // Soft-degraded subsystems are still probable — a 200 lets
        // the aggregator parse the body and surface the reason
        // rather than a load balancer dropping them prematurely.
        let provider = fixed_provider(HealthState::Degraded, 99);
        let r = router("cortex-test", "0.1.0", "now".into(), provider);
        let (status, body) = invoke_router(r).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body.state, HealthState::Degraded);
        assert_eq!(body.last_error.as_deref(), Some("stalled"));
    }

    #[tokio::test]
    async fn down_provider_returns_503() {
        // Down → 503 so a naive liveness probe can route on the
        // status code without parsing the body.
        let provider = fixed_provider(HealthState::Down, 0);
        let r = router("cortex-test", "0.1.0", "now".into(), provider);
        let (status, body) = invoke_router(r).await;
        assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.state, HealthState::Down);
    }

    #[tokio::test]
    async fn provider_is_invoked_per_probe_so_extras_are_live() {
        // The aggregator polls the same listener repeatedly; each
        // probe MUST re-invoke the closure so live counters
        // (queue depth, last claimed job ts) reflect the current
        // moment — not whatever they were at startup.
        let counter = Arc::new(AtomicU64::new(0));
        let provider: SnapshotProvider = {
            let c = counter.clone();
            Arc::new(move || {
                let n = c.fetch_add(1, Ordering::SeqCst);
                let mut extras = Map::new();
                extras.insert("call".to_string(), serde_json::json!(n));
                HealthSnapshot {
                    state: HealthState::Ok,
                    last_error: None,
                    extras,
                }
            })
        };
        let r = router("cortex-test", "0.1.0", "now".into(), provider);
        let (_, body_a) = invoke_router(r.clone()).await;
        let (_, body_b) = invoke_router(r).await;
        assert_eq!(body_a.extras.get("call"), Some(&serde_json::json!(0)));
        assert_eq!(body_b.extras.get("call"), Some(&serde_json::json!(1)));
    }
}
