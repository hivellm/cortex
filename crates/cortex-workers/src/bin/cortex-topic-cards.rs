//! `cortex-topic-cards` CLI — phase11r §2.8 operator binary.
//!
//! Subcommands cover the topic-card operator surface:
//! - `rewrite <topic_slug> --repo <slug>` — emit one rewrite for a topic.
//! - `scan-now` — sweep all topics whose triggers fire right now.
//! - `replay --since <ts>` — re-run rewrites against the evidence stream
//!   from the given timestamp.
//! - `nightly --dry-run` — preview tomorrow's batch without invoking the
//!   summariser.
//!
//! Live producer wiring against Synap + Vectorizer + Nexus lands
//! alongside the phase11r §3 routing pass; until then the run-* / scan /
//! replay subcommands print a `plan` header and exit with a `pending §3
//! routing wiring` status. The `nightly --dry-run` path runs offline.

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use cortex_workers::consolidator::cost_telemetry::{CostBudget, CostLedger};
use cortex_workers::consolidator::summariser::{AnthropicSummariser, SummariserKind};

#[derive(Debug, Parser)]
#[command(
    name = "cortex-topic-cards",
    about = "Living-synthesis topic-card operator CLI (phase11r §2).",
    version
)]
struct Cli {
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
    /// Anthropic API key (overrides `ANTHROPIC_API_KEY`). Required for
    /// non-dry-run subcommands.
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    api_key: Option<String>,
    /// Anthropic API base URL.
    #[arg(long, env = "ANTHROPIC_API_URL")]
    api_url: Option<String>,
    /// Monthly budget cap in USD cents (default 100 000 = $1 000).
    #[arg(long, default_value_t = 100_000)]
    monthly_cents_cap: u32,
    /// Force the Opus 4.7 escalation regardless of contradiction count.
    #[arg(long)]
    deep: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Emit one rewrite for the given topic.
    Rewrite {
        /// Topic slug (kebab-case).
        topic_slug: String,
        /// Repository scope.
        #[arg(long)]
        repo: String,
    },
    /// Sweep every topic whose trigger fires now.
    ScanNow,
    /// Replay topic rewrites against the evidence stream since `since`.
    Replay {
        /// RFC3339 lower bound on the evidence stream.
        #[arg(long)]
        since: String,
    },
    /// Preview tomorrow's batch without invoking the summariser.
    Nightly {
        /// Skip the live API call. Default `true` so a stray invocation
        /// never burns operator budget.
        #[arg(long, default_value_t = true)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match &cli.command {
        Command::Rewrite { topic_slug, repo } => run_rewrite(&cli, topic_slug, repo),
        Command::ScanNow => run_scan_now(&cli),
        Command::Replay { since } => run_replay(&cli, since),
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

fn print_plan_header(label: &str, topic: Option<&str>, repo: Option<&str>, deep: bool) {
    println!("plan");
    println!("  trigger : {label}");
    if let Some(t) = topic {
        println!("  topic   : {t}");
    }
    if let Some(r) = repo {
        println!("  repo    : {r}");
    }
    println!(
        "  model   : {}",
        if deep {
            "claude-opus-4-7"
        } else {
            "claude-haiku-4-5"
        }
    );
}

fn run_rewrite(cli: &Cli, topic_slug: &str, repo: &str) -> Result<()> {
    print_plan_header("rewrite", Some(topic_slug), Some(repo), cli.deep);
    let _ = build_summarisers(cli)?;
    let _ = budget_from(cli);
    println!("  status  : pending §3 routing wiring (live evidence read path)");
    println!("  next    : seed evidence set from cortex-storage + cortex-api ingest read API");
    Ok(())
}

fn run_scan_now(cli: &Cli) -> Result<()> {
    print_plan_header("scan_now", None, None, cli.deep);
    let _ = build_summarisers(cli)?;
    let _ = budget_from(cli);
    println!("  status  : pending §3 routing wiring (Synap subscriber on events.classified)");
    Ok(())
}

fn run_replay(cli: &Cli, since: &str) -> Result<()> {
    print_plan_header("replay", None, None, cli.deep);
    println!("  since   : {since}");
    let _ = build_summarisers(cli)?;
    let _ = budget_from(cli);
    println!("  status  : pending §3 routing wiring (replay evidence stream)");
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
            "phase11r §2.8 — live `nightly` run lands alongside §3 routing wiring; \
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
    fn cli_parses_rewrite_subcommand() {
        let cli = Cli::try_parse_from([
            "cortex-topic-cards",
            "rewrite",
            "auth-rewrite",
            "--repo",
            "cortex",
        ])
        .expect("parse");
        match cli.command {
            Command::Rewrite { topic_slug, repo } => {
                assert_eq!(topic_slug, "auth-rewrite");
                assert_eq!(repo, "cortex");
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_scan_now_subcommand() {
        let cli = Cli::try_parse_from(["cortex-topic-cards", "scan-now"]).expect("parse");
        assert!(matches!(cli.command, Command::ScanNow));
    }

    #[test]
    fn cli_parses_replay_with_since() {
        let cli = Cli::try_parse_from([
            "cortex-topic-cards",
            "replay",
            "--since",
            "2026-05-03T05:00:00Z",
        ])
        .expect("parse");
        match cli.command {
            Command::Replay { since } => assert_eq!(since, "2026-05-03T05:00:00Z"),
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_nightly_dry_run_default_true() {
        let cli = Cli::try_parse_from(["cortex-topic-cards", "nightly"]).expect("parse");
        match cli.command {
            Command::Nightly { dry_run } => assert!(dry_run),
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_top_level_deep_flag() {
        let cli = Cli::try_parse_from(["cortex-topic-cards", "--deep", "scan-now"]).expect("parse");
        assert!(cli.deep);
    }

    #[test]
    fn require_api_key_errors_when_unset() {
        let cli = Cli {
            verbose: false,
            api_key: None,
            api_url: None,
            monthly_cents_cap: 100_000,
            deep: false,
            command: Command::Nightly { dry_run: true },
        };
        let err = require_api_key(&cli).expect_err("no key");
        assert!(format!("{err:#}").contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn require_api_key_returns_value_when_set() {
        let cli = Cli {
            verbose: false,
            api_key: Some("sk-test-99999".into()),
            api_url: None,
            monthly_cents_cap: 100_000,
            deep: true,
            command: Command::Nightly { dry_run: true },
        };
        assert_eq!(require_api_key(&cli).unwrap(), "sk-test-99999");
    }
}
