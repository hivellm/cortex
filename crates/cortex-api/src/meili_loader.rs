//! Meilisearch → in-memory keyword lane bootstrap (decisions / laws /
//! memory / analyses).
//!
//! `cortex-fulltext-worker` indexes every enriched envelope into per-
//! project Meili indexes (spec-08 §Routing matrix): `cortex-{slug}-
//! decisions`, `cortex-{slug}-governance`, `cortex-{slug}-misc`,
//! `cortex-{slug}-turns`, `cortex-{slug}-code`, `cortex-{slug}-docs`.
//!
//! `archive_loader.rs` only sees `cortex-ingestion`'s raw archive,
//! which today only carries `Turn` / `ToolCall` / `AgentCall`
//! envelopes (the live capture surface). Decision / LawViolation /
//! Memory / Analysis envelopes flow exclusively through the bootstrap
//! pipeline and live in Meili — never in the raw archive — so
//! `dashboard.rs::decisions / violations / analyses / memory` always
//! returned `[]`.
//!
//! This loader closes that gap: at boot (and every refresh) it walks
//! every `cortex-*-{decisions,governance,misc,turns}` Meili index,
//! parses the envelope-body shape `cortex-fulltext-worker` writes,
//! and seeds a [`MemoryKeywordLane`] with the extracted fields. The
//! existing `dashboard.rs` handlers consume the lane unchanged.
//!
//! This is the surgical alternative to phase B of
//! `phase2_cortex_api_dashboard_real_backends` (the full
//! `MeiliKeywordLane` impl). It re-uses the keyword lane the
//! orchestrator already runs against and keeps the dashboard surface
//! pluggable: when phase B lands, this loader drops out and the
//! `KeywordLane` trait talks to Meili directly.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use tracing::{debug, warn};

use crate::lanes::{LaneHit, MemoryKeywordLane};
use crate::types::Props;

/// Lane index name we seed into for each family. Kept distinct from
/// the spec-08 Meili uids so the orchestrator's keyword strategies
/// can target either tier deliberately.
const LANE_INDEX_DECISIONS: &str = "cortex-meili-decisions";
const LANE_INDEX_GOVERNANCE: &str = "cortex-meili-governance";
const LANE_INDEX_MISC: &str = "cortex-meili-misc";
const LANE_INDEX_TURNS: &str = "cortex-meili-turns";

/// Per-Meili-index hard cap. Pull more than this and we either time
/// out the boot seeding or blow up the in-memory lane. Operators on
/// large stacks should swap to the phase-B `MeiliKeywordLane` long
/// before this becomes a real ceiling.
const MEILI_PAGE_LIMIT: usize = 1000;

/// Loader-side errors. The caller treats every variant as "skip and
/// keep going" — Meili being temporarily down should never break a
/// dashboard restart.
#[derive(Debug, Error)]
pub enum MeiliLoadError {
    /// Network / transport failure or 5xx from Meili.
    #[error("transport: {0}")]
    Transport(String),
    /// Body decode error.
    #[error("decode: {0}")]
    Decode(String),
}

/// Per-family counters exposed to the boot log.
#[derive(Debug, Clone, Default)]
pub struct MeiliLoadReport {
    /// Number of `cortex-*-{decisions,governance,misc,turns}` indexes visited.
    pub indexes_visited: usize,
    /// Decision envelopes seeded into the lane.
    pub decisions_seeded: usize,
    /// Law-violation envelopes seeded into the lane.
    pub violations_seeded: usize,
    /// Memory envelopes seeded into the lane.
    pub memories_seeded: usize,
    /// Analysis envelopes seeded into the lane.
    pub analyses_seeded: usize,
    /// Turn envelopes seeded into the lane (used by the timeline).
    pub turns_seeded: usize,
    /// Documents the loader could not project into a `LaneHit`.
    pub hits_skipped: usize,
}

impl MeiliLoadReport {
    /// Total hits seeded across every family — used by the boot log.
    pub fn total_seeded(&self) -> usize {
        self.decisions_seeded
            + self.violations_seeded
            + self.memories_seeded
            + self.analyses_seeded
            + self.turns_seeded
    }
}

/// Walk every relevant Meili index and seed the lane.
///
/// Returns the report so the caller can log it. Failures during the
/// walk degrade gracefully — the report carries whatever was seeded
/// and the caller logs a warning.
pub async fn load_meili_into_keyword_lane(
    meili_url: &str,
    meili_api_key: Option<&str>,
    lane: &MemoryKeywordLane,
) -> Result<MeiliLoadReport, MeiliLoadError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| MeiliLoadError::Transport(format!("reqwest builder: {e}")))?;

    let indexes = list_indexes(&client, meili_url, meili_api_key).await?;

    let mut report = MeiliLoadReport::default();
    let mut decision_hits: Vec<LaneHit> = Vec::new();
    let mut violation_hits: Vec<LaneHit> = Vec::new();
    let mut memory_hits: Vec<LaneHit> = Vec::new();
    let mut analysis_hits: Vec<LaneHit> = Vec::new();
    let mut turn_hits: Vec<LaneHit> = Vec::new();

    for uid in indexes {
        let family = match family_of(&uid) {
            Some(f) => f,
            None => continue,
        };
        report.indexes_visited += 1;

        let docs = match search_index(&client, meili_url, meili_api_key, &uid).await {
            Ok(d) => d,
            Err(e) => {
                warn!(index = %uid, error = %e, "meili_loader: skip index");
                continue;
            }
        };

        for doc in docs {
            match doc_to_hit(&doc, family) {
                Some((symbol, hit)) => match symbol {
                    "decision" => {
                        report.decisions_seeded += 1;
                        decision_hits.push(hit);
                    }
                    "law_violation" => {
                        report.violations_seeded += 1;
                        violation_hits.push(hit);
                    }
                    "memory" => {
                        report.memories_seeded += 1;
                        memory_hits.push(hit);
                    }
                    "analysis" => {
                        report.analyses_seeded += 1;
                        analysis_hits.push(hit);
                    }
                    "turn" => {
                        report.turns_seeded += 1;
                        turn_hits.push(hit);
                    }
                    _ => report.hits_skipped += 1,
                },
                None => report.hits_skipped += 1,
            }
        }
    }

    // `seed` replaces the index's hit list, so calling it once per
    // family with the aggregated vec is the right shape — the lane
    // sees a clean snapshot every refresh.
    if !decision_hits.is_empty() {
        lane.seed(LANE_INDEX_DECISIONS, decision_hits);
    }
    if !violation_hits.is_empty() {
        lane.seed(LANE_INDEX_GOVERNANCE, violation_hits);
    }
    if !memory_hits.is_empty() || !analysis_hits.is_empty() {
        let mut misc = memory_hits;
        misc.extend(analysis_hits);
        lane.seed(LANE_INDEX_MISC, misc);
    }
    if !turn_hits.is_empty() {
        lane.seed(LANE_INDEX_TURNS, turn_hits);
    }

    Ok(report)
}

/// Family suffix dispatch. Returns the lane-side family label so the
/// caller can route the doc to the correct hit bucket.
fn family_of(uid: &str) -> Option<&'static str> {
    if !uid.starts_with("cortex-") {
        return None;
    }
    if uid.ends_with("-decisions") {
        Some("decisions")
    } else if uid.ends_with("-governance") {
        Some("governance")
    } else if uid.ends_with("-misc") {
        Some("misc")
    } else if uid.ends_with("-turns") {
        Some("turns")
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct ListIndexesResponse {
    results: Vec<IndexEntry>,
}

#[derive(Debug, Deserialize)]
struct IndexEntry {
    uid: String,
}

async fn list_indexes(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, MeiliLoadError> {
    let url = format!("{}/indexes?limit=200", base_url.trim_end_matches('/'));
    let mut req = client.get(url);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| MeiliLoadError::Transport(format!("list_indexes: {e}")))?;
    if !resp.status().is_success() {
        return Err(MeiliLoadError::Transport(format!(
            "list_indexes: status {}",
            resp.status()
        )));
    }
    let body: ListIndexesResponse = resp
        .json()
        .await
        .map_err(|e| MeiliLoadError::Decode(format!("list_indexes: {e}")))?;
    Ok(body.results.into_iter().map(|e| e.uid).collect())
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<Value>,
}

async fn search_index(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    uid: &str,
) -> Result<Vec<Value>, MeiliLoadError> {
    let url = format!(
        "{}/indexes/{}/search",
        base_url.trim_end_matches('/'),
        uid
    );
    let body = serde_json::json!({ "q": "", "limit": MEILI_PAGE_LIMIT });
    let mut req = client.post(url).json(&body);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| MeiliLoadError::Transport(format!("search {uid}: {e}")))?;
    if !resp.status().is_success() {
        return Err(MeiliLoadError::Transport(format!(
            "search {uid}: status {}",
            resp.status()
        )));
    }
    let parsed: SearchResponse = resp
        .json()
        .await
        .map_err(|e| MeiliLoadError::Decode(format!("search {uid}: {e}")))?;
    Ok(parsed.hits)
}

/// Project one Meili document into a `LaneHit` plus the lane-side
/// symbol the dashboard handlers filter on.
///
/// Spec-08 doc bodies for non-Turn kinds are JSON-stringified envelope
/// payloads (the fulltext worker stores them whole so the dashboard
/// can re-parse them later). We extract the meaningful fields here so
/// the lane carries cleaned text instead of forcing every handler to
/// parse JSON twice.
fn doc_to_hit(doc: &Value, family: &str) -> Option<(&'static str, LaneHit)> {
    let kind = doc.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let event_id = doc.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
    if event_id.is_empty() {
        return None;
    }

    // Build the doc_id so it strips back to `event_id` via the same
    // `archive|`-style prefix the existing handlers expect — keeps
    // the dashboard's `doc_id.strip_prefix("archive|")` unchanged.
    let doc_id = format!("archive|{event_id}");

    let repo = doc
        .get("repo")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let path = doc
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let severity = doc
        .get("severity")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content_hash = doc
        .get("content_hash")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Classifier-stamped fields. These are how the Sonnet/Haiku
    // classifier output reaches the dashboard — `topics` is the
    // controlled-vocab tag list, `pii_risk` is the redaction
    // signal, `severity` (above) tracks the law-violation
    // disposition for governance kinds. The Classifications view
    // surfaces all three so the user can see what the classifier
    // is producing across the corpus.
    let topics: Vec<String> = doc
        .get("topics")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let pii_risk = doc
        .get("pii_risk")
        .and_then(|v| v.as_str())
        .map(String::from);
    let summary_text = doc
        .get("summary")
        .and_then(|v| v.as_str())
        .map(String::from);

    // The body field for non-Turn kinds is a JSON-encoded string of
    // the envelope payload. Decode once; if the inner shape parses,
    // hoist the meaningful fields into extras so the dashboard
    // handlers don't have to redo it. Failure to parse falls back to
    // treating `body` as plain text (covers the rare envelope where
    // the worker shipped the raw payload directly).
    let body_raw = doc.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let parsed_body: Option<Value> = serde_json::from_str(body_raw).ok();

    let mut extras: Props = BTreeMap::new();
    // Tag the source up-front so every hit this loader feeds into
    // `MemoryKeywordLane` carries the same `extras["source"] =
    // "keyword"` label `MeiliKeywordLane` stamps. The orchestrator's
    // `lane_label()` reads this field; without it, dev fallback hits
    // surface as `source = "vector"` and the audit drift returns.
    extras.insert("source".to_string(), Value::String("keyword".to_string()));

    // Classifier output — every classified Meili doc carries these
    // fields. Stamping them on extras lets the dashboard's
    // Classifications view aggregate topics + severity + pii_risk
    // across the corpus without re-fetching from Meili.
    if !topics.is_empty() {
        let arr: Vec<Value> = topics
            .iter()
            .map(|t| Value::String(t.clone()))
            .collect();
        extras.insert("topics".to_string(), Value::Array(arr));
    }
    if let Some(pii) = pii_risk.as_deref() {
        extras.insert("pii_risk".to_string(), Value::String(pii.to_string()));
    }
    if let Some(s) = summary_text.as_deref() {
        extras.insert("summary".to_string(), Value::String(s.to_string()));
    }

    let symbol: &'static str;
    let text: String;

    match (family, kind) {
        ("decisions", "decision") => {
            symbol = "decision";
            let title = parsed_body
                .as_ref()
                .and_then(|b| b.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(event_id)
                .to_string();
            let body_md = parsed_body
                .as_ref()
                .and_then(|b| b.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or(body_raw)
                .to_string();
            let status = parsed_body
                .as_ref()
                .and_then(|b| b.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("active")
                .to_string();
            let supersedes = parsed_body
                .as_ref()
                .and_then(|b| b.get("supersedes"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            extras.insert("title".to_string(), Value::String(title));
            extras.insert("status".to_string(), Value::String(status));
            if let Some(s) = supersedes {
                extras.insert("supersedes".to_string(), Value::String(s));
            }
            extras.insert("body_markdown".to_string(), Value::String(body_md.clone()));
            text = body_md;
        }
        ("governance", "law_violation") => {
            symbol = "law_violation";
            let law_id = parsed_body
                .as_ref()
                .and_then(|b| b.get("law_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let title = parsed_body
                .as_ref()
                .and_then(|b| b.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(event_id)
                .to_string();
            let body_md = parsed_body
                .as_ref()
                .and_then(|b| b.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or(body_raw)
                .to_string();
            extras.insert("title".to_string(), Value::String(title));
            if let Some(lid) = law_id {
                extras.insert("law_id".to_string(), Value::String(lid));
            }
            extras.insert("body_markdown".to_string(), Value::String(body_md.clone()));
            text = body_md;
        }
        ("misc", "memory") => {
            symbol = "memory";
            let title = parsed_body
                .as_ref()
                .and_then(|b| b.get("name"))
                .or_else(|| parsed_body.as_ref().and_then(|b| b.get("title")))
                .and_then(|v| v.as_str())
                .unwrap_or(event_id)
                .to_string();
            let body_md = parsed_body
                .as_ref()
                .and_then(|b| b.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or(body_raw)
                .to_string();
            extras.insert("title".to_string(), Value::String(title));
            extras.insert("body_markdown".to_string(), Value::String(body_md.clone()));
            text = body_md;
        }
        ("misc", "analysis") => {
            symbol = "analysis";
            let title = parsed_body
                .as_ref()
                .and_then(|b| b.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(event_id)
                .to_string();
            let body_md = parsed_body
                .as_ref()
                .and_then(|b| b.get("verdict"))
                .or_else(|| parsed_body.as_ref().and_then(|b| b.get("body")))
                .and_then(|v| v.as_str())
                .unwrap_or(body_raw)
                .to_string();
            extras.insert("title".to_string(), Value::String(title));
            extras.insert("body_markdown".to_string(), Value::String(body_md.clone()));
            text = body_md;
        }
        ("turns", "turn") => {
            symbol = "turn";
            let user = parsed_body
                .as_ref()
                .and_then(|b| b.get("user_message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let asst = parsed_body
                .as_ref()
                .and_then(|b| b.get("assistant_message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Stamp the two halves on extras separately so the
            // dashboard's conversation detail can pair them by
            // turn_id without re-parsing the concatenated `text`.
            // The Stop hook publishes a separate envelope with
            // user_message="" — we keep the empty marker on extras
            // so the pairing logic still distinguishes the two
            // sides.
            extras.insert("user_message".to_string(), Value::String(user.to_string()));
            extras.insert(
                "assistant_message".to_string(),
                Value::String(asst.to_string()),
            );
            text = if asst.is_empty() {
                user.to_string()
            } else if user.is_empty() {
                asst.to_string()
            } else {
                format!("{user}\n\n{asst}")
            };
        }
        _ => return None,
    }

    let session_id = doc
        .get("ext")
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .or_else(|| doc.get("session_id").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    if let Some(s) = session_id {
        extras.insert("session_id".to_string(), Value::String(s));
    }

    // Surface the adapter `turn_id` from the doc's `ext` block so
    // the dashboard's conversation pairing can fold UserPromptSubmit
    // and Stop envelopes that share the same turn into one row.
    let turn_id = doc
        .get("ext")
        .and_then(|v| v.get("turn_id"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            doc.get("ext")
                .and_then(|v| v.get("claude_code"))
                .and_then(|v| v.get("turn_id"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.to_string());
    if let Some(tid) = turn_id {
        let mut cc_obj = serde_json::Map::new();
        cc_obj.insert("turn_id".to_string(), Value::String(tid));
        extras.insert("claude_code".to_string(), Value::Object(cc_obj));
    }

    let ts = doc.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);

    debug!(
        family = family,
        symbol = symbol,
        event_id = event_id,
        "meili_loader: projected hit"
    );

    Some((
        symbol,
        LaneHit {
            doc_id,
            text,
            repo,
            path,
            symbol: Some(symbol.to_string()),
            content_hash,
            score: 1.0,
            ts,
            severity,
            extras,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_of_recognises_per_project_uids() {
        assert_eq!(family_of("cortex-cortex-decisions"), Some("decisions"));
        assert_eq!(family_of("cortex-vectorizer-governance"), Some("governance"));
        assert_eq!(family_of("cortex-rulebook-misc"), Some("misc"));
        assert_eq!(family_of("cortex-cortex-turns"), Some("turns"));
        assert_eq!(family_of("cortex-cortex-code"), None);
        assert_eq!(family_of("cortex-cortex-docs"), None);
        assert_eq!(family_of("cortex-decisions"), Some("decisions")); // legacy
        assert_eq!(family_of("not-a-cortex-index"), None);
    }

    #[test]
    fn doc_to_hit_decision_extracts_title_status_and_body() {
        let inner = serde_json::json!({
            "title": "1. Bypass vectorizer-sdk",
            "status": "superseded",
            "supersedes": null,
            "body": "# 1. Bypass vectorizer-sdk\n\n**Status**: superseded\n\nDecision body."
        });
        let body_str = serde_json::to_string(&inner).unwrap();
        let doc = serde_json::json!({
            "id": "01KQ82V7CE840RNQKRT2CSB5TT",
            "event_id": "01KQ82V7CE840RNQKRT2CSB5TT",
            "kind": "decision",
            "repo": "Cortex",
            "path": ".rulebook/decisions/001-foo.md",
            "severity": "notable",
            "ts": 0,
            "body": body_str,
        });
        let (symbol, hit) = doc_to_hit(&doc, "decisions").expect("decision projected");
        assert_eq!(symbol, "decision");
        assert_eq!(hit.symbol.as_deref(), Some("decision"));
        assert_eq!(hit.repo.as_deref(), Some("Cortex"));
        assert!(hit.text.starts_with("# 1. Bypass vectorizer-sdk"));
        assert_eq!(
            hit.extras.get("title").and_then(|v| v.as_str()),
            Some("1. Bypass vectorizer-sdk")
        );
        assert_eq!(
            hit.extras.get("status").and_then(|v| v.as_str()),
            Some("superseded")
        );
        assert!(hit.extras.contains_key("body_markdown"));
    }

    #[test]
    fn doc_to_hit_violation_extracts_law_id() {
        let inner = serde_json::json!({
            "title": "CONSULT-ANALYSIS-BEFORE-IMPLEMENTING",
            "law_id": "CONSULT-ANALYSIS-BEFORE-IMPLEMENTING",
            "severity": "info",
            "body": "## Consult analysis before implementing"
        });
        let doc = serde_json::json!({
            "event_id": "01KQ82V4WEDM6NPYRDQ0T4MSRN",
            "kind": "law_violation",
            "repo": "Cortex",
            "path": ".claude/rules/consult-analysis-before-implementing.md",
            "ts": 0,
            "body": serde_json::to_string(&inner).unwrap(),
        });
        let (symbol, hit) = doc_to_hit(&doc, "governance").expect("violation projected");
        assert_eq!(symbol, "law_violation");
        assert_eq!(
            hit.extras.get("law_id").and_then(|v| v.as_str()),
            Some("CONSULT-ANALYSIS-BEFORE-IMPLEMENTING")
        );
    }

    #[test]
    fn doc_to_hit_skips_unknown_kind_for_family() {
        let doc = serde_json::json!({
            "event_id": "evt-1",
            "kind": "tool_call",
            "body": "{}",
        });
        // tool_call lives in `cortex-*-code`; the loader should not
        // touch it from the `decisions` family.
        assert!(doc_to_hit(&doc, "decisions").is_none());
    }

    #[test]
    fn doc_to_hit_handles_missing_event_id() {
        let doc = serde_json::json!({
            "kind": "decision",
            "body": "{}",
        });
        assert!(doc_to_hit(&doc, "decisions").is_none());
    }
}
