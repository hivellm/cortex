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
use cortex_cli::bootstrap::{
    current_head_sha, estimate_repo, format_estimate, load_config, load_for_repo, load_workspace,
    open_archive_writer, preflight_workspace, run_repo, run_repo_graph_static, write_atomic,
    Checkpoint, CliArgs, GraphStaticReport, LiveSynapPublisher, LogFormat, MemoryPublisher,
    Metrics, Publisher, RepoRunReport, RunnerConfig, SynapHandle,
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
        cortex_cli::bootstrap::load_checkpoint(&args.checkpoint).context("load checkpoint")?
    } else {
        Checkpoint::new(chrono::Utc::now().to_rfc3339())
    };

    // Decide which repos to run. `--workspace` expands a TOML
    // listing of repos; positional args still work (and merge with
    // the workspace, deduped on path). `--only` / `--skip` filter
    // the resulting list by repo id.
    let mut targets: Vec<(PathBuf, String, Option<PathBuf>)> = Vec::new();
    if let Some(ws_path) = args.workspace.as_deref() {
        let ws = load_workspace(ws_path)
            .with_context(|| format!("load workspace `{}`", ws_path.display()))?;
        preflight_workspace(&ws).context("workspace preflight")?;
        for repo in &ws.repos {
            let cfg_override = repo.config.as_ref().map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    repo.path.join(p)
                }
            });
            targets.push((repo.path.clone(), repo.id.clone(), cfg_override));
        }
    }
    for root in &args.repo_roots {
        let id = resolve_repo_id(root)?;
        if targets.iter().any(|(p, _, _)| p == root) {
            continue;
        }
        targets.push((root.clone(), id, None));
    }
    targets.retain(|(_, id, _)| {
        if !args.only.is_empty() && !args.only.iter().any(|n| n == id) {
            return false;
        }
        if args.skip.iter().any(|n| n == id) {
            return false;
        }
        true
    });
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
        for (root, id, cfg_override) in &targets {
            let repo_cfg = match cfg_override {
                Some(p) => {
                    load_config(p)
                        .with_context(|| format!("load workspace cortex.toml for {id}"))?
                        .cortex
                }
                None => {
                    load_for_repo(root)
                        .with_context(|| format!("load cortex.toml for {id}"))?
                        .cortex
                }
            };
            let est = estimate_repo(root, id, &repo_cfg);
            print!("{}", format_estimate(&est));
        }
        return Ok(());
    }

    if args.apply_settings_only {
        return run_apply_settings_only(&args, &targets).await;
    }

    if args.graph_static {
        let archive_root = args.graph_archive_root.as_deref().ok_or_else(|| {
            anyhow::anyhow!("--graph-static requires --graph-archive-root <PATH>")
        })?;
        std::fs::create_dir_all(archive_root)
            .with_context(|| format!("create archive root {}", archive_root.display()))?;
        let writer = open_archive_writer(archive_root);
        let mut reports: Vec<GraphStaticReport> = Vec::with_capacity(targets.len());
        for (root, id, cfg_override) in &targets {
            if shutdown.load(Ordering::Relaxed) {
                tracing::info!("shutdown requested before repo {id}; exiting");
                break;
            }
            let cfg = match cfg_override {
                Some(p) => load_config(p)
                    .with_context(|| format!("load workspace cortex.toml for {id}"))?,
                None => {
                    load_for_repo(root).with_context(|| format!("load cortex.toml for {id}"))?
                }
            };
            let resolved_id = cfg.cortex.id.clone().unwrap_or_else(|| id.clone());
            let report = run_repo_graph_static(&resolved_id, root, &cfg.cortex, writer.as_ref())
                .with_context(|| format!("graph-static pass for {resolved_id}"))?;
            eprintln!(
                "[graph-static] {}: {} files analyzed, {} edges, {} envelopes, {:.1} s",
                report.repo_id,
                report.files_analyzed,
                report.edges_emitted,
                report.envelopes_written,
                report.duration_secs,
            );
            reports.push(report);
        }
        print_graph_static_summary(&reports);
        return Ok(());
    }

    // Build the publisher: live Synap unless --dry-run is set.
    let publisher: Arc<dyn Publisher> = if args.dry_run {
        Arc::new(MemoryPublisher::new())
    } else {
        let endpoint = args
            .synap_endpoint
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:17003".to_string());
        let handle = Arc::new(
            SynapHandle::new(&endpoint).with_context(|| format!("synap connect {endpoint}"))?,
        );
        let pub_ = LiveSynapPublisher::new(handle);
        pub_.prime_room(&args.stream)
            .await
            .with_context(|| format!("provision synap room {}", args.stream))?;
        Arc::new(pub_)
    };

    let mut summaries: Vec<RunSummary> = Vec::with_capacity(targets.len());
    for (root, id, cfg_override) in &targets {
        if shutdown.load(Ordering::Relaxed) {
            tracing::info!("shutdown requested before repo {id}; exiting");
            break;
        }
        let cfg = match cfg_override {
            Some(p) => {
                load_config(p).with_context(|| format!("load workspace cortex.toml for {id}"))?
            }
            None => load_for_repo(root).with_context(|| format!("load cortex.toml for {id}"))?,
        };
        let repo_cfg = cfg.cortex;
        let resolved_id = repo_cfg.id.clone().unwrap_or_else(|| id.clone());

        // Phase4b §3 — skip-when-done idempotency. When the
        // checkpoint reports `done` AND `last_git_ref` matches the
        // current HEAD of this checkout, the run was already
        // captured. `--force` overrides both checks.
        if !args.force {
            let entry = checkpoint.repos.get(&resolved_id);
            if let Some(progress) = entry {
                if progress.status == "done" {
                    let head = current_head_sha(root);
                    if head.is_some() && progress.last_git_ref == head {
                        tracing::info!(
                            repo = %resolved_id,
                            last_git_ref = %head.as_deref().unwrap_or(""),
                            "bypassing repo: checkpoint reports done at current HEAD"
                        );
                        summaries.push(RunSummary::bypassed(&resolved_id));
                        continue;
                    }
                }
            }
        } else if checkpoint.is_repo_done(&resolved_id) {
            tracing::info!(repo = %resolved_id, "--force re-running done repo");
        }

        let runner_cfg = RunnerConfig {
            repo_id: resolved_id.clone(),
            stream: args.stream.clone(),
            since: args.since.clone(),
            dry_run: args.is_dry_run(),
            // Phase11e §5 — propagate `--kinds` from CliArgs.
            kind_filter: args.kinds.clone(),
        };
        let last_file = checkpoint
            .repos
            .get(&resolved_id)
            .and_then(|p| p.last_file.clone());
        let last_git_ref = checkpoint
            .repos
            .get(&resolved_id)
            .and_then(|p| p.last_git_ref.clone());
        let result = run_repo(
            root,
            &runner_cfg,
            &repo_cfg,
            publisher.clone(),
            metrics.clone(),
            &mut checkpoint,
            last_file,
            last_git_ref,
        )
        .await;
        match result {
            Ok(report) => {
                eprintln!(
                    "[bootstrap] {}: {} events published, {} files dropped, {} commits, {:.1} s",
                    report.repo_id,
                    report.events_published,
                    report.files_dropped,
                    report.commits_walked,
                    report.duration_secs,
                );
                // Stamp the current HEAD on the checkpoint so the
                // next run's skip-when-done check has a value to
                // compare against. The runner already sets
                // `status = done` on success.
                if let Some(head) = current_head_sha(root) {
                    let progress = checkpoint.repo_mut(&resolved_id);
                    progress.last_git_ref = Some(head);
                }
                summaries.push(RunSummary::from_report(&report));
            }
            Err(e) => {
                eprintln!("[bootstrap] {id}: failed: {e}");
                metrics.incr_errors(id, "run_repo");
                summaries.push(RunSummary::failed(&resolved_id, &e.to_string()));
            }
        }
        if let Err(e) = write_atomic(&args.checkpoint, &checkpoint) {
            tracing::warn!(error = %e, "checkpoint write failed");
        }
    }

    print_summary_table(&summaries);

    let any_failed = summaries
        .iter()
        .any(|s| matches!(s.status, RunStatus::Failed { .. }));
    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}

/// One row of the final summary table. Mirrors the per-repo run
/// outcome with enough detail to fit in `id | events | dropped |
/// duration | status` columns.
#[derive(Debug, Clone)]
struct RunSummary {
    repo_id: String,
    events_published: u64,
    files_dropped: u64,
    duration_secs: f64,
    status: RunStatus,
}

#[derive(Debug, Clone)]
enum RunStatus {
    Done,
    Bypassed,
    Failed { reason: String },
}

impl RunSummary {
    fn from_report(report: &RepoRunReport) -> Self {
        Self {
            repo_id: report.repo_id.clone(),
            events_published: report.events_published,
            files_dropped: report.files_dropped,
            duration_secs: report.duration_secs,
            status: if report.completed {
                RunStatus::Done
            } else {
                RunStatus::Failed {
                    reason: "incomplete".into(),
                }
            },
        }
    }

    fn bypassed(repo_id: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            events_published: 0,
            files_dropped: 0,
            duration_secs: 0.0,
            status: RunStatus::Bypassed,
        }
    }

    fn failed(repo_id: &str, reason: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            events_published: 0,
            files_dropped: 0,
            duration_secs: 0.0,
            status: RunStatus::Failed {
                reason: reason.to_string(),
            },
        }
    }
}

fn print_graph_static_summary(rows: &[GraphStaticReport]) {
    if rows.is_empty() {
        return;
    }
    let id_width = rows
        .iter()
        .map(|r| r.repo_id.len())
        .max()
        .unwrap_or(4)
        .max(4);
    eprintln!();
    eprintln!(
        "{:<id_width$}  {:>10}  {:>10}  {:>10}  {:>10}",
        "repo",
        "analyzed",
        "edges",
        "nodes",
        "envelopes",
        id_width = id_width,
    );
    eprintln!(
        "{}",
        "-".repeat(id_width + 4 + 10 + 2 + 10 + 2 + 10 + 2 + 10)
    );
    for r in rows {
        eprintln!(
            "{:<id_width$}  {:>10}  {:>10}  {:>10}  {:>10}",
            r.repo_id,
            r.files_analyzed,
            r.edges_emitted,
            r.nodes_emitted,
            r.envelopes_written,
            id_width = id_width,
        );
    }
}

fn print_summary_table(rows: &[RunSummary]) {
    if rows.is_empty() {
        return;
    }
    let id_width = rows
        .iter()
        .map(|r| r.repo_id.len())
        .max()
        .unwrap_or(4)
        .max(4);
    eprintln!();
    eprintln!(
        "{:<id_width$}  {:>10}  {:>10}  {:>10}  {}",
        "repo",
        "events",
        "dropped",
        "duration",
        "status",
        id_width = id_width,
    );
    eprintln!(
        "{}",
        "-".repeat(id_width + 4 + 10 + 2 + 10 + 2 + 10 + 2 + 6)
    );
    for r in rows {
        let status_str = match &r.status {
            RunStatus::Done => "done".to_string(),
            RunStatus::Bypassed => "bypassed".to_string(),
            RunStatus::Failed { reason } => format!("failed: {reason}"),
        };
        eprintln!(
            "{:<id_width$}  {:>10}  {:>10}  {:>9.1}s  {}",
            r.repo_id,
            r.events_published,
            r.files_dropped,
            r.duration_secs,
            status_str,
            id_width = id_width,
        );
    }
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

/// Phase11j §3.7 — non-destructive settings push.
///
/// Walks every canonical Meilisearch index (`ALL_INDEXES` plus, when
/// the operator passed repo targets, every `cortex-{slug}-{family}`
/// uid the embedder + indexer would target on a normal run) and
/// PATCHes the baked-in `settings.v1.json`. Index creation is
/// idempotent; settings PATCH is additive (Meili applies new
/// attributes without dropping documents). Used to roll forward a
/// settings version (e.g. v3 → v4) without re-publishing every event.
async fn run_apply_settings_only(
    args: &CliArgs,
    targets: &[(PathBuf, String, Option<PathBuf>)],
) -> Result<()> {
    use cortex_storage::names::{repo_scoped_name, slug_for_repo, ALL_INDEXES, NS_PREFIX};
    use cortex_workers::fulltext::{
        settings_v1_json, FulltextConfig, LiveMeiliClient, MeiliClient, FAMILIES,
    };

    let settings = settings_v1_json().context("baked-in settings.v1.json unparseable")?;
    let version = settings
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("(unset)")
        .to_string();

    // Resolve Meili connection. CLI flag wins, then env, then default
    // (the cortex-up port).
    let meili_url = args
        .meili_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let meili_api_key = args.meili_api_key.clone();

    let client = LiveMeiliClient::new(&FulltextConfig {
        meili_url: meili_url.clone(),
        meili_api_key,
        ..FulltextConfig::default()
    })
    .map_err(|e| anyhow::anyhow!("LiveMeiliClient build: {e}"))?;

    // Build the index list: every canonical global, plus per-repo
    // permutations for any target the operator passed in.
    let mut indexes: Vec<String> = ALL_INDEXES.iter().map(|s| s.to_string()).collect();
    for (_, id, _) in targets {
        let slug = slug_for_repo(id);
        for family in FAMILIES {
            indexes.push(repo_scoped_name(NS_PREFIX, &slug, family));
        }
    }
    indexes.sort();
    indexes.dedup();

    tracing::info!(
        meili_url,
        version,
        canonical = ALL_INDEXES.len(),
        per_repo = indexes.len() - ALL_INDEXES.len(),
        total = indexes.len(),
        "apply-settings-only: starting"
    );

    let mut applied: u32 = 0;
    let mut failed: Vec<(String, String)> = Vec::new();
    for index in &indexes {
        match client.ensure_index(index, &settings).await {
            Ok(_) => {
                applied += 1;
                tracing::info!(index = %index, "apply-settings-only: ok");
            }
            Err(err) => {
                let msg = err.to_string();
                tracing::warn!(index = %index, error = %msg, "apply-settings-only: failed");
                failed.push((index.clone(), msg));
            }
        }
    }

    println!(
        "apply-settings-only: version={version} applied={applied}/{total} failed={fail}",
        total = indexes.len(),
        fail = failed.len(),
    );
    if !failed.is_empty() {
        for (idx, err) in &failed {
            println!("  - {idx}: {err}");
        }
        anyhow::bail!(
            "apply-settings-only: {} index/indexes failed (see log above)",
            failed.len()
        );
    }
    Ok(())
}
