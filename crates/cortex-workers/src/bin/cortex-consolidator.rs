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

use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use cortex_workers::consolidator::cost_telemetry::{CostBudget, CostLedger};
use cortex_workers::consolidator::orchestrator::{ProducerSelection, Trigger};
use cortex_workers::consolidator::summariser::{
    cost_cents, AnthropicSummariser, SummariserKind,
};
use serde::Serialize;

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

    #[command(subcommand)]
    command: Command,
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
        #[arg(long, default_value_t = true)]
        dry_run: bool,
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
        Command::RunSession { session_id } => run_session(&cli, session_id),
        Command::RunTopic { repo } => run_topic(&cli, repo),
        Command::RunDecision { decision_id } => run_decision(&cli, decision_id),
        Command::Nightly { dry_run } => run_nightly(&cli, *dry_run),
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

fn build_summarisers(
    cli: &Cli,
) -> Result<(
    std::sync::Arc<AnthropicSummariser>,
    std::sync::Arc<AnthropicSummariser>,
)> {
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
}

fn budget_from(cli: &Cli) -> CostBudget {
    CostBudget {
        monthly_cents_cap: cli.monthly_cents_cap,
    }
}

fn run_session(cli: &Cli, session_id: &str) -> Result<()> {
    let trigger = Trigger::SessionEnd {
        session_id: session_id.to_string(),
    };
    print_plan_header(&trigger);
    let _ = build_summarisers(cli)?;
    let _ = budget_from(cli);
    println!("  status  : pending §3 routing wiring (live envelope read path)");
    println!("  next    : seed the SessionInput from cortex-storage + cortex-api ingest read API");
    Ok(())
}

fn run_topic(cli: &Cli, repo: &str) -> Result<()> {
    let trigger = Trigger::NightlyTopic {
        repo: repo.to_string(),
    };
    print_plan_header(&trigger);
    let _ = build_summarisers(cli)?;
    let _ = budget_from(cli);
    println!("  status  : pending §3 routing wiring (HDBSCAN over Vectorizer turn embeddings)");
    println!(
        "  next    : seed cluster set from Vectorizer per-repo turn collection + classifier topics"
    );
    Ok(())
}

fn run_decision(cli: &Cli, decision_id: &str) -> Result<()> {
    let trigger = Trigger::DecisionLanded {
        decision_id: decision_id.to_string(),
        force_deep: false,
    };
    print_plan_header(&trigger);
    let _ = build_summarisers(cli)?;
    let _ = budget_from(cli);
    println!(
        "  status  : pending §3 routing wiring (parent_event_id chain walk via cortex-storage)"
    );
    println!("  next    : seed DecisionTraceInput from Nexus parent-edge traversal");
    Ok(())
}

fn run_nightly(cli: &Cli, dry_run: bool) -> Result<()> {
    let budget = budget_from(cli);
    let ledger = CostLedger::default();
    println!("nightly preview");
    println!(
        "  monthly cap : {} cents (${:.2})",
        budget.monthly_cents_cap,
        budget.monthly_cents_cap as f64 / 100.0
    );
    println!("  remaining   : {} cents", budget.remaining_cents(&ledger));
    println!("  dry-run     : {dry_run}");
    if !dry_run {
        return Err(anyhow::anyhow!(
            "phase11j §2.9 — live `nightly` run lands alongside §3 routing wiring; \
             rerun with `--dry-run` for a preview"
        ));
    }
    println!("  status      : preview only — live nightly path lands alongside §3 routing wiring");
    Ok(())
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
            Command::Nightly { dry_run } => assert!(dry_run),
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
            monthly_cents_cap: 100_000,
            command: Command::Nightly { dry_run: true },
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
            monthly_cents_cap: 100_000,
            command: Command::Nightly { dry_run: true },
        };
        assert_eq!(require_api_key(&cli).unwrap(), "sk-test-12345");
    }
}
