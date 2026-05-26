//! Phase14h §3.3 — `cortex-ops doctor-synap-workers`.
//!
//! Probes the four Synap-worker `/healthz` endpoints (embedder /
//! fulltext / graph / classifier) and prints a cross-cut table:
//!
//! | worker     | state | last_consume_age_s | consec_err | lag |
//!
//! `last_consume_ts_ms` + `consume_errors_consecutive` are
//! surfaced via the workers' health extras (classifier sets them
//! per phase11s §1.3; the other three workers expose
//! `last_job_ts_ms` which we fall back to). `lag` reads
//! `cortex_synap_worker_lag{worker=...}` from the worker's
//! `/metrics` body when present; otherwise renders as `n/a`.

use std::process::ExitCode;
use std::time::Duration;

use serde::Serialize;

/// One row in the doctor table.
#[derive(Debug, Clone, Serialize)]
struct WorkerRow {
    worker: &'static str,
    url: String,
    state: String,
    last_consume_age_s: Option<i64>,
    consume_errors_consecutive: u64,
    lag: Option<u64>,
    error: Option<String>,
}

/// Doctor entry point.
pub fn run(
    embedder_url: Option<String>,
    fulltext_url: Option<String>,
    graph_url: Option<String>,
    classifier_url: Option<String>,
    json: bool,
) -> ExitCode {
    let embedder = resolve_url(
        embedder_url,
        "CORTEX_EMBEDDER_HEALTH_URL",
        "http://127.0.0.1:17100/healthz",
    );
    let fulltext = resolve_url(
        fulltext_url,
        "CORTEX_FULLTEXT_HEALTH_URL",
        "http://127.0.0.1:17110/healthz",
    );
    let graph = resolve_url(
        graph_url,
        "CORTEX_GRAPH_HEALTH_URL",
        "http://127.0.0.1:17120/healthz",
    );
    let classifier = resolve_url(
        classifier_url,
        "CORTEX_CLASSIFIER_HEALTH_URL",
        "http://127.0.0.1:17130/healthz",
    );

    let rows = vec![
        probe("embedder", &embedder),
        probe("fulltext", &fulltext),
        probe("graph", &graph),
        probe("classifier", &classifier),
    ];

    if json {
        let payload = serde_json::json!({ "rows": rows });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("doctor-synap-workers: serialise failed: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        print_table(&rows);
    }

    let any_fail = rows
        .iter()
        .any(|r| r.error.is_some() || r.state != "ok");
    if any_fail {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn resolve_url(explicit: Option<String>, env_key: &str, default: &str) -> String {
    explicit
        .or_else(|| std::env::var(env_key).ok())
        .unwrap_or_else(|| default.to_string())
}

fn probe(worker: &'static str, url: &str) -> WorkerRow {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return WorkerRow {
                worker,
                url: url.to_string(),
                state: "error".into(),
                last_consume_age_s: None,
                consume_errors_consecutive: 0,
                lag: None,
                error: Some(format!("client build: {e}")),
            }
        }
    };
    let resp = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            return WorkerRow {
                worker,
                url: url.to_string(),
                state: "unreachable".into(),
                last_consume_age_s: None,
                consume_errors_consecutive: 0,
                lag: None,
                error: Some(format!("get: {e}")),
            }
        }
    };
    let status = resp.status();
    let body = match resp.text() {
        Ok(b) => b,
        Err(e) => {
            return WorkerRow {
                worker,
                url: url.to_string(),
                state: format!("http {}", status.as_u16()),
                last_consume_age_s: None,
                consume_errors_consecutive: 0,
                lag: None,
                error: Some(format!("read body: {e}")),
            }
        }
    };
    parse_health_row(worker, url, &body)
}

fn parse_health_row(worker: &'static str, url: &str, body: &str) -> WorkerRow {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return WorkerRow {
                worker,
                url: url.to_string(),
                state: "invalid_json".into(),
                last_consume_age_s: None,
                consume_errors_consecutive: 0,
                lag: None,
                error: Some(format!("parse: {e}")),
            }
        }
    };
    let state = parsed
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let extras = parsed
        .get("extras")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let last_consume_ts_ms = extras
        .get("last_consume_ts_ms")
        .and_then(|v| v.as_u64())
        .or_else(|| extras.get("last_job_ts_ms").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let last_consume_age_s = if last_consume_ts_ms == 0 {
        None
    } else {
        Some(((now_ms.saturating_sub(last_consume_ts_ms)) / 1000) as i64)
    };
    let consume_errors_consecutive = extras
        .get("consume_errors_consecutive")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let lag = extras
        .get("synap_worker_lag")
        .and_then(|v| v.as_u64());
    WorkerRow {
        worker,
        url: url.to_string(),
        state,
        last_consume_age_s,
        consume_errors_consecutive,
        lag,
        error: None,
    }
}

fn print_table(rows: &[WorkerRow]) {
    println!("cortex-ops doctor-synap-workers");
    println!(
        "  {:<11} {:<10} {:>18} {:>11} {:>8}  url",
        "worker", "state", "last_consume(s)", "consec_err", "lag"
    );
    for r in rows {
        let age = match r.last_consume_age_s {
            Some(n) => n.to_string(),
            None => "n/a".to_string(),
        };
        let lag = match r.lag {
            Some(n) => n.to_string(),
            None => "n/a".to_string(),
        };
        println!(
            "  {:<11} {:<10} {:>18} {:>11} {:>8}  {}",
            r.worker, r.state, age, r.consume_errors_consecutive, lag, r.url
        );
        if let Some(e) = &r.error {
            println!("    error: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_health_row_extracts_consume_errors_and_lag_from_extras() {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let body = serde_json::json!({
            "state": "ok",
            "last_error": null,
            "extras": {
                "last_consume_ts_ms": (now_ms - 5_000i64) as u64,
                "consume_errors_consecutive": 2,
                "synap_worker_lag": 7,
            }
        })
        .to_string();
        let r = parse_health_row("embedder", "http://localhost:17100/healthz", &body);
        assert_eq!(r.worker, "embedder");
        assert_eq!(r.state, "ok");
        assert_eq!(r.consume_errors_consecutive, 2);
        assert_eq!(r.lag, Some(7));
        let age = r.last_consume_age_s.unwrap();
        assert!(
            (4..=10).contains(&age),
            "last_consume_age_s should be close to 5s, got {age}"
        );
        assert!(r.error.is_none());
    }

    #[test]
    fn parse_health_row_falls_back_to_last_job_ts_when_consume_ts_absent() {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let body = serde_json::json!({
            "state": "ok",
            "extras": {
                "last_job_ts_ms": (now_ms - 12_000i64) as u64,
            }
        })
        .to_string();
        let r = parse_health_row("fulltext", "http://x/healthz", &body);
        let age = r.last_consume_age_s.unwrap();
        assert!((11..=14).contains(&age), "age={age}");
        assert_eq!(r.consume_errors_consecutive, 0);
        assert!(r.lag.is_none());
    }

    #[test]
    fn parse_health_row_handles_invalid_json() {
        let r = parse_health_row("graph", "http://x/healthz", "{not json}");
        assert_eq!(r.state, "invalid_json");
        assert!(r.error.is_some());
    }

    #[test]
    fn parse_health_row_handles_missing_extras() {
        let body = serde_json::json!({"state": "ok"}).to_string();
        let r = parse_health_row("classifier", "http://x/healthz", &body);
        assert_eq!(r.state, "ok");
        assert!(r.last_consume_age_s.is_none());
        assert_eq!(r.consume_errors_consecutive, 0);
        assert!(r.lag.is_none());
        assert!(r.error.is_none());
    }

    #[test]
    fn resolve_url_prefers_explicit_then_env_then_default() {
        // SAFETY-ish: each branch isolates the unique env key.
        let key = "TEST_CORTEX_DOC_SYNAP_W_RESOLVE_KEY";
        std::env::remove_var(key);
        assert_eq!(
            resolve_url(None, key, "http://default/"),
            "http://default/"
        );
        std::env::set_var(key, "http://from-env/");
        assert_eq!(
            resolve_url(None, key, "http://default/"),
            "http://from-env/"
        );
        assert_eq!(
            resolve_url(Some("http://explicit/".into()), key, "http://default/"),
            "http://explicit/"
        );
        std::env::remove_var(key);
    }
}
