//! `cortex-consolidator` CLI — phase11s §4 reconciliation of phase11j §2.9 +
//! phase11q §1 into a single operator entry point.
//!
//! Five subcommands cover the full operator surface:
//!
//! - `estimate` — phase11q §1 walk: reads Meili turn indexes per repo, projects
//!   USD cost across the three planned consolidator passes (session-grain
//!   Shallow Haiku, topic-grain Shallow Haiku, decision-trace Deep Opus). No
//!   Anthropic API calls fire. Operator gate before any live run.
//! - `run-session <id>` — phase11j §2.9: emit one Session consolidation.
//! - `run-topic --repo <slug>` — emit Topic consolidations for the repo's
//!   HDBSCAN clusters.
//! - `run-decision <id>` — emit one DecisionTrace consolidation.
//! - `nightly --dry-run` — preview tomorrow's batch without invoking any
//!   summariser.
//!
//! Live producer wiring against Synap + Vectorizer + Nexus lands alongside
//! phase11j §3 routing; until then the run-* / nightly subcommands surface
//! the producer plan + status. The `estimate` subcommand is fully working.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use cortex_workers::consolidator::consolidator_trait::ConsolidatorCtx;
use cortex_workers::consolidator::cost_telemetry::{CostBudget, CostLedger};
use cortex_workers::consolidator::daemon::{
    ConsolidatorDaemon, PendingTrigger, TriggerSource, TRIGGER_STREAM,
};
use cortex_workers::consolidator::grains::{
    DecisionTraceGrain, LiveDecisionTraceFetcher, LiveSessionInputFetcher, LiveTopicClusterFetcher,
    SessionGrain, TopicGrain,
};
use cortex_workers::consolidator::metrics::{
    metrics as consolidator_metrics, REASON_CLIENT_BUILD, REASON_ENV_UNSET, REASON_NETWORK,
    REASON_NON_2XX,
};
use cortex_workers::consolidator::orchestrator::{Orchestrator, ProducerSelection, Trigger};
use cortex_workers::consolidator::source::{
    LiveDecisionTraceSource, LiveSessionSource, LiveTopicSource, SourceError,
};
use cortex_workers::consolidator::summariser::{
    cost_cents, AnthropicSummariser, Summariser, SummariserKind,
};
use cortex_workers::consolidator::summariser_cli::ClaudeCliSummariser;
use cortex_workers::producer::ProducerMetadataHandle;
use serde::Serialize;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use synap_sdk::stream::StreamManager;
use synap_sdk::{SynapClient, SynapConfig};
use tokio::sync::{Mutex as TokioMutex, Notify};

#[derive(Debug, Parser)]
#[command(
    name = "cortex-consolidator",
    about = "Distil raw Cortex envelopes into evergreen Kind::Consolidation summaries (phase11j) or estimate cost (phase11q).",
    version
)]
struct Cli {
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
    /// Anthropic API key (overrides `ANTHROPIC_API_KEY`). Required for
    /// non-dry-run / non-estimate subcommands.
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    api_key: Option<String>,
    /// Anthropic API base URL (`https://api.anthropic.com` by default).
    #[arg(long, env = "ANTHROPIC_API_URL")]
    api_url: Option<String>,
    /// Monthly budget cap in USD cents (default 100 000 = $1 000).
    #[arg(long, default_value_t = 100_000)]
    monthly_cents_cap: u32,
    /// Phase11p §2.1 — archive root the live read path scans.
    /// Falls back to `CORTEX_ARCHIVE_ROOT` then `<home>/.cortex/archive`.
    #[arg(long, env = "CORTEX_ARCHIVE_ROOT")]
    archive_root: Option<PathBuf>,
    /// Phase11p §2.1 — SQLite metadata DB path used by `nightly`
    /// to enumerate sessions closed in the last 24h.
    #[arg(long, env = "CORTEX_METADATA_DB")]
    metadata_db: Option<PathBuf>,
    /// Path to the `claude` binary (used when `--api-key` is empty;
    /// falls back to PATH lookup of `claude`). The CLI summariser
    /// pipes the rendered prompt through `claude -p - --bare` so
    /// operators without an Anthropic API key can still consolidate
    /// using their logged-in Claude Code session.
    #[arg(long, env = "CLAUDE_CODE_BIN")]
    claude_bin: Option<PathBuf>,
    /// cortex-ingestion base URL the produced consolidation envelopes
    /// are POSTed to (`/v1/events`). Falls back to
    /// `CORTEX_INGESTION_URL` then `http://127.0.0.1:17010`. Set to
    /// the empty string (`--ingest-url=`) to skip publish — useful
    /// for dry-run verification.
    #[arg(long, env = "CORTEX_INGESTION_URL")]
    ingest_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

impl Cli {
    /// Resolve the archive root: explicit flag → env → default.
    fn resolve_archive_root(&self) -> Result<PathBuf> {
        if let Some(p) = &self.archive_root {
            return Ok(p.clone());
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .context("HOME / USERPROFILE unset; pass --archive-root explicitly")?;
        let mut p = PathBuf::from(home);
        p.push(".cortex");
        p.push("archive");
        Ok(p)
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Walk Meili turn indexes per repo and project consolidator cost across
    /// the three grains. Estimate-only — no Anthropic API calls fire.
    Estimate {
        /// Meilisearch base URL. Defaults to `$CORTEX_FULLTEXT_MEILI_URL`
        /// then `http://127.0.0.1:17004`.
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master / admin API key.
        #[arg(long)]
        meili_key: Option<String>,
        /// Restrict the estimate to one repo (matches the `cortex-{slug}-turns`
        /// index suffix).
        #[arg(long)]
        repo: Option<String>,
        /// Emit JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Emit one Session consolidation for the given session id.
    RunSession {
        /// Target session id.
        session_id: String,
    },
    /// Cluster the repo's sessions with HDBSCAN and emit one Topic
    /// consolidation per cluster.
    RunTopic {
        /// Repo slug.
        #[arg(long)]
        repo: String,
    },
    /// Walk the parent-event chain from a Decision and emit one DecisionTrace
    /// consolidation.
    RunDecision {
        /// Target decision id.
        decision_id: String,
    },
    /// Preview tomorrow's batch without invoking any summariser.
    Nightly {
        /// Skip the live API call — print the producer plan + cost estimate.
        /// Default `true` so a stray invocation never burns operator budget.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        dry_run: bool,
        /// Override the 24h enumeration window: when set, every
        /// session in the metadata DB is enumerated (used for the
        /// initial corpus back-fill pass).
        #[arg(long, default_value_t = false)]
        all: bool,
    },
    /// Phase14a §3 — long-running consolidator daemon. Subscribes
    /// to the trigger stream, dispatches each trigger to the
    /// matching grain, writes a producer-checkpoint row per
    /// successful run, and exits cleanly on SIGTERM / Ctrl-C
    /// (finishes the in-flight grain first).
    Daemon {
        /// Synap base URL the daemon pulls triggers from.
        /// Defaults to `$SYNAP_BASE_URL` then `http://127.0.0.1:18020`.
        #[arg(long, env = "SYNAP_BASE_URL")]
        synap_url: Option<String>,
        /// Stream name to subscribe to. Defaults to the canonical
        /// `cortex.consolidator.triggers` constant.
        #[arg(long, default_value = TRIGGER_STREAM)]
        stream: String,
        /// Idle-poll wait when the trigger queue is empty (ms).
        #[arg(long, default_value_t = 250)]
        idle_poll_ms: u64,
    },
    /// Phase14c topic-recluster — re-cluster already-published
    /// session consolidations into topic-grain consolidations using
    /// the claude CLI as the grouping engine. Closes the
    /// cross-session theme-dedup gap left by LiveTopicSource's
    /// embedding-inline requirement.
    TopicRecluster {
        /// Meilisearch base URL. Defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL` then `http://127.0.0.1:17004`.
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master key.
        #[arg(long)]
        meili_key: Option<String>,
        /// Restrict to a single repo slug (matches the
        /// `cortex-{slug}-consolidations` index suffix). All repos
        /// when omitted.
        #[arg(long)]
        repo: Option<String>,
        /// Minimum cluster size to emit a topic consolidation.
        #[arg(long, default_value_t = 2)]
        min_cluster_size: u32,
        /// When true, print the proposed clusters but skip the
        /// claude summary + publish. Default false because the
        /// claude CLI cost is negligible.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    // Phase 12a §1.2 — at boot, warn loudly when no ingestion URL is
    // resolvable for the upcoming run. Without this, the operator
    // discovers the silent envelope drop only post-hoc by inspecting
    // the fallback JSONL. The warning lets them set the env var (or
    // pass `--ingest-url`) before the first run wastes Anthropic
    // budget on summaries that go straight to the fallback file.
    if matches!(
        cli.command,
        Command::RunSession { .. }
            | Command::RunTopic { .. }
            | Command::RunDecision { .. }
            | Command::Nightly { dry_run: false, .. }
    ) && resolve_ingest_url(&cli).is_none()
    {
        tracing::warn!(
            "CORTEX_INGESTION_URL unset and --ingest-url empty: every consolidation will land in {} (replay with `cortex-ops consolidations-replay`)",
            fallback_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<unresolved>".to_string())
        );
    }

    match &cli.command {
        Command::Estimate {
            meili,
            meili_key,
            repo,
            json,
        } => estimate(meili.clone(), meili_key.clone(), repo.clone(), *json).await,
        Command::RunSession { session_id } => run_session(&cli, session_id).await,
        Command::RunTopic { repo } => run_topic(&cli, repo).await,
        Command::RunDecision { decision_id } => run_decision(&cli, decision_id).await,
        Command::Nightly { dry_run, all } => run_nightly(&cli, *dry_run, *all).await,
        Command::Daemon {
            synap_url,
            stream,
            idle_poll_ms,
        } => run_daemon(&cli, synap_url.clone(), stream.clone(), *idle_poll_ms).await,
        Command::TopicRecluster {
            meili,
            meili_key,
            repo,
            min_cluster_size,
            dry_run,
        } => {
            run_topic_recluster(
                &cli,
                meili.clone(),
                meili_key.clone(),
                repo.clone(),
                *min_cluster_size,
                *dry_run,
            )
            .await
        }
    }
}

fn init_tracing(verbose: bool) {
    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .try_init();
}

fn print_plan_header(trigger: &Trigger) {
    let sel = ProducerSelection::for_trigger(trigger);
    println!("plan");
    println!("  trigger : {trigger:?}");
    println!("  grain   : {}", sel.grain_label());
    println!(
        "  model   : {}",
        match sel.summariser {
            SummariserKind::Haiku45 => "claude-haiku-4-5",
            SummariserKind::Opus47 => "claude-opus-4-7",
        }
    );
    if let Some(repo) = &sel.repo {
        println!("  repo    : {repo}");
    }
}

fn require_api_key(cli: &Cli) -> Result<String> {
    cli.api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .context("ANTHROPIC_API_KEY (or --api-key) required for live runs")
}

fn has_api_key(cli: &Cli) -> bool {
    cli.api_key
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false)
}

fn build_summarisers(
    cli: &Cli,
) -> Result<(
    std::sync::Arc<dyn Summariser>,
    std::sync::Arc<dyn Summariser>,
)> {
    if has_api_key(cli) {
        let api_key = require_api_key(cli)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("build reqwest client")?;
        let mut haiku =
            AnthropicSummariser::new(client.clone(), api_key.clone(), SummariserKind::Haiku45);
        let mut opus = AnthropicSummariser::new(client, api_key, SummariserKind::Opus47);
        if let Some(url) = cli.api_url.as_deref().filter(|s| !s.trim().is_empty()) {
            haiku = haiku.with_api_url(url);
            opus = opus.with_api_url(url);
        }
        Ok((std::sync::Arc::new(haiku), std::sync::Arc::new(opus)))
    } else {
        let bin = cli
            .claude_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from("claude"));
        let haiku = ClaudeCliSummariser::new(bin.clone(), SummariserKind::Haiku45);
        let opus = ClaudeCliSummariser::new(bin, SummariserKind::Opus47);
        eprintln!(
            "[cortex-consolidator] no ANTHROPIC_API_KEY; routing through claude CLI subprocess"
        );
        Ok((std::sync::Arc::new(haiku), std::sync::Arc::new(opus)))
    }
}

fn resolve_ingest_url(cli: &Cli) -> Option<String> {
    let raw = cli
        .ingest_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:17010".into());
    if raw.trim().is_empty() {
        None
    } else {
        Some(raw.trim_end_matches('/').to_string())
    }
}

async fn publish_consolidation(
    cli: &Cli,
    payload: &cortex_core::events::ConsolidationPayload,
    session_id: &str,
    repo_hint: Option<&str>,
) -> Result<()> {
    use sha2::Digest as _;
    let payload_json = serde_json::to_value(payload).context("serialise payload")?;
    let canonical = serde_json::to_string(&payload_json).context("canonical payload")?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(canonical.as_bytes());
    let content_hash_hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let content_hash = format!("sha256:{content_hash_hex}");
    let event_id = ulid::Ulid::new().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let repo = repo_hint
        .map(|s| s.to_string())
        .or_else(|| payload.repos.first().cloned());
    let envelope = json!({
        "event_id": event_id,
        "schema_version": "1",
        "occurred_at": now,
        "session_id": session_id,
        "stream": "live",
        "tool": "cortex-cli",
        "kind": "consolidation",
        "context": {
            "repo": repo,
            "platform": match std::env::consts::OS {
                "windows" => "win32",
                "macos" => "darwin",
                other => other,
            },
        },
        "payload": payload_json,
        "content_hash": content_hash,
    });

    // Phase 12a — silent envelope drops are a documented P0 source of
    // data loss. Every failure path now: (a) emits a structured ERROR
    // log carrying `event_id`, `session_id`, and a `reason` label and
    // (b) appends the envelope to `${CORTEX_HOME}/consolidations.jsonl`
    // so the operator can replay it once the ingestion URL is healthy.
    let Some(base) = resolve_ingest_url(cli) else {
        tracing::error!(
            event_id = %event_id,
            session_id = %session_id,
            reason = "env_unset",
            "consolidator publish skipped — CORTEX_INGESTION_URL unset and --ingest-url empty; envelope appended to fallback JSONL"
        );
        consolidator_metrics().record_publish_failure(REASON_ENV_UNSET);
        append_publish_fallback(&envelope, "env_unset")?;
        return Ok(());
    };

    let url = format!("{base}/v1/events");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                event_id = %event_id,
                session_id = %session_id,
                reason = "client_build",
                error = %e,
                "consolidator publish skipped — reqwest client build failed; envelope appended to fallback JSONL"
            );
            consolidator_metrics().record_publish_failure(REASON_CLIENT_BUILD);
            append_publish_fallback(&envelope, "client_build")?;
            return Ok(());
        }
    };

    match client
        .post(&url)
        .header("content-type", "application/json")
        .json(&envelope)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                tracing::info!(
                    event_id = %event_id,
                    session_id = %session_id,
                    url = %url,
                    "consolidator publish ok"
                );
                consolidator_metrics().record_publish_ok();
                eprintln!("  publish : ok event_id={event_id}");
                Ok(())
            } else {
                let body = resp.text().await.unwrap_or_default();
                tracing::error!(
                    event_id = %event_id,
                    session_id = %session_id,
                    reason = "non_2xx",
                    status = %status,
                    body = %body,
                    "consolidator publish rejected; envelope appended to fallback JSONL"
                );
                consolidator_metrics().record_publish_failure(REASON_NON_2XX);
                append_publish_fallback(&envelope, "non_2xx")?;
                Ok(())
            }
        }
        Err(e) => {
            tracing::error!(
                event_id = %event_id,
                session_id = %session_id,
                reason = "network",
                url = %url,
                error = %e,
                "consolidator publish failed; envelope appended to fallback JSONL"
            );
            consolidator_metrics().record_publish_failure(REASON_NETWORK);
            append_publish_fallback(&envelope, "network")?;
            Ok(())
        }
    }
}

/// Resolve the JSONL fallback path:
/// `${CORTEX_CONSOLIDATIONS_FALLBACK_FILE}` → `${CORTEX_HOME}/consolidations.jsonl`
/// → `<HOME|USERPROFILE>/.cortex/consolidations.jsonl`.
fn fallback_path() -> Option<PathBuf> {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(p) = cfg.consolidator.fallback_file.as_deref() {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Some(p) = cfg.ingestion.home.as_deref() {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p).join("consolidations.jsonl"));
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".cortex")
            .join("consolidations.jsonl"),
    )
}

/// Soft cap on the live `consolidations.jsonl` file. Past this size
/// the file rotates to `<path>.1` (overwriting any prior rotation)
/// and a fresh empty file replaces it. Override with
/// `CORTEX_CONSOLIDATIONS_FALLBACK_ROTATE_BYTES`. Default 100 MB.
const FALLBACK_ROTATE_AT_BYTES: u64 = 100 * 1024 * 1024;

/// Append the envelope to the fallback JSONL with a reason wrapper so
/// the operator's replay tool can decide what to retry. Rotates the
/// live file when it crosses the size threshold so an operator who
/// leaves the daemon running with `CORTEX_INGESTION_URL` unset for a
/// month does not silently fill the disk.
fn append_publish_fallback(envelope: &serde_json::Value, reason: &str) -> Result<()> {
    let Some(path) = fallback_path() else {
        tracing::error!(
            reason = %reason,
            "fallback path unresolved (HOME / USERPROFILE / CORTEX_HOME all unset); envelope NOT persisted"
        );
        return Ok(());
    };
    let threshold = fallback_rotate_threshold();
    append_publish_fallback_to(&path, threshold, envelope, reason)
}

/// Pure-path variant the tests drive directly so concurrent test
/// runs do not stomp on each other's `CORTEX_CONSOLIDATIONS_FALLBACK_FILE`.
/// Takes both the destination file and the rotation threshold
/// explicitly. The production wrapper above resolves both from env.
fn append_publish_fallback_to(
    path: &std::path::Path,
    rotate_at_bytes: u64,
    envelope: &serde_json::Value,
    reason: &str,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    rotate_fallback_at(path, rotate_at_bytes);

    let line = serde_json::json!({
        "fallback_at": chrono::Utc::now().to_rfc3339(),
        "reason": reason,
        "envelope": envelope,
    });
    let mut serialised = serde_json::to_string(&line).context("serialise fallback envelope")?;
    serialised.push('\n');

    use std::fs::OpenOptions;
    use std::io::Write;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open fallback {}", path.display()))?;
    file.write_all(serialised.as_bytes())
        .with_context(|| format!("append fallback {}", path.display()))?;
    tracing::warn!(
        path = %path.display(),
        reason = %reason,
        "consolidator envelope persisted to fallback JSONL — replay with `cortex-ops consolidations-replay`"
    );
    Ok(())
}

/// Resolve the rotation threshold from env, falling back to
/// [`FALLBACK_ROTATE_AT_BYTES`].
fn fallback_rotate_threshold() -> u64 {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.consolidator.fallback_rotate_bytes)
        .filter(|n| *n > 0)
        .unwrap_or(FALLBACK_ROTATE_AT_BYTES)
}

/// Move `<path>` to `<path>.1` when the live file has crossed
/// `threshold` bytes. Threshold is passed explicitly so tests can
/// drive a tiny value without env var contention. Best-effort: every
/// error is swallowed so the hot path keeps appending.
fn rotate_fallback_at(path: &std::path::Path, threshold: u64) {
    let live_len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if live_len < threshold {
        return;
    }
    let mut rotated = path.as_os_str().to_os_string();
    rotated.push(".1");
    let rotated = PathBuf::from(rotated);
    let _ = std::fs::remove_file(&rotated);
    if std::fs::rename(path, &rotated).is_ok() {
        tracing::info!(
            from = %path.display(),
            to = %rotated.display(),
            live_len_bytes = live_len,
            threshold_bytes = threshold,
            "consolidator fallback rotated"
        );
    }
}

fn budget_from(cli: &Cli) -> CostBudget {
    CostBudget {
        monthly_cents_cap: cli.monthly_cents_cap,
    }
}

async fn run_session(cli: &Cli, session_id: &str) -> Result<()> {
    let trigger = Trigger::SessionEnd {
        session_id: session_id.to_string(),
    };
    print_plan_header(&trigger);
    let archive_root = cli.resolve_archive_root()?;
    let source = LiveSessionSource::new(&archive_root);
    let input = match source.fetch(session_id) {
        Ok(input) => input,
        Err(SourceError::EmptyResult) => {
            println!("  status  : empty session — no envelopes captured for {session_id}");
            return Ok(());
        }
        Err(e) => return Err(anyhow::anyhow!("session fetch: {e}")),
    };
    let (haiku, opus) = build_summarisers(cli)?;
    let orchestrator = Orchestrator::new(haiku, opus).with_budget(budget_from(cli));
    let produced = orchestrator
        .run_session(&input)
        .await
        .map_err(|e| anyhow::anyhow!("orchestrator: {e}"))?;
    publish_consolidation(
        cli,
        &produced.payload,
        &input.session_id,
        input.repo.as_deref(),
    )
    .await
    .with_context(|| format!("publish session={}", input.session_id))?;
    println!(
        "  status  : ok — consolidation_id={}, source_event_count={}, cost_cents={}",
        produced.payload.consolidation_id, produced.payload.source_event_count, produced.cost_cents,
    );
    Ok(())
}

async fn run_topic(cli: &Cli, repo: &str) -> Result<()> {
    let trigger = Trigger::NightlyTopic {
        repo: repo.to_string(),
    };
    print_plan_header(&trigger);
    let archive_root = cli.resolve_archive_root()?;
    // 7-day default window — the consolidator's nightly cadence
    // assumes turns younger than a week are the freshness ceiling.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - 7 * 24 * 3600 * 1000;
    let source = LiveTopicSource::new(&archive_root, 0);
    let clusters = source
        .fetch(repo, since_ms, now_ms)
        .map_err(|e| anyhow::anyhow!("topic fetch: {e}"))?;
    if clusters.is_empty() {
        println!("  status  : zero clusters in window — nothing to summarise");
        return Ok(());
    }
    let (haiku, opus) = build_summarisers(cli)?;
    let orchestrator = Orchestrator::new(haiku, opus).with_budget(budget_from(cli));
    let mut produced_count = 0u32;
    for cluster in &clusters {
        match orchestrator.run_topic(cluster).await {
            Ok(produced) => {
                produced_count += 1;
                println!(
                    "  cluster={} sessions={} consolidation_id={} cost_cents={}",
                    cluster.label,
                    cluster.sessions.len(),
                    produced.payload.consolidation_id,
                    produced.cost_cents,
                );
            }
            Err(e) => {
                eprintln!("  cluster={} skipped: {e}", cluster.label);
            }
        }
    }
    println!(
        "  status  : produced {produced_count} / {} clusters",
        clusters.len()
    );
    Ok(())
}

async fn run_decision(cli: &Cli, decision_id: &str) -> Result<()> {
    let trigger = Trigger::DecisionLanded {
        decision_id: decision_id.to_string(),
        force_deep: false,
    };
    print_plan_header(&trigger);
    let archive_root = cli.resolve_archive_root()?;
    let source = LiveDecisionTraceSource::new(&archive_root);
    let input = match source.fetch(decision_id) {
        Ok(input) => input,
        Err(SourceError::EmptyResult) => {
            println!("  status  : decision {decision_id} not found in archive");
            return Ok(());
        }
        Err(e) => return Err(anyhow::anyhow!("decision fetch: {e}")),
    };
    let chain_len = input.chain.len();
    let (haiku, opus) = build_summarisers(cli)?;
    let orchestrator = Orchestrator::new(haiku, opus).with_budget(budget_from(cli));
    let produced = orchestrator
        .run_decision_trace(&input)
        .await
        .map_err(|e| anyhow::anyhow!("orchestrator: {e}"))?;
    println!(
        "  status  : ok — consolidation_id={}, chain_len={}, cost_cents={}",
        produced.payload.consolidation_id, chain_len, produced.cost_cents,
    );
    Ok(())
}

/// Cursor file written at the end of every successful nightly run.
/// `<home>/.cortex/consolidator-cursor.json`.
#[derive(Debug, Serialize, serde::Deserialize, Default)]
struct NightlyCursor {
    /// RFC-3339 timestamp of the most recent successful run.
    last_run_ts: String,
    /// Sessions actually consolidated (ok-path only).
    sessions_processed: u32,
    /// Topic clusters consolidated.
    topics_processed: u32,
    /// Decision traces consolidated.
    decisions_processed: u32,
    /// Total cost charged across the run.
    cost_cents_total: u32,
}

fn cursor_path() -> Option<PathBuf> {
    if let Some(p) = cortex_config::Config::load()
        .ok()
        .and_then(|c| c.consolidator.cursor_file)
    {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let mut p = PathBuf::from(home);
    p.push(".cortex");
    p.push("consolidator-cursor.json");
    Some(p)
}

fn read_cursor() -> Option<NightlyCursor> {
    let path = cursor_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cursor(cursor: &NightlyCursor) -> Result<()> {
    let path = cursor_path().context("HOME / USERPROFILE unset")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let raw = serde_json::to_vec_pretty(cursor)?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&raw)?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

async fn run_nightly(cli: &Cli, dry_run: bool, all: bool) -> Result<()> {
    let budget = budget_from(cli);
    let prev = read_cursor();
    println!("nightly run");
    println!(
        "  monthly cap : {} cents (${:.2})",
        budget.monthly_cents_cap,
        budget.monthly_cents_cap as f64 / 100.0
    );
    if let Some(c) = &prev {
        println!(
            "  previous    : {} (sessions={} topics={} decisions={} cents={})",
            c.last_run_ts,
            c.sessions_processed,
            c.topics_processed,
            c.decisions_processed,
            c.cost_cents_total
        );
    }
    println!("  dry-run     : {dry_run}");
    let archive_root = cli.resolve_archive_root()?;

    // Enumerate sessions: 24h window by default; `--all` switches
    // to a corpus-wide back-fill enumeration. When the metadata DB
    // is unset / unreachable the loop bypasses the session leg
    // cleanly.
    let session_ids: Vec<String> = if all {
        enumerate_all_sessions(cli)?
    } else {
        enumerate_recent_sessions(cli)?
    };
    println!(
        "  sessions    : {} candidate(s){}",
        session_ids.len(),
        if all { " (--all)" } else { "" }
    );

    if dry_run {
        println!("  status      : dry-run preview only");
        return Ok(());
    }

    let (haiku, opus) = build_summarisers(cli)?;
    let orchestrator = Orchestrator::new(haiku, opus).with_budget(budget);

    let mut sessions_processed = 0u32;
    let decisions_processed = 0u32;
    let mut topics_processed = 0u32;
    let mut cost_cents_total = 0u32;
    let mut repos_seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    let session_source = LiveSessionSource::new(&archive_root);
    for (idx, sid) in session_ids.iter().enumerate() {
        match session_source.fetch(sid) {
            Ok(input) => {
                if let Some(r) = input.repo.as_deref() {
                    repos_seen.insert(r.to_string());
                }
                match orchestrator.run_session(&input).await {
                    Ok(produced) => {
                        if let Err(e) = publish_consolidation(
                            cli,
                            &produced.payload,
                            &input.session_id,
                            input.repo.as_deref(),
                        )
                        .await
                        {
                            eprintln!("  session {sid} publish error: {e}");
                            continue;
                        }
                        sessions_processed += 1;
                        cost_cents_total = cost_cents_total.saturating_add(produced.cost_cents);
                        if (idx + 1) % 10 == 0 {
                            eprintln!(
                                "  progress: {}/{} sessions, cost_cents={cost_cents_total}",
                                idx + 1,
                                session_ids.len()
                            );
                        }
                    }
                    Err(e) => eprintln!("  session {sid} skipped: {e}"),
                }
            }
            Err(SourceError::EmptyResult) => continue,
            Err(e) => eprintln!("  session {sid} fetch error: {e}"),
        }
    }

    // phase11x — topic consolidations across every repo touched by
    // tonight's session batch. Each repo's recent (7-day) turn
    // history feeds `LiveTopicSource` which clusters via HDBSCAN
    // and yields one consolidation per cluster ≥ min_cluster_size.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let topic_window_ms = 7i64 * 24 * 3600 * 1000;
    let since_ms = now_ms - topic_window_ms;
    let topic_source = LiveTopicSource::new(&archive_root, 0);
    println!(
        "  topic legs  : {} repo(s) — {}",
        repos_seen.len(),
        repos_seen.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    for repo in &repos_seen {
        match topic_source.fetch(repo, since_ms, now_ms) {
            Ok(clusters) => {
                if clusters.is_empty() {
                    continue;
                }
                for cluster in &clusters {
                    match orchestrator.run_topic(cluster).await {
                        Ok(produced) => {
                            if let Err(e) = publish_consolidation(
                                cli,
                                &produced.payload,
                                &cluster.label,
                                Some(repo),
                            )
                            .await
                            {
                                eprintln!(
                                    "  topic {repo}/{lbl} publish error: {e}",
                                    lbl = cluster.label
                                );
                                continue;
                            }
                            topics_processed += 1;
                            cost_cents_total = cost_cents_total.saturating_add(produced.cost_cents);
                        }
                        Err(e) => {
                            eprintln!("  topic {repo}/{lbl} skipped: {e}", lbl = cluster.label)
                        }
                    }
                }
            }
            Err(e) => eprintln!("  topic {repo} fetch error: {e}"),
        }
    }

    let cursor = NightlyCursor {
        last_run_ts: chrono::Utc::now().to_rfc3339(),
        sessions_processed,
        topics_processed,
        decisions_processed,
        cost_cents_total,
    };
    write_cursor(&cursor)?;

    println!(
        "  status      : ok — sessions={sessions_processed} topics={topics_processed} \
         decisions={decisions_processed} cost_cents={cost_cents_total}"
    );
    Ok(())
}

/// Read the metadata SQLite for sessions closed in the last 24h.
/// Returns an empty vec when the DB is unreachable / unset — the
/// nightly run still does the topic + decision legs.
fn enumerate_recent_sessions(cli: &Cli) -> Result<Vec<String>> {
    let path = match &cli.metadata_db {
        Some(p) => p.clone(),
        None => return Ok(Vec::new()),
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = match rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    let mut stmt = conn
        .prepare("SELECT session_id FROM sessions WHERE started_at >= ?1")
        .context("prepare sessions query")?;
    let rows = stmt
        .query_map(rusqlite::params![cutoff], |r| r.get::<_, String>(0))
        .context("execute sessions query")?;
    let mut out: Vec<String> = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

/// Enumerate every session id known to the project. Tries the
/// metadata DB first; if it has no rows (legacy installs do not
/// populate `sessions`), falls back to scanning the parquet archive
/// for distinct envelope `session_id` values.
fn enumerate_all_sessions(cli: &Cli) -> Result<Vec<String>> {
    let mut from_db: Vec<String> = Vec::new();
    if let Some(path) = &cli.metadata_db {
        if path.exists() {
            if let Ok(conn) = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            ) {
                if let Ok(mut stmt) =
                    conn.prepare("SELECT session_id FROM sessions ORDER BY started_at ASC")
                {
                    if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                        for row in rows.flatten() {
                            from_db.push(row);
                        }
                    }
                }
            }
        }
    }
    if !from_db.is_empty() {
        return Ok(from_db);
    }
    // Fallback: scan archive partitions for distinct envelope session ids.
    let archive_root = cli.resolve_archive_root()?;
    let ids = cortex_storage::archive::enumerate_session_ids(&archive_root)
        .context("scan archive for session ids")?;
    Ok(ids)
}

#[derive(Serialize)]
struct PerRepoEstimate {
    repo_slug: String,
    sessions: u64,
    total_envelopes: u64,
    total_body_bytes: u64,
    estimated_input_tokens: u64,
}

#[derive(Serialize)]
struct PassEstimate {
    grain: &'static str,
    model: &'static str,
    estimated_input_tokens: u64,
    estimated_output_tokens: u64,
    estimated_cost_usd: f64,
    notes: &'static str,
}

#[derive(Serialize)]
struct EstimateReport {
    mode: &'static str,
    meili_url: String,
    per_repo: Vec<PerRepoEstimate>,
    total_sessions: u64,
    total_envelopes: u64,
    total_body_bytes: u64,
    passes: Vec<PassEstimate>,
    total_cost_usd: f64,
}

async fn estimate(
    meili: Option<String>,
    meili_key: Option<String>,
    repo_filter: Option<String>,
    json: bool,
) -> Result<()> {
    let cfg_typed = cortex_config::Config::load().unwrap_or_default();
    let meili_url = meili
        .or_else(|| cfg_typed.meili.meili_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| cfg_typed.meili.meili_api_key.clone())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest builder")?;

    let auth = |req: reqwest::RequestBuilder| match api_key.as_deref() {
        Some(k) => req.bearer_auth(k),
        None => req,
    };

    let stats: serde_json::Value =
        auth(http.get(format!("{}/stats", meili_url.trim_end_matches('/'))))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
    let map = stats
        .get("indexes")
        .and_then(|v| v.as_object())
        .context("/stats payload missing `indexes`")?;
    let mut turn_indexes: Vec<(String, u64)> = map
        .iter()
        .filter_map(|(uid, v)| {
            if !uid.starts_with("cortex-") || !uid.ends_with("-turns") {
                return None;
            }
            let slug = &uid["cortex-".len()..uid.len() - "-turns".len()];
            if let Some(filter) = &repo_filter {
                if slug != filter {
                    return None;
                }
            }
            let count = v
                .get("numberOfDocuments")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            if count == 0 {
                return None;
            }
            Some((uid.clone(), count))
        })
        .collect();
    turn_indexes.sort_by_key(|(_, c)| std::cmp::Reverse(*c));

    let mut per_repo: Vec<PerRepoEstimate> = Vec::new();
    for (uid, _count) in &turn_indexes {
        let slug = uid["cortex-".len()..uid.len() - "-turns".len()].to_string();
        let (sessions, envelopes, body_bytes) =
            scan_index(&http, &meili_url, api_key.as_deref(), uid).await?;
        let estimated_input_tokens = body_bytes / 4;
        per_repo.push(PerRepoEstimate {
            repo_slug: slug,
            sessions,
            total_envelopes: envelopes,
            total_body_bytes: body_bytes,
            estimated_input_tokens,
        });
    }

    let total_sessions: u64 = per_repo.iter().map(|r| r.sessions).sum();
    let total_envelopes: u64 = per_repo.iter().map(|r| r.total_envelopes).sum();
    let total_body_bytes: u64 = per_repo.iter().map(|r| r.total_body_bytes).sum();
    let total_input_tokens: u64 = per_repo.iter().map(|r| r.estimated_input_tokens).sum();

    let session_input = total_input_tokens;
    let session_output = total_sessions.saturating_mul(512);
    let session_cost_cents = cost_cents(SummariserKind::Haiku45, session_input, session_output);

    let topic_clusters = (total_sessions / 4).max(1);
    let topic_input = total_input_tokens / 4;
    let topic_output = topic_clusters.saturating_mul(512);
    let topic_cost_cents = cost_cents(SummariserKind::Haiku45, topic_input, topic_output);

    let decision_traces: u64 = 100;
    let decision_input = decision_traces.saturating_mul(3_000);
    let decision_output = decision_traces.saturating_mul(1_024);
    let decision_cost_cents = cost_cents(SummariserKind::Opus47, decision_input, decision_output);

    let passes = vec![
        PassEstimate {
            grain: "session",
            model: "Haiku 4.5 (Shallow)",
            estimated_input_tokens: session_input,
            estimated_output_tokens: session_output,
            estimated_cost_usd: f64::from(session_cost_cents) / 100.0,
            notes: "one call per session; input = full session token volume",
        },
        PassEstimate {
            grain: "topic",
            model: "Haiku 4.5 (Shallow)",
            estimated_input_tokens: topic_input,
            estimated_output_tokens: topic_output,
            estimated_cost_usd: f64::from(topic_cost_cents) / 100.0,
            notes: "approx (total_sessions/4) clusters; refine when topic clusterer ships",
        },
        PassEstimate {
            grain: "decision_trace",
            model: "Opus 4.7 (Deep)",
            estimated_input_tokens: decision_input,
            estimated_output_tokens: decision_output,
            estimated_cost_usd: f64::from(decision_cost_cents) / 100.0,
            notes: "100 ADRs assumed (verify via /v1/dashboard/decisions count)",
        },
    ];

    let total_cost_usd =
        f64::from(session_cost_cents + topic_cost_cents + decision_cost_cents) / 100.0;

    let report = EstimateReport {
        mode: "estimate_only",
        meili_url,
        per_repo,
        total_sessions,
        total_envelopes,
        total_body_bytes,
        passes,
        total_cost_usd,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_text(&report);
    }
    Ok(())
}

async fn scan_index(
    http: &reqwest::Client,
    meili_url: &str,
    api_key: Option<&str>,
    uid: &str,
) -> Result<(u64, u64, u64)> {
    let mut sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut envelopes: u64 = 0;
    let mut body_bytes: u64 = 0;
    let mut offset = 0usize;
    let limit = 1000usize;
    loop {
        let url = format!(
            "{}/indexes/{}/documents?limit={}&offset={}&fields=session_id,body",
            meili_url.trim_end_matches('/'),
            uid,
            limit,
            offset
        );
        let mut req = http.get(&url);
        if let Some(k) = api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("scan {uid} offset={offset}: status {}", resp.status());
        }
        let body: serde_json::Value = resp.json().await?;
        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if results.is_empty() {
            break;
        }
        for d in &results {
            envelopes += 1;
            if let Some(s) = d.get("session_id").and_then(|v| v.as_str()) {
                if !s.is_empty() {
                    sessions.insert(s.to_string());
                }
            }
            if let Some(b) = d.get("body").and_then(|v| v.as_str()) {
                body_bytes += b.len() as u64;
            }
        }
        offset += results.len();
        let total = body.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if total > 0 && offset >= total {
            break;
        }
    }
    Ok((sessions.len() as u64, envelopes, body_bytes))
}

fn render_text(r: &EstimateReport) {
    println!("cortex-consolidator estimate (mode={})", r.mode);
    println!("meili: {}", r.meili_url);
    println!();
    println!("per-repo:");
    for p in &r.per_repo {
        println!(
            "  {}: sessions={}, envelopes={}, body_bytes={}, est_input_tokens={}",
            p.repo_slug,
            p.sessions,
            p.total_envelopes,
            p.total_body_bytes,
            p.estimated_input_tokens
        );
    }
    println!();
    println!("totals:");
    println!("  sessions:    {}", r.total_sessions);
    println!("  envelopes:   {}", r.total_envelopes);
    println!("  body_bytes:  {}", r.total_body_bytes);
    println!();
    println!("per-grain projection:");
    for p in &r.passes {
        println!(
            "  {grain:>15} ({model}): in={input} out={output} cost=${cost:.2}  -- {notes}",
            grain = p.grain,
            model = p.model,
            input = p.estimated_input_tokens,
            output = p.estimated_output_tokens,
            cost = p.estimated_cost_usd,
            notes = p.notes
        );
    }
    println!();
    println!("TOTAL ESTIMATED COST: ${:.2} USD", r.total_cost_usd);
    println!();
    println!("This is an ESTIMATE-ONLY pass. No Anthropic API calls fired.");
    println!("Operator must approve the USD total before the actual run is triggered.");
}

// ============================================================================
// Phase14a §3.3 — daemon subcommand + Synap trigger source + signal handler
// ============================================================================

/// Parse one raw Synap event payload into a [`Trigger`]. The wire
/// envelope is a JSON object with a `kind` discriminator + the
/// per-variant payload. Unknown kinds surface as an error so the
/// daemon does not silently drop unsupported events.
fn parse_trigger_event(payload: &serde_json::Value) -> anyhow::Result<Trigger> {
    let kind = payload
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("trigger envelope missing `kind` field"))?;
    match kind {
        "session_end" => {
            let session_id = payload
                .get("session_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("session_end trigger missing `session_id`"))?
                .to_string();
            Ok(Trigger::SessionEnd { session_id })
        }
        "nightly_topic" => {
            let repo = payload
                .get("repo")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("nightly_topic trigger missing `repo`"))?
                .to_string();
            Ok(Trigger::NightlyTopic { repo })
        }
        "decision_landed" => {
            let decision_id = payload
                .get("decision_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("decision_landed trigger missing `decision_id`"))?
                .to_string();
            let force_deep = payload
                .get("force_deep")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(Trigger::DecisionLanded {
                decision_id,
                force_deep,
            })
        }
        other => Err(anyhow::anyhow!(
            "unknown consolidator trigger kind: {other}"
        )),
    }
}

/// Synap-backed trigger source. Pulls events from `stream` via the
/// 0.11 pull API with a local offset cursor (no server-side
/// consumer group available yet — same shape as the embedder).
struct SynapTriggerSource {
    streams: StreamManager,
    stream: String,
    cursor: AtomicU64,
}

impl SynapTriggerSource {
    fn new(streams: StreamManager, stream: String) -> Self {
        Self {
            streams,
            stream,
            cursor: AtomicU64::new(0),
        }
    }
}

#[async_trait::async_trait]
impl TriggerSource for SynapTriggerSource {
    async fn next_trigger(&self) -> anyhow::Result<Option<PendingTrigger>> {
        let offset = self.cursor.load(Ordering::Relaxed);
        let events = match self
            .streams
            .consume(&self.stream, Some(offset), Some(1))
            .await
        {
            Ok(evs) => evs,
            Err(err) => {
                // Synap creates rooms on first publish. Until the
                // supervisor publishes its first trigger, the
                // consume call returns "Room not found" which the
                // SDK surfaces as a generic server error. Treat
                // that as Idle so the daemon waits patiently
                // instead of crash-looping under restart:
                // unless-stopped.
                let msg = err.to_string();
                if msg.contains("not found") || msg.contains("Not Found") {
                    return Ok(None);
                }
                return Err(anyhow::anyhow!("synap consume {}: {err}", self.stream));
            }
        };
        let Some(event) = events.into_iter().next() else {
            return Ok(None);
        };
        match parse_trigger_event(&event.data) {
            Ok(trigger) => Ok(Some(PendingTrigger {
                offset: event.offset,
                trigger,
            })),
            Err(err) => {
                tracing::warn!(
                    stream = %self.stream,
                    offset = event.offset,
                    error = %err,
                    "consolidator daemon: dropping malformed trigger envelope"
                );
                // Advance past the malformed event so the loop does
                // not infinitely retry it. Surfacing as Idle keeps
                // the daemon polling the next message on the next
                // tick.
                self.cursor.fetch_max(event.offset + 1, Ordering::AcqRel);
                Ok(None)
            }
        }
    }

    async fn ack(&self, offset: u64) -> anyhow::Result<()> {
        // `fetch_max` keeps the cursor monotonic even if multiple
        // dispatchers race.
        self.cursor.fetch_max(offset + 1, Ordering::AcqRel);
        Ok(())
    }
}

/// Wait for SIGTERM / SIGINT (Ctrl-C). Returns once any one fires.
/// Cross-platform: `tokio::signal::ctrl_c` handles SIGINT everywhere;
/// the SIGTERM branch is unix-only and noop on Windows.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "consolidator daemon: ctrl_c install failed");
        }
    };

    #[cfg(unix)]
    let sigterm = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "consolidator daemon: sigterm install failed");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("consolidator daemon: ctrl_c received, shutting down");
        }
        _ = sigterm => {
            tracing::info!("consolidator daemon: sigterm received, shutting down");
        }
    }
}

async fn run_daemon(
    cli: &Cli,
    synap_url: Option<String>,
    stream: String,
    idle_poll_ms: u64,
) -> Result<()> {
    let base = synap_url
        .or_else(|| std::env::var("SYNAP_BASE_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:18020".into());
    println!("cortex-consolidator daemon");
    println!("  synap : {base}");
    println!("  stream: {stream}");
    println!("  idle  : {idle_poll_ms} ms");

    let synap_config = SynapConfig::new(&base);
    let client =
        SynapClient::new(synap_config).map_err(|e| anyhow::anyhow!("synap client build: {e}"))?;
    let streams = client.stream();
    let source: Arc<dyn TriggerSource> = Arc::new(SynapTriggerSource::new(streams, stream.clone()));

    let archive_root = cli.resolve_archive_root()?;
    let (haiku, opus) = build_summarisers(cli)?;
    let orchestrator = Arc::new(Orchestrator::new(haiku, opus).with_budget(budget_from(cli)));

    let session_fetcher = Arc::new(LiveSessionInputFetcher::new(LiveSessionSource::new(
        &archive_root,
    )));
    let topic_fetcher = Arc::new(LiveTopicClusterFetcher::new(LiveTopicSource::new(
        &archive_root,
        0,
    )));
    let decision_fetcher = Arc::new(LiveDecisionTraceFetcher::new(LiveDecisionTraceSource::new(
        &archive_root,
    )));

    let session_grain = Arc::new(SessionGrain::new(orchestrator.clone(), session_fetcher));
    let topic_grain = Arc::new(TopicGrain::new(orchestrator.clone(), topic_fetcher));
    let decision_grain = Arc::new(DecisionTraceGrain::new(orchestrator, decision_fetcher));

    let cost = Arc::new(std::sync::Mutex::new(CostLedger::default()));
    let ctx = ConsolidatorCtx::with_ledger(chrono::Utc::now(), cost);

    let metadata: ProducerMetadataHandle = {
        let store = if let Some(path) = &cli.metadata_db {
            cortex_storage::MetadataStore::open(path)
                .map_err(|e| anyhow::anyhow!("open metadata db {}: {e}", path.display()))?
        } else {
            cortex_storage::MetadataStore::open_in_memory()
                .map_err(|e| anyhow::anyhow!("open in-memory metadata db: {e}"))?
        };
        Arc::new(TokioMutex::new(store))
    };

    let daemon = ConsolidatorDaemon::new(
        session_grain,
        topic_grain,
        decision_grain,
        source,
        ctx,
        metadata,
    )
    .with_idle_poll(Duration::from_millis(idle_poll_ms));

    let shutdown = Arc::new(Notify::new());
    let signal_handle = shutdown.clone();
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        signal_handle.notify_one();
    });

    let report = daemon
        .run_forever(async move { shutdown.notified().await })
        .await?;

    // The signal task is no longer needed once the daemon has
    // returned; abort + drop in case ctrl_c never fired (e.g. the
    // daemon stopped on a fatal Synap error).
    signal_task.abort();
    let _ = signal_task.await;

    println!(
        "  status: exited cleanly — dispatched={} failed={} idle_polls={}",
        report.dispatched, report.failed, report.idle_polls
    );
    Ok(())
}

// ============================================================================
// Phase14c §topic-recluster — posthoc cross-session theme dedup
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
struct ConsolidationDoc {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    excerpt: Option<String>,
    #[serde(default)]
    body_markdown: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ClusterPlan {
    topic_label: String,
    #[serde(default)]
    rationale: Option<String>,
    member_ids: Vec<String>,
}

/// Strip a `````json` / `````` fence the model often wraps responses
/// in. Returns the original string when no fence is found.
fn strip_json_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_start();
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

/// Build the cluster-prompt the claude CLI sees. Lists every
/// consolidation's id + title + first 200 chars of body so the
/// model can group on substance, not just title.
fn render_cluster_prompt(repo: &str, docs: &[ConsolidationDoc], min_cluster_size: u32) -> String {
    let mut lines = Vec::with_capacity(docs.len());
    for d in docs {
        let snippet = d
            .body_markdown
            .as_deref()
            .or(d.excerpt.as_deref())
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        lines.push(format!(
            "- id={} | title={} | excerpt={}",
            d.id, d.title, snippet
        ));
    }
    let listing = lines.join("\n");
    format!(
        "You are a deduplication judge for the Cortex consolidations index. \
Given the {n} session-grain consolidations below for repo {repo:?}, identify groups \
that cover the SAME engineering theme (same feature, same incident, same refactor, \
same investigation thread). Emit one cluster per recurring theme; ignore singletons.\n\n\
Hard rules:\n\
- Only emit clusters with at least {min} members.\n\
- Every member_id MUST be one of the ids listed below.\n\
- topic_label MUST be ≤80 chars, lower-kebab-case-ish (free form OK but stable).\n\
- Skip clusters whose theme is generic infrastructure (\"misc cleanup\", \"adhoc\") \
unless the members are clearly the SAME thread.\n\n\
Consolidations (id | title | first 200 chars of body):\n{listing}\n\n\
Return EXACTLY one JSON object on a single line with shape:\n\
{{\"clusters\": [{{\"topic_label\":\"...\",\"rationale\":\"...\",\"member_ids\":[\"...\",\"...\"]}}, ...]}}\n\
No markdown fence, no extra prose.",
        n = docs.len(),
        repo = repo,
        min = min_cluster_size,
        listing = listing
    )
}

/// Fetch every doc in `cortex-{slug}-consolidations` via Meili.
/// Pages by 200; 200 should be enough for any current repo (the
/// largest repo today holds <50).
async fn fetch_repo_consolidations(
    http: &reqwest::Client,
    meili_url: &str,
    api_key: Option<&str>,
    slug: &str,
) -> Result<Vec<ConsolidationDoc>> {
    let uid = format!("cortex-{slug}-consolidations");
    let url = format!(
        "{}/indexes/{}/documents?limit=200&fields=id,title,excerpt,body_markdown",
        meili_url.trim_end_matches('/'),
        uid
    );
    let mut req = http.get(&url);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await?;
    if resp.status().as_u16() == 404 {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        anyhow::bail!("fetch {uid}: status {}", resp.status());
    }
    let body: serde_json::Value = resp.json().await?;
    let arr = body
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        match serde_json::from_value::<ConsolidationDoc>(v) {
            Ok(d) => out.push(d),
            Err(err) => tracing::warn!(error = %err, "skipping malformed consolidation doc"),
        }
    }
    Ok(out)
}

/// Enumerate every `cortex-{slug}-consolidations` index Meili
/// currently knows about. Returns the slug list.
async fn enumerate_consolidation_repos(
    http: &reqwest::Client,
    meili_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>> {
    let url = format!("{}/indexes?limit=1000", meili_url.trim_end_matches('/'));
    let mut req = http.get(&url);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let resp = req
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let results = resp
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut slugs = Vec::new();
    for entry in results {
        if let Some(uid) = entry.get("uid").and_then(|v| v.as_str()) {
            if let Some(slug) = uid
                .strip_prefix("cortex-")
                .and_then(|s| s.strip_suffix("-consolidations"))
            {
                slugs.push(slug.to_string());
            }
        }
    }
    slugs.sort();
    Ok(slugs)
}

/// Parse the claude CLI cluster response into [`ClusterPlan`].
fn parse_cluster_response(raw: &str) -> Result<Vec<ClusterPlan>> {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        clusters: Vec<ClusterPlan>,
    }
    let body = strip_json_fence(raw);
    let wrapped: Wrapper = serde_json::from_str(body)
        .with_context(|| format!("claude cluster response not parseable: body={body}"))?;
    Ok(wrapped.clusters)
}

async fn run_topic_recluster(
    cli: &Cli,
    meili: Option<String>,
    meili_key: Option<String>,
    repo: Option<String>,
    min_cluster_size: u32,
    dry_run: bool,
) -> Result<()> {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let meili_url = meili
        .or_else(|| cfg.meili.meili_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| cfg.meili.meili_api_key.clone())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest build")?;

    println!("topic-recluster");
    println!("  meili    : {meili_url}");
    println!("  min_size : {min_cluster_size}");
    println!("  dry_run  : {dry_run}");

    // Build the claude CLI summariser (Haiku — fast + cheap enough
    // for one prompt per repo). Always uses claude CLI; no Anthropic
    // API key path because the cluster prompt is small and CLI is
    // the universally-available fallback.
    let claude_bin = cli
        .claude_bin
        .clone()
        .unwrap_or_else(|| PathBuf::from("claude"));
    let summariser = ClaudeCliSummariser::new(
        claude_bin,
        cortex_workers::consolidator::summariser::SummariserKind::Haiku45,
    );

    let repos = match repo {
        Some(r) => vec![r],
        None => enumerate_consolidation_repos(&http, &meili_url, api_key.as_deref()).await?,
    };
    println!("  repos    : {} candidate(s)", repos.len());

    let mut emitted = 0u32;
    let mut clusters_total = 0u32;
    let mut singletons_skipped = 0u32;

    for slug in &repos {
        let docs = fetch_repo_consolidations(&http, &meili_url, api_key.as_deref(), slug).await?;
        if (docs.len() as u32) < min_cluster_size {
            println!("  [{slug}] {} doc(s) — below floor, skipping", docs.len());
            continue;
        }
        println!(
            "  [{slug}] {} doc(s) — clustering via claude CLI",
            docs.len()
        );
        let prompt = render_cluster_prompt(slug, &docs, min_cluster_size);
        let result = match summariser
            .summarise(
                cortex_workers::consolidator::summariser::SummariserRequest {
                    prompt,
                    max_output_tokens: None,
                },
            )
            .await
        {
            Ok(r) => r,
            Err(err) => {
                eprintln!("  [{slug}] cluster prompt failed: {err}");
                continue;
            }
        };
        let plans = match parse_cluster_response(&result.text) {
            Ok(p) => p,
            Err(err) => {
                eprintln!("  [{slug}] cluster parse failed: {err}");
                continue;
            }
        };
        let id_set: std::collections::HashSet<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        for plan in plans {
            // Sanitise: drop unknown ids, drop below floor.
            let members: Vec<String> = plan
                .member_ids
                .into_iter()
                .filter(|m| id_set.contains(m.as_str()))
                .collect();
            if (members.len() as u32) < min_cluster_size {
                singletons_skipped += 1;
                continue;
            }
            clusters_total += 1;
            println!(
                "    cluster topic_label={:?} members={} rationale={}",
                plan.topic_label,
                members.len(),
                plan.rationale.as_deref().unwrap_or("")
            );
            if dry_run {
                continue;
            }
            // Build a TopicCluster + run through the topic producer
            // to ride the existing prompt template + payload
            // assembly + validation.
            match emit_topic_consolidation(
                cli,
                &summariser,
                slug,
                &plan.topic_label,
                &members,
                &docs,
            )
            .await
            {
                Ok(()) => emitted += 1,
                Err(err) => eprintln!("    publish failed: {err}"),
            }
        }
    }
    println!(
        "  status   : clusters={clusters_total} emitted={emitted} singletons_skipped={singletons_skipped}"
    );
    Ok(())
}

/// Topic-recluster bypass path. Builds the topic prompt + parses
/// the claude CLI response + assembles a `ConsolidationPayload`
/// directly, skipping `producer::topic::produce` because that
/// helper hardcodes `MIN_CLUSTER_SIZE = 3` (the recluster path
/// has its own operator-tunable floor) and uses a stricter
/// `{title, summary_markdown, takeaways}` template that claude CLI
/// occasionally answers with prose. Validation still runs against
/// the canonical `validate_produced` rules so the wire envelope
/// stays honest.
async fn emit_topic_consolidation(
    cli: &Cli,
    summariser: &ClaudeCliSummariser,
    slug: &str,
    topic_label: &str,
    member_ids: &[String],
    docs: &[ConsolidationDoc],
) -> Result<()> {
    use cortex_core::events::{
        ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, TimeSpan,
        CONSOLIDATION_SOURCE_IDS_INLINE_CAP, CONSOLIDATION_TITLE_MAX_CHARS,
    };
    use cortex_workers::consolidator::producer::{derive_consolidation_id, validate_produced};
    use cortex_workers::consolidator::summariser::{Summariser, SummariserRequest};

    let by_id: std::collections::HashMap<&str, &ConsolidationDoc> =
        docs.iter().map(|d| (d.id.as_str(), d)).collect();
    let mut digests = Vec::with_capacity(member_ids.len());
    for id in member_ids {
        if let Some(d) = by_id.get(id.as_str()) {
            digests.push(format!(
                "- {id} | {}",
                d.title.chars().take(200).collect::<String>()
            ));
        }
    }
    let prompt = format!(
        "You are summarising a set of session-grain consolidations that all cover the SAME engineering theme into a single topic-grain consolidation for repo {slug:?}.\n\n\
Theme label (working title): {topic_label}\n\
Member session consolidations (id | title excerpt):\n{listing}\n\n\
Produce a JSON OBJECT (no markdown fence, no prose around it) with this shape:\n\
{{\"title\":\"<<=80 char descriptive title>\",\
\"summary_markdown\":\"<300-1800 char narrative covering the shared thread across the members; mention concrete artifacts when present>\",\
\"takeaways\":[\"<3-5 bullet takeaways>\"]}}\n\
STRICTLY JSON ONLY. The summary_markdown MUST be at least 300 characters.",
        listing = digests.join("\n")
    );
    let result = summariser
        .summarise(SummariserRequest {
            prompt,
            max_output_tokens: None,
        })
        .await?;
    let body = extract_first_json_object(&result.text)
        .ok_or_else(|| anyhow::anyhow!("claude returned no parseable JSON object"))?;
    #[derive(serde::Deserialize)]
    struct TopicResp {
        title: String,
        summary_markdown: String,
        #[serde(default)]
        takeaways: Vec<String>,
    }
    let parsed: TopicResp = serde_json::from_str(&body)
        .with_context(|| format!("topic response shape mismatch: {body}"))?;

    let scope = ConsolidationScope::Topic(topic_label.to_string());
    let consolidation_id = derive_consolidation_id(ConsolidationGrain::Topic, &scope);
    let title: String = parsed
        .title
        .chars()
        .take(CONSOLIDATION_TITLE_MAX_CHARS)
        .collect();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut source_event_ids = member_ids.to_vec();
    let source_event_count = source_event_ids.len() as u32;
    if source_event_ids.len() > CONSOLIDATION_SOURCE_IDS_INLINE_CAP {
        source_event_ids.truncate(CONSOLIDATION_SOURCE_IDS_INLINE_CAP);
    }
    let payload = ConsolidationPayload {
        consolidation_id,
        grain: ConsolidationGrain::Topic,
        scope,
        title,
        summary_markdown: parsed.summary_markdown,
        takeaways: parsed.takeaways,
        source_event_ids,
        source_event_count,
        model: result.kind.model_id().to_string(),
        depth: match result.kind {
            cortex_workers::consolidator::summariser::SummariserKind::Haiku45 => {
                ConsolidationDepth::Shallow
            }
            cortex_workers::consolidator::summariser::SummariserKind::Opus47 => {
                ConsolidationDepth::Deep
            }
        },
        outcome_distribution: std::collections::BTreeMap::new(),
        temporal_span: TimeSpan {
            start_ms: now_ms - 86_400_000,
            end_ms: now_ms,
            duration_ms: 86_400_000,
        },
        repos: vec![slug.to_string()],
        tags: vec![topic_label.to_string()],
    };
    validate_produced(&payload).map_err(|e| anyhow::anyhow!("topic payload invalid: {e}"))?;
    // session_id slot on the envelope is a 26-char ULID per ADR-001;
    // synthesise one so the ingestion's regex gate accepts the
    // envelope. The real per-session ids live in source_event_ids.
    let session_id = ulid::Ulid::new().to_string();
    publish_consolidation(cli, &payload, &session_id, Some(slug)).await?;
    Ok(())
}

/// Pull the first balanced `{...}` JSON object out of `raw`. Claude
/// CLI occasionally wraps the requested JSON in prose; this helper
/// extracts the first top-level object regardless.
fn extract_first_json_object(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let s = start?;
                    return Some(raw[s..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_run_session_subcommand() {
        let cli = Cli::try_parse_from([
            "cortex-consolidator",
            "run-session",
            "01HXSESS00000000000000000A",
        ])
        .expect("parse");
        match cli.command {
            Command::RunSession { session_id } => {
                assert_eq!(session_id, "01HXSESS00000000000000000A");
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_run_topic_subcommand_with_repo_flag() {
        let cli = Cli::try_parse_from(["cortex-consolidator", "run-topic", "--repo", "cortex"])
            .expect("parse");
        match cli.command {
            Command::RunTopic { repo } => assert_eq!(repo, "cortex"),
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_nightly_dry_run_default_true() {
        let cli = Cli::try_parse_from(["cortex-consolidator", "nightly"]).expect("parse");
        match cli.command {
            Command::Nightly { dry_run, all } => {
                assert!(dry_run);
                assert!(!all);
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_estimate_subcommand_with_json() {
        let cli = Cli::try_parse_from([
            "cortex-consolidator",
            "estimate",
            "--repo",
            "cortex",
            "--json",
        ])
        .expect("parse");
        match cli.command {
            Command::Estimate { repo, json, .. } => {
                assert_eq!(repo.as_deref(), Some("cortex"));
                assert!(json);
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn require_api_key_errors_when_unset() {
        let cli = Cli {
            verbose: false,
            api_key: None,
            api_url: None,
            claude_bin: None,
            ingest_url: None,
            monthly_cents_cap: 100_000,
            archive_root: None,
            metadata_db: None,
            command: Command::Nightly {
                dry_run: true,
                all: false,
            },
        };
        let err = require_api_key(&cli).expect_err("no key");
        assert!(format!("{err:#}").contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn require_api_key_returns_value_when_set() {
        let cli = Cli {
            verbose: false,
            api_key: Some("sk-test-12345".into()),
            api_url: None,
            claude_bin: None,
            ingest_url: None,
            monthly_cents_cap: 100_000,
            archive_root: None,
            metadata_db: None,
            command: Command::Nightly {
                dry_run: true,
                all: false,
            },
        };
        assert_eq!(require_api_key(&cli).unwrap(), "sk-test-12345");
    }

    // ----------------------------------------------------------------
    // Phase11p §2.6 — bin tests for the live read-path wiring.
    // ----------------------------------------------------------------

    #[test]
    fn cli_parses_archive_root_flag() {
        let cli = Cli::try_parse_from([
            "cortex-consolidator",
            "--archive-root",
            "C:/data/archive",
            "nightly",
        ])
        .expect("parse");
        assert_eq!(
            cli.archive_root.as_deref(),
            Some(std::path::Path::new("C:/data/archive"))
        );
    }

    #[test]
    fn nightly_cursor_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursor.json");
        std::env::set_var("CORTEX_CONSOLIDATOR_CURSOR_FILE", &path);
        let cursor = NightlyCursor {
            last_run_ts: "2026-05-04T03:00:00Z".to_string(),
            sessions_processed: 7,
            topics_processed: 3,
            decisions_processed: 1,
            cost_cents_total: 42,
        };
        write_cursor(&cursor).unwrap();
        let back = read_cursor().expect("cursor must round-trip");
        assert_eq!(back.sessions_processed, 7);
        assert_eq!(back.topics_processed, 3);
        assert_eq!(back.decisions_processed, 1);
        assert_eq!(back.cost_cents_total, 42);
        assert_eq!(back.last_run_ts, "2026-05-04T03:00:00Z");
        std::env::remove_var("CORTEX_CONSOLIDATOR_CURSOR_FILE");
    }

    #[test]
    fn enumerate_recent_sessions_returns_empty_when_db_unset() {
        let cli = Cli {
            verbose: false,
            api_key: None,
            api_url: None,
            claude_bin: None,
            ingest_url: None,
            monthly_cents_cap: 100_000,
            archive_root: None,
            metadata_db: None,
            command: Command::Nightly {
                dry_run: true,
                all: false,
            },
        };
        let got = enumerate_recent_sessions(&cli).unwrap();
        assert!(
            got.is_empty(),
            "missing --metadata-db must yield empty list"
        );
    }

    #[test]
    fn resolve_archive_root_honours_explicit_flag() {
        let cli = Cli {
            verbose: false,
            api_key: None,
            api_url: None,
            claude_bin: None,
            ingest_url: None,
            monthly_cents_cap: 100_000,
            archive_root: Some(PathBuf::from("D:/explicit")),
            metadata_db: None,
            command: Command::Nightly {
                dry_run: true,
                all: false,
            },
        };
        let got = cli.resolve_archive_root().unwrap();
        assert_eq!(got, PathBuf::from("D:/explicit"));
    }

    #[test]
    fn fallback_path_honours_override_env() {
        let saved = std::env::var_os("CORTEX_CONSOLIDATIONS_FALLBACK_FILE");
        std::env::set_var(
            "CORTEX_CONSOLIDATIONS_FALLBACK_FILE",
            "D:/explicit/fallback.jsonl",
        );
        let p = fallback_path().expect("fallback path resolved");
        match saved {
            Some(v) => std::env::set_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE", v),
            None => std::env::remove_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE"),
        }
        assert_eq!(p, PathBuf::from("D:/explicit/fallback.jsonl"));
    }

    // ADR-016 §5.3 — `fallback_path_falls_back_to_cortex_home_when_
    // override_empty` removed. It mutated CORTEX_HOME +
    // CORTEX_CONSOLIDATIONS_FALLBACK_FILE at process-global scope and
    // raced the sibling rotate / append tests. The
    // `fallback_path()` helper's resolution order is centrally tested
    // by cortex-config's `load.rs::tests::empty_env_value_is_
    // treated_as_unset` (which pins the "empty-env-falls-through-to-
    // default" semantic) plus the round-trip IT in
    // `cortex-config/tests/toml_round_trip_it.rs` that asserts
    // ingestion.home loads from TOML.

    #[test]
    fn append_publish_fallback_rotates_when_threshold_crossed() {
        // Explicit-path variant: no env var contention with parallel
        // tests. The production code path resolves the same
        // arguments from `fallback_path()` + `fallback_rotate_threshold()`.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("consolidations.jsonl");

        // Pre-seed the live file just past the test threshold.
        std::fs::write(&target, vec![b'x'; 300]).unwrap();

        let envelope = serde_json::json!({
            "event_id": "01ULID0000000000000099",
            "kind": "consolidation",
        });
        append_publish_fallback_to(&target, 256, &envelope, "non_2xx").unwrap();

        // Live file rotated → only the new line lives in `target`.
        let live = std::fs::read_to_string(&target).unwrap();
        assert_eq!(live.matches('\n').count(), 1, "live file holds one line");
        assert!(live.contains("01ULID0000000000000099"));
        let live_len = live.len() as u64;
        assert!(
            live_len < 300,
            "live file must be smaller than the 300-byte pre-rotation seed, got {live_len}"
        );

        // Rotated tail lives at `<target>.1`.
        let mut rotated = target.clone().into_os_string();
        rotated.push(".1");
        let rotated = PathBuf::from(rotated);
        assert!(rotated.exists(), ".1 rotation produced");
        let rotated_len = std::fs::metadata(&rotated).unwrap().len();
        assert_eq!(rotated_len, 300, ".1 holds the pre-rotation seed bytes");
    }

    #[test]
    fn append_publish_fallback_does_not_rotate_below_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("consolidations.jsonl");
        let envelope = serde_json::json!({
            "event_id": "01ULID0000000000000010",
            "kind": "consolidation",
        });
        // Threshold huge → no rotation possible.
        append_publish_fallback_to(&target, u64::MAX, &envelope, "env_unset").unwrap();
        append_publish_fallback_to(&target, u64::MAX, &envelope, "env_unset").unwrap();

        let live = std::fs::read_to_string(&target).unwrap();
        assert_eq!(
            live.matches('\n').count(),
            2,
            "both lines retained in live file"
        );

        let mut rotated = target.clone().into_os_string();
        rotated.push(".1");
        let rotated = PathBuf::from(rotated);
        assert!(!rotated.exists(), "no .1 rotation when below threshold");
    }

    #[test]
    fn append_publish_fallback_writes_one_jsonl_line_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("consolidations.jsonl");

        let envelope = serde_json::json!({
            "event_id": "01TESTULID0000000000000001",
            "kind": "consolidation",
            "payload": {"summary": "first"},
        });
        append_publish_fallback_to(&target, u64::MAX, &envelope, "env_unset").unwrap();
        let envelope2 = serde_json::json!({
            "event_id": "01TESTULID0000000000000002",
            "kind": "consolidation",
            "payload": {"summary": "second"},
        });
        append_publish_fallback_to(&target, u64::MAX, &envelope2, "non_2xx").unwrap();

        let content = std::fs::read_to_string(&target).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "exactly one JSON line per call");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["reason"].as_str(), Some("env_unset"));
        assert_eq!(
            first["envelope"]["event_id"].as_str(),
            Some("01TESTULID0000000000000001")
        );
        assert!(first["fallback_at"].is_string());
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["reason"].as_str(), Some("non_2xx"));
    }

    #[test]
    fn cli_parses_daemon_subcommand_with_defaults() {
        let cli = Cli::try_parse_from(["cortex-consolidator", "daemon"]).expect("parse");
        match cli.command {
            Command::Daemon {
                synap_url,
                stream,
                idle_poll_ms,
            } => {
                assert!(synap_url.is_none());
                assert_eq!(stream, "cortex.consolidator.triggers");
                assert_eq!(idle_poll_ms, 250);
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_daemon_subcommand_with_overrides() {
        let cli = Cli::try_parse_from([
            "cortex-consolidator",
            "daemon",
            "--synap-url",
            "http://example.test:1234",
            "--stream",
            "alt.stream",
            "--idle-poll-ms",
            "1000",
        ])
        .expect("parse");
        match cli.command {
            Command::Daemon {
                synap_url,
                stream,
                idle_poll_ms,
            } => {
                assert_eq!(synap_url.as_deref(), Some("http://example.test:1234"));
                assert_eq!(stream, "alt.stream");
                assert_eq!(idle_poll_ms, 1000);
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn parse_trigger_event_recognises_session_end() {
        let raw = serde_json::json!({
            "kind": "session_end",
            "session_id": "01HXSESS00000000000000000A",
        });
        match parse_trigger_event(&raw).expect("parse") {
            Trigger::SessionEnd { session_id } => {
                assert_eq!(session_id, "01HXSESS00000000000000000A");
            }
            other => panic!("wrong trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_trigger_event_recognises_nightly_topic() {
        let raw = serde_json::json!({"kind": "nightly_topic", "repo": "cortex"});
        match parse_trigger_event(&raw).expect("parse") {
            Trigger::NightlyTopic { repo } => assert_eq!(repo, "cortex"),
            other => panic!("wrong trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_trigger_event_recognises_decision_landed_with_force_deep_default_false() {
        let raw = serde_json::json!({"kind": "decision_landed", "decision_id": "DEC-1"});
        match parse_trigger_event(&raw).expect("parse") {
            Trigger::DecisionLanded {
                decision_id,
                force_deep,
            } => {
                assert_eq!(decision_id, "DEC-1");
                assert!(!force_deep);
            }
            other => panic!("wrong trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_trigger_event_honours_explicit_force_deep_flag() {
        let raw = serde_json::json!({
            "kind": "decision_landed",
            "decision_id": "DEC-2",
            "force_deep": true,
        });
        match parse_trigger_event(&raw).expect("parse") {
            Trigger::DecisionLanded { force_deep, .. } => assert!(force_deep),
            other => panic!("wrong trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_trigger_event_rejects_unknown_kind() {
        let raw = serde_json::json!({"kind": "wild_card"});
        let err = parse_trigger_event(&raw).expect_err("unknown kind must error");
        assert!(format!("{err:#}").contains("unknown consolidator trigger kind"));
    }

    #[test]
    fn parse_trigger_event_rejects_missing_required_fields() {
        let raw = serde_json::json!({"kind": "session_end"});
        let err = parse_trigger_event(&raw).expect_err("missing session_id");
        assert!(format!("{err:#}").contains("session_id"));

        let raw = serde_json::json!({});
        let err = parse_trigger_event(&raw).expect_err("missing kind");
        assert!(format!("{err:#}").contains("kind"));
    }

    #[test]
    fn metrics_each_reason_increments_its_dedicated_counter() {
        // Phase12a §3.2 — exercise every documented reason path against
        // the live publish-metrics registry and assert the counter for
        // that reason advances exactly once. The registry is process-
        // wide, so we read the baseline first and assert the delta to
        // stay independent of test ordering.
        use cortex_workers::consolidator::metrics::{
            metrics, REASON_CLIENT_BUILD, REASON_ENV_UNSET, REASON_NETWORK, REASON_NON_2XX,
        };
        let baseline = metrics().snapshot();
        metrics().record_publish_failure(REASON_ENV_UNSET);
        metrics().record_publish_failure(REASON_CLIENT_BUILD);
        metrics().record_publish_failure(REASON_NON_2XX);
        metrics().record_publish_failure(REASON_NETWORK);
        metrics().record_publish_ok();
        let after = metrics().snapshot();
        assert_eq!(after.env_unset - baseline.env_unset, 1, "env_unset");
        assert_eq!(
            after.client_build - baseline.client_build,
            1,
            "client_build"
        );
        assert_eq!(after.non_2xx - baseline.non_2xx, 1, "non_2xx");
        assert_eq!(after.network - baseline.network, 1, "network");
        assert_eq!(after.ok - baseline.ok, 1, "ok");
    }
}
