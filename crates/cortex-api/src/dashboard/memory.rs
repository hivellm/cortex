use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{
    clip, collect_lane_hits, normalize_repo, resolve_kind_filter, session_id_of, symbol_to_kind,
    title_from_hit, ts_to_relative, DashboardState, CANONICAL_KINDS,
};

/// Query params for `/v1/dashboard/memory`.
#[derive(Debug, Deserialize)]
pub struct MemoryQuery {
    /// Free-text query. Empty / missing means "list everything".
    #[serde(default)]
    pub q: Option<String>,
    /// Cap the result count. Defaults to 50, max 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to one session.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Restrict to one or more repos. Each `repo=<name>` query param
    /// is appended; the filter passes when the hit matches ANY of
    /// the listed repos.
    #[serde(default)]
    pub repo: Vec<String>,
    /// phase10f — restrict to one or more canonical kinds. Repeat
    /// the param (`?kind=decision&kind=analysis`) to OR multiple
    /// kinds. Empty list means "all kinds".
    #[serde(default)]
    pub kind: Vec<String>,
    /// phase10f — alias for [`Self::kind`]. The pre-phase10f
    /// dashboard contract documented `?facets=` but the handler
    /// never read it; treating it as a synonym keeps existing
    /// callers working while the GUI migrates to `?kind=`.
    #[serde(default)]
    pub facets: Vec<String>,
}

/// One memory row — shape matches the prototype's `MOCK.memories`.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryEntry {
    /// Memory title — derived from the hit's first line.
    pub title: String,
    /// Body excerpt clipped at ~320 chars.
    pub excerpt: String,
    /// Canonical kind.
    pub kind: String,
    /// Repo identifier.
    pub repo: Option<String>,
    /// Topics — empty for now (the lane doesn't carry topic
    /// metadata yet; the classifier worker fills this when spec 05
    /// lands).
    pub topics: Vec<String>,
    /// Relative wall-clock label (`"2 minutes ago"` / `"3 days ago"`)
    /// — empty when no timestamp was preserved.
    pub updated: String,
}

pub(super) async fn memory(
    State(state): State<DashboardState>,
    Query(params): Query<MemoryQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let q = params.q.unwrap_or_default();

    // phase10f — resolve the kind filter UP-FRONT so an unknown
    // value short-circuits to a structured 400 before we touch
    // the lane snapshot. The audit caught the pre-phase10f
    // handler ignoring the param silently and surfacing
    // `tool_call`/`turn` regardless of what the caller asked
    // for.
    let kind_allow = match resolve_kind_filter(&params.kind, &params.facets) {
        Ok(allow) => allow,
        Err(received) => {
            let body = serde_json::json!({
                "error": "unknown_kind",
                "received": received,
                "canonical": CANONICAL_KINDS,
            });
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };

    let mut hits = collect_lane_hits(&state.lane);
    if let Some(sid) = params.session_id.as_deref().filter(|s| !s.is_empty()) {
        hits.retain(|h| session_id_of(h) == Some(sid));
    }
    if !params.repo.is_empty() {
        let allow: std::collections::HashSet<String> =
            params.repo.iter().map(|r| normalize_repo(r)).collect();
        hits.retain(|h| {
            h.repo
                .as_deref()
                .map(|r| allow.contains(&normalize_repo(r)))
                .unwrap_or(false)
        });
    }
    // phase10f §1.2 — apply the kind filter BEFORE pagination so
    // `limit=80` returns 80 rows of the requested kinds, not 80
    // mixed-kind rows from which the requested kinds are then
    // sliced down to a handful.
    if let Some(allow) = kind_allow.as_ref() {
        hits.retain(|h| allow.contains(symbol_to_kind(h.symbol.as_deref())));
    }
    if !q.trim().is_empty() {
        let needle = q.to_ascii_lowercase();
        hits.retain(|h| h.text.to_ascii_lowercase().contains(&needle));
    }
    hits.sort_by(|a, b| b.ts.cmp(&a.ts));
    hits.truncate(limit);

    let entries: Vec<MemoryEntry> = hits
        .into_iter()
        .map(|h| MemoryEntry {
            title: title_from_hit(&h),
            excerpt: clip(&h.text, 320),
            kind: symbol_to_kind(h.symbol.as_deref()).to_string(),
            repo: h.repo.clone(),
            topics: Vec::new(),
            updated: ts_to_relative(h.ts),
        })
        .collect();
    (StatusCode::OK, Json(entries)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum_extra::extract::Query;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn make_state(hits: Vec<crate::lanes::LaneHit>) -> super::super::DashboardState {
        let lane = crate::lanes::MemoryKeywordLane::new();
        lane.seed("cortex-code", hits);
        super::super::DashboardState {
            lane: Arc::new(lane),
            nexus: None,
            analyzer: Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: Arc::new(crate::tasks_loader::MultiTaskLoader::new(vec![
                crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from(
                    "__tests_no_rulebook__",
                )),
            ])),
            metadata: None,
            loader_metrics: Arc::new(crate::LoaderMetrics::new()),
            events_bus: crate::dashboard_watcher::DashboardEventBus::new(),
        }
    }

    fn turn_hit(text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        let mut extras = BTreeMap::new();
        extras.insert(
            "session_id".to_string(),
            serde_json::Value::String("session-default".to_string()),
        );
        crate::lanes::LaneHit {
            doc_id: format!("archive|{}", text),
            text: text.to_string(),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some("turn".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras,
        }
    }

    fn tool_call_hit(tool: &str, body: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        crate::lanes::LaneHit {
            doc_id: format!("archive|{tool}-{ts}"),
            text: format!("[{tool}] {body}"),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some(format!("tool_call:{tool}")),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    fn decision_hit(text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        crate::lanes::LaneHit {
            doc_id: format!("archive|dec-{ts}"),
            text: text.to_string(),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some("decision".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    fn analysis_hit(text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        crate::lanes::LaneHit {
            doc_id: format!("archive|an-{ts}"),
            text: text.to_string(),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some("analysis".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    fn knowledge_hit(text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        crate::lanes::LaneHit {
            doc_id: format!("archive|k-{ts}"),
            text: text.to_string(),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some("knowledge".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn memory_filter_matches_q_substring_case_insensitive() {
        let state = make_state(vec![
            turn_hit("HNSW recall floor benchmark", "Vectorizer", 100),
            turn_hit("unrelated thoughts", "Cortex", 200),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: Some("hnsw".to_string()),
                limit: None,
                session_id: None,
                repo: Vec::new(),
                kind: Vec::new(),
                facets: Vec::new(),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0]["title"].as_str().unwrap().contains("HNSW"));
    }

    #[tokio::test]
    async fn memory_with_empty_query_returns_everything_newest_first() {
        let state = make_state(vec![
            turn_hit("a", "Cortex", 100),
            turn_hit("b", "Cortex", 200),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Vec::new(),
                kind: Vec::new(),
                facets: Vec::new(),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["title"], "b");
    }

    #[tokio::test]
    async fn memory_kind_filter_returns_only_decisions_when_decision_requested() {
        let state = make_state(vec![
            turn_hit("note", "Cortex", 100),
            tool_call_hit("Edit", "x", "Cortex", 200),
            decision_hit("DEC-0042", "Cortex", 150),
            analysis_hit("audit-a", "Cortex", 175),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Vec::new(),
                kind: vec!["decision".to_string()],
                facets: Vec::new(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["kind"], "decision");
        assert_eq!(parsed[0]["excerpt"], "DEC-0042");
    }

    #[tokio::test]
    async fn memory_kind_filter_ors_multiple_kinds() {
        let state = make_state(vec![
            turn_hit("note", "Cortex", 100),
            decision_hit("DEC-0042", "Cortex", 150),
            analysis_hit("audit-a", "Cortex", 175),
            knowledge_hit("Pattern: RRF", "Cortex", 180),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Vec::new(),
                kind: vec!["decision".to_string(), "analysis".to_string()],
                facets: Vec::new(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 2);
        let kinds: std::collections::HashSet<String> = parsed
            .iter()
            .map(|p| p["kind"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(kinds.contains("decision"));
        assert!(kinds.contains("analysis"));
        assert!(!kinds.contains("turn"));
        assert!(!kinds.contains("knowledge"));
    }

    #[tokio::test]
    async fn memory_facets_param_is_alias_for_kind() {
        let state = make_state(vec![
            turn_hit("note", "Cortex", 100),
            knowledge_hit("Pattern: RRF", "Cortex", 200),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Vec::new(),
                kind: Vec::new(),
                facets: vec!["knowledge".to_string()],
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["kind"], "knowledge");
    }

    #[tokio::test]
    async fn memory_unknown_kind_returns_structured_400() {
        let state = make_state(vec![turn_hit("note", "Cortex", 100)]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Vec::new(),
                kind: vec!["foo".to_string()],
                facets: Vec::new(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["error"], "unknown_kind");
        assert_eq!(parsed["received"], "foo");
        let canonical = parsed["canonical"].as_array().unwrap();
        assert!(canonical.iter().any(|v| v == "decision"));
        assert!(canonical.iter().any(|v| v == "knowledge"));
    }

    #[tokio::test]
    async fn memory_kind_filter_applies_before_pagination() {
        let state = make_state(vec![
            turn_hit("note 1", "Cortex", 1000),
            turn_hit("note 2", "Cortex", 990),
            turn_hit("note 3", "Cortex", 980),
            decision_hit("DEC-A", "Cortex", 200),
            decision_hit("DEC-B", "Cortex", 150),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: Some(2),
                session_id: None,
                repo: Vec::new(),
                kind: vec!["decision".to_string()],
                facets: Vec::new(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 2);
        for row in &parsed {
            assert_eq!(row["kind"], "decision");
        }
    }

    #[tokio::test]
    async fn memory_filter_by_repo_and_kind_combine() {
        let state = make_state(vec![
            turn_hit("note A", "Cortex", 100),
            tool_call_hit("Edit", "x", "Cortex", 200),
            turn_hit("note V", "Vectorizer", 150),
        ]);
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: vec!["Cortex".to_string()],
                kind: vec!["turn".to_string()],
                facets: Vec::new(),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["title"], "note A");
    }
}
