//! Phase19 §2.5 — `GET /v1/consolidations/{id}/lineage` handler.
//!
//! Returns a structured citation view for one consolidation:
//! `{source_session_ids, decisions, files, cost}` derived from the
//! envelope's own searchable + projected fields. The handler does
//! ONE Meili lookup (via the same OR filter
//! `event_id = "X" OR ext.consolidation.consolidation_id = "X"` the
//! `consolidation_get` route uses) and then projects the lineage
//! locally — no per-source-event fan-out, no archive scan.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec ("read joins `consolidations` + `decisions` + per-
//! session tool-call file extracts") assumed the consolidator writer
//! projects `source_event_ids`, `source_session_ids`,
//! `cost_cents`, `prompt_tokens`, `completion_tokens` onto the Meili
//! consolidation document. Today only `model` and
//! `source_event_count` are projected (see
//! `crates/cortex-workers/src/fulltext/builders.rs::apply_extensions`
//! for `Kind::Consolidation`). The aggregate `grain_costs` ledger is
//! an in-process [`CostLedger`] keyed by grain label, NOT per
//! consolidation id, so it cannot answer "how much did THIS
//! consolidation cost".
//!
//! The handler instead derives the lineage from the doc itself:
//!
//! - **source_session_ids**: parsed from `topics` (`session:<ulid>`
//!   convention) — the classifier stamps these whenever the
//!   consolidator emits a Session-grain rollup.
//! - **decisions**: union of (a) `topics` matching `decision:DEC-…`
//!   and (b) `DEC-\d{3,}` mentions inside `body` / `summary` /
//!   `title`. Identifiers deduped + sorted.
//! - **files**: parsed from `topics` (`file:<path>` convention) —
//!   the consolidator stamps these for Session-grain rollups that
//!   touched files. Cross-session file aggregation requires the
//!   archive scan path that `cortex_files_touched` already exposes;
//!   callers needing the per-touch count should pivot through that
//!   tool with the lineage's `source_session_ids`.
//! - **cost**: `{model, cents, prompt_tokens, completion_tokens}`
//!   with `model` read from `ext.consolidation.model` and the rest
//!   `null` — the per-consolidation cost telemetry is not yet
//!   projected.
//!
//! The shape is forward-compatible: a follow-up that adds the
//! writer-side projection only needs to stamp the new fields; this
//! handler will pick them up without a schema change. Today's
//! response surfaces `match_strategy = "doc_only"` so callers can
//! tell which lineage shape they got.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// One decision the consolidation references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRef {
    /// ADR identifier (`DEC-0042`, `DEC-001`, …).
    pub decision_id: String,
}

/// One file the consolidation references (Session-grain only —
/// cross-session lineage punts to `cortex_files_touched`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    /// Repo-rooted path (matches the convention `files_touched`
    /// emits).
    pub path: String,
}

/// Per-consolidation cost block. `cents` / `prompt_tokens` /
/// `completion_tokens` are `null` today — see module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostBlock {
    /// Summariser model id (`claude-haiku-4-5`, `claude-opus-4-7`).
    pub model: String,
    /// Reserved — per-consolidation cents not yet projected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cents: Option<u32>,
    /// Reserved — per-consolidation prompt tokens not yet projected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    /// Reserved — per-consolidation completion tokens not yet projected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
}

/// Response body for `GET /v1/consolidations/{id}/lineage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationLineageResponse {
    /// Consolidation id the lineage describes.
    pub consolidation_id: String,
    /// Source session ids the consolidation drew from. Empty for
    /// Topic / DecisionTrace grains until the writer projects them.
    pub source_session_ids: Vec<String>,
    /// ADRs the consolidation references.
    pub decisions: Vec<DecisionRef>,
    /// Files the consolidation touched (Session-grain only — see
    /// module docs).
    pub files: Vec<FileRef>,
    /// Cost block (model only today).
    pub cost: CostBlock,
    /// Always `"doc_only"` today. Reserved value `"joined"` will
    /// land when the writer projects source_event_ids /
    /// cost telemetry on the doc.
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

/// Same id-shape validator the `consolidation_get` route uses.
pub(crate) fn id_is_valid(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Extract source session ids from the doc's `topics` array
/// (`session:<ulid>` convention). Returns deduped + sorted.
pub(crate) fn extract_sessions(doc: &Value) -> Vec<String> {
    let mut out: Vec<String> = topic_strings(doc)
        .filter_map(|t| t.strip_prefix("session:").map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Extract files from the doc's `topics` array (`file:<path>`
/// convention). Returns deduped + sorted.
pub(crate) fn extract_files(doc: &Value) -> Vec<FileRef> {
    let mut out: Vec<String> = topic_strings(doc)
        .filter_map(|t| t.strip_prefix("file:").map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out.into_iter().map(|path| FileRef { path }).collect()
}

/// Extract decision ids from (a) `topics` (`decision:DEC-NNNN`) and
/// (b) `DEC-\d{3,}` mentions inside `title` / `summary` / `body`.
/// Returns deduped + sorted.
pub(crate) fn extract_decisions(doc: &Value) -> Vec<DecisionRef> {
    let mut ids: std::collections::BTreeSet<String> = topic_strings(doc)
        .filter_map(|t| t.strip_prefix("decision:").map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect();
    for field in ["title", "summary", "body", "summary_markdown"] {
        if let Some(s) = doc.get(field).and_then(Value::as_str) {
            for found in find_dec_ids(s) {
                ids.insert(found);
            }
        }
    }
    ids.into_iter()
        .map(|decision_id| DecisionRef { decision_id })
        .collect()
}

/// Find every `DEC-NNN+` (3+ digits) inside `text`. Conservative —
/// the consolidator's controlled vocab uses `DEC-` as the canonical
/// prefix, so the gate keeps the false-positive rate near zero.
fn find_dec_ids(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] == b"DEC-" {
            let mut j = i + 4;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - (i + 4) >= 3 {
                // SAFETY: ASCII-only window, slice is valid utf-8.
                let id = std::str::from_utf8(&bytes[i..j]).unwrap_or("");
                if !id.is_empty() {
                    out.push(id.to_string());
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn topic_strings(doc: &Value) -> impl Iterator<Item = &str> {
    doc.get("topics")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
}

/// Build the cost block from the doc — `model` only today.
pub(crate) fn extract_cost(doc: &Value) -> CostBlock {
    let model = doc
        .get("ext")
        .and_then(|e| e.get("consolidation"))
        .and_then(|c| c.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    CostBlock {
        model,
        cents: None,
        prompt_tokens: None,
        completion_tokens: None,
    }
}

/// Project the resolved consolidation id (preferring
/// `ext.consolidation.consolidation_id`, falling back to `event_id`).
pub(crate) fn resolved_consolidation_id(doc: &Value, fallback: &str) -> String {
    doc.get("ext")
        .and_then(|e| e.get("consolidation"))
        .and_then(|c| c.get("consolidation_id"))
        .and_then(Value::as_str)
        .or_else(|| doc.get("event_id").and_then(Value::as_str))
        .unwrap_or(fallback)
        .to_string()
}

/// Handler — `GET /v1/consolidations/{id}/lineage`.
pub async fn handle_consolidation_lineage(
    State(_state): State<ApiState>,
    Path(id): Path<String>,
) -> Response {
    use axum::response::IntoResponse;

    let id = id.trim().to_string();
    if !id_is_valid(&id) {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "id must be alphanumeric (ULID or slug shape, ≤128 chars)",
        );
    }

    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        cortex_storage::names::INDEX_CONSOLIDATIONS
    );
    let filter = format!(
        "event_id = \"{}\" OR ext.consolidation.consolidation_id = \"{}\"",
        meili_escape(&id),
        meili_escape(&id),
    );
    let body = json!({
        "q": "",
        "limit": 1,
        "filter": filter,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
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
            .unwrap_or("meili rejected the lookup")
            .to_string();
        return json_err(StatusCode::BAD_GATEWAY, "api_http_error", detail);
    }
    let doc = parsed
        .get("hits")
        .and_then(Value::as_array)
        .and_then(|v| v.first().cloned());
    let Some(doc) = doc else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("no consolidation matches id `{id}`"),
        );
    };

    Json(ConsolidationLineageResponse {
        consolidation_id: resolved_consolidation_id(&doc, &id),
        source_session_ids: extract_sessions(&doc),
        decisions: extract_decisions(&doc),
        files: extract_files(&doc),
        cost: extract_cost(&doc),
        match_strategy: "doc_only",
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn id_is_valid_matches_consolidation_get_rules() {
        assert!(id_is_valid("01HZCONS01"));
        assert!(id_is_valid("cons-ses-aaaa"));
        assert!(!id_is_valid(""));
        assert!(!id_is_valid("bad/id"));
        assert!(!id_is_valid(&"x".repeat(129)));
    }

    #[test]
    fn extract_sessions_pulls_session_topics_deduped_sorted() {
        let doc = json!({
            "topics": ["session:01HSESS02", "decision:DEC-0042", "session:01HSESS01", "session:01HSESS01"]
        });
        let out = extract_sessions(&doc);
        assert_eq!(out, vec!["01HSESS01", "01HSESS02"]);
    }

    #[test]
    fn extract_sessions_returns_empty_when_topics_missing() {
        assert!(extract_sessions(&json!({})).is_empty());
        assert!(extract_sessions(&json!({"topics": []})).is_empty());
    }

    #[test]
    fn extract_files_pulls_file_topics_deduped_sorted() {
        let doc = json!({
            "topics": ["file:src/b.rs", "file:src/a.rs", "decision:DEC-0042", "file:src/a.rs"]
        });
        let out = extract_files(&doc);
        assert_eq!(
            out,
            vec![
                FileRef {
                    path: "src/a.rs".into()
                },
                FileRef {
                    path: "src/b.rs".into()
                }
            ]
        );
    }

    #[test]
    fn extract_decisions_unions_topics_and_body_mentions() {
        let doc = json!({
            "topics": ["decision:DEC-0001", "file:src/lib.rs"],
            "title": "Implements DEC-0042",
            "summary": "Resolves DEC-0099 + DEC-0042 (dup)",
            "body": "Closes DEC-0001 (already in topics)"
        });
        let out = extract_decisions(&doc);
        let ids: Vec<&str> = out.iter().map(|d| d.decision_id.as_str()).collect();
        assert_eq!(ids, vec!["DEC-0001", "DEC-0042", "DEC-0099"]);
    }

    #[test]
    fn extract_decisions_ignores_short_or_non_dash_matches() {
        let doc = json!({
            "title": "DEC-12 short, DECISION-001 wrong prefix, dec-0042 wrong case"
        });
        assert!(extract_decisions(&doc).is_empty());
    }

    #[test]
    fn extract_cost_reads_model_off_ext_consolidation() {
        let doc = json!({
            "ext": {"consolidation": {"model": "claude-opus-4-7"}}
        });
        let cost = extract_cost(&doc);
        assert_eq!(cost.model, "claude-opus-4-7");
        assert!(cost.cents.is_none());
        assert!(cost.prompt_tokens.is_none());
        assert!(cost.completion_tokens.is_none());
    }

    #[test]
    fn extract_cost_returns_empty_model_when_ext_missing() {
        let cost = extract_cost(&json!({}));
        assert_eq!(cost.model, "");
    }

    #[test]
    fn resolved_consolidation_id_prefers_ext_then_event_id_then_fallback() {
        let with_ext = json!({
            "event_id": "01HZE01",
            "ext": {"consolidation": {"consolidation_id": "cons-ses-aaaa"}}
        });
        assert_eq!(resolved_consolidation_id(&with_ext, "fb"), "cons-ses-aaaa");
        let only_event = json!({"event_id": "01HZE01"});
        assert_eq!(resolved_consolidation_id(&only_event, "fb"), "01HZE01");
        assert_eq!(resolved_consolidation_id(&json!({}), "fb"), "fb");
    }
}
