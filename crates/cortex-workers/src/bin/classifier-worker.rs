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
use cortex_classifier::{
    build_offline_stack, build_stack, BudgetTracker, Classifier, ClassifierCache, ClassifierStack,
    HaikuCliClassifier, HaikuCliConfig, InMemoryCache, PricingTable,
};
use cortex_workers::classifier::{
    ClassifierMode, ClassifierWorkerConfig, LiveSynapConsumer, LiveSynapPublisher, SynapHandle,
    Worker,
};
use cortex_storage::MetadataStore;
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
    // now" — exit 0 before consuming the input streams or opening
    // a Synap connection. Process supervisors that loop on
    // non-zero exit (systemd, docker restart=always, …) treat this
    // as healthy so they don't churn the binary. Re-enable by
    // flipping the env to `cli` (LLM-backed) or `static`
    // (deterministic offline fallback) and restarting the worker.
    if matches!(config.mode, ClassifierMode::Disabled) {
        tracing::info!(
            "CORTEX_CLASSIFIER_MODE=disabled — classifier worker exiting cleanly without consuming events"
        );
        return Ok(());
    }

    let synap = Arc::new(
        SynapHandle::new(&config.synap_url)
            .with_context(|| format!("synap connect {}", config.synap_url))?,
    );
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

    worker.run_pool(shutdown).await
}

/// Resolve the SQLite metadata database path. Precedence:
/// 1. `CORTEX_METADATA_DB` (full path override).
/// 2. `${CORTEX_HOME}/metadata.sqlite` when `CORTEX_HOME` is set.
/// 3. `<home>/.cortex/metadata.sqlite` (cross-platform default).
fn resolve_metadata_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("CORTEX_METADATA_DB") {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("CORTEX_HOME") {
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
