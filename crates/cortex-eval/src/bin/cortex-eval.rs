//! Phase14c `cortex-eval` binary — runs a single golden-set
//! suite against a live cortex-api and prints the report.
//!
//! Subcommand-less; selects the suite via `--suite`. Three modes:
//!
//! - `cortex-eval --suite retrieval --golden tests/golden/retrieval.csv [--api-url URL] [--output json|md]`
//! - `cortex-eval --suite consolidation --golden tests/golden/consolidation.csv [--api-url URL]`
//! - `cortex-eval --suite classification --golden tests/golden/classification.csv [--classifier-url URL]`
//!
//! When `--baseline <path>` is supplied, the binary loads the prior
//! JSON report and fails (exit 2) on a regression > `--threshold`
//! (default 0.05 = 5%). Used by the CI gate (phase14c §4).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_eval::report::{is_regression, SuiteReport};
use cortex_eval::suite::{
    classification, consolidation, retrieval, AcceptanceVerdict, SuiteName,
};
use cortex_eval::CI_REGRESSION_THRESHOLD;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "cortex-eval",
    about = "Cortex golden-set eval harness (phase14c). Runs retrieval / consolidation / classification suites against a live stack.",
    version
)]
struct Cli {
    /// Suite to run. One of `retrieval`, `consolidation`,
    /// `classification`.
    #[arg(long)]
    suite: String,
    /// Path to the suite's golden CSV. Defaults to
    /// `tests/golden/<suite>.csv` next to the binary's cwd.
    #[arg(long)]
    golden: Option<PathBuf>,
    /// cortex-api base URL. Defaults to
    /// `$CORTEX_API_URL` then `http://127.0.0.1:17000`.
    #[arg(long, env = "CORTEX_API_URL")]
    api_url: Option<String>,
    /// Classifier worker base URL. Defaults to
    /// `$CORTEX_CLASSIFIER_URL` then `http://127.0.0.1:17021`.
    #[arg(long, env = "CORTEX_CLASSIFIER_URL")]
    classifier_url: Option<String>,
    /// When set, the binary compares the fresh report against the
    /// JSON at this path and fails (exit 2) on regression. Used by
    /// CI.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// CI regression threshold (fraction). Default 0.05 = 5%.
    #[arg(long, default_value_t = CI_REGRESSION_THRESHOLD)]
    threshold: f64,
    /// Output format. `md` (default) or `json`.
    #[arg(long, default_value = "md")]
    output: String,
    /// Verbose logging.
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match run(&cli).await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("cortex-eval error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn init_tracing(verbose: bool) {
    let filter = if verbose { "debug" } else { "info" };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .try_init();
}

async fn run(cli: &Cli) -> Result<ExitCode> {
    let suite = SuiteName::parse(&cli.suite)
        .with_context(|| format!("unknown --suite={}", cli.suite))?;
    let golden = cli
        .golden
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("tests/golden/{}.csv", suite.as_str())));
    let api_url = cli
        .api_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
    let classifier_url = cli
        .classifier_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:17021".to_string());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest build")?;

    let (report, acceptance) = match suite {
        SuiteName::Retrieval => run_retrieval(&http, &api_url, &golden).await?,
        SuiteName::Consolidation => run_consolidation(&http, &api_url, &golden).await?,
        SuiteName::Classification => {
            run_classification(&http, &classifier_url, &golden).await?
        }
    };

    emit(&report, &cli.output)?;

    // CI gate: regression vs baseline.
    if let Some(path) = &cli.baseline {
        let baseline: SuiteReport = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("read baseline {}", path.display()))?,
        )
        .context("parse baseline JSON")?;
        let regressed = regression_summary(&baseline, &report, cli.threshold);
        if !regressed.is_empty() {
            eprintln!("REGRESSION: {} metric(s) slipped > {} vs baseline:", regressed.len(), cli.threshold);
            for (m, delta) in &regressed {
                eprintln!("  {m}: -{delta:.4}");
            }
            return Ok(ExitCode::from(2));
        }
    }

    // Acceptance gate.
    if !acceptance.passed {
        eprintln!(
            "ACCEPTANCE FAILED: {} metric(s) below floor: {:?}",
            acceptance.failed_metrics.len(),
            acceptance.failed_metrics
        );
        return Ok(ExitCode::from(3));
    }

    Ok(ExitCode::SUCCESS)
}

fn emit(report: &SuiteReport, output: &str) -> Result<()> {
    match output {
        "json" => {
            println!("{}", serde_json::to_string_pretty(report)?);
        }
        _ => {
            print!("{}", report.to_markdown());
        }
    }
    Ok(())
}

fn regression_summary(
    baseline: &SuiteReport,
    current: &SuiteReport,
    threshold: f64,
) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for m in &baseline.metrics {
        // Only check floored metrics (advisory rows skip the gate).
        if m.floor.is_none() {
            continue;
        }
        if let Some(cur) = current.metric(&m.name) {
            if is_regression(m.value, cur.value, threshold) {
                out.push((m.name.clone(), m.value - cur.value));
            }
        }
    }
    out
}

// ---- per-suite drivers --------------------------------------------------

async fn run_retrieval(
    http: &reqwest::Client,
    api_url: &str,
    golden: &std::path::Path,
) -> Result<(SuiteReport, AcceptanceVerdict)> {
    let rows = retrieval::load_csv(golden)?;
    let mut observed: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let body = serde_json::json!({
            "query": row.query,
            "repo": row.repo,
            "limit": 10,
        });
        let url = format!("{}/v1/query", api_url.trim_end_matches('/'));
        let resp = match http.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(row = %row.id, error = %err, "retrieval: HTTP error, treating as empty");
                observed.push(Vec::new());
                continue;
            }
        };
        if !resp.status().is_success() {
            tracing::warn!(row = %row.id, status = %resp.status(), "retrieval: non-2xx, treating as empty");
            observed.push(Vec::new());
            continue;
        }
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        let hits = json
            .get("hits")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let ids: Vec<String> = hits
            .iter()
            .filter_map(|h| {
                h.get("event_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        observed.push(ids);
    }
    let report = retrieval::build_report(&rows, &observed);
    let verdict = retrieval::retrieval_acceptance(&report);
    Ok((report, verdict))
}

async fn run_consolidation(
    http: &reqwest::Client,
    api_url: &str,
    golden: &std::path::Path,
) -> Result<(SuiteReport, AcceptanceVerdict)> {
    let rows = consolidation::load_csv(golden)?;
    let mut observed: Vec<String> = Vec::with_capacity(rows.len());
    for row in &rows {
        let url = format!(
            "{}/v1/dashboard/consolidations?session_id={}",
            api_url.trim_end_matches('/'),
            row.session_id
        );
        let body_text = match http.get(&url).send().await {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(row = %row.id, error = %err, "consolidation: HTTP error, treating as empty");
                observed.push(String::new());
                continue;
            }
        };
        observed.push(body_text);
    }
    let report = consolidation::build_report(&rows, &observed);
    let verdict = consolidation::consolidation_acceptance(&report);
    Ok((report, verdict))
}

async fn run_classification(
    http: &reqwest::Client,
    classifier_url: &str,
    golden: &std::path::Path,
) -> Result<(SuiteReport, AcceptanceVerdict)> {
    let rows = classification::load_csv(golden)?;
    let mut predicted: Vec<String> = Vec::with_capacity(rows.len());
    for row in &rows {
        let url = format!("{}/v1/classify", classifier_url.trim_end_matches('/'));
        let envelope: serde_json::Value =
            serde_json::from_str(&row.envelope_json).unwrap_or(serde_json::Value::Null);
        let resp = match http.post(&url).json(&envelope).send().await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(row = %row.id, error = %err, "classification: HTTP error, treating as Unknown");
                predicted.push("Unknown".into());
                continue;
            }
        };
        if !resp.status().is_success() {
            predicted.push("Unknown".into());
            continue;
        }
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        let kind = json
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        predicted.push(kind);
    }
    let report = classification::build_report(&rows, &predicted);
    let verdict = classification::classification_acceptance(&report);
    Ok((report, verdict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_eval::report::MetricRow;

    fn baseline_report() -> SuiteReport {
        SuiteReport {
            suite: "retrieval".into(),
            finished_at: "t".into(),
            rows_total: 1,
            rows_errored: 0,
            metrics: vec![
                MetricRow {
                    name: "mrr_at_10".into(),
                    value: 0.70,
                    floor: Some(0.60),
                    passed: true,
                },
                MetricRow {
                    name: "advisory".into(),
                    value: 0.5,
                    floor: None,
                    passed: true,
                },
            ],
            per_row: Default::default(),
        }
    }

    fn current(value: f64) -> SuiteReport {
        SuiteReport {
            suite: "retrieval".into(),
            finished_at: "t".into(),
            rows_total: 1,
            rows_errored: 0,
            metrics: vec![MetricRow {
                name: "mrr_at_10".into(),
                value,
                floor: Some(0.60),
                passed: value >= 0.60,
            }],
            per_row: Default::default(),
        }
    }

    #[test]
    fn regression_summary_flags_drop_over_threshold() {
        let r = regression_summary(&baseline_report(), &current(0.60), 0.05);
        // 0.70 -> 0.60 = 0.10 drop > 0.05
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, "mrr_at_10");
    }

    #[test]
    fn regression_summary_ignores_advisory_metrics() {
        // Advisory (no floor) metric drop must not count.
        let mut cur = current(0.70);
        cur.metrics.push(MetricRow {
            name: "advisory".into(),
            value: 0.01,
            floor: None,
            passed: true,
        });
        let r = regression_summary(&baseline_report(), &cur, 0.05);
        assert!(r.is_empty());
    }

    #[test]
    fn regression_summary_skips_small_drops() {
        let r = regression_summary(&baseline_report(), &current(0.68), 0.05);
        assert!(r.is_empty(), "0.70->0.68 = 0.02 below threshold");
    }
}
