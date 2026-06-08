//! `cortex-graph-worker` binary entrypoint.
//!
//! Wires the live Nexus client to a [`NexusGraphWriter`] and hands the
//! resulting writer to [`Worker::run_pool`]. A `SIGINT` (Ctrl-C) flips
//! the shared shutdown flag; the worker exits at the next iteration
//! boundary.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cortex_storage::MetadataStore;
use cortex_workers::admin_health::{
    resolve_port_from_env, rules, spawn_health_listener_with_metrics, DEFAULT_GRAPH_PORT,
};
use cortex_workers::graph::{
    cypher::{load_from_dir, REQUIRED_TEMPLATES},
    schema,
    worker::{LiveSynapConsumer, LiveSynapPublisher, SynapHandle, STREAM_ENRICHED},
    GraphClient, GraphConfig, LiveNexusClient, Metrics, NexusGraphWriter, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_workers=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = GraphConfig::from_env();
    tracing::info!(?config, "loaded cortex-graph config");

    let cypher_dir = cortex_config::Config::load()
        .ok()
        .and_then(|c| c.nexus.cypher_dir)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/cortex-graph/cypher"));
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

    // phase25 §3 — apply the graph schema (constraints + the single-prop
    // MERGE-key indexes that Nexus NodeIndexSeek / index-backed MERGE
    // need) on startup. The live worker previously NEVER ensured schema
    // (only the backfill did), so every node/edge MERGE fell back to an
    // O(N) label scan and melted Nexus under write load — the original
    // meltdown. Idempotent (`CREATE ... IF NOT EXISTS`); also restores
    // indexes dropped by a Nexus restart (nexus#11) on the worker's next
    // boot. A schema failure is fatal: running un-indexed is worse than
    // not running.
    {
        let stmts = schema::statements();
        client
            .ensure_schema(&stmts)
            .await
            .map_err(|e| anyhow::anyhow!("ensure graph schema: {e}"))?;
        tracing::info!(statements = stmts.len(), "ensured graph schema (indexes)");
    }

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
    // Phase11s §2.3 — open the metadata store so the consumer's
    // offset survives container restarts. `CORTEX_GRAPH_METADATA_DB`
    // overrides the path; legacy default is `${CORTEX_HOME}/metadata.sqlite`.
    let metadata_path = resolve_metadata_db_path();
    let metadata = Arc::new(Mutex::new(
        MetadataStore::open(&metadata_path)
            .with_context(|| format!("open metadata store at {metadata_path:?}"))?,
    ));
    // Phase11s §2.3 — `consumer_id` partitions the offset ledger so
    // multiple graph-worker replicas can share the SQLite file
    // without colliding. Default to the env hostname / container
    // name; fall back to "cortex-graph-0" for single-instance dev.
    let consumer_id = cortex_config::Config::load()
        .ok()
        .and_then(|c| c.nexus.consumer_id)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "cortex-graph-0".to_string());
    let consumer = Arc::new(
        LiveSynapConsumer::with_persistent_offset(
            synap.clone(),
            metadata.clone(),
            &consumer_id,
            STREAM_ENRICHED,
        )
        .with_context(|| format!("build durable graph consumer ({consumer_id})"))?,
    );
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

/// Phase11s §2.3 — resolve the SQLite metadata path the durable
/// consumer-offset ledger lives in. Precedence matches the
/// classifier worker so a single-host deploy points every worker
/// at the same DB.
/// 1. `CORTEX_GRAPH_METADATA_DB` (graph-worker-only override).
/// 2. `CORTEX_METADATA_DB` (shared override).
/// 3. `${CORTEX_HOME}/metadata.sqlite` when `CORTEX_HOME` is set.
/// 4. `<home>/.cortex/metadata.sqlite` (cross-platform default).
fn resolve_metadata_db_path() -> PathBuf {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(p) = cfg.nexus.metadata_db.as_deref() {
        return PathBuf::from(p);
    }
    if let Some(p) = cfg.ingestion.metadata_db.as_deref() {
        return PathBuf::from(p);
    }
    if let Some(home) = cfg.ingestion.home.as_deref() {
        return PathBuf::from(home).join("metadata.sqlite");
    }
    home_dir().join(".cortex").join("metadata.sqlite")
}

fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}
