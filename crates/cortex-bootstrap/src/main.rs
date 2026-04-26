//! `cortex-bootstrap` binary entrypoint.
//!
//! Parses CLI args, loads the per-repo config, walks each repo through
//! the runner, and either prints a sizing block (`--estimate`) or
//! publishes events on `cortex.events.bootstrap` (default).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_bootstrap::{
    estimate_repo, format_estimate, load_config, load_for_repo, run_repo, write_atomic,
    Checkpoint, CliArgs, LiveSynapPublisher, LogFormat, MemoryPublisher, Metrics, Publisher,
    RunnerConfig, SynapHandle,
};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();
    init_tracing(args.verbose, args.log_format);

    if args.requires_repo_roots() && args.repo_roots.is_empty() {
        anyhow::bail!("at least one <REPO_ROOT> required (or pass --resume)");
    }

    // Load checkpoint (fresh or resumed).
    let mut checkpoint = if args.resume {
        cortex_bootstrap::load_checkpoint(&args.checkpoint).context("load checkpoint")?
    } else {
        Checkpoint::new(chrono::Utc::now().to_rfc3339())
    };

    // Decide which repos to run. `--only` / `--skip` filters the
    // CLI-supplied roots by their resolved id.
    let mut targets: Vec<(PathBuf, String)> = Vec::new();
    for root in &args.repo_roots {
        let id = resolve_repo_id(root)?;
        if !args.only.is_empty() && !args.only.iter().any(|n| n == &id) {
            continue;
        }
        if args.skip.iter().any(|n| n == &id) {
            continue;
        }
        targets.push((root.clone(), id));
    }
    if targets.is_empty() && !args.resume {
        anyhow::bail!("no repos selected after applying --only / --skip");
    }

    // Load global config (fallback when per-repo cortex.toml is missing).
    let _global = if let Some(ref cfg_path) = args.config {
        load_config(cfg_path).context("load --config")?
    } else {
        Default::default()
    };

    let metrics = Arc::new(Metrics::new());
    let shutdown = install_ctrlc_handler();

    if args.estimate {
        for (root, id) in &targets {
            let repo_cfg = load_for_repo(root)
                .with_context(|| format!("load cortex.toml for {id}"))?
                .cortex;
            let est = estimate_repo(root, id, &repo_cfg);
            print!("{}", format_estimate(&est));
        }
        return Ok(());
    }

    // Build the publisher: live Synap unless --dry-run is set.
    let publisher: Arc<dyn Publisher> = if args.dry_run {
        Arc::new(MemoryPublisher::new())
    } else {
        let endpoint = args
            .synap_endpoint
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:15003".to_string());
        let handle = Arc::new(
            SynapHandle::new(&endpoint)
                .with_context(|| format!("synap connect {endpoint}"))?,
        );
        Arc::new(LiveSynapPublisher::new(handle))
    };

    for (root, id) in &targets {
        if shutdown.load(Ordering::Relaxed) {
            tracing::info!("shutdown requested before repo {id}; exiting");
            break;
        }
        let cfg = load_for_repo(root)
            .with_context(|| format!("load cortex.toml for {id}"))?;
        let repo_cfg = cfg.cortex;
        let resolved_id = repo_cfg.id.clone().unwrap_or_else(|| id.clone());
        let runner_cfg = RunnerConfig {
            repo_id: resolved_id.clone(),
            stream: args.stream.clone(),
            since: args.since.clone(),
            dry_run: args.is_dry_run(),
        };
        let last_file = checkpoint
            .repos
            .get(&resolved_id)
            .and_then(|p| p.last_file.clone());
        let last_git_ref = checkpoint
            .repos
            .get(&resolved_id)
            .and_then(|p| p.last_git_ref.clone());
        match run_repo(
            root,
            &runner_cfg,
            &repo_cfg,
            publisher.clone(),
            metrics.clone(),
            &mut checkpoint,
            last_file,
            last_git_ref,
        )
        .await
        {
            Ok(report) => {
                eprintln!(
                    "[bootstrap] {}: {} events published, {} files dropped, {} commits, {:.1} s",
                    report.repo_id,
                    report.events_published,
                    report.files_dropped,
                    report.commits_walked,
                    report.duration_secs,
                );
            }
            Err(e) => {
                eprintln!("[bootstrap] {id}: failed: {e}");
                metrics.incr_errors(id, "run_repo");
            }
        }
        if let Err(e) = write_atomic(&args.checkpoint, &checkpoint) {
            tracing::warn!(error = %e, "checkpoint write failed");
        }
    }

    Ok(())
}

fn resolve_repo_id(root: &Path) -> Result<String> {
    Ok(root
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| root.display().to_string()))
}

fn init_tracing(verbose: bool, format: LogFormat) {
    let default_level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("{default_level},cortex_bootstrap={default_level}"))
    });
    let subscriber = fmt().with_env_filter(filter).with_target(true);
    match format {
        LogFormat::Json => subscriber.json().init(),
        LogFormat::Text => subscriber.init(),
    }
}

fn install_ctrlc_handler() -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = shutdown.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "ctrl_c handler install failed");
            return;
        }
        tracing::info!("ctrl-c received; flushing checkpoint and exiting");
        handle.store(true, Ordering::Relaxed);
    });
    shutdown
}
