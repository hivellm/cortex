//! Phase18 §7.3 — `cortex-ops temporal-digest`.
//!
//! Exports a digest of the temporal/branch/cross-project observability
//! signals from cortex-api's `/metrics` endpoint. The digest can be
//! rendered as Markdown (default) or JSON (`--json`).

use std::collections::BTreeMap;
use std::process::ExitCode;

use serde_json::json;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Parsed snapshot of the phase18 temporal metric families.
#[derive(Debug, Default)]
pub(super) struct TemporalDigestData {
    /// `cortex_temporal_queries_total`
    pub queries: u64,
    /// `cortex_temporal_queries_as_of_total`
    pub queries_as_of: u64,
    /// `cortex_temporal_classified_total{state="<s>"}` keyed by state.
    pub classified: BTreeMap<String, u64>,
    /// `cortex_temporal_action_total{action="<a>"}` keyed by action.
    pub actions: BTreeMap<String, u64>,
    /// `cortex_branch_queries_total{kind="<k>"}` keyed by kind.
    pub branch: BTreeMap<String, u64>,
    /// `cortex_cross_project_propagation_runs_total`
    pub cross_runs: u64,
    /// `cortex_cross_project_discovered_total`
    pub cross_discovered: u64,
    /// `cortex_cross_project_propagated_total`
    pub cross_propagated: u64,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Phase18 §7.3 — export a digest of the temporal/branch/cross-project
/// signals from cortex-api's `/metrics` endpoint.
pub(super) fn temporal_digest(api_url: Option<String>, json: bool) -> ExitCode {
    let api = resolve_api_url(api_url.clone());
    let url = format!("{}/metrics", api.trim_end_matches('/'));

    let body = match fetch_metrics(&url) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ERROR: GET {url}: {e}");
            return ExitCode::from(2);
        }
    };

    let data = parse_temporal_metrics(&body);

    if json {
        print_json(&data, &api);
    } else {
        print_markdown(&data);
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// HTTP fetch — reqwest blocking client inside a tokio runtime, matching
// the pattern used in query_cmd.rs.
// ---------------------------------------------------------------------------

fn resolve_api_url(opt: Option<String>) -> String {
    opt.or_else(|| {
        cortex_config::Config::load()
            .unwrap_or_default()
            .dashboard
            .api_url
    })
    .unwrap_or_else(|| "http://127.0.0.1:17000".to_string())
}

fn fetch_metrics(url: &str) -> Result<String, String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("build tokio runtime: {e}"))?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("build http client: {e}"))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("connect: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                text.chars().take(256).collect::<String>()
            ));
        }
        Ok(text)
    })
}

// ---------------------------------------------------------------------------
// Pure parser — unit-testable, no I/O.
// ---------------------------------------------------------------------------

/// Parse a Prometheus text-format metrics body into a [`TemporalDigestData`].
///
/// Lines starting with `#` (HELP/TYPE) are ignored. Metric families not in
/// the phase18 temporal set are ignored. Recognises two line forms:
/// - `name{label="value"} N`
/// - `name N`
pub(super) fn parse_temporal_metrics(body: &str) -> TemporalDigestData {
    let mut data = TemporalDigestData::default();

    for line in body.lines() {
        let line = line.trim();
        // Skip comment / metadata lines.
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(v) = parse_scalar(line, "cortex_temporal_queries_as_of_total") {
            data.queries_as_of = v;
        } else if let Some(v) = parse_scalar(line, "cortex_temporal_queries_total") {
            // Must come AFTER the as_of check so the prefix match doesn't
            // swallow `cortex_temporal_queries_as_of_total`.
            data.queries = v;
        } else if let Some((label_val, v)) =
            parse_labelled(line, "cortex_temporal_classified_total", "state")
        {
            data.classified.insert(label_val, v);
        } else if let Some((label_val, v)) =
            parse_labelled(line, "cortex_temporal_action_total", "action")
        {
            data.actions.insert(label_val, v);
        } else if let Some((label_val, v)) =
            parse_labelled(line, "cortex_branch_queries_total", "kind")
        {
            data.branch.insert(label_val, v);
        } else if let Some(v) = parse_scalar(line, "cortex_cross_project_propagation_runs_total") {
            data.cross_runs = v;
        } else if let Some(v) = parse_scalar(line, "cortex_cross_project_discovered_total") {
            data.cross_discovered = v;
        } else if let Some(v) = parse_scalar(line, "cortex_cross_project_propagated_total") {
            data.cross_propagated = v;
        }
        // All other lines (loader metrics, etc.) fall through and are ignored.
    }

    data
}

/// Match `name N` (no labels).  Returns `Some(N)` on a match.
fn parse_scalar(line: &str, name: &str) -> Option<u64> {
    // The metric name must be followed by a space (scalar form, no `{`).
    let prefix = format!("{name} ");
    if line.starts_with(&prefix) {
        let rest = line[prefix.len()..].trim();
        // Prometheus may use float notation for counters (e.g. `42.0`).
        return rest.parse::<f64>().ok().map(|f| f as u64);
    }
    None
}

/// Match `name{label="value"} N`.  Returns `Some((value, N))` on a match.
fn parse_labelled(line: &str, name: &str, label: &str) -> Option<(String, u64)> {
    // Line must start with `name{`.
    let name_brace = format!("{name}{{");
    if !line.starts_with(&name_brace) {
        return None;
    }
    // Extract the label set string between `{` and `}`.
    let brace_open = line.find('{')?;
    let brace_close = line.find('}')?;
    if brace_close <= brace_open {
        return None;
    }
    let labels_str = &line[brace_open + 1..brace_close];

    // Find `label="value"` inside the label set.
    let key_prefix = format!("{label}=\"");
    let key_start = labels_str.find(&key_prefix)?;
    let val_start = key_start + key_prefix.len();
    let val_end = labels_str[val_start..].find('"')? + val_start;
    let label_value = labels_str[val_start..val_end].to_string();

    // The value follows `} N`.
    let after_brace = line[brace_close + 1..].trim();
    let n = after_brace.parse::<f64>().ok().map(|f| f as u64)?;
    Some((label_value, n))
}

// ---------------------------------------------------------------------------
// Derived signals — pure, testable helper.
// ---------------------------------------------------------------------------

/// Compute the derived ratios from a parsed snapshot.
/// All divisions guard against divide-by-zero by returning 0.0.
pub(super) fn compute_ratios(data: &TemporalDigestData) -> DerivedRatios {
    let as_of_ratio = if data.queries > 0 {
        data.queries_as_of as f64 / data.queries as f64
    } else {
        0.0
    };

    let classified_total: u64 = data.classified.values().sum();
    let classified_shares: BTreeMap<String, f64> = data
        .classified
        .iter()
        .map(|(k, v)| {
            let share = if classified_total > 0 {
                *v as f64 / classified_total as f64
            } else {
                0.0
            };
            (k.clone(), share)
        })
        .collect();

    let cross_hit_ratio = if data.cross_discovered > 0 {
        data.cross_propagated as f64 / data.cross_discovered as f64
    } else {
        0.0
    };

    let branch_sum: u64 = data.branch.values().sum();
    let branch_main_share = if branch_sum > 0 {
        data.branch.get("main").copied().unwrap_or(0) as f64 / branch_sum as f64
    } else {
        0.0
    };

    DerivedRatios {
        as_of_ratio,
        classified_shares,
        cross_hit_ratio,
        branch_main_share,
    }
}

/// Derived ratio signals computed from a [`TemporalDigestData`].
#[derive(Debug)]
pub(super) struct DerivedRatios {
    pub as_of_ratio: f64,
    pub classified_shares: BTreeMap<String, f64>,
    pub cross_hit_ratio: f64,
    pub branch_main_share: f64,
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

fn print_markdown(data: &TemporalDigestData) {
    let r = compute_ratios(data);

    println!("# Cortex temporal digest\n");

    // -- Queries ---------------------------------------------------------------
    println!("## Queries\n");
    println!("| Metric | Value |");
    println!("|--------|-------|");
    println!("| Total queries | {} |", data.queries);
    println!("| As-of queries | {} |", data.queries_as_of);
    println!("| As-of ratio | {:.1}% |", r.as_of_ratio * 100.0);
    println!();

    // -- Classifier states -----------------------------------------------------
    println!("## Classifier states\n");
    println!("| State | Count | Share |");
    println!("|-------|-------|-------|");
    for (state, count) in &data.classified {
        let share = r.classified_shares.get(state).copied().unwrap_or(0.0);
        println!("| {state} | {count} | {:.1}% |", share * 100.0);
    }
    if data.classified.is_empty() {
        println!("| _(none)_ | 0 | — |");
    }
    println!();

    // -- Classifier actions ----------------------------------------------------
    println!("## Classifier actions\n");
    println!("| Action | Count |");
    println!("|--------|-------|");
    for (action, count) in &data.actions {
        println!("| {action} | {count} |");
    }
    if data.actions.is_empty() {
        println!("| _(none)_ | 0 |");
    }
    println!();

    // -- Branch usage ----------------------------------------------------------
    println!("## Branch usage\n");
    println!("| Kind | Count | Share |");
    println!("|------|-------|-------|");
    let branch_sum: u64 = data.branch.values().sum();
    for (kind, count) in &data.branch {
        let share = if branch_sum > 0 {
            *count as f64 / branch_sum as f64
        } else {
            0.0
        };
        println!("| {kind} | {count} | {:.1}% |", share * 100.0);
    }
    if data.branch.is_empty() {
        println!("| _(none)_ | 0 | — |");
    }
    println!("\nMain-branch share: {:.1}%", r.branch_main_share * 100.0);
    println!();

    // -- Cross-project ---------------------------------------------------------
    println!("## Cross-project propagation\n");
    println!("| Metric | Value |");
    println!("|--------|-------|");
    println!("| Runs | {} |", data.cross_runs);
    println!("| Discovered | {} |", data.cross_discovered);
    println!("| Propagated | {} |", data.cross_propagated);
    println!("| Hit ratio | {:.1}% |", r.cross_hit_ratio * 100.0);
    println!();

    // -- Note ------------------------------------------------------------------
    println!(
        "> Counters are cumulative since the cortex-api process started; \
for a true rolling 7-day window scrape `/metrics` into Prometheus \
and use the spec-36 PromQL \
(docs/observability/grafana/cortex-temporal.json)."
    );
}

fn print_json(data: &TemporalDigestData, api_url: &str) {
    let r = compute_ratios(data);

    let classified_obj: serde_json::Map<String, serde_json::Value> = data
        .classified
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::Number((*v).into())))
        .collect();

    let actions_obj: serde_json::Map<String, serde_json::Value> = data
        .actions
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::Number((*v).into())))
        .collect();

    let branch_obj: serde_json::Map<String, serde_json::Value> = data
        .branch
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::Number((*v).into())))
        .collect();

    let classified_shares_obj: serde_json::Map<String, serde_json::Value> = r
        .classified_shares
        .iter()
        .map(|(k, v)| (k.clone(), json!(*v)))
        .collect();

    let payload = json!({
        "api_url": api_url,
        "queries": data.queries,
        "queries_as_of": data.queries_as_of,
        "as_of_ratio": r.as_of_ratio,
        "classified": classified_obj,
        "classified_shares": classified_shares_obj,
        "actions": actions_obj,
        "branch": branch_obj,
        "branch_main_share": r.branch_main_share,
        "cross_runs": data.cross_runs,
        "cross_discovered": data.cross_discovered,
        "cross_propagated": data.cross_propagated,
        "cross_hit_ratio": r.cross_hit_ratio,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BODY: &str = r#"
# HELP cortex_temporal_queries_total Total temporal queries.
# TYPE cortex_temporal_queries_total counter
cortex_temporal_queries_total 1200
# HELP cortex_temporal_queries_as_of_total Queries with an as-of filter.
# TYPE cortex_temporal_queries_as_of_total counter
cortex_temporal_queries_as_of_total 300
# HELP cortex_temporal_classified_total Classifier output by state.
# TYPE cortex_temporal_classified_total counter
cortex_temporal_classified_total{state="valid"} 600
cortex_temporal_classified_total{state="temporal"} 200
cortex_temporal_classified_total{state="superseded"} 150
cortex_temporal_classified_total{state="expired"} 100
cortex_temporal_classified_total{state="not_yet_valid"} 80
cortex_temporal_classified_total{state="abandoned"} 70
# HELP cortex_temporal_action_total Classifier action by kind.
# TYPE cortex_temporal_action_total counter
cortex_temporal_action_total{action="pass"} 600
cortex_temporal_action_total{action="boost"} 200
cortex_temporal_action_total{action="demote"} 300
cortex_temporal_action_total{action="drop"} 100
# HELP cortex_branch_queries_total Branch queries by kind.
# TYPE cortex_branch_queries_total counter
cortex_branch_queries_total{kind="main"} 900
cortex_branch_queries_total{kind="non_main"} 300
# HELP cortex_cross_project_propagation_runs_total Cross-project runs.
# TYPE cortex_cross_project_propagation_runs_total counter
cortex_cross_project_propagation_runs_total 50
# HELP cortex_cross_project_discovered_total Cross-project discovered.
# TYPE cortex_cross_project_discovered_total counter
cortex_cross_project_discovered_total 400
# HELP cortex_cross_project_propagated_total Cross-project propagated.
# TYPE cortex_cross_project_propagated_total counter
cortex_cross_project_propagated_total 320
# This is an unrelated loader metric that must be ignored.
cortex_loader_events_total 99999
"#;

    #[test]
    fn parse_temporal_metrics_extracts_all_families() {
        let d = parse_temporal_metrics(SAMPLE_BODY);

        assert_eq!(d.queries, 1200);
        assert_eq!(d.queries_as_of, 300);

        // Classifier states — all six.
        assert_eq!(d.classified.get("valid").copied(), Some(600));
        assert_eq!(d.classified.get("temporal").copied(), Some(200));
        assert_eq!(d.classified.get("superseded").copied(), Some(150));
        assert_eq!(d.classified.get("expired").copied(), Some(100));
        assert_eq!(d.classified.get("not_yet_valid").copied(), Some(80));
        assert_eq!(d.classified.get("abandoned").copied(), Some(70));

        // Classifier actions.
        assert_eq!(d.actions.get("pass").copied(), Some(600));
        assert_eq!(d.actions.get("boost").copied(), Some(200));
        assert_eq!(d.actions.get("demote").copied(), Some(300));
        assert_eq!(d.actions.get("drop").copied(), Some(100));

        // Branch kinds.
        assert_eq!(d.branch.get("main").copied(), Some(900));
        assert_eq!(d.branch.get("non_main").copied(), Some(300));

        // Cross-project.
        assert_eq!(d.cross_runs, 50);
        assert_eq!(d.cross_discovered, 400);
        assert_eq!(d.cross_propagated, 320);
    }

    #[test]
    fn parse_temporal_metrics_ignores_comments_and_unknowns() {
        let body = "# HELP something ignored\n\
                    # TYPE something counter\n\
                    cortex_loader_events_total 99999\n\
                    cortex_temporal_queries_total 42\n";
        let d = parse_temporal_metrics(body);
        assert_eq!(d.queries, 42);
        // The loader metric MUST NOT corrupt any field.
        assert_eq!(d.queries_as_of, 0);
        assert!(d.classified.is_empty());
        assert!(d.actions.is_empty());
        assert!(d.branch.is_empty());
        assert_eq!(d.cross_runs, 0);
        assert_eq!(d.cross_discovered, 0);
        assert_eq!(d.cross_propagated, 0);
    }

    #[test]
    fn derived_ratios_guard_divide_by_zero() {
        // All zero inputs — no panic, all ratios are 0.0.
        let data = TemporalDigestData::default();
        let r = compute_ratios(&data);
        assert_eq!(r.as_of_ratio, 0.0);
        assert_eq!(r.cross_hit_ratio, 0.0);
        assert_eq!(r.branch_main_share, 0.0);
    }
}
