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
use cortex_workers::admin_health::{
    resolve_port_from_env, rules, spawn_health_listener_with_metrics, DEFAULT_GRAPH_PORT,
};
use cortex_workers::graph::{
    cypher::{load_from_dir, REQUIRED_TEMPLATES},
    worker::{LiveSynapConsumer, LiveSynapPublisher, SynapHandle},
    GraphConfig, LiveNexusClient, Metrics, NexusGraphWriter, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_workers=info"));
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

    let synap = Arc::new(
        SynapHandle::new(&config.synap_url)
            .with_context(|| format!("failed to connect to Synap at {}", config.synap_url))?,
    );
    let consumer = Arc::new(LiveSynapConsumer::new(synap.clone()));
    let publisher = Arc::new(LiveSynapPublisher::new(synap));

    let worker = Arc::new(Worker::new(config, writer, consumer, publisher, metrics));

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

    // Phase8a §2.10 / Phase8b §4.4 — admin /healthz + /metrics.
    let port = resolve_port_from_env("CORTEX_GRAPH_HEALTH_PORT", DEFAULT_GRAPH_PORT);
    let metrics_for_health = worker.metrics.clone();
    let started_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    // Phase8c — version block in /healthz extras.
    let version_block =
        serde_json::to_value(cortex_build::version_info!()).unwrap_or(serde_json::Value::Null);
    let provider: cortex_health::server::SnapshotProvider = std::sync::Arc::new(move || {
        let mut extras = serde_json::Map::new();
        extras.insert("version".into(), version_block.clone());
        extras.insert(
            "edges_dropped_total".into(),
            serde_json::json!(metrics_for_health.edges_dropped_total()),
        );
        extras.insert(
            "jobs_processed_total".into(),
            serde_json::json!(metrics_for_health.jobs_processed_total()),
        );
        let last_job_ts_ms = metrics_for_health.last_job_ts_ms();
        extras.insert("last_job_ts_ms".into(), serde_json::json!(last_job_ts_ms));
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
    let renderer: cortex_health::server::MetricsRenderer =
        std::sync::Arc::new(move || metrics_for_prom.render_prom());
    spawn_health_listener_with_metrics(
        port,
        "cortex-graph-worker",
        env!("CARGO_PKG_VERSION"),
        provider,
        Some(renderer),
    );

    worker.run_pool(shutdown).await
}
