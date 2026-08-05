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

use crate::lanes::{
    collection_missing_marker, collection_missing_report_enabled, KeywordLane, KeywordRequest,
    LaneError, LaneHit,
};

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
        let resp = req.send().await.map_err(|e| format!("probe {url}: {e}"))?;
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
        //
        // phase10a / phase10h — multi-clause Meili filter. Caps
        // are per-index because Meili rejects filter clauses that
        // reference a non-filterable attribute with a 4xx
        // (`Attribute X is not filterable`). The filter expression
        // ANDs every present scope dimension.
        let mut body = serde_json::json!({
            "q": req.query,
            "limit": req.limit,
            "showRankingScore": true,
        });
        // Build the composite filter: scope clauses AND (when AC is active)
        // the Bell-LaPadula ACL clause from the resolved principal.
        let scope_filter = build_meili_filter(&req.scope, &req.index);
        let acl_filter = req.acl.as_ref().map(|acl| {
            cortex_workers::acl::filter::meili_acl_filter(
                acl.clearance_level,
                &acl.compartment_grants,
            )
        });
        let combined_filter = match (scope_filter, acl_filter) {
            (Some(s), Some(a)) => Some(format!("{s} AND {a}")),
            (Some(s), None) => Some(s),
            (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        if let Some(filter) = combined_filter {
            if let Some(map) = body.as_object_mut() {
                map.insert("filter".to_string(), serde_json::Value::String(filter));
            }
        }

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
            //
            // Phase11e §3 — when the env switch is on, emit a
            // synthetic marker so the orchestrator can surface
            // the missing index in `debug.notes`. Default (env
            // unset) keeps fail-open so the response shape is
            // backwards-compatible.
            if status == reqwest::StatusCode::NOT_FOUND {
                if collection_missing_report_enabled() {
                    return Ok(vec![collection_missing_marker(req.index.clone())]);
                }
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

/// phase10h — capability matrix for one Meili index. Filter
/// clauses reference these attributes only when the index
/// supports them; Meili rejects unfilterable attributes with a
/// 4xx, so silently dropping unsupported clauses is the right
/// default.
#[derive(Debug, Clone, Copy, Default)]
struct IndexFilterCaps {
    /// `repo` is a filterable attribute on this index.
    repo: bool,
    /// Field name carrying the event timestamp (`ts` for per-repo
    /// legacy indexes, `occurred_at` for global indexes). `None`
    /// means the index does not expose a filterable timestamp.
    since_field: Option<&'static str>,
    /// Field name carrying the topic vocabulary (always
    /// `topics`). `None` when the index has no topic facet.
    topics_field: Option<&'static str>,
    /// `path` is a filterable attribute (per-repo legacy
    /// indexes). Global indexes don't expose path.
    path: bool,
}

/// phase10h — resolve per-index filter capabilities. Per-repo
/// legacy indexes (`cortex-{slug}-{family}`) inherit the worker's
/// `settings.v1.json` filterable attributes (kind, repo, path,
/// topics, severity, pii_risk, ts, language, ext.*). Global
/// indexes carry their own per-schema settings — see
/// `crates/cortex-storage/schemas/meili/*.settings.v1.json`.
fn caps_for(index: &str) -> IndexFilterCaps {
    use cortex_storage::names::{
        INDEX_ANALYSES, INDEX_DECISIONS, INDEX_KNOWLEDGE, INDEX_LAWS, INDEX_LEARNINGS,
        INDEX_MEMORIES, INDEX_TURNS,
    };
    if index.starts_with("cortex-") && !index.starts_with("cortex_") {
        // Per-repo legacy `cortex-{slug}-{family}` index. Worker
        // settings (`crates/cortex-workers/settings/settings.v1.json`)
        // declares: kind, repo, path, topics, severity, pii_risk,
        // ts, language, ext.*.
        return IndexFilterCaps {
            repo: true,
            since_field: Some("ts"),
            topics_field: Some("topics"),
            path: true,
        };
    }
    match index {
        i if i == INDEX_TURNS => IndexFilterCaps {
            repo: true,
            since_field: Some("occurred_at"),
            topics_field: Some("topics"),
            path: false,
        },
        i if i == INDEX_DECISIONS => IndexFilterCaps {
            repo: true,
            since_field: Some("occurred_at"),
            topics_field: Some("topics"),
            path: false,
        },
        i if i == INDEX_ANALYSES => IndexFilterCaps {
            repo: true,
            since_field: Some("occurred_at"),
            topics_field: Some("topics"),
            path: false,
        },
        i if i == INDEX_LAWS => IndexFilterCaps {
            // Laws are cross-repo + carry no topics / occurred_at
            // facet (severity / applies_to only).
            repo: false,
            since_field: None,
            topics_field: None,
            path: false,
        },
        i if i == INDEX_MEMORIES => IndexFilterCaps {
            repo: false,
            since_field: Some("occurred_at"),
            topics_field: None,
            path: false,
        },
        i if i == INDEX_KNOWLEDGE => IndexFilterCaps {
            repo: true,
            since_field: Some("occurred_at"),
            topics_field: None,
            path: false,
        },
        i if i == INDEX_LEARNINGS => IndexFilterCaps {
            repo: true,
            since_field: Some("occurred_at"),
            topics_field: None,
            path: false,
        },
        _ => IndexFilterCaps::default(),
    }
}

/// phase10h — build the Meili filter expression for one search
/// call. Returns `None` when the resolved scope adds no clauses
/// (every dimension is empty or unsupported by the target
/// index). Each present clause ANDs into the final expression
/// using Meili's filter grammar.
///
/// Quoting strategy: every literal is single-quoted with `'`
/// escaped as `\'`. Slugs / paths / RFC-3339 timestamps + topic
/// labels are conservative ASCII identifiers in practice, so the
/// extra escape is belt-and-suspenders rather than a hot path.
fn build_meili_filter(scope: &crate::types::Scope, index: &str) -> Option<String> {
    let caps = caps_for(index);
    let mut clauses: Vec<String> = Vec::new();

    if caps.repo {
        if let Some(repo) = scope.repo.as_deref().filter(|s| !s.is_empty()) {
            let slug = cortex_storage::names::slug_for_repo(repo);
            clauses.push(format!("repo = {}", quote_meili(&slug)));
        }
    }
    if let Some(field) = caps.since_field {
        if let Some(since) = scope.since.as_deref().filter(|s| !s.is_empty()) {
            clauses.push(format!("{field} >= {}", quote_meili(since)));
        }
    }
    if let Some(field) = caps.topics_field {
        if !scope.topics.is_empty() {
            let list = scope
                .topics
                .iter()
                .filter(|t| !t.is_empty())
                .map(|t| quote_meili(t))
                .collect::<Vec<_>>()
                .join(", ");
            if !list.is_empty() {
                clauses.push(format!("{field} IN [{list}]"));
            }
        }
    }
    if caps.path && !scope.files.is_empty() {
        // Phase11b — Meili's filter grammar does NOT support
        // `STARTS WITH`; it rejects the request with
        // `invalid_search_filter`. The indexer projects every
        // ancestor path plus the full path into a filterable
        // array `path_prefixes`, so prefix scoping becomes
        // `path_prefixes IN ['a/', 'a/b/', 'a/b/c.rs']` — a
        // constant-cost equality match against the array. Each
        // `scope.files` entry is forwarded verbatim (callers send
        // either `dir/` or `dir/file.ext`; both forms appear in
        // the indexed array).
        let prefixes = scope
            .files
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| quote_meili(p))
            .collect::<Vec<_>>();
        if !prefixes.is_empty() {
            clauses.push(format!("(path_prefixes IN [{}])", prefixes.join(", ")));
        }
    }

    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// phase10h — single-quote a literal for Meili's filter
/// expression grammar. Inner single quotes are escaped as `\'`.
fn quote_meili(value: &str) -> String {
    let escaped = value.replace('\'', "\\'");
    format!("'{escaped}'")
}

/// phase10a — UTF-8-safe byte excerpt. Truncates at a char boundary
/// at or below `max_bytes` so we never cleave a multi-byte code
/// point. Used by the decision-rationale projection (§1.2).
fn take_excerpt(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

/// Phase6g — bytes threshold above which a projected body emits a
/// `tracing::debug!` so operators can flag oversized chunks. Mirrors
/// the upstream value the fulltext worker uses to decide whether to
/// route a body through the summary-substitution path
/// (`cortex_workers::fulltext::OVERSIZE_BODY_BYTES = 4 KiB`); kept
/// as a local const because the API crate does not depend on the
/// workers crate (avoids the cyclic-feeling reverse dep).
const PROJECTED_BODY_DEBUG_BYTES: usize = 4 * 1024;

/// Phase6g — pick the field that actually carries searchable
/// content for `kind`. The pre-phase6g chain (`summary > title >
/// body`) was wrong for `kind=artifact`: code/doc files have
/// `summary = ""`, `title = path`, `body = real content`, so the
/// chain stopped at `title` and every artifact hit landed in the
/// response with `text = "<path>"`. Meili's BM25 ranking already
/// considered `body`, so the bug was purely read-side: the right
/// document was found, then projected through the wrong field.
///
/// Per-kind precedence:
///
/// | kind                          | precedence                   |
/// | ----------------------------- | ---------------------------- |
/// | `artifact`, `law_violation`   | `body > summary > title`     |
/// | `decision`, `analysis`,       | `summary > title > body`     |
/// | `memory`                      |                              |
/// | `turn`, `tool_call`,          | `summary > body > title`     |
/// | `agent_call`                  |                              |
/// | _anything else / `None`_      | `summary > title > body`     |
///
/// Each branch returns a `Vec<&Option<String>>` of references to
/// the typed `MeiliDoc` fields in the chosen order; the caller
/// picks the first non-empty one. Returning references keeps the
/// hot path allocation-free.
fn projection_chain<'a>(
    kind: Option<&str>,
    summary: &'a Option<String>,
    title: &'a Option<String>,
    body: &'a Option<String>,
) -> [&'a Option<String>; 3] {
    match kind {
        Some("artifact") | Some("law_violation") => [body, summary, title],
        Some("decision") | Some("analysis") | Some("memory") => [summary, title, body],
        Some("turn") | Some("tool_call") | Some("agent_call") => [summary, body, title],
        _ => [summary, title, body],
    }
}

/// Project one Meili document into a `LaneHit`. Uses a kind-aware
/// precedence chain to pick the most-content-bearing field for
/// `LaneHit.text` (phase6g). The `source` label is always
/// `"keyword"` so the orchestrator's source-attribution stays
/// honest.
fn project(doc: MeiliDoc, req: &KeywordRequest) -> LaneHit {
    let event_id = doc
        .event_id
        .clone()
        .or(doc.id.clone())
        .unwrap_or_else(|| "unknown".to_string());
    // Doc-id namespaces the hit so the orchestrator's RRF bucket is
    // distinct from archive / vector hits keyed on the same event.
    let doc_id = format!("meili|{}|{}", req.index, event_id);

    // Phase6g — kind-aware text projection. See `projection_chain`
    // for the per-kind table. Each fallback is a smaller-but-still-
    // honest projection — never an empty string when the doc has
    // any text content.
    //
    // phase10b §1 — when a slot in the chain holds the same string
    // as `doc.path`, treat it as empty. The audit logged
    // `text='crates/cortex-api/src/types.rs'` snippets that the
    // bundle renderer formatted as `path:artifact — \n   path` —
    // an `ls`-grade result the agent can't act on. Title for
    // artifact docs is exactly the path (see `derive_title` in
    // `cortex-workers::fulltext::builders.rs`), so this filter
    // skips the title slot when the chain reached it from the
    // path; non-path-equal titles still surface unchanged.
    let path_str = doc.path.as_deref().unwrap_or("");
    let chain = projection_chain(doc.kind.as_deref(), &doc.summary, &doc.title, &doc.body);
    let text = chain
        .iter()
        .find_map(|slot| {
            slot.as_ref()
                .filter(|s| !s.is_empty())
                .filter(|s| path_str.is_empty() || s.as_str() != path_str)
                .cloned()
        })
        .unwrap_or_default();
    // phase10b §1 — flag the projector as having no body to
    // project. Set when the kind-aware chain produced no text
    // distinct from the path; the orchestrator's
    // `snippet_from_hit` pulls this from `extras` and stamps
    // `Snippet.body_truncated` so the bundle renderer downgrades
    // to header-only formatting instead of repeating the path.
    let body_truncated = text.is_empty();
    if text.len() > PROJECTED_BODY_DEBUG_BYTES {
        tracing::debug!(
            doc_id = %doc_id,
            kind = ?doc.kind,
            bytes = text.len(),
            threshold = PROJECTED_BODY_DEBUG_BYTES,
            "projected body exceeds debug threshold; orchestrator trim ladder will clamp"
        );
    }

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
    if body_truncated {
        extras.insert("body_truncated".to_string(), serde_json::Value::Bool(true));
    }

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
            "summary" => doc.summary.clone().map(serde_json::Value::String),
            "severity" => doc.severity.clone().map(serde_json::Value::String),
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
    if doc.kind.as_deref() == Some("decision") && !extras.contains_key("decision_id") {
        tracing::debug!(
            doc_id = %doc_id,
            "decision row without decision_id — worker projection gap"
        );
    }

    // phase10a §1.2 — stamp the decision's title and rationale
    // excerpt into extras so the orchestrator's `derive_decisions`
    // overlay can surface the rationale body (not just the title /
    // curated summary) on `decision_lookup` responses. Cap the
    // excerpt at the first 1 KiB of the body to keep the wire
    // payload tight; the snippet section still carries the full
    // body when the caller asked for `IncludeField::Snippets`.
    if doc.kind.as_deref() == Some("decision") {
        if let Some(title) = doc.title.as_deref().filter(|s| !s.is_empty()) {
            extras
                .entry("decision_title".to_string())
                .or_insert_with(|| serde_json::Value::String(title.to_string()));
        }
        if let Some(body) = doc.body.as_deref().filter(|s| !s.is_empty()) {
            let excerpt = take_excerpt(body, 1024);
            extras
                .entry("rationale_excerpt".to_string())
                .or_insert_with(|| serde_json::Value::String(excerpt));
        }
    }

    // Phase30b §1.2 — stamp the consolidation lane-projection keys
    // into extras. Unlike `decision_id` / `turn_id` / `law_id`,
    // `consolidation_id` and `grain` are nested under the document's
    // shared `ext.consolidation.*` extension bag (spec-08 §Document
    // schema; see `cortex_workers::fulltext::builders::apply_extensions`
    // — Consolidation/TopicCard never got a top-level projection like
    // Decision/LawViolation did) rather than flattened to the top
    // level, so `LANE_EXTRAS_KEYS` never reaches them; pull the
    // nested object out of `extras_raw["ext"]["consolidation"]`
    // explicitly. `consolidation_title` mirrors the `decision_title`
    // degrade above — without it `ConsolidationRef.title` would fall
    // back to the summary/body text instead of the consolidator's
    // curated one-line title.
    if doc.kind.as_deref() == Some("consolidation") {
        let cons_ext = doc
            .extras_raw
            .get("ext")
            .and_then(|v| v.get("consolidation"));
        if let Some(id) = cons_ext
            .and_then(|c| c.get("consolidation_id"))
            .and_then(|v| v.as_str())
        {
            extras
                .entry("consolidation_id".to_string())
                .or_insert_with(|| serde_json::Value::String(id.to_string()));
        }
        if let Some(title) = doc.title.as_deref().filter(|s| !s.is_empty()) {
            extras
                .entry("consolidation_title".to_string())
                .or_insert_with(|| serde_json::Value::String(title.to_string()));
        }
    }

    // phase10d — `repo` is canonical lowercase across every
    // Cortex surface. Lane projection always lowercases so a
    // pre-phase10d corpus indexed under capitalized repos
    // (`Cortex`, `Vectorizer`) still aligns with the
    // lowercase-only filter the orchestrator applies. The
    // original-case label is preserved in `extras.repo_label`
    // for the dashboard's display column.
    let canonical = doc.repo.as_deref().map(str::to_ascii_lowercase);
    if let Some(label) = doc.repo.as_deref() {
        if canonical.as_deref() != Some(label) && !label.is_empty() {
            extras.insert(
                "repo_label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
    }
    // ADR-011 — populate the typed overlay alongside extras during
    // the staged migration. The orchestrator's overlay derivers
    // continue reading from extras until §4 cuts the cord; the
    // typed fields here let the new code path verify parity
    // before the cut.
    let overlay = crate::lanes::Overlay {
        decision_id: extras
            .get("decision_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        decision_status: extras
            .get("decision_status")
            .and_then(|v| v.as_str())
            .and_then(parse_decision_status),
        superseded_by: extras
            .get("supersedes")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        turn_id: extras
            .get("turn_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        model: extras
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        summary: extras
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        law_id: extras
            .get("law_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        violation_id: extras
            .get("violation_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        // Phase30b §1.2 — same nested `ext.consolidation.grain`
        // source as the `consolidation_id` / `consolidation_title`
        // extras above; parsed through the typed
        // `ConsolidationGrain` enum (snake_case wire form) so
        // `derive_consolidations` reads a typed value rather than a
        // raw string.
        consolidation_grain: doc
            .extras_raw
            .get("ext")
            .and_then(|v| v.get("consolidation"))
            .and_then(|c| c.get("grain"))
            .and_then(|v| v.as_str())
            .and_then(|s| {
                serde_json::from_value::<cortex_core::events::ConsolidationGrain>(
                    serde_json::Value::String(s.to_string()),
                )
                .ok()
            }),
        severity: doc.severity.clone(),
        source: crate::lanes::LaneSource::Keyword,
        ..crate::lanes::Overlay::default()
    };
    LaneHit {
        doc_id,
        text,
        repo: canonical,
        path: doc.path,
        symbol: doc.kind,
        content_hash: doc.content_hash,
        score: doc.ranking_score.unwrap_or(0.0),
        ts: doc.ts.unwrap_or(0),
        severity: doc.severity,
        extras,
        overlay,
    }
}

fn parse_decision_status(s: &str) -> Option<crate::lanes::DecisionStatus> {
    match s {
        "proposed" => Some(crate::lanes::DecisionStatus::Proposed),
        "accepted" => Some(crate::lanes::DecisionStatus::Accepted),
        "superseded" => Some(crate::lanes::DecisionStatus::Superseded),
        "deprecated" => Some(crate::lanes::DecisionStatus::Deprecated),
        "rejected" => Some(crate::lanes::DecisionStatus::Rejected),
        _ => None,
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
            acl: None,
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
        // phase10d — `repo` is canonical lowercase on the lane
        // hit; the original `Cortex` casing is preserved in
        // `extras.repo_label` for the dashboard's display column.
        assert_eq!(hit.repo.as_deref(), Some("cortex"));
        assert_eq!(
            hit.extras.get("repo_label").and_then(|v| v.as_str()),
            Some("Cortex")
        );
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

    // ---- Phase6g — kind-aware projection regression set ----

    fn doc_with(
        kind: &str,
        title: Option<&str>,
        body: Option<&str>,
        summary: Option<&str>,
    ) -> MeiliDoc {
        MeiliDoc {
            event_id: Some("evt-x".into()),
            kind: Some(kind.into()),
            path: title.map(String::from),
            title: title.map(String::from),
            body: body.map(String::from),
            summary: summary.map(String::from),
            ..MeiliDoc::default()
        }
    }

    #[test]
    fn artifact_kind_prefers_body_over_path_title() {
        // Pre-phase6g regression: artifact docs landed with `text =
        // path` because the chain stopped at `title`. With the new
        // chain, body wins and the actual file content reaches the
        // bundle. This is the headline behaviour change.
        let doc = doc_with(
            "artifact",
            Some("crates/cortex-api/src/vectorizer_lane.rs"),
            Some("pub struct LoginCreds { ... } pub fn refresh_token() {}"),
            None,
        );
        let hit = project(doc, &req("LoginCreds"));
        assert_eq!(
            hit.text, "pub struct LoginCreds { ... } pub fn refresh_token() {}",
            "artifact text must surface body, not the file path"
        );
        // The path is preserved separately on `LaneHit.path` — no
        // information lost, only the masking removed.
        assert_eq!(
            hit.path.as_deref(),
            Some("crates/cortex-api/src/vectorizer_lane.rs")
        );
    }

    #[test]
    fn artifact_kind_falls_through_to_summary_when_body_empty() {
        let doc = doc_with(
            "artifact",
            Some("README.md"),
            None,
            Some("project overview"),
        );
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "project overview");
    }

    #[test]
    fn artifact_kind_emits_empty_text_when_title_equals_path_and_no_body() {
        // phase10b §1 — `derive_title` sets `title = path` for
        // artifact events without a richer heading. Surfacing the
        // path as `text` produced the audit-flagged
        // `path:artifact — \n   path` rendering. The new contract
        // returns empty text + `body_truncated = true` so the
        // bundle renderer collapses to a header-only line and the
        // path stays in `Snippet.path` (not in `text`).
        let doc = doc_with("artifact", Some("README.md"), None, None);
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "");
        assert_eq!(
            hit.extras.get("body_truncated").and_then(|v| v.as_bool()),
            Some(true),
            "body_truncated extras must flag the no-content case"
        );
    }

    #[test]
    fn law_violation_kind_prefers_body() {
        // Violation messages live in body — same shape as artifact.
        let doc = doc_with(
            "law_violation",
            Some("LAW-007"),
            Some("operator ran git push --force on main"),
            None,
        );
        let hit = project(doc, &req("force push"));
        assert_eq!(hit.text, "operator ran git push --force on main");
    }

    #[test]
    fn decision_kind_keeps_summary_first() {
        // Curated kinds always have a summary; the inverted chain
        // would surface raw body and lose the curated lede. Pin the
        // summary-first contract for decision/analysis/memory.
        let doc = doc_with(
            "decision",
            Some("ADR-0042"),
            Some("long body verbatim"),
            Some("Adopt Meilisearch over Lexum (curated)"),
        );
        let hit = project(doc, &req("meili"));
        assert_eq!(hit.text, "Adopt Meilisearch over Lexum (curated)");
    }

    #[test]
    fn analysis_kind_falls_through_to_body_when_title_equals_path() {
        // phase10b §1 — `doc_with` sets `path = title`, so the
        // path-equal-title slot is skipped and the analysis chain
        // (`summary > title > body`) reaches `body`. Surfacing the
        // body keeps the snippet useful instead of echoing the
        // file path back to the agent.
        let doc = doc_with(
            "analysis",
            Some("Relevance audit 2026-04"),
            Some("body verbatim"),
            None,
        );
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "body verbatim");
    }

    #[test]
    fn analysis_kind_keeps_title_when_distinct_from_path() {
        // When title and path diverge (the worker stamped a real
        // heading), the title slot is still preferred — the
        // path-equal filter only fires on the
        // path-as-title artifact case.
        let doc = MeiliDoc {
            event_id: Some("evt-x".into()),
            kind: Some("analysis".into()),
            path: Some("docs/audit.md".into()),
            title: Some("Relevance audit 2026-04".into()),
            body: Some("body verbatim".into()),
            ..MeiliDoc::default()
        };
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "Relevance audit 2026-04");
    }

    #[test]
    fn memory_kind_falls_through_to_body_when_summary_and_title_empty() {
        let doc = doc_with("memory", None, Some("note body"), None);
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "note body");
    }

    #[test]
    fn turn_kind_uses_summary_then_body_then_title() {
        // Turn / tool_call / agent_call: summary first (classifier
        // crops the wall of text), body second (raw transcript when
        // no summary), title last.
        let doc = doc_with(
            "turn",
            Some("title"),
            Some("body verbatim"),
            Some("classifier summary"),
        );
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "classifier summary");

        let doc_no_summary = doc_with("turn", Some("title"), Some("body verbatim"), None);
        let hit2 = project(doc_no_summary, &req("x"));
        assert_eq!(hit2.text, "body verbatim");

        // phase10b §1 — when the only non-empty slot in the chain
        // is the title and `title == path`, return empty text +
        // body_truncated rather than echoing the path back.
        let doc_only_title = doc_with("turn", Some("title"), None, None);
        let hit3 = project(doc_only_title, &req("x"));
        assert_eq!(hit3.text, "");
        assert_eq!(
            hit3.extras.get("body_truncated").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn tool_call_kind_uses_same_chain_as_turn() {
        let doc = doc_with("tool_call", Some("Edit"), Some("touched src/lib.rs"), None);
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "touched src/lib.rs");
    }

    #[test]
    fn unknown_kind_falls_back_to_summary_title_body() {
        let doc = doc_with("widget", Some("title"), Some("body"), Some("summary"));
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "summary");
    }

    #[test]
    fn missing_kind_uses_default_chain() {
        // phase10b §1 — `doc_with` sets `path == title`, so the
        // path-equal filter pushes the chain past the title slot
        // and lands on `body`. The default chain is still
        // `summary > title > body`; the filter just elides
        // path-shaped titles.
        let mut doc = doc_with("widget", Some("title"), Some("body"), None);
        doc.kind = None;
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "body");
    }

    #[test]
    fn artifact_with_all_fields_empty_returns_empty_text() {
        // Orchestrator's degenerate-hit filter drops these — the
        // projection just hands back an empty string. Confirm the
        // chain doesn't panic on the all-None case.
        let doc = MeiliDoc {
            event_id: Some("evt-empty".into()),
            kind: Some("artifact".into()),
            ..MeiliDoc::default()
        };
        let hit = project(doc, &req("x"));
        assert_eq!(hit.text, "");
    }

    // ---- Phase10b — body capture regression set ----

    #[test]
    fn artifact_with_real_body_keeps_body_text_and_clears_truncated_flag() {
        // phase10b §4.1 — known-good envelope: an artifact doc
        // carrying a real source body must round-trip with that
        // body in `text` (not the path), and `body_truncated`
        // must NOT be set. This is the regression for the audit
        // finding "snippet text equals path".
        let doc = doc_with(
            "artifact",
            Some("crates/cortex-api/src/strategies.rs"),
            Some("pub fn build_plan(req: &QueryRequest) -> Plan { ... }"),
            None,
        );
        let hit = project(doc, &req("build_plan"));
        assert_eq!(
            hit.text,
            "pub fn build_plan(req: &QueryRequest) -> Plan { ... }"
        );
        assert_ne!(
            hit.text.as_str(),
            "crates/cortex-api/src/strategies.rs",
            "snippet.text MUST NOT carry the path verbatim — that was the audit-flagged regression"
        );
        assert_eq!(
            hit.extras.get("body_truncated").and_then(|v| v.as_bool()),
            None,
            "body_truncated must be absent when body is non-empty"
        );
    }

    // ---- phase10h — Meili scope-filter builder ----

    fn scope() -> crate::types::Scope {
        crate::types::Scope::default()
    }

    #[test]
    fn build_meili_filter_emits_since_clause_on_index_with_occurred_at() {
        let mut s = scope();
        s.since = Some("2026-04-01T00:00:00Z".to_string());
        let filter = build_meili_filter(&s, "cortex_decisions").expect("clause expected");
        assert!(
            filter.contains("occurred_at >= '2026-04-01T00:00:00Z'"),
            "decisions index uses occurred_at; got `{filter}`"
        );
    }

    #[test]
    fn build_meili_filter_uses_ts_field_for_per_repo_legacy_index() {
        // Per-repo legacy `cortex-{slug}-{family}` indexes
        // inherit the worker's settings.v1.json which uses `ts`
        // not `occurred_at` for the timestamp facet.
        let mut s = scope();
        s.since = Some("2026-04-01T00:00:00Z".to_string());
        let filter = build_meili_filter(&s, "cortex-cortex-code").expect("clause expected");
        assert!(
            filter.contains("ts >= '2026-04-01T00:00:00Z'"),
            "per-repo index uses ts; got `{filter}`"
        );
    }

    #[test]
    fn build_meili_filter_ors_topic_list_into_in_clause() {
        let mut s = scope();
        s.topics = vec!["law".to_string(), "governance".to_string()];
        let filter = build_meili_filter(&s, "cortex_turns").expect("clause expected");
        assert!(
            filter.contains("topics IN ['law', 'governance']"),
            "topics list must compose into a single IN clause; got `{filter}`"
        );
    }

    #[test]
    fn build_meili_filter_uses_path_prefixes_in_clause_for_files() {
        // Phase11b §3.4 — the broken predecessor of this test
        // pinned `path STARTS WITH 'x' OR path STARTS WITH 'y'`,
        // which Meili rejects with `invalid_search_filter`. The
        // worker now writes a filterable `path_prefixes` array
        // covering every ancestor + the full path, so a single
        // `IN [..]` predicate replaces the OR chain and is the
        // shape Meili actually accepts.
        let mut s = scope();
        s.files = vec![
            "crates/cortex-api/src/".to_string(),
            "docs/specs/".to_string(),
        ];
        let filter = build_meili_filter(&s, "cortex-cortex-code").expect("clause expected");
        assert!(
            filter.contains("(path_prefixes IN ['crates/cortex-api/src/', 'docs/specs/'])"),
            "expected single IN clause over path_prefixes; got `{filter}`"
        );
        assert!(
            !filter.contains("STARTS WITH"),
            "STARTS WITH must never appear in emitted Meili filters; got `{filter}`"
        );
    }

    #[test]
    fn build_meili_filter_path_prefixes_single_entry_still_in_clause() {
        // Phase11b §3.1 — even with one file the emitter uses the
        // `IN [..]` shape. Mixing `IN [..]` and `=` for the same
        // attribute would force the orchestrator to know which
        // shape the indexer wrote; keeping a single shape avoids
        // that coupling.
        let mut s = scope();
        s.files = vec!["crates/cortex-api/src/meili_lane.rs".to_string()];
        let filter = build_meili_filter(&s, "cortex-cortex-code").expect("clause expected");
        assert!(
            filter.contains("(path_prefixes IN ['crates/cortex-api/src/meili_lane.rs'])"),
            "single-entry path scope must still use IN; got `{filter}`"
        );
    }

    #[test]
    fn build_meili_filter_path_prefixes_composes_with_repo_via_and() {
        // Phase11b spec scenario "Composes with other clauses via
        // AND" — the path-prefix clause is wrapped in parens and
        // joined with sibling clauses by ` AND `.
        let mut s = scope();
        s.repo = Some("Cortex".into());
        s.files = vec!["crates/cortex-api/src/".to_string()];
        let filter = build_meili_filter(&s, "cortex-cortex-code").expect("clause expected");
        assert!(filter.starts_with("repo = 'cortex' AND "), "got `{filter}`");
        assert!(
            filter.contains(" AND (path_prefixes IN ['crates/cortex-api/src/'])"),
            "path_prefixes clause must AND in with parens; got `{filter}`"
        );
    }

    #[test]
    fn build_meili_filter_skips_topics_on_index_without_facet() {
        // `cortex_laws` doesn't carry `topics` as a filterable
        // attribute — emitting the clause would 4xx on Meili.
        let mut s = scope();
        s.topics = vec!["law".to_string()];
        s.since = Some("2026-04-01T00:00:00Z".to_string());
        let filter = build_meili_filter(&s, "cortex_laws");
        // No filterable timestamp + no topics + no repo on
        // cortex_laws → no clauses at all.
        assert!(
            filter.is_none(),
            "laws index has no filterable scope facets; expected None, got {filter:?}"
        );
    }

    #[test]
    fn build_meili_filter_combines_repo_since_topics_with_and() {
        let mut s = scope();
        s.repo = Some("Cortex".into());
        s.since = Some("2026-04-01T00:00:00Z".to_string());
        s.topics = vec!["law".to_string()];
        let filter = build_meili_filter(&s, "cortex_turns").expect("clause expected");
        // Order is repo, since, topics. AND-joined.
        assert!(filter.starts_with("repo = 'cortex' AND "), "got `{filter}`");
        assert!(filter.contains("occurred_at >= '2026-04-01T00:00:00Z'"));
        assert!(filter.contains("topics IN ['law']"));
    }

    #[test]
    fn build_meili_filter_never_emits_starts_with_across_input_matrix() {
        // phase11h §4.4 regression guard — exhaust the input matrix
        // across every dimension `build_meili_filter` reads and assert
        // no combination produces `STARTS WITH`. The previous narrow
        // test pinned a single fixture; this one walks the cross
        // product of files / repo / topics / since over both per-repo
        // and global index names so a future filter-builder edit cannot
        // reintroduce the broken predicate via a path the old test
        // didn't exercise.
        let file_shapes: Vec<Vec<String>> = vec![
            vec![],
            vec!["a/".into()],
            vec!["a/b/c.rs".into()],
            vec!["a/".into(), "b/".into(), "c/d/e.rs".into()],
            vec!["very/deep/nested/path/with/many/segments/".into()],
            vec![
                "with-dash/".into(),
                "with_underscore/".into(),
                "with.dot/".into(),
            ],
        ];
        let repo_shapes = vec![
            None,
            Some("Cortex".to_string()),
            Some("VectorIzer".to_string()),
        ];
        let topic_shapes = vec![
            vec![],
            vec!["code".into()],
            vec!["code".into(), "doc".into(), "decision".into()],
        ];
        let since_shapes = vec![None, Some("2026-04-01T00:00:00Z".to_string())];
        let index_shapes = vec![
            "cortex-cortex-code",
            "cortex-cortex-decisions",
            "cortex-vectorizer-turns",
            "cortex_turns",
            "cortex_decisions",
            "cortex_laws",
        ];
        let mut combos = 0_usize;
        for files in &file_shapes {
            for repo in &repo_shapes {
                for topics in &topic_shapes {
                    for since in &since_shapes {
                        for index in &index_shapes {
                            let mut s = scope();
                            s.files = files.clone();
                            s.repo = repo.clone();
                            s.topics = topics.clone();
                            s.since = since.clone();
                            if let Some(filter) = build_meili_filter(&s, index) {
                                assert!(
                                    !filter.contains("STARTS WITH"),
                                    "STARTS WITH leaked at index={index} files={files:?} repo={repo:?} topics={topics:?} since={since:?} → `{filter}`"
                                );
                                // While we're here, pin the positive
                                // shape: any filter touching files
                                // must use the `path_prefixes IN [..]`
                                // form; any filter touching topics
                                // must use `topics IN [..]`. Both are
                                // the contract Meili accepts.
                                // When the filter mentions
                                // `path_prefixes`, it MUST be in the
                                // `IN [..]` form. Some indexes drop
                                // the clause entirely (global ones
                                // without `path_prefixes` as a
                                // filterable attribute) — that's the
                                // expected "skip" behaviour and not
                                // a regression.
                                if filter.contains("path_prefixes") {
                                    assert!(
                                        filter.contains("path_prefixes IN ["),
                                        "path_prefixes mentioned but not in IN form at index={index} → `{filter}`"
                                    );
                                }
                                if filter.contains(" topics ")
                                    || filter.contains("(topics ")
                                    || filter.starts_with("topics ")
                                {
                                    assert!(
                                        filter.contains("topics IN ["),
                                        "topics mentioned but not in IN form at index={index} → `{filter}`"
                                    );
                                }
                            }
                            combos += 1;
                        }
                    }
                }
            }
        }
        // 6 file shapes × 3 repos × 3 topic shapes × 2 since × 6 indexes = 648.
        assert_eq!(combos, 6 * 3 * 3 * 2 * 6);
    }
}
