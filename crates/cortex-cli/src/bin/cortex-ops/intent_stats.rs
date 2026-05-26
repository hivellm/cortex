//! Phase14g §2.3 — `cortex-ops intent-stats` subcommand.
//!
//! Fetches `/v1/health/pre-thinking` (extended in phase14g with
//! `intent_mismatch_top` + `rewriter_path_total`) and prints the
//! per-(from, to) mismatch table + the per-path rewriter cascade
//! counts. The `--since` flag is accepted today as a forward-
//! compat label that the endpoint will honour once it surfaces
//! windowed counters; the current implementation always reports
//! lifetime counters since process boot.

use std::process::ExitCode;
use std::time::Duration;

use serde::Deserialize;

pub fn run(api_url: Option<String>, since: String, json: bool) -> ExitCode {
    let base = api_url
        .or_else(|| std::env::var("CORTEX_API_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
    let url = format!("{}/v1/health/pre-thinking", base.trim_end_matches('/'));
    let report = match fetch(&url) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("intent-stats: fetch failed: {e}");
            return ExitCode::from(1);
        }
    };
    if json {
        let payload = serde_json::json!({
            "api_url": base,
            "since": since,
            "report": report,
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("intent-stats: serialise failed: {e}");
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }
    print_text(&base, &since, &report);
    ExitCode::SUCCESS
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct ReportShape {
    #[serde(default)]
    breaker_state: String,
    #[serde(default)]
    intent_mismatch_top: Vec<MismatchRow>,
    #[serde(default)]
    rewriter_path_total: std::collections::BTreeMap<String, u64>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct MismatchRow {
    from: String,
    to: String,
    count: u64,
}

fn fetch(url: &str) -> anyhow::Result<ReportShape> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let resp = client.get(url).send()?.error_for_status()?;
    Ok(resp.json::<ReportShape>()?)
}

fn print_text(api_url: &str, since: &str, r: &ReportShape) {
    println!("cortex-ops intent-stats");
    println!("  api      : {api_url}");
    println!("  since    : {since}");
    println!("  breaker  : {}", r.breaker_state);
    println!();
    println!("intent mismatches (from -> to | count)");
    if r.intent_mismatch_top.is_empty() {
        println!("  (none — feedback recorder has not flagged any mismatch)");
    } else {
        for row in &r.intent_mismatch_top {
            println!("  {:24} -> {:24} | {}", row.from, row.to, row.count);
        }
    }
    println!();
    println!("rewriter cascade paths");
    if r.rewriter_path_total.is_empty() {
        println!("  (none — rewriter cascade has not run yet)");
    } else {
        let total: u64 = r.rewriter_path_total.values().copied().sum();
        for (path, n) in &r.rewriter_path_total {
            let pct = if total > 0 {
                100.0 * (*n as f64) / (total as f64)
            } else {
                0.0
            };
            println!("  {:28} {:>8}  ({:>5.1}%)", path, n, pct);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_shape_round_trips_via_serde() {
        let mut paths = std::collections::BTreeMap::new();
        paths.insert("sonnet_hit".to_string(), 4u64);
        paths.insert("sonnet_cache_hit".to_string(), 2u64);
        let r = ReportShape {
            breaker_state: "closed".into(),
            intent_mismatch_top: vec![
                MismatchRow {
                    from: "explain".into(),
                    to: "decision_lookup".into(),
                    count: 3,
                },
                MismatchRow {
                    from: "pre_change_context".into(),
                    to: "similar_problems".into(),
                    count: 1,
                },
            ],
            rewriter_path_total: paths,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: ReportShape = serde_json::from_str(&j).unwrap();
        assert_eq!(back.breaker_state, "closed");
        assert_eq!(back.intent_mismatch_top.len(), 2);
        assert_eq!(back.rewriter_path_total.get("sonnet_hit").copied(), Some(4));
    }

    #[test]
    fn empty_report_renders_idle_messages() {
        let r = ReportShape {
            breaker_state: "closed".into(),
            intent_mismatch_top: vec![],
            rewriter_path_total: std::collections::BTreeMap::new(),
        };
        // Smoke — just exercise the formatter; we don't assert on
        // stdout because the test framework swallows println!.
        print_text("http://x", "lifetime", &r);
    }
}
