//! Phase8b — pipeline-stage freshness + divergence aggregators.
//!
//! Two new endpoints under `cortex-api`:
//!
//! - `GET /v1/health/freshness` — fans out the same probes
//!   `/v1/health` uses, parses the per-stage `last_*_ts` extras, and
//!   returns a flat map keyed by `<stage>.<kind>` (or `<stage>`)
//!   carrying `{ last_event_ts_ms, gap_seconds, severity }` rows.
//!
//! - `GET /v1/health/divergence` — pairs adjacent-stage counters
//!   from `/healthz` extras and reports per-pair `(upstream,
//!   downstream, delta, severity)` rows so a silent drop between
//!   two stages localises the moment it lands.
//!
//! Both endpoints share the same `gather_subsystem_extras` helper so
//! a single fan-out feeds both paths — the cortex-api freshness +
//! divergence views never disagree about what the underlying
//! `/healthz` extras said.
//!
//! Severity rules — both endpoints:
//! - `gap_seconds > 60`  → `warn`
//! - `gap_seconds > 300` → `critical`
//! - `delta_growth_60s > 10` → `warn`
//! - `delta_growth_60s > 50` → `critical`
//!
//! `delta_growth_60s` requires a one-sample-back state which the
//! aggregator caches in process memory; the first probe always
//! returns growth = 0 (we don't have anything to compare against
//! yet) and the row is therefore `ok` until the second probe lands.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cortex_health::client::{aggregate, build_client, AggregatorConfig, ProbeTarget};
use cortex_health::SubsystemStatus;
use serde::Serialize;
use serde_json::Value;

/// Severity bucket the GUI colour-codes on.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Within tolerance.
    Ok,
    /// Stale or growing — action recommended.
    Warn,
    /// Hard stall or persistent silent drop.
    Critical,
}

impl Severity {
    fn from_gap_seconds(gap_seconds: i64) -> Self {
        if gap_seconds > 300 {
            Severity::Critical
        } else if gap_seconds > 60 {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
    fn from_growth(growth: i64) -> Self {
        if growth > 50 {
            Severity::Critical
        } else if growth > 10 {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
}

/// One row in the `/v1/health/freshness` table.
#[derive(Debug, Clone, Serialize)]
pub struct FreshnessRow {
    /// Stage + kind/hook key (e.g. `adapter.publisher.tool_call`).
    pub key: String,
    /// Most recent activity timestamp in Unix-epoch ms (`0` when the
    /// stage has never emitted that signal).
    pub last_event_ts_ms: u64,
    /// Now - last_event_ts in whole seconds. `-1` when
    /// `last_event_ts_ms == 0` so the GUI can render `—`.
    pub gap_seconds: i64,
    /// Bucket the GUI colour-codes on.
    pub severity: Severity,
}

/// One row in the `/v1/health/divergence` table.
#[derive(Debug, Clone, Serialize)]
pub struct DivergenceRow {
    /// Pair name (e.g. `adapter.envelopes_built.tool_call ->
    /// adapter.envelopes_publish_ok.tool_call`).
    pub pair: String,
    /// Upstream counter value at probe time.
    pub upstream: u64,
    /// Downstream counter value at probe time.
    pub downstream: u64,
    /// `upstream - downstream`. Saturating: never negative; if the
    /// downstream beat the upstream (counter wrap on a restart) the
    /// delta clamps to zero.
    pub delta: u64,
    /// Change in `delta` since the last probe (~60s ago, depending on
    /// scrape cadence). `0` on the first probe — there is nothing to
    /// compare against. A *positive* growth means upstream pulled
    /// further ahead — the smoking-gun signal.
    pub delta_growth: i64,
    /// Bucket the GUI colour-codes on.
    pub severity: Severity,
}

/// Per-pair history slot the divergence aggregator keeps in memory
/// to compute `delta_growth` between probes.
#[derive(Debug, Clone, Default)]
struct DivergenceSample {
    delta: u64,
    captured_at: Option<Instant>,
}

/// In-process state shared by both endpoints — the per-pair history
/// table the divergence aggregator consults to produce `delta_growth`.
#[derive(Debug, Default)]
pub struct HealthAggregatorState {
    history: Mutex<BTreeMap<String, DivergenceSample>>,
}

impl HealthAggregatorState {
    /// Fresh aggregator state.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Handler state — the shared aggregator history plus an Arc so the
/// freshness handler can read the cortex-api-local LoaderMetrics
/// without re-probing itself across the network.
#[derive(Clone)]
pub struct HealthState {
    /// Per-pair sample history.
    pub aggregator: Arc<HealthAggregatorState>,
    /// LoaderMetrics shared with the dashboard. Read directly so a
    /// fan-out probe to the cortex-api self URL is unnecessary.
    pub loader_metrics: Arc<crate::LoaderMetrics>,
}

/// Probe every subsystem (same target list as `/v1/health`) and
/// return the `(name → SubsystemStatus)` map. Network failures and
/// 5xx still appear in the map as `Down` rows so the freshness /
/// divergence endpoints can flag them honestly instead of dropping
/// the row silently.
async fn gather_subsystem_extras() -> BTreeMap<String, SubsystemStatus> {
    let candidates: &[(&'static str, &str, &str)] = &[
        (
            "cortex-adapter",
            "CORTEX_ADAPTER_ADMIN_URL",
            "http://127.0.0.1:17011/healthz",
        ),
        (
            "cortex-ingestion",
            "CORTEX_INGESTION_URL",
            "http://127.0.0.1:17010/v1/healthz",
        ),
        (
            "cortex-classifier-worker",
            "CORTEX_CLASSIFIER_WORKER_URL",
            "http://127.0.0.1:17021/healthz",
        ),
        (
            "cortex-embedder-worker",
            "CORTEX_EMBEDDER_WORKER_URL",
            "http://127.0.0.1:17022/healthz",
        ),
        (
            "cortex-fulltext-worker",
            "CORTEX_FULLTEXT_WORKER_URL",
            "http://127.0.0.1:17023/healthz",
        ),
        (
            "cortex-graph-worker",
            "CORTEX_GRAPH_WORKER_URL",
            "http://127.0.0.1:17024/healthz",
        ),
    ];
    let mut targets: Vec<ProbeTarget> = Vec::with_capacity(candidates.len());
    for (name, key, default) in candidates {
        let url = std::env::var(key)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| (*default).to_string());
        targets.push(ProbeTarget {
            name,
            url,
        });
    }
    let client = match build_client(&AggregatorConfig::default()) {
        Ok(c) => c,
        Err(_) => return BTreeMap::new(),
    };
    let report = aggregate(&client, &targets, &AggregatorConfig::default()).await;
    let mut by_name = BTreeMap::new();
    for sub in report.subsystems {
        by_name.insert(sub.name.clone(), sub);
    }
    by_name
}

/// `GET /v1/health/freshness` handler.
pub async fn freshness_handler(State(state): State<HealthState>) -> Response {
    let by_name = gather_subsystem_extras().await;
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let mut rows: Vec<FreshnessRow> = Vec::new();

    // ---- adapter.* ---------------------------------------------------------
    if let Some(adapter) = by_name.get("cortex-adapter") {
        // Per-hook last_frame_ts.
        push_per_label_ts(
            &mut rows,
            now_ms,
            "adapter.last_frame",
            adapter.extras.get("last_frame_ts_ms"),
        );
        // Per-kind last_envelope_ts (built).
        push_per_label_ts(
            &mut rows,
            now_ms,
            "adapter.last_envelope",
            adapter.extras.get("last_envelope_ts_ms"),
        );
        // Per-kind last_publish_ok_ts.
        push_per_label_ts(
            &mut rows,
            now_ms,
            "adapter.last_publish_ok",
            adapter.extras.get("last_publish_ok_ts_ms_by_kind"),
        );
        // Aggregate (any-kind) publish-ok timestamp from phase8a.
        if let Some(ms) = adapter.extras.get("last_publish_ok_ts_ms").and_then(|v| v.as_u64()) {
            rows.push(freshness_row("adapter.last_publish_ok", ms, now_ms));
        }
    }

    // ---- ingestion.* -------------------------------------------------------
    if let Some(ingest) = by_name.get("cortex-ingestion") {
        push_per_label_ts(
            &mut rows,
            now_ms,
            "ingestion.last_archive_write",
            ingest.extras.get("last_archive_write_ts_ms"),
        );
        if let Some(ms) = ingest.extras.get("last_batch_accepted_ts_ms").and_then(|v| v.as_u64()) {
            rows.push(freshness_row("ingestion.last_batch_accepted", ms, now_ms));
        }
    }

    // ---- cortex-api loader stages (read directly from in-process LoaderMetrics) -----
    let arch_ts = state.loader_metrics.archive_last_refresh_ts_ms();
    rows.push(freshness_row("api.archive_loader.last_refresh", arch_ts, now_ms));
    let meili_ts = state.loader_metrics.meili_last_refresh_ts_ms();
    rows.push(freshness_row("api.meili_loader.last_refresh", meili_ts, now_ms));

    // ---- workers — use last_job_ts_ms exposed in extras --------------------
    for worker in [
        "cortex-classifier-worker",
        "cortex-embedder-worker",
        "cortex-fulltext-worker",
        "cortex-graph-worker",
    ] {
        if let Some(s) = by_name.get(worker) {
            if let Some(ms) = s.extras.get("last_job_ts_ms").and_then(|v| v.as_u64()) {
                let key = format!("{}.last_job", short_worker_name(worker));
                rows.push(freshness_row(&key, ms, now_ms));
            }
        }
    }

    rows.sort_by(|a, b| a.key.cmp(&b.key));
    let _ = state; // aggregator history not needed here, kept for API symmetry.
    (StatusCode::OK, Json(rows)).into_response()
}

/// `GET /v1/health/divergence` handler.
pub async fn divergence_handler(State(state): State<HealthState>) -> Response {
    let by_name = gather_subsystem_extras().await;
    let now = Instant::now();
    let mut rows: Vec<DivergenceRow> = Vec::new();

    // The divergence pairs span adjacent pipeline stages. Each pair
    // is `(pair_key, upstream_lookup, downstream_lookup)` where
    // each lookup is a small closure that pulls the relevant counter
    // out of `by_name`.
    let pairs: Vec<(String, u64, u64)> = build_divergence_pairs(&by_name);

    let mut history = state
        .aggregator
        .history
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    for (key, up, down) in pairs {
        let delta = up.saturating_sub(down);
        let prev = history.get(&key).cloned().unwrap_or_default();
        let delta_growth = match prev.captured_at {
            Some(when) if now.saturating_duration_since(when) >= Duration::from_secs(30) => {
                i64::try_from(delta).unwrap_or(0) - i64::try_from(prev.delta).unwrap_or(0)
            }
            // First probe or sub-30s second probe — no growth signal yet.
            _ => 0,
        };
        let severity = if delta_growth > 0 {
            Severity::from_growth(delta_growth)
        } else {
            Severity::Ok
        };
        rows.push(DivergenceRow {
            pair: key.clone(),
            upstream: up,
            downstream: down,
            delta,
            delta_growth,
            severity,
        });
        history.insert(
            key,
            DivergenceSample {
                delta,
                captured_at: Some(now),
            },
        );
    }

    if let Ok(mut guard) = state.aggregator.history.lock() {
        *guard = history;
    }

    rows.sort_by(|a, b| a.pair.cmp(&b.pair));
    (StatusCode::OK, Json(rows)).into_response()
}

/// Pull the canonical divergence pairs from the gathered extras.
/// Returns `(pair_key, upstream_total, downstream_total)` rows.
fn build_divergence_pairs(by_name: &BTreeMap<String, SubsystemStatus>) -> Vec<(String, u64, u64)> {
    let mut out: Vec<(String, u64, u64)> = Vec::new();

    let adapter = by_name.get("cortex-adapter");
    let ingestion = by_name.get("cortex-ingestion");

    // Pair 1 — IPC parsed vs envelopes built (per kind we don't pivot;
    // the aggregate suffices because they should match envelope
    // production rate).
    if let Some(a) = adapter {
        let parsed_total = sum_u64_map(a.extras.get("frames_parsed_total"));
        let built_total = sum_u64_map(a.extras.get("envelopes_built_total"));
        out.push((
            "adapter.frames_parsed -> adapter.envelopes_built".to_string(),
            parsed_total,
            built_total,
        ));
        // Pair 2 — built vs publish_ok (queue drops + WAL spills).
        let publish_ok_total = sum_u64_map(a.extras.get("envelopes_publish_ok_total"));
        out.push((
            "adapter.envelopes_built -> adapter.envelopes_publish_ok".to_string(),
            built_total,
            publish_ok_total,
        ));
    }
    // Pair 3 — adapter publish_ok vs ingestion archived.
    if let (Some(a), Some(ing)) = (adapter, ingestion) {
        let pub_ok_map = a.extras.get("envelopes_publish_ok_total");
        let archived_map = ing.extras.get("events_archived_total");
        // Per-kind pairs so a divergence localises which kind dropped.
        let pub_ok_by_kind = parse_u64_map(pub_ok_map);
        let archived_by_kind = parse_u64_map(archived_map);
        let mut keys: std::collections::BTreeSet<&String> = pub_ok_by_kind.keys().collect();
        keys.extend(archived_by_kind.keys());
        for k in keys {
            let up = pub_ok_by_kind.get(k).copied().unwrap_or(0);
            let down = archived_by_kind.get(k).copied().unwrap_or(0);
            out.push((
                format!(
                    "adapter.publish_ok.{k} -> ingestion.archived.{k}"
                ),
                up,
                down,
            ));
        }
    }
    out
}

fn short_worker_name(name: &str) -> &str {
    match name {
        "cortex-classifier-worker" => "classifier",
        "cortex-embedder-worker" => "embedder",
        "cortex-fulltext-worker" => "fulltext",
        "cortex-graph-worker" => "graph",
        other => other,
    }
}

fn freshness_row(key: &str, ts_ms: u64, now_ms: u64) -> FreshnessRow {
    let gap_seconds = if ts_ms == 0 {
        -1
    } else {
        ((now_ms.saturating_sub(ts_ms)) / 1000) as i64
    };
    let severity = if ts_ms == 0 {
        Severity::Warn
    } else {
        Severity::from_gap_seconds(gap_seconds)
    };
    FreshnessRow {
        key: key.to_string(),
        last_event_ts_ms: ts_ms,
        gap_seconds,
        severity,
    }
}

/// Iterate over a `Map<String, u64>` extras value and emit one row per
/// entry, using `<prefix>.<label>` as the row key.
fn push_per_label_ts(
    rows: &mut Vec<FreshnessRow>,
    now_ms: u64,
    prefix: &str,
    extras_value: Option<&Value>,
) {
    let map = match extras_value {
        Some(v) => v,
        None => return,
    };
    if let Some(obj) = map.as_object() {
        for (label, raw) in obj {
            let ms = raw.as_u64().unwrap_or(0);
            let key = format!("{prefix}.{label}");
            rows.push(freshness_row(&key, ms, now_ms));
        }
    }
}

fn parse_u64_map(extras_value: Option<&Value>) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(obj)) = extras_value {
        for (k, v) in obj {
            if let Some(n) = v.as_u64() {
                out.insert(k.clone(), n);
            }
        }
    }
    out
}

fn sum_u64_map(extras_value: Option<&Value>) -> u64 {
    parse_u64_map(extras_value).values().copied().sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_buckets_match_spec_thresholds() {
        assert_eq!(Severity::from_gap_seconds(0), Severity::Ok);
        assert_eq!(Severity::from_gap_seconds(60), Severity::Ok);
        assert_eq!(Severity::from_gap_seconds(61), Severity::Warn);
        assert_eq!(Severity::from_gap_seconds(300), Severity::Warn);
        assert_eq!(Severity::from_gap_seconds(301), Severity::Critical);

        assert_eq!(Severity::from_growth(0), Severity::Ok);
        assert_eq!(Severity::from_growth(10), Severity::Ok);
        assert_eq!(Severity::from_growth(11), Severity::Warn);
        assert_eq!(Severity::from_growth(50), Severity::Warn);
        assert_eq!(Severity::from_growth(51), Severity::Critical);
    }

    #[test]
    fn freshness_row_with_zero_ts_is_warn_with_neg_gap() {
        let row = freshness_row("x.y", 0, 1_000_000);
        assert_eq!(row.gap_seconds, -1);
        assert_eq!(row.severity, Severity::Warn);
    }

    #[test]
    fn freshness_row_recent_ts_is_ok() {
        let now_ms = 1_000_000u64;
        let row = freshness_row("x.y", now_ms - 30_000, now_ms);
        assert_eq!(row.gap_seconds, 30);
        assert_eq!(row.severity, Severity::Ok);
    }

    #[test]
    fn freshness_row_stale_ts_is_critical_past_5_minutes() {
        let now_ms = 1_000_000u64;
        // 350 seconds in the past.
        let row = freshness_row("x.y", now_ms - 350_000, now_ms);
        assert_eq!(row.gap_seconds, 350);
        assert_eq!(row.severity, Severity::Critical);
    }

    #[test]
    fn parse_u64_map_extracts_numeric_values() {
        let raw = serde_json::json!({"turn": 5, "tool_call": 3});
        let m = parse_u64_map(Some(&raw));
        assert_eq!(m.get("turn"), Some(&5));
        assert_eq!(m.get("tool_call"), Some(&3));
    }

    #[test]
    fn sum_u64_map_adds_values() {
        let raw = serde_json::json!({"a": 1, "b": 2, "c": 7});
        assert_eq!(sum_u64_map(Some(&raw)), 10);
    }

    #[test]
    fn push_per_label_ts_emits_one_row_per_entry() {
        let raw = serde_json::json!({"PostToolUse": 1_000_000u64, "UserPromptSubmit": 0});
        let mut rows = Vec::new();
        let now_ms = 1_500_000u64;
        push_per_label_ts(&mut rows, now_ms, "adapter.last_frame", Some(&raw));
        assert_eq!(rows.len(), 2);
        let by_key: BTreeMap<_, _> = rows.iter().map(|r| (r.key.as_str(), r)).collect();
        assert_eq!(
            by_key["adapter.last_frame.PostToolUse"].gap_seconds,
            500
        );
        // Zero ts → -1 gap, Warn severity.
        assert_eq!(
            by_key["adapter.last_frame.UserPromptSubmit"].gap_seconds,
            -1
        );
        assert_eq!(
            by_key["adapter.last_frame.UserPromptSubmit"].severity,
            Severity::Warn
        );
    }

    #[test]
    fn build_divergence_pairs_handles_missing_subsystems() {
        // No subsystems at all — no pairs.
        let empty = BTreeMap::new();
        assert!(build_divergence_pairs(&empty).is_empty());

        // Adapter-only → frames_parsed and built pairs but no
        // ingestion-side rows.
        let mut adapter = SubsystemStatus::ok("cortex-adapter", "0.1.0", "now");
        adapter.extras.insert(
            "frames_parsed_total".into(),
            serde_json::json!({"PostToolUse": 5, "UserPromptSubmit": 2}),
        );
        adapter.extras.insert(
            "envelopes_built_total".into(),
            serde_json::json!({"tool_call": 5, "turn": 2}),
        );
        adapter.extras.insert(
            "envelopes_publish_ok_total".into(),
            serde_json::json!({"tool_call": 3, "turn": 2}),
        );
        let mut by_name = BTreeMap::new();
        by_name.insert("cortex-adapter".to_string(), adapter);
        let rows = build_divergence_pairs(&by_name);
        // Two intra-adapter pairs.
        assert!(rows
            .iter()
            .any(|(k, _, _)| k == "adapter.frames_parsed -> adapter.envelopes_built"));
        assert!(rows
            .iter()
            .any(|(k, _, _)| k == "adapter.envelopes_built -> adapter.envelopes_publish_ok"));
        // No ingestion pairs because `cortex-ingestion` is missing.
        assert!(!rows
            .iter()
            .any(|(k, _, _)| k.starts_with("adapter.publish_ok.")));
    }

    #[test]
    fn build_divergence_pairs_localises_per_kind_drop_to_ingestion() {
        let mut adapter = SubsystemStatus::ok("cortex-adapter", "0.1.0", "now");
        adapter.extras.insert(
            "envelopes_publish_ok_total".into(),
            serde_json::json!({"tool_call": 100, "turn": 50}),
        );
        let mut ingest = SubsystemStatus::ok("cortex-ingestion", "0.1.0", "now");
        ingest.extras.insert(
            "events_archived_total".into(),
            serde_json::json!({"tool_call": 100, "turn": 0}), // turn dropped silently
        );
        let mut by_name = BTreeMap::new();
        by_name.insert("cortex-adapter".to_string(), adapter);
        by_name.insert("cortex-ingestion".to_string(), ingest);
        let rows = build_divergence_pairs(&by_name);
        let turn_row = rows
            .iter()
            .find(|(k, _, _)| k == "adapter.publish_ok.turn -> ingestion.archived.turn")
            .expect("turn pair must exist");
        assert_eq!(turn_row.1, 50, "upstream is the adapter publish_ok value");
        assert_eq!(turn_row.2, 0, "downstream is ingestion archived");
        let tool_row = rows
            .iter()
            .find(|(k, _, _)| k == "adapter.publish_ok.tool_call -> ingestion.archived.tool_call")
            .expect("tool_call pair must exist");
        assert_eq!(tool_row.1, 100);
        assert_eq!(tool_row.2, 100);
    }
}
