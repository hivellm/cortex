//! `cortex-classifier-worker` binary entrypoint.
//!
//! Wires the live Synap consumer/publisher to a [`ClassifierStack`]
//! (`Budgeted ← Cached ← backend`) and runs the pool until ctrl-c.
//! The backend defaults to `StaticClassifier` (offline, deterministic);
//! `CORTEX_CLASSIFIER_MODE=cli` opts into `HaikuCliClassifier`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cortex_storage::MetadataStore;
use cortex_workers::admin_health::{
    resolve_port_from_env, rules, spawn_health_listener_with_metrics_and_router,
    DEFAULT_CLASSIFIER_PORT,
};
use cortex_workers::classifier::{
    build_offline_stack, build_stack, BudgetTracker, Classifier, ClassifierCache, ClassifierStack,
    HaikuCliClassifier, HaikuCliConfig, InMemoryCache, PricingTable,
};
use cortex_workers::classifier_worker::{
    ClassifierMode, ClassifierWorkerConfig, LiveSynapConsumer, LiveSynapPublisher, SynapHandle,
    Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_workers=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = ClassifierWorkerConfig::from_env();
    tracing::info!(?config, "loaded cortex-classifier-worker config");

    // `CORTEX_CLASSIFIER_MODE=disabled` (or `off` / `none`) is the
    // operator escape hatch for "don't run classification right
    // now" — skip consuming the input streams and opening a Synap
    // connection, but DO NOT exit. Idle until a stop signal instead.
    //
    // Exiting here (even with code 0) churns the container under the
    // common supervisor restart policies: docker `restart: always`
    // and `restart: unless-stopped` both restart on ANY exit code —
    // only `on-failure` respects a clean exit. An earlier version of
    // this guard returned `Ok(())` on the wrong assumption that exit 0
    // is supervisor-friendly, which produced a ~per-minute restart
    // loop (RestartCount climbing forever) for a worker that was meant
    // to sit quietly. Parking the process keeps the container `Up` and
    // quiet. `docker stop` (SIGTERM) still terminates it normally.
    // Re-enable by flipping the env to `cli` (LLM-backed) or `static`
    // (deterministic offline fallback) and recreating the worker.
    if matches!(config.mode, ClassifierMode::Disabled) {
        tracing::info!(
            "CORTEX_CLASSIFIER_MODE=disabled — classifier worker idling (no event consumption); send SIGINT/SIGTERM to stop"
        );
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("ctrl-c received; classifier worker (disabled mode) exiting");
        return Ok(());
    }

    let synap = Arc::new(
        SynapHandle::new(&config.synap_url)
            .with_context(|| format!("synap connect {}", config.synap_url))?,
    );
    // Phase29 / synap 1.0 — declare every room this worker touches
    // BEFORE the consume loop starts. Synap 1.0 rejects consumes on a
    // room that was never created ("Invalid request: Room not found"),
    // and the poll loop turned that into server-side ERROR spam every
    // ~200ms whenever no bootstrap run had ever created
    // `cortex.events.bootstrap` on a fresh stack. `get_or_create_room`
    // is the SDK's idempotent declare — safe on every startup.
    for room in [
        cortex_workers::classifier_worker::worker::STREAM_RAW,
        cortex_workers::classifier_worker::worker::STREAM_BOOTSTRAP,
        cortex_workers::classifier_worker::worker::STREAM_ENRICHED,
    ] {
        synap
            .streams()
            .get_or_create_room(room, None)
            .await
            .with_context(|| format!("declare synap room {room}"))?;
    }
    let consumer = Arc::new(LiveSynapConsumer::new(synap.clone()));
    let publisher = Arc::new(LiveSynapPublisher::new(synap.clone()));

    let budget = Arc::new(BudgetTracker::new(
        config.daily_limit_cents,
        PricingTable::HAIKU_4_5,
    ));
    let cache: Box<dyn ClassifierCache> = Box::new(InMemoryCache::default());
    let stack = build_stack_for_mode(&config, cache, budget);

    let metadata_path = resolve_metadata_db_path();
    let metadata = match MetadataStore::open(&metadata_path) {
        Ok(store) => {
            tracing::info!(
                metadata_db = %metadata_path.display(),
                "metadata store opened — classifier_spend_hourly will accumulate"
            );
            Some(Arc::new(std::sync::Mutex::new(store)))
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                metadata_db = %metadata_path.display(),
                "metadata store unavailable — running without spend recording"
            );
            None
        }
    };

    let mut worker = Worker::with_stack(config, stack, consumer, publisher);
    if let Some(store) = metadata {
        worker = worker.with_metadata(store);
    }
    let worker = Arc::new(worker);

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_handle = shutdown.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "ctrl_c handler install failed");
            return;
        }
        tracing::info!("ctrl-c received; initiating shutdown");
        shutdown_handle.store(true, Ordering::Relaxed);
    });

    // Phase8a §2.7 — admin /healthz listener. The classifier
    // worker doesn't carry a metric registry yet, so the snapshot
    // reports the process as alive (Synap connection succeeded
    // above — otherwise we'd have errored out before reaching
    // here) and surfaces the configured worker pool size as an
    // extra. The freshness rule keeps the report honest: until
    // the worker observes its first message, the state stays
    // `Degraded` with a "warming up" reason.
    let port = resolve_port_from_env("CORTEX_CLASSIFIER_HEALTH_PORT", DEFAULT_CLASSIFIER_PORT);
    let started_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let worker_for_health = worker.clone();
    // Phase8c — version block in /healthz extras.
    let version_block =
        serde_json::to_value(cortex_build::version_info!()).unwrap_or(serde_json::Value::Null);
    let provider: cortex_health::server::SnapshotProvider = std::sync::Arc::new(move || {
        let mut extras = serde_json::Map::new();
        extras.insert("version".into(), version_block.clone());
        extras.insert("workers_configured".into(), serde_json::json!(2u64));
        extras.insert(
            "jobs_processed_total".into(),
            serde_json::json!(worker_for_health.jobs_processed_total()),
        );
        let last_job_ts_ms = worker_for_health.last_job_ts_ms();
        extras.insert("last_job_ts_ms".into(), serde_json::json!(last_job_ts_ms));
        // Phase11s §1.3 — surface the consume-loop liveness fields
        // so `cortex-ops doctor` can flag a stuck worker even
        // when the process is alive but the consume loop is dead.
        let last_consume_ts_ms = worker_for_health.last_consume_ts_ms();
        extras.insert(
            "last_consume_ts_ms".into(),
            serde_json::json!(last_consume_ts_ms),
        );
        extras.insert(
            "consume_errors_consecutive".into(),
            serde_json::json!(worker_for_health.consume_errors_consecutive()),
        );
        // Use `last_consume_ts_ms` as the freshness signal — the
        // 2026-05-02 incident showed `last_job_ts_ms` stays fresh
        // when the consume loop hasn't returned a non-empty batch
        // for hours; the consume timestamp is the load-bearing
        // probe.
        let activity_ts = if last_consume_ts_ms > 0 {
            last_consume_ts_ms
        } else if last_job_ts_ms > 0 {
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
    let worker_for_prom = worker.clone();
    let renderer: cortex_health::server::MetricsRenderer =
        std::sync::Arc::new(move || worker_for_prom.render_prom());
    spawn_health_listener_with_metrics_and_router(
        port,
        "cortex-classifier-worker",
        env!("CARGO_PKG_VERSION"),
        provider,
        Some(renderer),
        // Phase28 (retrieval-eval-gate-live §3) — POST /v1/classify
        // for the cortex-eval classification suite.
        Some(cortex_workers::classifier::http::classify_router()),
    );

    worker.run_pool(shutdown).await
}

/// Resolve the SQLite metadata database path. Precedence:
/// 1. `CORTEX_METADATA_DB` (full path override).
/// 2. `${CORTEX_HOME}/metadata.sqlite` when `CORTEX_HOME` is set.
/// 3. `<home>/.cortex/metadata.sqlite` (cross-platform default).
fn resolve_metadata_db_path() -> PathBuf {
    let cfg = cortex_config::Config::load().unwrap_or_default();
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

/// Build the production classifier stack that matches the configured mode.
fn build_stack_for_mode(
    config: &ClassifierWorkerConfig,
    cache: Box<dyn ClassifierCache>,
    budget: Arc<BudgetTracker>,
) -> ClassifierStack {
    match config.mode {
        ClassifierMode::Static => build_offline_stack(cache, budget),
        ClassifierMode::Cli => {
            let cli_cfg = HaikuCliConfig {
                claude_bin: PathBuf::from(&config.claude_bin),
                model: config.model.clone(),
                timeout: std::time::Duration::from_secs(config.cli_timeout_secs),
                ..Default::default()
            };
            let backend: Box<dyn Classifier> = Box::new(HaikuCliClassifier::new(cli_cfg));
            build_stack(backend, cache, budget, config.prompt_version.clone())
        }
        // `main` short-circuits with a clean exit before this
        // match runs whenever the operator sets
        // `CORTEX_CLASSIFIER_MODE=disabled`, so reaching this arm
        // would mean the early-exit guard was lost in a refactor.
        // `unreachable!` makes that bug loud instead of silently
        // running the worker against an unconfigured backend.
        ClassifierMode::Disabled => unreachable!(
            "build_stack_for_mode called with Disabled — main() should have exited 0 before now"
        ),
    }
}
