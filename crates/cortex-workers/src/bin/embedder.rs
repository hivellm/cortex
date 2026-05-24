//! `cortex-embedder-worker` binary entrypoint.
//!
//! Wires the live Vectorizer + Synap transports to [`Worker`] and hands
//! control to [`Worker::run_pool`]. A `SIGINT` (Ctrl-C) flips the shared
//! shutdown flag; each pool-worker task exits from its `run_forever` loop
//! at the next iteration boundary.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use cortex_workers::admin_health::{
    resolve_port_from_env, rules, spawn_health_listener_with_metrics, DEFAULT_EMBEDDER_PORT,
};
use cortex_workers::embedder::{
    EmbedderConfig, LiveSynapConsumer, LiveSynapPublisher, LiveVectorizerClient, Metrics,
    SynapHandle, VectorizerEmbedder, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_workers=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = EmbedderConfig::from_env();
    tracing::info!(?config, "loaded cortex-embedder config");

    // Phase11s §3.2 — the 2026-05-03 incident showed
    // `LiveVectorizerClient::new` taking the JWT once at boot and
    // never refreshing — every embed call returned 401 after the
    // 1-hour TTL. The `with_credentials` path runs `/auth/login`
    // at construction AND retains the username + password so the
    // client refreshes the JWT 60s before expiry. The legacy
    // direct-JWT path stays available for callers that supply a
    // pre-minted token (test paths, single-shot CLIs).
    let vectorizer_client = if let Some(pwd) = config.vectorizer_password.clone() {
        let looks_like_jwt = pwd.split('.').count() == 3 && pwd.split('.').all(|s| !s.is_empty());
        if looks_like_jwt {
            tracing::info!("vectorizer password is a pre-minted JWT — auto-refresh disabled");
            Arc::new(
                LiveVectorizerClient::new(config.clone())
                    .map_err(|e| anyhow::anyhow!("failed to build Vectorizer client: {e}"))?,
            )
        } else {
            tracing::info!(
                user = %config.vectorizer_user,
                "vectorizer password is a credential — building auto-refreshing client"
            );
            let credentials = cortex_workers::embedder::vectorizer_client::VectorizerCredentials {
                base_url: config.vectorizer_url.clone(),
                username: config.vectorizer_user.clone(),
                password: pwd,
            };
            Arc::new(
                LiveVectorizerClient::with_credentials(config.clone(), credentials)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "vectorizer with_credentials failed for user `{}`: {e}",
                            config.vectorizer_user
                        )
                    })?,
            )
        }
    } else {
        Arc::new(
            LiveVectorizerClient::new(config.clone())
                .map_err(|e| anyhow::anyhow!("failed to build Vectorizer client: {e}"))?,
        )
    };

    // Synap handle shared by consumer + publisher so both ride the same
    // underlying TCP connection.
    let synap = Arc::new(SynapHandle::new(&config.synap_url)?);
    let consumer = Arc::new(LiveSynapConsumer::new(synap.clone()));
    let publisher = Arc::new(LiveSynapPublisher::new(synap.clone()));

    let metrics = Arc::new(Metrics::new());
    // Phase11s §3.4 — clone the Arc so the /healthz closures below
    // can read the token cache while the embedder owns its own
    // reference for the upsert path.
    let token_cache_for_health_arc = vectorizer_client.token_cache().clone();
    let embedder = Arc::new(VectorizerEmbedder::with_metrics(
        config.clone(),
        vectorizer_client,
        metrics.clone(),
    ));

    let worker = Arc::new(Worker::new(config, embedder, consumer, publisher, metrics));

    // Graceful shutdown — Ctrl-C flips the flag, each `run_forever` loop
    // notices at the next iteration boundary.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_handle = shutdown.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "failed to install ctrl_c handler");
            return;
        }
        tracing::info!("ctrl-c received; initiating shutdown");
        shutdown_handle.store(true, Ordering::Relaxed);
    });

    // Phase8a §2.8 / Phase8b §4.2 — admin /healthz + /metrics.
    // Reads jobs_processed / chunks_written / vectorizer_errors from
    // the shared Metrics registry. Freshness uses the per-job
    // last_job_ts (when the worker has processed at least one job)
    // and falls back to the boot timestamp until then.
    let port = resolve_port_from_env("CORTEX_EMBEDDER_HEALTH_PORT", DEFAULT_EMBEDDER_PORT);
    let metrics_for_health = worker.metrics.clone();
    // Phase11s §3.4 — alias the cache Arc captured before the
    // embedder took ownership of the client.
    let token_cache_for_health = token_cache_for_health_arc.clone();
    let started_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    // Phase8c — capture version block once; closure clones the JSON
    // each probe (cheap; stamps git_sha / build_ts / dirty).
    let version_block =
        serde_json::to_value(cortex_build::version_info!()).unwrap_or(serde_json::Value::Null);
    let provider: cortex_health::server::SnapshotProvider = std::sync::Arc::new(move || {
        let mut extras = serde_json::Map::new();
        extras.insert("version".into(), version_block.clone());
        extras.insert(
            "chunks_written_total".into(),
            serde_json::json!(metrics_for_health.chunks_written_total()),
        );
        extras.insert(
            "vectorizer_errors_total".into(),
            serde_json::json!(metrics_for_health.vectorizer_errors_total()),
        );
        extras.insert(
            "jobs_processed_total".into(),
            serde_json::json!(metrics_for_health.jobs_processed_total()),
        );
        let last_job_ts_ms = metrics_for_health.last_job_ts_ms();
        extras.insert("last_job_ts_ms".into(), serde_json::json!(last_job_ts_ms));
        // Phase11s §3.4 — JWT-refresh telemetry. Distinct from the
        // freshness signal because a stale jobs counter does not
        // imply a token issue (and vice versa); operators read
        // these alongside `vectorizer_errors_total` to attribute
        // 401s to either auth-rotation lag or upstream auth misconfig.
        extras.insert(
            "last_login_ts".into(),
            serde_json::json!(token_cache_for_health.last_login_ts_ms()),
        );
        extras.insert(
            "jwt_refresh_total".into(),
            serde_json::json!(token_cache_for_health.refreshes_total()),
        );
        extras.insert(
            "jwt_refresh_errors_total".into(),
            serde_json::json!(token_cache_for_health.refresh_errors_total()),
        );
        let activity_ts = if last_job_ts_ms > 0 {
            last_job_ts_ms
        } else {
            started_ms
        };
        let (state, last_error) = rules::freshness_state(activity_ts, None);
        cortex_health::server::HealthSnapshot {
            state,
            last_error,
            extras,
        }
    });
    let metrics_for_prom = worker.metrics.clone();
    let token_cache_for_prom = token_cache_for_health_arc.clone();
    let renderer: cortex_health::server::MetricsRenderer = std::sync::Arc::new(move || {
        // Phase11s §3.4 — append JWT-refresh gauges to the
        // existing Prometheus body.
        let mut body = metrics_for_prom.render_prom();
        use std::fmt::Write as _;
        body.push_str("# TYPE cortex_embedder_jwt_last_login_ts_ms gauge\n");
        let _ = writeln!(
            body,
            "cortex_embedder_jwt_last_login_ts_ms {}",
            token_cache_for_prom.last_login_ts_ms()
        );
        body.push_str("# TYPE cortex_embedder_jwt_refresh_total counter\n");
        let _ = writeln!(
            body,
            "cortex_embedder_jwt_refresh_total {}",
            token_cache_for_prom.refreshes_total()
        );
        body.push_str("# TYPE cortex_embedder_jwt_refresh_errors_total counter\n");
        let _ = writeln!(
            body,
            "cortex_embedder_jwt_refresh_errors_total {}",
            token_cache_for_prom.refresh_errors_total()
        );
        body
    });
    spawn_health_listener_with_metrics(
        port,
        "cortex-embedder-worker",
        env!("CARGO_PKG_VERSION"),
        provider,
        Some(renderer),
    );

    worker.run_pool(shutdown).await
}
