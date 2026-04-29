//! Live Meilisearch-backed `KeywordLane`.
//!
//! `MemoryKeywordLane` is a test double — its `search` method ignores
//! `req.query` and returns whatever the seeder loaded. The 2026-04-27
//! audit captured the symptom directly: every `/v1/query` call
//! returned the same five smoke-test envelopes regardless of input.
//!
//! This module ships the production read-path: per-query search
//! against the same per-project Meili indexes
//! (`cortex-{slug}-{family}`) the spec-08 fulltext-worker upserts to.
//! Translates the orchestrator's `KeywordRequest { index, query,
//! limit, scope }` into Meili's `POST /indexes/{uid}/search` body and
//! maps the response back into `LaneHit` rows. A connection failure
//! returns `LaneError::Transport` — the orchestrator's fail-open
//! policy handles the rest.
//!
//! The lane's `source` label on every emitted hit is
//! `Some("keyword")` so the orchestrator's source-attribution stays
//! honest (the audit also caught keyword hits being labelled
//! `"vector"` because the previous double didn't stamp anything and
//! downstream code defaulted to the wrong field).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::lanes::{KeywordLane, KeywordRequest, LaneError, LaneHit};

/// Concrete `KeywordLane` backed by a live Meilisearch instance.
#[derive(Debug, Clone)]
pub struct MeiliKeywordLane {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl MeiliKeywordLane {
    /// Build a new lane against `base_url` (e.g. `http://127.0.0.1:17004`).
    /// `api_key` is the Meili master / search key (required when the
    /// server enforces auth, optional in the no-auth dev profile).
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("reqwest builder: {e}"))?;
        Ok(Self {
            base_url: base_url.into(),
            api_key,
            http,
        })
    }

    /// Probe `/health` so the caller can decide whether to swap in
    /// the lane or fall back to `MemoryKeywordLane`. Returns `Ok(())`
    /// only when the server answers a 2xx within the timeout.
    pub async fn probe(&self) -> Result<(), String> {
        let url = format!("{}/health", self.base_url.trim_end_matches('/'));
        let mut req = self.http.get(&url);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("probe {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("probe {url}: status {}", resp.status()));
        }
        Ok(())
    }
}

/// Spec-08 doc shape (subset) — only the fields the keyword lane
/// projects into `LaneHit`. Stays serde-flexible: the worker
/// upserts a richer body, but the lane only needs these.
///
/// `extras_raw` (`#[serde(flatten)]`) sweeps up every field on the
/// document that doesn't map to one of the typed slots above —
/// including the spec-11 lane-projection-contract keys
/// (`decision_id`, `turn_id`, `law_id`, …). The Meili upserter
/// stamps those at the top level of the indexed document, so they
/// land here and `project` copies them into `LaneHit.extras`.
///
/// `meta` (`_meta`) is the legacy nesting used by older fulltext
/// builds; `project` reads it as a fallback when the same key is
/// missing from the top level.
#[derive(Debug, Default, Deserialize)]
struct MeiliDoc {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    ts: Option<i64>,
    #[serde(default)]
    severity: Option<String>,
    /// Meili's per-hit ranking score when `showRankingScore=true` is
    /// passed on the search body. `[0, 1]`.
    #[serde(default, rename = "_rankingScore")]
    ranking_score: Option<f64>,
    /// Legacy nesting — older fulltext-worker builds stamped the
    /// projection-contract keys under `_meta` instead of the top
    /// level. `project` reads here as a fallback.
    #[serde(default, rename = "_meta")]
    meta: Option<serde_json::Map<String, serde_json::Value>>,
    /// Catch-all for every field not matched above — captures the
    /// projection-contract keys (`decision_id`, `turn_id`, etc.)
    /// the worker writes at the top level.
    #[serde(flatten)]
    extras_raw: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct MeiliSearchResponse {
    hits: Vec<MeiliDoc>,
}

#[async_trait]
impl KeywordLane for MeiliKeywordLane {
    async fn search(&self, req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        let url = format!(
            "{}/indexes/{}/search",
            self.base_url.trim_end_matches('/'),
            req.index
        );
        // The `q` field carries the user's query verbatim. Meili
        // tokenises and applies typo-tolerance per the v1 settings
        // the fulltext-worker baked in. `showRankingScore=true`
        // surfaces the per-hit `_rankingScore` so the orchestrator
        // can fuse with vector / graph rather than relying on the
        // positional `1/(60+rank)` artefact today's MemoryKeywordLane
        // produces. `limit` is the orchestrator's per-field cap
        // (already validated in upstream code).
        let body = serde_json::json!({
            "q": req.query,
            "limit": req.limit,
            "showRankingScore": true,
        });

        let mut http_req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key);
        }
        let resp = http_req
            .send()
            .await
            .map_err(|e| LaneError::Transport(format!("{url}: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            // 404 on the per-project index is the legitimate
            // empty-index case (the spec-08 worker materialises
            // indexes lazily on first upsert). Return zero hits
            // rather than failing the whole orchestrator turn.
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(Vec::new());
            }
            let detail = resp.text().await.unwrap_or_default();
            return Err(LaneError::Rejected(format!("{url}: {status}: {detail}")));
        }

        let parsed: MeiliSearchResponse = resp
            .json()
            .await
            .map_err(|e| LaneError::Transport(format!("{url}: decode: {e}")))?;

        let hits = parsed
            .hits
            .into_iter()
            .map(|doc| project(doc, req))
            .collect();
        Ok(hits)
    }
}

/// Crate-internal test seam — deserialise an arbitrary upstream
/// shape into [`MeiliDoc`] and run [`project`] against it. The
/// regression guard in `crate::lane_contract` uses this to drive
/// the Meili projection without the live HTTP path.
#[cfg(test)]
pub(crate) fn project_doc(
    json: serde_json::Value,
    req: &KeywordRequest,
) -> Result<LaneHit, serde_json::Error> {
    let doc: MeiliDoc = serde_json::from_value(json)?;
    Ok(project(doc, req))
}

/// Project one Meili document into a `LaneHit`. Uses `summary` as
/// the snippet text when present (the spec-08 body-selection rule
/// stamps it), otherwise the raw `body`. The `source` label is
/// always `"keyword"` so the orchestrator's source-attribution
/// stays honest.
fn project(doc: MeiliDoc, req: &KeywordRequest) -> LaneHit {
    let event_id = doc
        .event_id
        .clone()
        .or(doc.id.clone())
        .unwrap_or_else(|| "unknown".to_string());
    // Doc-id namespaces the hit so the orchestrator's RRF bucket is
    // distinct from archive / vector hits keyed on the same event.
    let doc_id = format!("meili|{}|{}", req.index, event_id);

    // Prefer the curated summary when the worker stamped one;
    // otherwise the title gives shape; otherwise the raw body. Each
    // fallback is a smaller-but-still-honest projection — never an
    // empty string when the doc has any text content.
    let text = doc
        .summary
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| doc.title.clone().filter(|s| !s.is_empty()))
        .or_else(|| doc.body.clone())
        .unwrap_or_default();

    let mut extras = std::collections::BTreeMap::new();
    // The orchestrator's `lane_label` reads `extras["source"]` and
    // falls back to `"vector"` when missing. Stamp `"keyword"` so the
    // RRF snippet `source` field matches the lane that actually
    // produced the hit — the audit caught this (every hit was
    // labelled `"vector"` because the previous double left the field
    // empty).
    extras.insert(
        "source".to_string(),
        serde_json::Value::String("keyword".to_string()),
    );

    // Phase6b — spec-11 lane projection contract.
    //
    // Every overlay derivation (`derive_decisions`,
    // `derive_similar_turns`, `derive_laws`) reads its inputs out of
    // `extras` directly, so the live keyword lane MUST stamp the
    // contract keys here. Lookup order matches the documented
    // contract: `_meta.<key>` (older fulltext-worker shape) wins
    // over the top-level key, since when both exist the worker is
    // mid-migration and `_meta` is the canonical post-migration
    // location.
    //
    // `summary` and `severity` overlap with typed slots on
    // `MeiliDoc` (used for snippet text + the top-level
    // `LaneHit.severity` field respectively), so `serde_flatten`
    // does NOT route them through `extras_raw`. The match arms
    // below pull the typed value as a final fallback so the
    // contract still fires when the worker stamps only the
    // top-level field.
    for key in crate::lanes::LANE_EXTRAS_KEYS {
        let from_meta = doc.meta.as_ref().and_then(|m| m.get(*key)).cloned();
        let from_top = doc.extras_raw.get(*key).cloned();
        let from_typed: Option<serde_json::Value> = match *key {
            "summary" => doc
                .summary
                .clone()
                .map(serde_json::Value::String),
            "severity" => doc
                .severity
                .clone()
                .map(serde_json::Value::String),
            _ => None,
        };
        let val = from_meta.or(from_top).or(from_typed);
        if let Some(v) = val {
            if !v.is_null() {
                extras.insert((*key).to_string(), v);
            }
        }
    }

    // §2.3 — surface a debug log when a `kind=decision` doc lands
    // without a `decision_id`. That's a worker-side projection bug
    // (`spec-08` requires every decision row to carry the id);
    // dropping it here would leave the decisions overlay
    // permanently empty for that row.
    if doc.kind.as_deref() == Some("decision")
        && !extras.contains_key("decision_id")
    {
        tracing::debug!(
            doc_id = %doc_id,
            "decision row without decision_id — worker projection gap"
        );
    }

    LaneHit {
        doc_id,
        text,
        repo: doc.repo,
        path: doc.path,
        symbol: doc.kind,
        content_hash: doc.content_hash,
        score: doc.ranking_score.unwrap_or(0.0),
        ts: doc.ts.unwrap_or(0),
        severity: doc.severity,
        extras,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scope;

    fn req(query: &str) -> KeywordRequest {
        KeywordRequest {
            index: "cortex-cortex-code".into(),
            query: query.into(),
            limit: 10,
            scope: Scope::default(),
        }
    }

    #[test]
    fn projects_a_doc_with_summary_into_a_lane_hit() {
        let doc = MeiliDoc {
            id: Some("evt-1".into()),
            event_id: Some("evt-1".into()),
            kind: Some("turn".into()),
            repo: Some("Cortex".into()),
            path: Some("src/lib.rs".into()),
            title: Some("Add embedder lane".into()),
            body: Some("Body verbatim".into()),
            summary: Some("Add embedder lane (curated)".into()),
            content_hash: Some("sha256:abc".into()),
            ts: Some(1714200000000),
            severity: Some("info".into()),
            ranking_score: Some(0.83),
            ..MeiliDoc::default()
        };
        let hit = project(doc, &req("embedder"));
        assert_eq!(hit.doc_id, "meili|cortex-cortex-code|evt-1");
        assert_eq!(hit.text, "Add embedder lane (curated)");
        assert_eq!(hit.repo.as_deref(), Some("Cortex"));
        assert_eq!(hit.symbol.as_deref(), Some("turn"));
        assert!((hit.score - 0.83).abs() < 1e-6);
        assert_eq!(hit.ts, 1714200000000);
        assert_eq!(
            hit.extras.get("source").and_then(|v| v.as_str()),
            Some("keyword"),
            "lane_label() falls back to 'vector' when extras['source'] is missing"
        );
    }

    #[test]
    fn projects_falls_back_through_title_then_body() {
        let mut doc = MeiliDoc {
            event_id: Some("evt-2".into()),
            title: Some("Just a title".into()),
            body: Some("body line".into()),
            ..MeiliDoc::default()
        };
        let hit = project(doc.clone_for_test(), &req("x"));
        assert_eq!(hit.text, "Just a title");

        // Drop the title — body should win.
        doc.title = None;
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "body line");
    }

    #[test]
    fn projects_emits_empty_text_only_when_doc_has_no_content() {
        let doc = MeiliDoc {
            id: Some("evt-3".into()),
            ..MeiliDoc::default()
        };
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "");
        // event_id falls back to id when present.
        assert!(hit.doc_id.ends_with("|evt-3"));
    }

    #[test]
    fn projects_uses_unknown_event_id_when_both_id_fields_missing() {
        let doc = MeiliDoc {
            body: Some("body".into()),
            ..MeiliDoc::default()
        };
        let hit = project(doc, &req("x"));
        assert!(hit.doc_id.ends_with("|unknown"));
    }

    impl MeiliDoc {
        // Test helper — manual clone since serde_derive doesn't
        // emit one and we don't want to add the Clone derive to
        // the production type.
        fn clone_for_test(&self) -> Self {
            Self {
                id: self.id.clone(),
                event_id: self.event_id.clone(),
                kind: self.kind.clone(),
                repo: self.repo.clone(),
                path: self.path.clone(),
                title: self.title.clone(),
                body: self.body.clone(),
                summary: self.summary.clone(),
                content_hash: self.content_hash.clone(),
                ts: self.ts,
                severity: self.severity.clone(),
                ranking_score: self.ranking_score,
                meta: self.meta.clone(),
                extras_raw: self.extras_raw.clone(),
            }
        }
    }
}
