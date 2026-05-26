//! Axum HTTP router. Wraps [`QueryService`] in the `POST /v1/query`
//! endpoint and threads the spec-11 status codes (`200`, `400`,
//! `403`, `429`). Also exposes a small `GET /v1/status` health
//! snapshot consumed by the spec-18 `cortex.status` MCP tool.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
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
    /// Phase11m §1 — Nexus client shared with the dashboard so the
    /// coverage handler can include a `nexus` backend in the diff.
    /// `None` when the daemon was started without a Nexus URL.
    pub nexus: Option<Arc<nexus_sdk::NexusClient>>,
    /// Phase13e §3.2 — typed Config snapshot the handlers read
    /// instead of re-calling `std :: env :: var ("CORTEX_*")` on every
    /// request. Builders default this to `Config::load()` /
    /// `Config::default()`; main.rs threads the boot-time snapshot
    /// in via the `_with_cfg` builder variants so the API and the
    /// dashboard agree on the same resolution.
    pub cfg: Arc<cortex_config::Config>,
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
    ///
    /// **Phase11e note:** this list reflects whatever the keyword
    /// lane has seen, which is NOT the same as what every backend
    /// hosts. Use `coverage` for the per-backend honest view.
    pub indexed_repos: Vec<String>,
    /// Phase11e §6 — per-backend collection / index inventory
    /// summary. `None` when no coverage audit has run yet (cold
    /// boot before the boot-time audit completes, or unit tests).
    /// Populated from the latest snapshot the boot-time audit
    /// recorded; the full diff is at `/v1/health/coverage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageBackendSummaries>,
}

/// Compact per-backend coverage summary embedded in `/v1/status`.
/// Designed to fit comfortably inside a `cortex_status` MCP response
/// without crossing the transport's size cap; the full diff lives
/// at `/v1/health/coverage`.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageBackendSummaries {
    /// Overall severity across all backends (`ok` / `warn` /
    /// `critical`). Mirrors the field on `/v1/health/coverage`.
    pub overall_severity: &'static str,
    /// Per-backend summary, one row per probed backend.
    pub backends: Vec<CoverageBackendSummary>,
    /// Pointer to the full diff endpoint so callers know where to
    /// read the per-collection breakdown when they need it.
    pub details_endpoint: &'static str,
}

/// One row of [`CoverageBackendSummaries`].
#[derive(Debug, Clone, Serialize)]
pub struct CoverageBackendSummary {
    /// `vectorizer` or `meili`.
    pub backend: &'static str,
    /// `ok` / `warn` / `critical`.
    pub severity: &'static str,
    /// Total expected collection / index names.
    pub expected: usize,
    /// Subset of expected that the live backend hosts.
    pub present: usize,
    /// Subset of expected that is missing.
    pub missing: usize,
    /// Live names that are NOT in the expected set (orphans).
    pub unexpected: usize,
}

/// Build the router. The state Arc is cheap to clone per request.
pub fn build_router(service: Arc<QueryService>) -> Router {
    build_router_with(service, None)
}

/// `build_router_with` mounting the dashboard but no opt-in auth.
/// Phase3 §7.4 callers (the daemon's `main.rs`) prefer
/// [`build_router_with_auth`] so the dashboard sub-router can be
/// guarded by `require_api_key` when `CORTEX_DASHBOARD_AUTH=1`.
pub fn build_router_with(
    service: Arc<QueryService>,
    dashboard: Option<crate::dashboard::DashboardState>,
) -> Router {
    build_router_with_auth(service, dashboard, None)
}

/// Phase3 §7.4 — full router builder. When `api_key_store` is
/// `Some(..)` and `CORTEX_DASHBOARD_AUTH=1` the dashboard sub-router
/// is wrapped with the `require_api_key` middleware (the
/// `/v1/status`, `/healthz`, and `/v1/query` routes always stay
/// anonymous). When the env var is unset/false the wrapper is a
/// no-op, so localhost dev keeps the legacy zero-auth behaviour.
pub fn build_router_with_auth(
    service: Arc<QueryService>,
    dashboard: Option<crate::dashboard::DashboardState>,
    api_key_store: Option<Arc<crate::storage::api_keys::ApiKeyStore>>,
) -> Router {
    let cfg = Arc::new(cortex_config::Config::load().unwrap_or_default());
    build_router_with_auth_and_cfg(service, dashboard, api_key_store, cfg)
}

/// Phase13e §3.2 — variant that accepts a pre-resolved Config so
/// the daemon and tests share the same snapshot the boot path used.
pub fn build_router_with_auth_and_cfg(
    service: Arc<QueryService>,
    dashboard: Option<crate::dashboard::DashboardState>,
    api_key_store: Option<Arc<crate::storage::api_keys::ApiKeyStore>>,
    cfg: Arc<cortex_config::Config>,
) -> Router {
    let state = ApiState {
        service,
        started_at: Arc::new(Instant::now()),
        nexus: dashboard.as_ref().and_then(|d| d.nexus.clone()),
        cfg,
    };
    let mut router = Router::new()
        .route("/v1/query", post(handle_query))
        .route("/v1/status", get(handle_status))
        .route("/v1/audit/{query_id}", get(handle_audit))
        .route("/v1/ingest", post(crate::ingest_proxy::handle_ingest))
        .route("/healthz", get(handle_healthz))
        .route("/v1/health", get(handle_v1_health))
        .route("/v1/health/coverage", get(handle_health_coverage))
        .route("/v1/admin/forget", post(handle_admin_forget))
        .route(
            "/v1/search/keyword",
            post(crate::search_proxy::handle_keyword_search),
        )
        .route(
            "/v1/search/vector",
            post(crate::search_proxy::handle_vector_search),
        )
        .route(
            "/v1/search/graph",
            post(crate::search_proxy::handle_graph_query),
        )
        .route(
            "/v1/search/events",
            post(crate::events_by_kind::handle_events_by_kind),
        )
        .route(
            "/v1/search/tool-calls",
            post(crate::tool_calls::handle_tool_calls),
        )
        .route(
            "/v1/search/files-touched",
            post(crate::files_touched::handle_window_files_touched),
        )
        .route(
            "/v1/sessions/{session_id}/timeline",
            get(crate::dashboard::handle_session_timeline),
        )
        .route(
            "/v1/sessions/{session_id}/files-touched",
            get(crate::files_touched::handle_session_files_touched),
        )
        .route(
            "/v1/topic-cards/search",
            post(crate::topic_search::handle_topic_search),
        )
        .route(
            "/v1/consolidations/recent",
            get(crate::consolidations_recent::handle_consolidations_recent),
        )
        .route(
            "/v1/consolidations/{id}",
            get(crate::consolidation_get::handle_consolidation_get),
        )
        .with_state(state);
    // phase13g §2.3 — similar-sessions route. Lives on its own
    // sub-router because the handler reads from
    // `SimilarSessionsState` (a `dyn ConsolidationSearchSource`)
    // rather than the global `ApiState`. The default state carries
    // the honest-empty source so the route returns `{ hits: [], total: 0 }`
    // until main.rs wires a live Vectorizer lane.
    router = router.merge(crate::similar_sessions::build_router(
        crate::similar_sessions::SimilarSessionsState::default(),
    ));
    // phase13g §3.3 — decision-chain route. Same lazy-init pattern
    // as similar-sessions: the boot-time default is the
    // honest-empty `UnwiredDecisionChainSource`; a live Nexus
    // walker is wired by main.rs in a follow-up.
    router = router.merge(crate::decision_chain::build_router(
        crate::decision_chain::DecisionChainState::default(),
    ));
    // phase14a §4.1 — consolidator health endpoint. Default state
    // carries the honest-empty source until the daemon (phase14a
    // §3) writes `consolidation_reports` rows main.rs can read.
    router = router.merge(crate::health::consolidator::build_router(
        crate::health::consolidator::ConsolidatorHealthState::default(),
    ));
    // phase14e — pre-thinking circuit-breaker health endpoint.
    // Default state carries the unwired source (closed + zero
    // counters); main.rs threads a `LivePreThinkingHealthSource`
    // over the shared `Arc<Breaker>` + `Arc<Metrics>` when wired.
    router = router.merge(crate::health::pre_thinking::build_router(
        crate::health::pre_thinking::PreThinkingHealthState::default(),
    ));
    // phase14f — feedback ingest endpoint. Default state carries
    // an in-memory MetadataStore so the route is mountable
    // standalone in tests; main.rs threads the shared metadata
    // handle when wiring the full router.
    {
        use std::sync::Arc;
        use tokio::sync::Mutex;
        let metadata = Arc::new(Mutex::new(
            cortex_storage::MetadataStore::open_in_memory()
                .expect("phase14f: open in-memory metadata for default feedback state"),
        ));
        router = router.merge(crate::feedback::build_router(
            crate::feedback::FeedbackState { metadata },
        ));
    }
    if let Some(dash) = dashboard {
        // Phase8b — mount /v1/health/freshness + /v1/health/divergence
        // alongside the dashboard routes. Both endpoints share the
        // dashboard's `loader_metrics` Arc and a fresh aggregator
        // history so their probes stay consistent across calls.
        // Phase8c — additionally mount /v1/health/versions and
        // capture workspace HEAD once at boot.
        let health_state = crate::health::HealthState {
            aggregator: Arc::new(crate::health::HealthAggregatorState::new()),
            loader_metrics: dash.loader_metrics.clone(),
            head_sha: crate::health::HeadSha::resolve_now().map(Arc::new),
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
            .route("/v1/health/versions", get(crate::health::versions_handler))
            .route("/v1/health/config", get(handle_health_config))
            .route("/v1/health/stream", get(crate::health::stream_handler))
            .with_state(health_state);
        // Phase8b — Prometheus-text `/metrics` endpoint exposing the
        // LoaderMetrics counters. The freshness aggregator already
        // surfaces the same numbers in JSON, but a parallel Prom
        // endpoint keeps cortex-api consistent with cortex-ingestion
        // (which already exposes one) so an external scraper picks
        // up every stage uniformly.
        let metrics_state = dash.loader_metrics.clone();
        let metrics_router = Router::new().route(
            "/metrics",
            get({
                let m = metrics_state.clone();
                move || async move {
                    (
                        StatusCode::OK,
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; version=0.0.4",
                        )],
                        m.render_prom(),
                    )
                }
            }),
        );
        let dashboard_router = crate::dashboard::build_dashboard_router(dash);
        let dashboard_router = if let Some(store) = api_key_store.clone() {
            // Phase3 §7.4 — only the dashboard sub-router is
            // wrapped. health_router + metrics_router stay
            // anonymous so liveness probes don't require a key.
            crate::auth::wrap_dashboard_router(dashboard_router, store)
        } else {
            dashboard_router
        };
        // Phase3 §7.1 — apply the dashboard CORS layer. Permissive
        // default for localhost dev; tight allow-list when
        // CORTEX_API_ALLOWED_ORIGINS is set.
        let dashboard_router = dashboard_router.layer(crate::auth::dashboard_cors_layer());
        router = router.merge(dashboard_router);
        router = router.merge(health_router);
        router = router.merge(metrics_router);
    }
    // Phase3 §7.1 — apply CORS to the FULL router so `/v1/health`,
    // `/v1/health/coverage`, `/v1/query`, `/v1/status`, `/v1/audit/*`
    // (registered on the base router above) also carry
    // Access-Control-Allow-Origin. Without this, the GUI in the Vite
    // dev server (5173) hits `net::ERR_FAILED` on every health poll
    // even though the dashboard sub-router already had CORS.
    router = router.layer(crate::auth::dashboard_cors_layer());
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
    let mut status = SubsystemStatus::ok("cortex-api", env!("CARGO_PKG_VERSION"), started_at);
    // Phase8c — version block (git_sha + build_ts + git_dirty) so
    // the drift aggregator can flag stale running binaries.
    let version_block =
        serde_json::to_value(cortex_build::version_info!()).unwrap_or(serde_json::Value::Null);
    status.extras.insert("version".into(), version_block);
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
    status.extras.insert(
        "uptime_ms".into(),
        serde_json::json!(state.started_at.elapsed().as_millis() as u64),
    );
    // Phase8a §2.2 — `degraded` when the keyword-lane snapshot is
    // unavailable. The lane is the canonical source for repo
    // coverage; without it the query path returns degraded results
    // but the daemon itself is still answering.
    if state.service.indexed_repos.is_none() {
        status.state = HealthState::Degraded;
        status.last_error =
            Some("keyword-lane snapshot not wired (no indexed_repos source)".into());
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
        // The env vars share their names with the base-URL contract
        // used by ingest_proxy / pre_thinking, so when the operator
        // overrides them with a bare base URL (e.g.
        // `http://cortex-ingestion:17010`), the aggregator must
        // append the healthz path itself. We do that by extracting
        // the suffix from the default URL — that keeps both
        // contracts aligned without requiring a parallel env var.
        let raw = std::env::var(key)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| (*default).to_string());
        let suffix = default
            .splitn(4, '/')
            .nth(3)
            .map(|s| format!("/{s}"))
            .unwrap_or_default();
        let url = if raw.contains("/healthz") || raw.contains("/health") {
            // Operator (or default) supplied the full path already.
            raw
        } else if !suffix.is_empty() {
            format!("{}{}", raw.trim_end_matches('/'), suffix)
        } else {
            raw
        };
        targets.push(ProbeTarget { name, url });
    }

    // Self-report — the aggregator is part of cortex-api, so we
    // emit a synthetic Ok row instead of fanning out to ourselves
    // and risking a deadlock on a busy worker pool.
    let mut self_report = SubsystemStatus::ok(
        "cortex-api",
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().to_rfc3339(),
    );
    self_report.extras.insert(
        "uptime_ms".into(),
        serde_json::json!(state.started_at.elapsed().as_millis() as u64),
    );

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

/// Phase11e — `GET /v1/health/coverage` runs a live diff between
/// the routing-matrix expected collection / index set and what the
/// Vectorizer + Meilisearch actually host. Same code path the
/// boot-time inventory uses; consumers (dashboard Health view,
/// `cortex-ops doctor-coverage` CLI) read the JSON to surface
/// missing collections inline.
async fn handle_health_coverage(State(state): State<ApiState>) -> Response {
    use std::collections::BTreeSet;
    use std::time::Duration;

    let mut slugs: BTreeSet<String> = state
        .service
        .indexed_repos
        .as_ref()
        .map(|lane| lane.indexed_repos())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let env_slugs = crate::coverage::slugs_from_env(state.cfg.dashboard.coverage_slugs.as_deref());
    slugs.extend(env_slugs);
    // Allow-list override: when `CORTEX_COVERAGE_SLUGS_ONLY` is set,
    // restrict the audit to those slugs only. Stops the lane-derived
    // set from including classifier-stamped language tags (rust /
    // python / typescript / …) that aren't real repos and inflate
    // the "missing" count under every backend.
    let only_set =
        crate::coverage::slugs_from_env(state.cfg.dashboard.coverage_slugs_only.as_deref());
    if !only_set.is_empty() {
        slugs.retain(|s| only_set.contains(s));
    }
    if slugs.is_empty() {
        slugs.insert("cortex".to_string());
    }

    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("reqwest builder: {e}"),
                })),
            )
                .into_response();
        }
    };

    let vectorizer_url = state
        .cfg
        .embedder
        .vectorizer_url
        .clone()
        .or_else(|| std::env::var("VECTORIZER_URL").ok());
    let bearer = match vectorizer_url.as_deref() {
        Some(url) => resolve_vectorizer_bearer_for_audit(&http, url, &state.cfg).await,
        None => None,
    };
    let meili_url = state.cfg.meili.meili_url.clone();
    let meili_key = state
        .cfg
        .meili
        .meili_api_key
        .clone()
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let archive_watchers = crate::coverage::resolve_archive_watcher_urls();
    let response = crate::coverage::run_coverage_audit_full(
        &http,
        slugs,
        vectorizer_url.as_deref(),
        bearer.as_deref(),
        meili_url.as_deref(),
        meili_key.as_deref(),
        state.nexus.as_deref(),
        &archive_watchers,
        Duration::from_secs(5),
    )
    .await;
    (StatusCode::OK, Json(response)).into_response()
}

/// Phase11e — resolve a Vectorizer bearer for the on-demand audit
/// endpoint. Mirrors the boot-time precedence in
/// `cortex-api/src/main.rs::resolve_vectorizer_bearer`.
async fn resolve_vectorizer_bearer_for_audit(
    http: &reqwest::Client,
    base_url: &str,
    cfg: &cortex_config::Config,
) -> Option<String> {
    if let Some(key) = cfg
        .embedder
        .vectorizer_api_key
        .clone()
        .or_else(|| std::env::var("VECTORIZER_API_KEY").ok())
    {
        if !key.is_empty() {
            return Some(key);
        }
    }
    if cfg.embedder.vectorizer_user.is_empty() {
        return None;
    }
    let user = cfg.embedder.vectorizer_user.clone();
    let pass = cfg.embedder.vectorizer_password.clone()?;
    let url = format!("{}/auth/login", base_url.trim_end_matches('/'));
    let body = serde_json::json!({ "username": user, "password": pass });
    let resp = http
        .post(&url)
        .timeout(std::time::Duration::from_secs(5))
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let j: serde_json::Value = resp.json().await.ok()?;
    j.get("access_token")
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Phase8d — `GET /v1/health/config` answers with the same audit
/// `cortex-ops doctor-config` produces. Read-only static analysis;
/// no external probes, so the call is cheap (file reads only). The
/// dashboard renders the audit findings as a table; CI can curl
/// the endpoint and gate on `worst_severity`.
async fn handle_health_config(_state: State<crate::health::HealthState>) -> Response {
    // The audit is pure-function over local file paths; no aggregator
    // history needed. Run inside `spawn_blocking` so the reqwest /
    // axum runtime never blocks on a slow filesystem.
    let audit = tokio::task::spawn_blocking(crate::config_audit::audit_default)
        .await
        .unwrap_or_default();
    (StatusCode::OK, Json(audit)).into_response()
}

/// Phase10j — `GET /v1/audit/{query_id}` returns the recorded audit
/// envelope for the named query. Backed by the in-memory ring buffer
/// the service writes alongside the synap publisher; a 404 means
/// either the query never ran on this daemon (process was restarted
/// or the request hit a different replica) or the envelope has aged
/// out of the ring buffer.
///
/// Response shape: the canonical envelope produced by
/// [`crate::audit::build_envelope_with_rewrite_context`] enriched
/// with a `lanes` array derived from `debug.lanes` so the agent can
/// pinpoint which retrieval lane returned zero hits without having
/// to walk the original query response.
async fn handle_audit(State(state): State<ApiState>, Path(query_id): Path<String>) -> Response {
    let q = query_id.trim();
    if q.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                reason: "query_id_required".into(),
            }),
        )
            .into_response();
    }
    match state.service.audit_store.get(q) {
        Some(envelope) => (StatusCode::OK, Json(envelope)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                reason: "audit_envelope_not_found".into(),
            }),
        )
            .into_response(),
    }
}

async fn handle_status(State(state): State<ApiState>) -> Response {
    let indexed_repos = state
        .service
        .indexed_repos
        .as_ref()
        .map(|lane| lane.indexed_repos())
        .unwrap_or_default();
    // Phase11e §6 — surface a per-backend coverage summary alongside
    // the legacy `indexed_repos` field. The keyword-lane snapshot
    // alone hides the embedder/fulltext asymmetry the operator needs
    // to see (e.g. Meili has 29 indexes but Vectorizer has 4).
    let coverage = if let Some(handle) = state.service.coverage_snapshot.as_ref() {
        let guard = handle.read().await;
        guard.as_ref().map(summarize_coverage)
    } else {
        None
    };
    let body = StatusBody {
        service: "cortex-api",
        version: env!("CARGO_PKG_VERSION"),
        pid: std::process::id(),
        uptime_ms: u64::try_from(state.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        indexed_repos,
        coverage,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Phase11e §6 — collapse a full [`crate::coverage::CoverageResponse`]
/// down to the compact per-backend summary embedded in `/v1/status`.
/// Operators pull the full diff from `/v1/health/coverage` when they
/// need the per-collection breakdown.
fn summarize_coverage(full: &crate::coverage::CoverageResponse) -> CoverageBackendSummaries {
    let backends = full
        .backends
        .iter()
        .map(|b| CoverageBackendSummary {
            backend: b.backend,
            severity: b.severity,
            expected: b.expected_count,
            present: b.present_count,
            missing: b.missing_count,
            unexpected: b.unexpected_count,
        })
        .collect();
    CoverageBackendSummaries {
        overall_severity: full.overall_severity,
        backends,
        details_endpoint: "/v1/health/coverage",
    }
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
    match state
        .service
        .handle_with_headers(&caller, req, &headers)
        .await
    {
        ServiceOutcome::Ok(resp) => (StatusCode::OK, Json::<QueryResponse>(*resp)).into_response(),
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

/// Phase11t §1.4 — `POST /v1/admin/forget` handler. Builds the
/// live backend ops on-demand (rare admin endpoint — per-request
/// instantiation is fine) and forwards to
/// [`crate::admin_forget::handle_forget`].
async fn handle_admin_forget(
    State(state): State<ApiState>,
    Json(req): Json<crate::admin_forget::ForgetRequest>,
) -> Response {
    use std::path::PathBuf;
    use std::sync::Arc;

    // Resolve config from the typed cortex_config snapshot the
    // builder threaded onto ApiState. Missing knobs land 500 with a
    // structured reason so the operator can fix the deployment
    // instead of guessing.
    let archive_root: PathBuf = match state.cfg.ingestion.archive_root.as_deref() {
        Some(s) if !s.is_empty() => PathBuf::from(s),
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "reason": "missing_config",
                    "message": "CORTEX_ARCHIVE_ROOT is not set",
                })),
            )
                .into_response()
        }
    };

    let nexus = match state.nexus.as_ref() {
        Some(n) => n.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "reason": "missing_config",
                    "message": "Nexus client not configured on this cortex-api instance",
                })),
            )
                .into_response()
        }
    };

    // Vectorizer client: build inline using the embedder config
    // env vars. Resolves URL via `CORTEX_VECTORIZER_URL` /
    // `CORTEX_EMBEDDER_VECTORIZER_URL` (priority order matches the
    // boot path). When user+password are present, calls
    // `with_credentials` so the client mints a JWT via
    // `/auth/login` instead of presenting the password as a raw
    // bearer (the SDK forwards `vectorizer_password` as `api_key`,
    // which the server rejects with 401 unless it's a real JWT).
    // Failure here is a transient deployment issue, not a cascade
    // error — surface as 500 with a clear reason.
    let mut embed_cfg = cortex_workers::embedder::EmbedderConfig::default();
    let vectorizer_url = state
        .cfg
        .embedder
        .vectorizer_url
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("VECTORIZER_URL")
                .ok()
                .filter(|s| !s.is_empty())
        });
    if let Some(url) = vectorizer_url.clone() {
        embed_cfg.vectorizer_url = url;
    }
    let vectorizer_user = if state.cfg.embedder.vectorizer_user.is_empty() {
        None
    } else {
        Some(state.cfg.embedder.vectorizer_user.clone())
    };
    let vectorizer_pw = state
        .cfg
        .embedder
        .vectorizer_password
        .clone()
        .filter(|s| !s.is_empty());
    let vec_client = match (vectorizer_user, vectorizer_pw, vectorizer_url) {
        (Some(user), Some(pw), Some(url)) => {
            // Login path — mint a JWT then build the client.
            let creds = cortex_workers::embedder::vectorizer_client::VectorizerCredentials {
                base_url: url,
                username: user,
                password: pw,
            };
            match cortex_workers::embedder::vectorizer_client::LiveVectorizerClient::with_credentials(
                embed_cfg, creds,
            )
            .await
            {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "reason": "vectorizer_client",
                            "message": format!("with_credentials login failed: {e}"),
                        })),
                    )
                        .into_response()
                }
            }
        }
        _ => {
            // Legacy / dev path: no creds, build the unauthenticated
            // client. Cascade calls that need auth will surface 401
            // and the caller's safety-gated retry kicks in.
            match cortex_workers::embedder::vectorizer_client::LiveVectorizerClient::new(embed_cfg)
            {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "reason": "vectorizer_client",
                            "message": e.to_string(),
                        })),
                    )
                        .into_response()
                }
            }
        }
    };

    // Meili client: same shape — pull URL + key from env, build
    // the LiveMeiliClient (which carries the §1.3 MeiliPruneOps
    // impl).
    let mut meili_cfg = cortex_workers::fulltext::FulltextConfig::default();
    if let Some(url) = state.cfg.meili.meili_url.as_deref() {
        if !url.is_empty() {
            meili_cfg.meili_url = url.to_string();
        }
    }
    if let Some(key) = state.cfg.meili.meili_api_key.as_deref() {
        if !key.is_empty() {
            meili_cfg.meili_api_key = Some(key.to_string());
        }
    }
    let meili_client =
        match cortex_workers::fulltext::meili_client::LiveMeiliClient::new(&meili_cfg) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "reason": "meili_client",
                        "message": e.to_string(),
                    })),
                )
                    .into_response()
            }
        };

    let nexus_purger = crate::admin_forget::LiveNexusPurger::new(nexus);
    let archive_purger = crate::admin_forget::LiveArchivePurger::new(&archive_root);

    let outcome = crate::admin_forget::handle_forget(
        &req,
        &archive_root,
        vec_client.as_ref(),
        &meili_client,
        &nexus_purger,
        &archive_purger,
        cortex_storage::names::INDEX_CONSOLIDATIONS,
        // ADR-012 §4.3 — identity-index handle plumbing lands
        // alongside §4.1 (doctor rewrite), which also adds the
        // `MetadataStore` field to `ApiState`. Until then the
        // forget cascade keeps the legacy archive-derived
        // dispatch + skips the identity row delete cleanly.
        None,
    )
    .await;

    match outcome {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(err) => {
            let (code, body) = crate::admin_forget::forget_error_response(&err);
            let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, Json(body)).into_response()
        }
    }
}
