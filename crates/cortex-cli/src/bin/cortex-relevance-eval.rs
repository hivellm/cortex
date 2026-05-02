//! `cortex-relevance-eval` — CLI front-end for the harness.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_cli::relevance_eval::{
    harness::{run_harness, HarnessOptions, HttpFetcher},
    queries::QuerySet,
    report::{RegressionVerdict, RelevanceReport},
};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "cortex-relevance-eval",
    about = "Run the relevance harness (recall@10 + MRR) against cortex-api.",
    long_about = "Replays a labeled query set against a running cortex-api, computes \
                  per-intent + global recall@10 and MRR, emits a deterministic JSON \
                  report, and (when --baseline is supplied) gates on a 2pp regression."
)]
struct Cli {
    /// Base URL for cortex-api.
    #[arg(long, default_value = "http://127.0.0.1:17000")]
    api_url: String,
    /// Path to the labeled query set fixture (TOML).
    #[arg(long, default_value = "tests/relevance/queries.toml")]
    query_set: PathBuf,
    /// Output directory for the report (`<dir>/<basename>.json`).
    #[arg(long, default_value = "target/relevance")]
    out_dir: PathBuf,
    /// Basename for the JSON report — defaults to `<git-sha>`.
    #[arg(long)]
    out_basename: Option<String>,
    /// Per-query budget in milliseconds (propagated to cortex-api +
    /// used as the HTTP timeout).
    #[arg(long, default_value_t = 1_500)]
    budget_ms: u64,
    /// Top-k window for recall@k. Spec: 10.
    #[arg(long, default_value_t = 10)]
    top_k: usize,
    /// Optional baseline report — when provided, the harness exits
    /// `2` on a hard regression (>2pp drop on global recall or MRR*100).
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Absolute pp threshold for the regression gate. Defaults to
    /// the spec's `2.0`.
    #[arg(long, default_value_t = 2.0)]
    threshold_pp: f64,
}

fn detect_git_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,cortex_relevance_eval=info")),
        )
        .with_target(false)
        .try_init();

    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let set = QuerySet::load(&cli.query_set)
        .with_context(|| format!("load query set {}", cli.query_set.display()))?;

    let opts = HarnessOptions {
        api_url: cli.api_url.clone(),
        budget_ms: cli.budget_ms,
        top_k: cli.top_k,
    };

    let git_sha = detect_git_sha();
    let fetcher = HttpFetcher::new(&cli.api_url, cli.budget_ms)?;
    let report = run_harness(&fetcher, &set, &opts, &git_sha).await?;

    let basename = cli.out_basename.clone().unwrap_or_else(|| git_sha.clone());
    let path = report
        .write_pretty(&cli.out_dir, &basename)
        .context("write report")?;
    eprintln!("wrote report → {}", path.display());

    print_summary(&report);

    let verdict = if let Some(baseline_path) = cli.baseline.as_ref() {
        let baseline = RelevanceReport::load(baseline_path)
            .with_context(|| format!("load baseline {}", baseline_path.display()))?;
        let v = RegressionVerdict::evaluate(&report, &baseline, cli.threshold_pp);
        print_regression(&v);
        Some(v)
    } else {
        None
    };

    let exit = match verdict {
        Some(v) if v.hard_regression => ExitCode::from(2),
        _ => ExitCode::from(0),
    };
    Ok(exit)
}

fn print_summary(report: &RelevanceReport) {
    println!();
    println!("=== Relevance harness ===");
    println!("git_sha:        {}", report.git_sha);
    println!("generated_at:   {}", report.generated_at);
    println!(
        "api_version:    {}",
        report.api_version.as_deref().unwrap_or("unknown")
    );
    if !report.omitted_intents.is_empty() {
        println!("omitted:        {}", report.omitted_intents.join(", "));
    }
    println!();
    println!(
        "{:<22} {:>8} {:>10} {:>10}",
        "intent", "queries", "recall@10", "mrr_avg"
    );
    println!("{}", "-".repeat(54));
    for (intent, scores) in &report.per_intent {
        println!(
            "{:<22} {:>8} {:>9.2}% {:>10.4}",
            intent, scores.total, scores.recall_at_10_pct, scores.mrr_avg
        );
    }
    println!("{}", "-".repeat(54));
    println!(
        "{:<22} {:>8} {:>9.2}% {:>10.4}",
        "GLOBAL", report.global.total, report.global.recall_at_10_pct, report.global.mrr_avg
    );
    println!();
}

fn print_regression(v: &RegressionVerdict) {
    println!(
        "=== Regression vs baseline (threshold {:.1}pp) ===",
        v.threshold_pp
    );
    println!(
        "global recall delta: {:+.2}pp   global mrr delta: {:+.4}",
        v.recall_delta_pp, v.mrr_delta
    );
    if !v.soft_regressions.is_empty() {
        println!(
            "⚠ per-intent soft regressions: {}",
            v.soft_regressions.join(", ")
        );
    }
    if !v.worst_queries.is_empty() {
        println!("worst regressed queries: {}", v.worst_queries.join(", "));
    }
    if v.hard_regression {
        eprintln!(
            "✗ HARD REGRESSION — global metrics dropped beyond {:.1}pp",
            v.threshold_pp
        );
    } else {
        println!("✓ within threshold");
    }
    println!();
}
