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
use cortex_classifier_worker::{
    ClassifierMode, ClassifierWorkerConfig, LiveSynapConsumer, LiveSynapPublisher, SynapHandle,
    Worker,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cortex_classifier_worker=info"));
    fmt().with_env_filter(filter).with_target(true).init();

    let config = ClassifierWorkerConfig::from_env();
    tracing::info!(?config, "loaded cortex-classifier-worker config");

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

    let worker = Arc::new(Worker::with_stack(config, stack, consumer, publisher));

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
    }
}
