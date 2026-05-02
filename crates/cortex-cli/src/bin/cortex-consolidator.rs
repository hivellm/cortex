//! Phase11j §2.9 — `cortex-consolidator` CLI.
//!
//! Operator surface for the [`cortex_consolidator`] crate. Four
//! subcommands map onto the producer triggers:
//!
//! - `run-session <session_id>` — emit one Session consolidation.
//! - `run-topic --repo <slug>` — run HDBSCAN over the repo's
//!   sessions (clustering itself is the orchestrator's job; the CLI
//!   passes through to the topic producer).
//! - `run-decision <decision_id>` — emit one DecisionTrace
//!   consolidation.
//! - `nightly --dry-run` — preview tomorrow's batch without
//!   invoking any summariser.
//!
//! Today the CLI surface is the operator handhold; the underlying
//! producer wiring against the live Synap stream + Vectorizer +
//! Nexus storage layer lands alongside §3 (routing) — at which
//! point the `--dry-run` path stops being a preview and starts
//! exercising the real reader chain. The `--api-key` flag (or
//! `ANTHROPIC_API_KEY` env var) is required for any non-dry-run
//! invocation; the dry-run path runs offline.

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use cortex_consolidator::cost_telemetry::{CostBudget, CostLedger};
use cortex_consolidator::orchestrator::{ProducerSelection, Trigger};
use cortex_consolidator::summariser::{AnthropicSummariser, SummariserKind};

#[derive(Debug, Parser)]
#[command(
    name = "cortex-consolidator",
    about = "Distil raw Cortex envelopes into evergreen Kind::Consolidation summaries (phase11j).",
    version
)]
struct Cli {
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
    /// Anthropic API key (overrides `ANTHROPIC_API_KEY`). Required
    /// for non-dry-run subcommands.
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    api_key: Option<String>,
    /// Anthropic API base URL (`https://api.anthropic.com` by
    /// default; `ANTHROPIC_API_URL` env var overrides).
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
    /// Emit one Session consolidation for the given session id.
    RunSession {
        /// Target session id.
        session_id: String,
    },
    /// Cluster the repo's sessions with HDBSCAN and emit one
    /// Topic consolidation per cluster.
    RunTopic {
        /// Repo slug.
        #[arg(long)]
        repo: String,
    },
    /// Walk the parent-event chain from a Decision and emit one
    /// DecisionTrace consolidation.
    RunDecision {
        /// Target decision id.
        decision_id: String,
    },
    /// Preview tomorrow's batch without invoking any summariser.
    Nightly {
        /// Skip the live API call — print the producer plan + cost
        /// estimate. Default `true` so a stray invocation never
        /// burns operator budget.
        #[arg(long, default_value_t = true)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match &cli.command {
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

/// Print the producer plan that would land for the given trigger.
/// Pure stdout — no API calls, no storage reads. Used by every
/// non-dry-run subcommand as the "planned execution" header so the
/// operator sees what is about to happen before the summariser fires.
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
        .timeout(std::time::Duration::from_secs(60))
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
    // Phase11j §2.9 — the live producer wiring (loading the
    // session's envelopes from Synap + Vectorizer + Nexus, building
    // a `SessionInput`, calling `Orchestrator::run_session`, and
    // shipping the resulting envelope through the ingestion path)
    // lands alongside §3 routing. Until then the CLI surfaces the
    // plan + tells the operator what is missing. This keeps the
    // surface forward-compatible without burning API budget on a
    // half-wired path.
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
            Command::Nightly { dry_run } => assert!(dry_run),
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
