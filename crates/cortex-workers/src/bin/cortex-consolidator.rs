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
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use cortex_workers::consolidator::cost_telemetry::CostBudget;
use cortex_workers::consolidator::orchestrator::{Orchestrator, ProducerSelection, Trigger};
use cortex_workers::consolidator::source::{
    LiveDecisionTraceSource, LiveSessionSource, LiveTopicSource, SourceError,
};
use cortex_workers::consolidator::summariser::{
    cost_cents, AnthropicSummariser, Summariser, SummariserKind,
};
use cortex_workers::consolidator::summariser_cli::ClaudeCliSummariser;
use serde::Serialize;
use serde_json::json;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
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
    let Some(base) = resolve_ingest_url(cli) else {
        eprintln!("  publish : skipped (--ingest-url empty)");
        return Ok(());
    };
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
    let url = format!("{base}/v1/events");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("build publish client")?;
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .json(&envelope)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("publish rejected: {status} {body}");
    }
    eprintln!("  publish : ok event_id={event_id}");
    Ok(())
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
        produced.payload.consolidation_id,
        produced.payload.source_event_count,
        produced.cost_cents,
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
    println!("  status  : produced {produced_count} / {} clusters", clusters.len());
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
    if let Ok(p) = std::env::var("CORTEX_CONSOLIDATOR_CURSOR_FILE") {
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
        println!("  previous    : {} (sessions={} topics={} decisions={} cents={})",
            c.last_run_ts, c.sessions_processed, c.topics_processed, c.decisions_processed, c.cost_cents_total);
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
                                eprintln!("  topic {repo}/{lbl} publish error: {e}", lbl = cluster.label);
                                continue;
                            }
                            topics_processed += 1;
                            cost_cents_total = cost_cents_total.saturating_add(produced.cost_cents);
                        }
                        Err(e) => eprintln!("  topic {repo}/{lbl} skipped: {e}", lbl = cluster.label),
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
    let meili_url = meili
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest builder")?;

    let auth = |req: reqwest::RequestBuilder| match api_key.as_deref() {
        Some(k) => req.bearer_auth(k),
        None => req,
    };

    let stats: serde_json::Value = auth(http.get(format!(
        "{}/stats",
        meili_url.trim_end_matches('/')
    )))
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
    let decision_cost_cents =
        cost_cents(SummariserKind::Opus47, decision_input, decision_output);

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
        let total = body
            .get("total")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
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
            p.repo_slug, p.sessions, p.total_envelopes, p.total_body_bytes, p.estimated_input_tokens
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
        let cli =
            Cli::try_parse_from(["cortex-consolidator", "run-topic", "--repo", "cortex"])
                .expect("parse");
        match cli.command {
            Command::RunTopic { repo } => assert_eq!(repo, "cortex"),
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_nightly_dry_run_default_true() {
        let cli =
            Cli::try_parse_from(["cortex-consolidator", "nightly"]).expect("parse");
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
            command: Command::Nightly { dry_run: true, all: false },
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
            command: Command::Nightly { dry_run: true, all: false },
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
            command: Command::Nightly { dry_run: true, all: false },
        };
        let got = enumerate_recent_sessions(&cli).unwrap();
        assert!(got.is_empty(), "missing --metadata-db must yield empty list");
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
            command: Command::Nightly { dry_run: true, all: false },
        };
        let got = cli.resolve_archive_root().unwrap();
        assert_eq!(got, PathBuf::from("D:/explicit"));
    }
}
