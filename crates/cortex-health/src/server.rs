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

/// Closure type the listener calls per `/metrics` scrape to render
/// the Prometheus text-format payload. Phase8b — the freshness +
/// divergence aggregators read counters out of `/healthz` extras,
/// but a parallel `/metrics` endpoint keeps the Prometheus story
/// consistent with `cortex-ingestion` (which already exposes one)
/// so an external scraper can collect lock-step counters.
pub type MetricsRenderer = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Clone)]
struct ServerState {
    name: &'static str,
    version: &'static str,
    since: String,
    provider: SnapshotProvider,
    metrics: Option<MetricsRenderer>,
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
    router_with_metrics(name, version, since, provider, None)
}

/// Build the `/healthz` router with an optional `/metrics`
/// Prometheus-text endpoint mounted on the same listener. Phase8b —
/// every long-running Cortex binary exposes `/metrics` so the
/// per-stage counters land in a uniform scrape surface.
pub fn router_with_metrics(
    name: &'static str,
    version: &'static str,
    since: String,
    provider: SnapshotProvider,
    metrics: Option<MetricsRenderer>,
) -> Router {
    let state = ServerState {
        name,
        version,
        since,
        provider,
        metrics,
    };
    let mut router = Router::new().route("/healthz", get(handle_healthz));
    if state.metrics.is_some() {
        router = router.route("/metrics", get(handle_metrics));
    }
    router.with_state(state)
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

async fn handle_metrics(State(state): State<ServerState>) -> impl IntoResponse {
    let body = match state.metrics.as_ref() {
        Some(renderer) => renderer(),
        // Should never reach this branch — `/metrics` route is only
        // registered when `metrics` is `Some`. Defense-in-depth.
        None => String::new(),
    };
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
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
    serve_standalone_with_metrics(port, name, version, since, provider, None).await
}

/// `serve_standalone` with an optional Prometheus-text `/metrics`
/// renderer mounted on the same port. Phase8b — workers and the
/// adapter expose stage counters via this path.
pub async fn serve_standalone_with_metrics(
    port: u16,
    name: &'static str,
    version: &'static str,
    since: String,
    provider: SnapshotProvider,
    metrics: Option<MetricsRenderer>,
) -> std::io::Result<()> {
    let app = router_with_metrics(name, version, since, provider, metrics);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(port, name, "health endpoint listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(std::io::Error::other)
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
            extras.insert("queue_depth".to_string(), serde_json::json!(queue_depth));
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
    async fn metrics_route_renders_when_provider_set() {
        // Phase8b — when a metrics renderer is wired, the same
        // listener exposes `/metrics` with the renderer's output.
        // When it isn't, the route is absent entirely so an external
        // scraper sees a 404 rather than an empty body.
        use tower::ServiceExt;
        let provider = fixed_provider(HealthState::Ok, 0);
        let renderer: MetricsRenderer = Arc::new(|| "# TYPE foo counter\nfoo 7\n".to_string());
        let r = router_with_metrics(
            "cortex-test",
            "0.1.0",
            "now".into(),
            provider,
            Some(renderer),
        );
        let resp = r
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("foo 7"));
    }

    #[tokio::test]
    async fn metrics_route_absent_when_no_renderer_wired() {
        use tower::ServiceExt;
        let provider = fixed_provider(HealthState::Ok, 0);
        let r = router("cortex-test", "0.1.0", "now".into(), provider);
        let resp = r
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
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
