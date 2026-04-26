//! `cortex-graph-worker` binary entrypoint.
//!
//! Wires the live Nexus client to a [`NexusGraphWriter`] and hands the
//! resulting writer to [`Worker::run_pool`]. A `SIGINT` (Ctrl-C) flips
//! the shared shutdown flag; the worker exits at the next iteration
//! boundary.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cortex_graph::{
    cypher::{load_from_dir, REQUIRED_TEMPLATES},
    GraphConfig, LiveNexusClient, Metrics, NexusGraphWriter, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_graph=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = GraphConfig::from_env();
    tracing::info!(?config, "loaded cortex-graph config");

    let cypher_dir = std::env::var("CORTEX_GRAPH_CYPHER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("crates/cortex-graph/cypher"));
    let templates_loaded = load_from_dir(&cypher_dir)
        .with_context(|| format!("failed to load cypher templates from {:?}", cypher_dir))?;
    templates_loaded
        .ensure_required(REQUIRED_TEMPLATES)
        .with_context(|| format!("required cypher templates missing under {:?}", cypher_dir))?;
    let templates = Arc::new(templates_loaded);
    tracing::info!(template_count = templates.len(), "loaded cypher templates");

    let client = Arc::new(
        LiveNexusClient::new(config.clone())
            .map_err(|e| anyhow::anyhow!("failed to build Nexus client: {e}"))?,
    );
    let metrics = Arc::new(Metrics::new());
    let writer = Arc::new(NexusGraphWriter::new(
        config.clone(),
        client,
        templates,
        metrics.clone(),
    ));

    let worker = Arc::new(Worker::new(config, writer, metrics));

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
