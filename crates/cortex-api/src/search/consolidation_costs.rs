//! Phase19 §3.4 — `POST /v1/consolidations/costs` handler.
//!
//! Aggregate consolidation counts grouped by some combination of
//! `grain` / `model` / `day` across a time window. Drives the
//! cost-telemetry dashboard.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec referenced a `grain_costs` SQL table; reality
//! (see `crates/cortex-workers/src/consolidator/cost_telemetry.rs`)
//! is an in-process `CostLedger` keyed by grain label with NO
//! SQLite persistence — counters reset on process restart and are
//! NOT addressable per-consolidation. The Meili consolidation
//! documents project `model` + `source_event_count` but neither
//! `cost_cents` nor token counters
//! (`crates/cortex-workers/src/fulltext/builders.rs::apply_extensions`
//! Kind::Consolidation arm). Adding either projection would
//! require a writer-side change + reindex.
//!
//! The handler instead aggregates **counts** from the
//! `cortex_consolidations` Meili index: how many consolidations
//! landed in `[since, until]` grouped by the requested axes. The
//! response carries `consolidation_count` + the set of
//! `distinct_models` per group; `cost_cents` /
//! `prompt_tokens` / `completion_tokens` are `null` until the
//! writer projection lands. Callers needing live-process spend
//! should consult `/v1/health/coverage` (the `CostLedger` ships
//! there per phase11j §2.8 wiring).
//!
//! Meili has no GROUP BY operator, so the handler fetches the
//! window via the filterable `occurred_at` attribute and folds
//! locally. A hard fetch cap (`FETCH_CAP`) bounds the work; when
//! the window's hit count exceeds the cap the response surfaces
//! `truncated = true` so callers know to narrow the window.

use std::collections::{BTreeMap, BTreeSet};

use axum::{extract::State, http::StatusCode, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Hard cap on the underlying Meili fetch (per-search). Bounds the
/// fold work + the response size. The handler surfaces
/// `truncated = true` when the window's hit count exceeds this.
const FETCH_CAP: u32 = 500;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Allowed group-by axes. Mirrors the original spec's vocab.
const AXES_ALLOWED: &[&str] = &["grain", "model", "day"];

/// Request body for `POST /v1/consolidations/costs`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsolidationCostsRequest {
    /// RFC3339 lower bound on `occurred_at`. Required.
    pub since: String,
    /// RFC3339 upper bound on `occurred_at`. Required.
    pub until: String,
    /// Group-by axes — any subset of `grain` / `model` / `day`.
    /// Order matters: the response keys reflect the axis order.
    pub group_by: Vec<String>,
    /// Optional repo filter — narrows the corpus before grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// One row in the aggregated response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CostBucket {
    /// Axis key map (`grain="session"`, `model="claude-haiku-4-5"`,
    /// `day="2026-05-26"`). Only the axes the caller requested
    /// appear here.
    pub key: BTreeMap<String, String>,
    /// Number of consolidations folded into this bucket.
    pub consolidation_count: u32,
    /// Distinct summariser models that contributed to this bucket
    /// (useful as a "model-fan-out" indicator when `model` is not
    /// part of the group-by).
    pub distinct_models: Vec<String>,
    /// Reserved — per-bucket cents not yet projected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_cents: Option<u64>,
    /// Reserved — per-bucket prompt tokens not yet projected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    /// Reserved — per-bucket completion tokens not yet projected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
}

/// Response body for `POST /v1/consolidations/costs`.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationCostsResponse {
    /// Echo of the requested group-by axes (order preserved).
    pub group_by: Vec<String>,
    /// Aggregated buckets sorted by `consolidation_count` desc
    /// with a deterministic key tie-break.
    pub buckets: Vec<CostBucket>,
    /// Total consolidations folded into the response. Equal to
    /// `sum(buckets[].consolidation_count)`.
    pub total: u32,
    /// `true` when the underlying Meili fetch hit [`FETCH_CAP`]
    /// and the response may be incomplete.
    pub truncated: bool,
    /// Always `"doc_only"` today. Reserved value `"joined"` lands
    /// when the writer projects cost telemetry on the doc.
    pub match_strategy: &'static str,
}

fn meili_base_url() -> String {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.meili.meili_url)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| MEILI_URL_DEFAULT.to_string())
}

fn meili_api_key() -> Option<String> {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.meili.meili_api_key)
        .filter(|s| !s.trim().is_empty())
}

fn json_err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    use axum::response::IntoResponse;
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Map an epoch-ms timestamp to a `YYYY-MM-DD` UTC day label.
pub(crate) fn day_label_from_ms(ts_ms: i64) -> String {
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
        .unwrap_or_else(chrono::Utc::now);
    dt.format("%Y-%m-%d").to_string()
}

/// Validate the group-by axes against the controlled vocab.
/// Returns the deduped + ordered list of recognised axes when ok,
/// `Err(unknown)` when any axis is outside the vocab.
pub(crate) fn validate_group_by(input: &[String]) -> Result<Vec<String>, String> {
    if input.is_empty() {
        return Err("group_by must list at least one axis".into());
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for axis in input {
        let trimmed = axis.trim();
        if !AXES_ALLOWED.contains(&trimmed) {
            return Err(format!(
                "unknown axis `{trimmed}`; allowed: {}",
                AXES_ALLOWED.join(", ")
            ));
        }
        if seen.insert(trimmed) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

/// Build the bucket key for one doc + axis set. Missing fields
/// fall back to the `"unknown"` literal so a sparsely-stamped doc
/// still folds into a deterministic bucket.
pub(crate) fn bucket_key(
    doc: &Value,
    axes: &[String],
    ts_ms_for_day: Option<i64>,
) -> BTreeMap<String, String> {
    let mut key: BTreeMap<String, String> = BTreeMap::new();
    for axis in axes {
        let value = match axis.as_str() {
            "grain" => doc
                .get("ext")
                .and_then(|e| e.get("consolidation"))
                .and_then(|c| c.get("grain"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            "model" => doc
                .get("ext")
                .and_then(|e| e.get("consolidation"))
                .and_then(|c| c.get("model"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            "day" => ts_ms_for_day
                .map(day_label_from_ms)
                .unwrap_or_else(|| "unknown".to_string()),
            _ => "unknown".to_string(),
        };
        key.insert(axis.clone(), value);
    }
    key
}

/// Fold raw Meili hits into the bucket list. Pure (same input ⇒
/// same output) so it tests independently of any HTTP layer.
pub(crate) fn fold_buckets(hits: &[Value], axes: &[String]) -> Vec<CostBucket> {
    #[derive(Default)]
    struct Acc {
        count: u32,
        models: BTreeSet<String>,
    }
    let mut groups: BTreeMap<BTreeMap<String, String>, Acc> = BTreeMap::new();
    for doc in hits {
        let ts_ms = doc
            .get("occurred_at")
            .and_then(Value::as_i64)
            .or_else(|| doc.get("ts").and_then(Value::as_i64));
        let key = bucket_key(doc, axes, ts_ms);
        let acc = groups.entry(key).or_default();
        acc.count = acc.count.saturating_add(1);
        let model = doc
            .get("ext")
            .and_then(|e| e.get("consolidation"))
            .and_then(|c| c.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !model.is_empty() {
            acc.models.insert(model.to_string());
        }
    }
    let mut out: Vec<CostBucket> = groups
        .into_iter()
        .map(|(key, acc)| CostBucket {
            key,
            consolidation_count: acc.count,
            distinct_models: acc.models.into_iter().collect(),
            cost_cents: None,
            prompt_tokens: None,
            completion_tokens: None,
        })
        .collect();
    // Sort: highest count first; tie-break by the joined key string
    // so the ordering is reproducible in tests.
    out.sort_by(|a, b| {
        let count_cmp = b.consolidation_count.cmp(&a.consolidation_count);
        if count_cmp.is_eq() {
            let ka = a.key.values().cloned().collect::<Vec<_>>().join("|");
            let kb = b.key.values().cloned().collect::<Vec<_>>().join("|");
            ka.cmp(&kb)
        } else {
            count_cmp
        }
    });
    out
}

/// Handler — `POST /v1/consolidations/costs`.
pub async fn handle_consolidation_costs(
    State(_state): State<ApiState>,
    Json(req): Json<ConsolidationCostsRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let since_ms = match parse_rfc3339_to_ms(req.since.trim()) {
        Some(v) => v,
        None => {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!("`{}` is not a valid RFC3339 timestamp", req.since),
            );
        }
    };
    let until_ms = match parse_rfc3339_to_ms(req.until.trim()) {
        Some(v) => v,
        None => {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!("`{}` is not a valid RFC3339 timestamp", req.until),
            );
        }
    };
    if since_ms > until_ms {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "`since` must be on or before `until`",
        );
    }
    let axes = match validate_group_by(&req.group_by) {
        Ok(a) => a,
        Err(detail) => {
            return json_err(StatusCode::BAD_REQUEST, "bad_input", detail);
        }
    };

    let mut filter = format!("ts >= {since_ms} AND ts <= {until_ms}");
    if let Some(repo) = req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        filter.push_str(&format!(" AND repo = \"{}\"", meili_escape(repo)));
    }
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        super::resolve_family_index(
            req.repo.as_deref(),
            "consolidations",
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
    );
    let body = json!({
        "q": "",
        "limit": FETCH_CAP,
        "filter": filter,
        "sort": ["ts:asc"],
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("build http client: {e}"),
            );
        }
    };
    let mut request = client.post(&url).json(&body);
    if let Some(key) = meili_api_key() {
        request = request.bearer_auth(key);
    }
    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "api_unreachable",
                format!("meili unreachable: {e}"),
            );
        }
    };
    let status = resp.status();
    let parsed = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "api_http_error",
                format!("meili body parse: {e}"),
            );
        }
    };
    if !status.is_success() {
        let detail = parsed
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("meili rejected the search")
            .to_string();
        return json_err(StatusCode::BAD_GATEWAY, "api_http_error", detail);
    }

    let hits = parsed
        .get("hits")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let estimated_total_hits = parsed
        .get("estimatedTotalHits")
        .and_then(Value::as_u64)
        .unwrap_or(hits.len() as u64);
    let truncated = estimated_total_hits as u32 > FETCH_CAP;
    let buckets = fold_buckets(&hits, &axes);
    let total: u32 = buckets.iter().map(|b| b.consolidation_count).sum();

    Json(ConsolidationCostsResponse {
        group_by: axes,
        buckets,
        total,
        truncated,
        match_strategy: "doc_only",
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(grain: &str, model: &str, occurred_at_ms: i64) -> Value {
        json!({
            "occurred_at": occurred_at_ms,
            "ext": {"consolidation": {"grain": grain, "model": model}}
        })
    }

    #[test]
    fn day_label_from_ms_renders_iso_date() {
        // 2026-05-26T00:00:00Z = 1779840000000
        let ms = chrono::DateTime::parse_from_rfc3339("2026-05-26T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(day_label_from_ms(ms), "2026-05-26");
    }

    #[test]
    fn validate_group_by_accepts_known_axes_and_dedups() {
        let ok = validate_group_by(&["grain".into(), "model".into(), "day".into(), "grain".into()])
            .unwrap();
        assert_eq!(ok, vec!["grain", "model", "day"]);
    }

    #[test]
    fn validate_group_by_rejects_empty_or_unknown_axis() {
        assert!(validate_group_by(&[]).is_err());
        assert!(validate_group_by(&["nonsense".into()]).is_err());
        assert!(validate_group_by(&["grain".into(), "topic".into()]).is_err());
    }

    #[test]
    fn bucket_key_unknown_falls_back_for_missing_fields() {
        let empty = json!({});
        let k = bucket_key(
            &empty,
            &["grain".into(), "model".into(), "day".into()],
            None,
        );
        assert_eq!(k.get("grain"), Some(&"unknown".to_string()));
        assert_eq!(k.get("model"), Some(&"unknown".to_string()));
        assert_eq!(k.get("day"), Some(&"unknown".to_string()));
    }

    #[test]
    fn fold_buckets_aggregates_per_grain_with_count_desc() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-05-26T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let hits = vec![
            doc("session", "claude-haiku-4-5", t),
            doc("session", "claude-haiku-4-5", t),
            doc("topic", "claude-opus-4-7", t),
        ];
        let out = fold_buckets(&hits, &["grain".into()]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key.get("grain"), Some(&"session".to_string()));
        assert_eq!(out[0].consolidation_count, 2);
        assert_eq!(out[1].key.get("grain"), Some(&"topic".to_string()));
        assert_eq!(out[1].consolidation_count, 1);
    }

    #[test]
    fn fold_buckets_collects_distinct_models_per_bucket() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-05-26T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let hits = vec![
            doc("session", "claude-haiku-4-5", t),
            doc("session", "claude-opus-4-7", t),
        ];
        let out = fold_buckets(&hits, &["grain".into()]);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].distinct_models,
            vec![
                "claude-haiku-4-5".to_string(),
                "claude-opus-4-7".to_string()
            ]
        );
    }

    #[test]
    fn fold_buckets_groups_by_day_label() {
        let t1 = chrono::DateTime::parse_from_rfc3339("2026-05-25T23:59:00Z")
            .unwrap()
            .timestamp_millis();
        let t2 = chrono::DateTime::parse_from_rfc3339("2026-05-26T00:01:00Z")
            .unwrap()
            .timestamp_millis();
        let hits = vec![
            doc("session", "m", t1),
            doc("session", "m", t2),
            doc("session", "m", t2),
        ];
        let out = fold_buckets(&hits, &["day".into()]);
        // Sort: 2026-05-26 (count=2) first, 2026-05-25 (count=1) second.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].consolidation_count, 2);
        assert_eq!(out[0].key.get("day"), Some(&"2026-05-26".to_string()));
        assert_eq!(out[1].key.get("day"), Some(&"2026-05-25".to_string()));
    }
}
