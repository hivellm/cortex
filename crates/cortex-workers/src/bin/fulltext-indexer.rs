//! `cortex-fulltext-worker` binary entrypoint.
//!
//! Wires the Meilisearch HTTP client + Synap consumer/publisher into a
//! [`Worker`] and runs the pool until SIGINT (Ctrl-C) flips the
//! shutdown flag.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cortex_workers::admin_health::{
    resolve_port_from_env, rules, spawn_health_listener_with_metrics, DEFAULT_FULLTEXT_PORT,
};
use cortex_workers::fulltext::{
    replay_missing_partitions, settings_v1_json, sweep_stale_indexes, FulltextConfig,
    LiveMeiliClient, LiveSynapConsumer, LiveSynapPublisher, MeiliFulltextIndexer, Metrics,
    SynapHandle, Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_workers=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = FulltextConfig::from_env();
    tracing::info!(?config, "loaded cortex-fulltext config");

    let meili_client = Arc::new(
        LiveMeiliClient::new(&config)
            .map_err(|e| anyhow::anyhow!("failed to build Meili client: {e}"))?,
    );

    // Phase4a §3 — stale-index sweep on boot. The legacy startup
    // path used to seed every `cortex-{family}` (no slug) index for
    // schema bootstrap, but per-project uids materialise lazily on
    // first upsert via `MeiliFulltextIndexer::ensure_settings`, so
    // those legacy names were always orphans. The sweep drops every
    // empty non-canonical index and warns (without deleting)
    // anything non-empty so operator state is never lost.
    match sweep_stale_indexes(meili_client.as_ref()).await {
        Ok(report) => tracing::info!(
            examined = report.examined,
            kept_canonical = report.kept_canonical,
            deleted_stale_empty = report.deleted_stale_empty,
            kept_warning = report.kept_warning,
            warned = ?report.warned_names,
            "fulltext sweep complete",
        ),
        Err(e) => tracing::warn!(error = %e, "fulltext sweep failed; continuing without sweep"),
    }

    // The settings bag is still loaded so `MeiliFulltextIndexer`
    // can apply it lazily when each per-project uid lands its
    // first upsert. No eager `ensure_index` against the legacy
    // family set — that's exactly the source of the stale names
    // the sweep just dropped.
    let _settings = settings_v1_json().context("baked-in v1 settings unparseable")?;
    let metrics = Arc::new(Metrics::new());

    let indexer = Arc::new(MeiliFulltextIndexer::with_ensured(
        config.clone(),
        meili_client.clone(),
        metrics.clone(),
        Vec::new(),
    ));

    // Phase4f — opt-in boot-time replay-missing-partitions defense.
    // Walks the archive once after the sweep and replays any
    // (repo_slug, family) partitions that exist in the archive but not
    // in Meili. Off by default so a hot restart never triggers a
    // multi-minute archive scan; ops flips it on after a worker outage
    // or for a cold archive-only deployment.
    if std::env::var("CORTEX_FULLTEXT_REPLAY_MISSING")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
    {
        let archive_root: PathBuf = std::env::var("CORTEX_ARCHIVE_ROOT")
            .map(PathBuf::from)
            .or_else(|_| {
                std::env::var("USERPROFILE")
                    .or_else(|_| std::env::var("HOME"))
                    .map(|home| PathBuf::from(home).join(".cortex").join("archive"))
            })
            .context("CORTEX_FULLTEXT_REPLAY_MISSING=1 but no CORTEX_ARCHIVE_ROOT and no HOME/USERPROFILE")?;
        match replay_missing_partitions(
            meili_client.as_ref(),
            indexer.clone(),
            metrics.as_ref(),
            &archive_root,
            &config.index_prefix,
        )
        .await
        {
            Ok(report) => tracing::info!(
                examined_archives = report.examined_archives,
                missing_partitions = report.missing_partitions,
                replayed_events = report.replayed_events,
                latency_ms = report.latency_ms,
                "fulltext replay-missing complete",
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "fulltext replay-missing failed; continuing without replay",
            ),
        }
    }

    let synap = Arc::new(
        SynapHandle::new(&config.synap_url)
            .with_context(|| format!("failed to connect to Synap at {}", config.synap_url))?,
    );
    let consumer = Arc::new(LiveSynapConsumer::new(synap.clone()));
    let publisher = Arc::new(LiveSynapPublisher::new(synap));

    let worker = Arc::new(Worker::new(config, indexer, consumer, publisher, metrics));

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

    // Phase8a §2.9 / Phase8b §4.3 — admin /healthz + /metrics.
    let port = resolve_port_from_env("CORTEX_FULLTEXT_HEALTH_PORT", DEFAULT_FULLTEXT_PORT);
    let metrics_for_health = worker.metrics.clone();
    let started_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    // Phase8c — version block in /healthz extras.
    let version_block =
        serde_json::to_value(cortex_build::version_info!()).unwrap_or(serde_json::Value::Null);
    let provider: cortex_health::server::SnapshotProvider = std::sync::Arc::new(move || {
        let mut extras = serde_json::Map::new();
        extras.insert("version".into(), version_block.clone());
        extras.insert(
            "documents_total".into(),
            serde_json::json!(metrics_for_health.documents_total()),
        );
        extras.insert(
            "skipped_empty_total".into(),
            serde_json::json!(metrics_for_health.skipped_empty_total()),
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
        "cortex-fulltext-worker",
        env!("CARGO_PKG_VERSION"),
        provider,
        Some(renderer),
    );

    worker.run_pool(shutdown).await
}
