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
    resolve_port_from_env, rules, spawn_health_listener, DEFAULT_EMBEDDER_PORT,
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

    let mut config = EmbedderConfig::from_env();
    tracing::info!(?config, "loaded cortex-embedder config");

    // Vectorizer 3.x rejects raw passwords on `Authorization: Bearer`.
    // When the password in config does NOT look like a JWT (3 dot-
    // separated segments) treat it as a username/password pair, run
    // `POST /auth/login` once, and replace the password in the config
    // with the minted JWT before building the SDK client. Subsequent
    // worker writes ride that bearer token.
    if let Some(pwd) = config.vectorizer_password.clone() {
        let looks_like_jwt = pwd.split('.').count() == 3
            && pwd.split('.').all(|s| !s.is_empty());
        if !looks_like_jwt {
            tracing::info!(
                user = %config.vectorizer_user,
                "vectorizer password does not look like a JWT — running /auth/login"
            );
            match LiveVectorizerClient::login(
                &config.vectorizer_url,
                &config.vectorizer_user,
                &pwd,
            )
            .await
            {
                Ok(jwt) => {
                    tracing::info!("vectorizer /auth/login succeeded");
                    config.vectorizer_password = Some(jwt.access_token);
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "vectorizer /auth/login failed for user `{}`: {e}",
                        config.vectorizer_user
                    ));
                }
            }
        }
    }

    // Vectorizer client — HTTP via vectorizer-sdk v3.
    let vectorizer_client = Arc::new(
        LiveVectorizerClient::new(config.clone())
            .map_err(|e| anyhow::anyhow!("failed to build Vectorizer client: {e}"))?,
    );

    // Synap handle shared by consumer + publisher so both ride the same
    // underlying TCP connection.
    let synap = Arc::new(SynapHandle::new(&config.synap_url)?);
    let consumer = Arc::new(LiveSynapConsumer::new(synap.clone()));
    let publisher = Arc::new(LiveSynapPublisher::new(synap.clone()));

    let metrics = Arc::new(Metrics::new());
    let embedder = Arc::new(VectorizerEmbedder::with_metrics(
        config.clone(),
        vectorizer_client,
        metrics.clone(),
    ));

    let worker = Arc::new(Worker::new(
        config,
        embedder,
        consumer,
        publisher,
        metrics,
    ));

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

    // Phase8a §2.8 — admin /healthz listener. Reads chunks-written
    // / vectorizer-errors counters from the shared Metrics
    // registry so the report carries the live throughput signal.
    let port = resolve_port_from_env(
        "CORTEX_EMBEDDER_HEALTH_PORT",
        DEFAULT_EMBEDDER_PORT,
    );
    let metrics_for_health = worker.metrics.clone();
    let started_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let provider: cortex_health::server::SnapshotProvider = std::sync::Arc::new(move || {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "chunks_written_total".into(),
            serde_json::json!(metrics_for_health.chunks_written_total()),
        );
        extras.insert(
            "vectorizer_errors_total".into(),
            serde_json::json!(metrics_for_health.vectorizer_errors_total()),
        );
        let (state, last_error) = rules::freshness_state(started_ms, None);
        cortex_health::server::HealthSnapshot {
            state,
            last_error,
            extras,
        }
    });
    spawn_health_listener(
        port,
        "cortex-embedder-worker",
        env!("CARGO_PKG_VERSION"),
        provider,
    );

    worker.run_pool(shutdown).await
}
