//! `cortex-fulltext-worker` binary entrypoint.
//!
//! Wires the Meilisearch HTTP client + Synap consumer/publisher into a
//! [`Worker`] and runs the pool until SIGINT (Ctrl-C) flips the
//! shutdown flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cortex_fulltext::routing::FAMILIES;
use cortex_fulltext::{
    settings_v1_json, FulltextConfig, LiveMeiliClient, LiveSynapConsumer, LiveSynapPublisher,
    MeiliClient, MeiliFulltextIndexer, Metrics, SynapHandle, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_fulltext=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = FulltextConfig::from_env();
    tracing::info!(?config, "loaded cortex-fulltext config");

    let meili_client = Arc::new(
        LiveMeiliClient::new(&config)
            .map_err(|e| anyhow::anyhow!("failed to build Meili client: {e}"))?,
    );

    // Bootstrap every per-kind index against Meili so the worker
    // never tries to upsert into an unconfigured index. The
    // per-project (`cortex-{slug}-{family}`) uids are materialised
    // lazily by the indexer on first upsert (see
    // `MeiliFulltextIndexer::ensure_settings`), so we only need to
    // seed the legacy family set here.
    let settings = settings_v1_json().context("baked-in v1 settings unparseable")?;
    let metrics = Arc::new(Metrics::new());
    let mut seeded: Vec<String> = Vec::with_capacity(FAMILIES.len());
    for family in FAMILIES {
        let index = format!("{}{}", config.index_prefix, family);
        meili_client
            .ensure_index(&index, &settings)
            .await
            .map_err(|e| anyhow::anyhow!("ensure_index({index}): {e}"))?;
        metrics.incr_settings_bump();
        tracing::info!(index = %index, "ensured fulltext index");
        seeded.push(index);
    }

    let indexer = Arc::new(MeiliFulltextIndexer::with_ensured(
        config.clone(),
        meili_client,
        metrics.clone(),
        seeded,
    ));

    let synap = Arc::new(
        SynapHandle::new(&config.synap_url)
            .with_context(|| format!("failed to connect to Synap at {}", config.synap_url))?,
    );
    let consumer = Arc::new(LiveSynapConsumer::new(synap.clone()));
    let publisher = Arc::new(LiveSynapPublisher::new(synap));

    let worker = Arc::new(Worker::new(
        config,
        indexer,
        consumer,
        publisher,
        metrics,
    ));

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
