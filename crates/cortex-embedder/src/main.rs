//! `cortex-embedder-worker` binary entrypoint.
//!
//! Wires the live Vectorizer + Synap transports to [`Worker`] and hands
//! control to [`Worker::run_pool`]. A `SIGINT` (Ctrl-C) flips the shared
//! shutdown flag; each pool-worker task exits from its `run_forever` loop
//! at the next iteration boundary.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use cortex_embedder::{
    EmbedderConfig, LiveSynapConsumer, LiveSynapPublisher, LiveVectorizerClient, Metrics,
    SynapHandle, VectorizerEmbedder, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_embedder=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = EmbedderConfig::from_env();
    tracing::info!(?config, "loaded cortex-embedder config");

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

    worker.run_pool(shutdown).await
}
