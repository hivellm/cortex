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
    access_control, classification, consolidation, mcp_search, retrieval, AcceptanceVerdict,
    SuiteName,
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
    let suite =
        SuiteName::parse(&cli.suite).with_context(|| format!("unknown --suite={}", cli.suite))?;
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
        SuiteName::Classification => run_classification(&http, &classifier_url, &golden).await?,
        SuiteName::McpSearch => run_mcp_search(&http, &api_url, &golden).await?,
        SuiteName::AccessControl => run_access_control(&golden).await?,
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
            eprintln!(
                "REGRESSION: {} metric(s) slipped > {} vs baseline:",
                regressed.len(),
                cli.threshold
            );
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
        // phase0 2026-06-22 — match the live /v1/query contract: `intent`
        // is required, `repo` lives under `scope`, and the response is
        // `results.snippets[]` keyed by `path` (content-addressed), not
        // `hits[].event_id`.
        let body = serde_json::json!({
            "query": row.query,
            "intent": "free_search",
            "scope": { "repo": row.repo },
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
        let snippets = json
            .get("results")
            .and_then(|r| r.get("snippets"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let ids: Vec<String> = snippets
            .iter()
            .filter_map(|h| {
                h.get("path")
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
            "{}/v1/dashboard/consolidations/{}",
            api_url.trim_end_matches('/'),
            row.consolidation_id
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

/// Phase19 §5.3 — `mcp_search` driver. Routes each row to the
/// matching `cortex-api` endpoint per `row.tool` and folds the
/// observed top-K identifier list back through `build_report`. A
/// row whose `tool` is unknown is treated as empty so the suite
/// still completes (the recall floor catches it).
async fn run_mcp_search(
    http: &reqwest::Client,
    api_url: &str,
    golden: &std::path::Path,
) -> Result<(SuiteReport, AcceptanceVerdict)> {
    let rows = mcp_search::load_csv(golden)?;
    let mut observed: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let ids = run_mcp_search_row(http, api_url, row).await;
        observed.push(ids);
    }
    let report = mcp_search::build_report(&rows, &observed);
    let verdict = mcp_search::mcp_search_acceptance(&report);
    Ok((report, verdict))
}

/// Per-row dispatch. Returns the top-K identifier list. Each tool
/// maps query/value semantics + the response shape; unknown tools
/// log a warning and return an empty Vec.
async fn run_mcp_search_row(
    http: &reqwest::Client,
    api_url: &str,
    row: &mcp_search::McpSearchRow,
) -> Vec<String> {
    let base = api_url.trim_end_matches('/');
    let (url, body, key) = match row.tool.as_str() {
        "cortex_events_by_kind" => (
            format!("{base}/v1/search/events"),
            serde_json::json!({"kind": row.query, "repo": row.repo, "limit": 5}),
            "event_id",
        ),
        "cortex_tool_calls" => (
            format!("{base}/v1/search/tool-calls"),
            serde_json::json!({"tool_name": row.query, "repo": row.repo, "limit": 5}),
            "event_id",
        ),
        "cortex_topic_search" => (
            format!("{base}/v1/topic-cards/search"),
            serde_json::json!({"topic_prefix": row.query, "repo": row.repo, "limit": 5}),
            "event_id",
        ),
        "cortex_consolidations_recent" => (
            format!("{base}/v1/consolidations/recent?limit=5&repo={}", row.repo),
            serde_json::Value::Null,
            "event_id",
        ),
        "cortex_consolidations_by_entity" => (
            format!("{base}/v1/consolidations/by-entity"),
            serde_json::json!({
                "entity": {"kind": "decision_id", "value": row.query},
                "limit": 5
            }),
            "event_id",
        ),
        "cortex_consolidations_search" => (
            format!("{base}/v1/consolidations/search"),
            serde_json::json!({"query": row.query, "repo": row.repo, "k": 5}),
            "event_id",
        ),
        "cortex_decision_search" => (
            format!("{base}/v1/decisions/search"),
            serde_json::json!({"status": row.query, "repo": row.repo, "limit": 5}),
            "event_id",
        ),
        "cortex_law_violations" => (
            format!("{base}/v1/laws/violations"),
            serde_json::json!({"repo": row.repo, "law_id": row.query, "limit": 5}),
            "event_id",
        ),
        "cortex_files_touched" => (
            format!("{base}/v1/search/files-touched"),
            serde_json::json!({"repo": row.repo, "limit": 5}),
            "path",
        ),
        // Tools without a stable "top-K id list" semantic
        // (`session_timeline`, `consolidation_get`, `consolidation_lineage`,
        // `consolidations_diff`, `feedback_signals`,
        // `consolidation_costs`, `query_explain`) are exercised by
        // their wiremock ITs; the eval suite skips them so the
        // recall@5 metric stays interpretable.
        other => {
            tracing::warn!(
                row = %row.id,
                tool = %other,
                "mcp_search: tool not wired into eval driver, returning empty"
            );
            return Vec::new();
        }
    };
    let resp = if body.is_null() {
        http.get(&url).send().await
    } else {
        http.post(&url).json(&body).send().await
    };
    let resp = match resp {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(row = %row.id, error = %err, "mcp_search: HTTP error");
            return Vec::new();
        }
    };
    if !resp.status().is_success() {
        tracing::warn!(row = %row.id, status = %resp.status(), "mcp_search: non-2xx");
        return Vec::new();
    }
    let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    let hits = json
        .get("hits")
        .or_else(|| json.get("paths"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    hits.iter()
        .filter_map(|h| h.get(key).and_then(|v| v.as_str()).map(String::from))
        .take(5)
        .collect()
}

/// Phase21 §9.1 — access-control golden suite driver.
///
/// Unlike the other suites, this driver evaluates the Bell-LaPadula
/// predicate locally (no HTTP stack required). Each row in the CSV
/// is evaluated by `can_read_local` and the result is compared to
/// the `expected` label. The acceptance gate is `false_grant_count == 0`.
async fn run_access_control(
    golden: &std::path::Path,
) -> Result<(SuiteReport, AcceptanceVerdict)> {
    let rows = access_control::load_csv(golden)?;
    let observed: Vec<bool> = rows.iter().map(|r| r.evaluate()).collect();
    let report = access_control::build_report(&rows, &observed);
    let acceptance = access_control::access_control_acceptance(&report);
    Ok((report, acceptance))
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
