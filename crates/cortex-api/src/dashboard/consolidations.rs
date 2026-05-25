//! `/v1/dashboard/consolidations` — surfaces phase11j Consolidation
//! envelopes (session / topic / decision_trace digests produced by
//! `cortex-consolidator` and `cortex-ops tool-call-digest`).
//!
//! Reads the lane's `cortex-meili-consolidations` index that
//! [`crate::meili_loader`] seeds at boot + every Meili refresh. Any
//! envelope whose [`crate::dashboard::symbol_to_kind`] resolves to
//! `"consolidation"` shows up here.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{clip, collect_lane_hits, normalize_repo, ts_to_relative, DashboardState};

/// One row in the `/v1/dashboard/consolidations` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationRow {
    /// Envelope ULID — opaque identifier, also the route id for
    /// `/v1/dashboard/consolidations/{id}`.
    pub id: String,
    /// Stable consolidator key (`cons-ses-…` / `cons-top-…` / etc.)
    /// when present; some bootstrap envelopes omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consolidation_id: Option<String>,
    /// Display title — first non-empty of `title` / first body line.
    pub title: String,
    /// `session` | `topic` | `decision_trace`. Empty when the
    /// envelope predates the schema bump.
    pub grain: String,
    /// `shallow` | `deep`. Empty when the envelope predates the bump.
    pub depth: String,
    /// Model id that produced the summary (e.g. `claude-haiku-4-5`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Number of source envelopes folded into this consolidation.
    /// `0` when the envelope omits the field.
    #[serde(default)]
    pub source_event_count: u64,
    /// Repo this consolidation is anchored to. `None` for cross-repo
    /// rollups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Free-text topics list — controlled vocab from the classifier.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Body excerpt clipped at 800 chars; full markdown ships under
    /// `/v1/dashboard/consolidations/{id}` `body_markdown`.
    pub excerpt: String,
    /// Relative time label.
    pub occurred_at: String,
}

/// Detail body for `/v1/dashboard/consolidations/{id}` — the row plus
/// the unclipped markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationDetail {
    /// Spread of [`ConsolidationRow`].
    #[serde(flatten)]
    pub row: ConsolidationRow,
    /// Full envelope body (markdown).
    pub body_markdown: String,
}

/// Filter echo carried back on every [`ConsolidationReportView`].
/// ADR-014 / phase13f §2.2 — handlers MUST surface what they actually
/// filtered against so the GUI never re-derives the active filter
/// from URL state alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationFilter {
    /// Repo allow-list after normalisation. Empty = no repo filter.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Grain filter (`session` | `topic` | `decision_trace`). `None`
    /// when no grain filter was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
}

/// Dashboard projection of `/v1/dashboard/consolidations`. The handler
/// renders these without doing any additional state inference — see
/// ADR-014 / phase13f §2.2. `total` is carried explicitly so handlers
/// and GUI never need to recompute from `rows.len()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationReportView {
    /// Rows that survived filtering, sorted newest-first.
    pub rows: Vec<ConsolidationRow>,
    /// Number of rows in `rows`. Equal to `rows.len()` but carried
    /// on the view so consumers cannot drift.
    pub total: u64,
    /// Echo of the filter that produced this list.
    pub filter: ConsolidationFilter,
}

impl ConsolidationReportView {
    /// Build a view from a row collection + the active filter. The
    /// caller owns the sort order — this constructor does not
    /// re-sort.
    pub fn from_rows(rows: Vec<ConsolidationRow>, filter: ConsolidationFilter) -> Self {
        let total = rows.len() as u64;
        Self {
            rows,
            total,
            filter,
        }
    }
}

impl ConsolidationFilter {
    /// Build a filter from the wire query params. Multi-value
    /// `?repos=` and singular `?repo=` collapse into one normalised
    /// repo allow-list; empty grain strings collapse to `None` so
    /// `?grain=` does not match every row.
    pub fn from_query(params: ConsolidationsQuery) -> Self {
        let mut repos: Vec<String> = params
            .repos
            .into_iter()
            .map(|r| normalize_repo(&r))
            .collect();
        if let Some(r) = params.repo.filter(|s| !s.is_empty()) {
            let n = normalize_repo(&r);
            if !repos.contains(&n) {
                repos.push(n);
            }
        }
        Self {
            repos,
            grain: params.grain.filter(|s| !s.is_empty()),
        }
    }
}

/// Source the consolidations handler reads from. Defaults to the
/// in-process Meili lane (`MeiliConsolidationSource`); tests
/// substitute a fixture implementation so the projection logic is
/// exercised without Meili. The live handler reads the lane
/// directly today; the trait is the typed surface for future
/// dependency-injection of alternative backends.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait ConsolidationReportSource: Send + Sync {
    /// Return rows that match `filter`, sorted newest-first.
    async fn list(&self, filter: &ConsolidationFilter) -> anyhow::Result<Vec<ConsolidationRow>>;
}

/// Query params — mirrors [`super::DecisionsQuery`] / [`super::AnalysesQuery`]
/// so the GUI's filter dropdowns work the same way.
#[derive(Debug, Default, Deserialize)]
pub struct ConsolidationsQuery {
    /// Single-repo filter — `?repo=Cortex`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Multi-repo filter — `?repos=Cortex&repos=Nexus`.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Single-grain filter — `?grain=session`.
    #[serde(default)]
    pub grain: Option<String>,
}

fn build_row(h: &crate::lanes::LaneHit) -> (ConsolidationRow, String) {
    let title = h
        .extras
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| clip(h.text.lines().next().unwrap_or(""), 120));
    let body_markdown = h
        .extras
        .get("body_markdown")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| h.text.clone());
    let grain = h
        .extras
        .get("grain")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let depth = h
        .extras
        .get("depth")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model = h
        .extras
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let consolidation_id = h
        .extras
        .get("consolidation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let source_event_count = h
        .extras
        .get("source_event_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let topics: Vec<String> = h
        .extras
        .get("topics")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let row = ConsolidationRow {
        id: h
            .doc_id
            .strip_prefix("archive|")
            .unwrap_or(&h.doc_id)
            .to_string(),
        consolidation_id,
        title,
        grain,
        depth,
        model,
        source_event_count,
        repo: h.repo.clone(),
        topics,
        excerpt: clip(&body_markdown, 800),
        occurred_at: ts_to_relative(h.ts),
    };
    (row, body_markdown)
}

pub(super) async fn consolidations(
    State(state): State<DashboardState>,
    Query(params): Query<ConsolidationsQuery>,
) -> Response {
    let filter = ConsolidationFilter::from_query(params);
    let rows = collect_consolidation_rows(&state, &filter);
    Json(ConsolidationReportView::from_rows(rows, filter)).into_response()
}

/// Pure projection of lane hits → consolidation rows that match
/// `filter`, sorted newest-first. Extracted from
/// [`consolidations`] so the handler stays a thin
/// filter+wrap+respond shell (ADR-014 / phase13f §3.2).
fn collect_consolidation_rows(
    state: &DashboardState,
    filter: &ConsolidationFilter,
) -> Vec<ConsolidationRow> {
    let allow: std::collections::HashSet<&str> =
        filter.repos.iter().map(String::as_str).collect();
    let mut rows: Vec<ConsolidationRow> = collect_lane_hits(&state.lane)
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("consolidation"))
        .filter(|h| {
            if allow.is_empty() {
                return true;
            }
            match h.repo.as_deref() {
                Some(r) => allow.contains(normalize_repo(r).as_str()),
                None => false,
            }
        })
        .map(|h| build_row(&h).0)
        .filter(|r| match filter.grain.as_deref() {
            Some(g) => r.grain == g,
            None => true,
        })
        .collect();
    // Newest first by ts proxy — ts is 0 for many envelopes so fall
    // back to id sort to keep the order stable across reloads.
    rows.sort_by(|a, b| b.id.cmp(&a.id));
    rows
}

pub(super) async fn consolidation_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let want = id.trim();
    for h in &hits {
        if h.symbol.as_deref() != Some("consolidation") {
            continue;
        }
        let row_id = h.doc_id.strip_prefix("archive|").unwrap_or(&h.doc_id);
        if row_id == want {
            let (row, body_markdown) = build_row(h);
            return Json(ConsolidationDetail { row, body_markdown }).into_response();
        }
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "consolidation not found", "id": id })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(id: &str, grain: &str) -> ConsolidationRow {
        ConsolidationRow {
            id: id.to_string(),
            consolidation_id: Some(format!("cons-{id}")),
            title: format!("{grain} digest {id}"),
            grain: grain.to_string(),
            depth: "shallow".to_string(),
            model: Some("claude-haiku-4-5".to_string()),
            source_event_count: 3,
            repo: Some("Cortex".to_string()),
            topics: vec!["retention".to_string(), "phase13f".to_string()],
            excerpt: "first 800 chars of body".to_string(),
            occurred_at: "2 minutes ago".to_string(),
        }
    }

    #[test]
    fn report_view_carries_total_from_row_count() {
        let rows = vec![sample_row("01A", "session"), sample_row("01B", "topic")];
        let filter = ConsolidationFilter {
            repos: vec!["Cortex".into()],
            grain: None,
        };
        let view = ConsolidationReportView::from_rows(rows.clone(), filter.clone());
        assert_eq!(view.total, 2);
        assert_eq!(view.rows, rows);
        assert_eq!(view.filter, filter);
    }

    #[test]
    fn report_view_round_trips_via_serde() {
        let rows = vec![sample_row("01A", "session")];
        let filter = ConsolidationFilter {
            repos: vec!["Cortex".into()],
            grain: Some("session".into()),
        };
        let view = ConsolidationReportView::from_rows(rows, filter);
        let j = serde_json::to_string(&view).unwrap();
        let p: ConsolidationReportView = serde_json::from_str(&j).unwrap();
        assert_eq!(p, view);
    }

    #[test]
    fn filter_serialises_without_none_grain_field() {
        let filter = ConsolidationFilter {
            repos: vec!["Cortex".into()],
            grain: None,
        };
        let j = serde_json::to_string(&filter).unwrap();
        // `grain: None` MUST be skipped; clients distinguish
        // "no filter" from "explicit empty string".
        assert!(!j.contains("grain"), "got: {j}");
    }

    #[test]
    fn row_round_trips_via_serde_with_optional_fields_absent() {
        let row = ConsolidationRow {
            id: "01Z".into(),
            consolidation_id: None,
            title: "rollup".into(),
            grain: String::new(),
            depth: String::new(),
            model: None,
            source_event_count: 0,
            repo: None,
            topics: Vec::new(),
            excerpt: String::new(),
            occurred_at: String::new(),
        };
        let j = serde_json::to_string(&row).unwrap();
        let p: ConsolidationRow = serde_json::from_str(&j).unwrap();
        assert_eq!(p, row);
    }

    /// Fixture source used by tests to prove the trait shape is
    /// object-safe + drives the projection without Meili.
    struct StubSource(Vec<ConsolidationRow>);

    #[async_trait::async_trait]
    impl ConsolidationReportSource for StubSource {
        async fn list(
            &self,
            filter: &ConsolidationFilter,
        ) -> anyhow::Result<Vec<ConsolidationRow>> {
            let allow: std::collections::HashSet<String> = filter.repos.iter().cloned().collect();
            let g = filter.grain.clone();
            let out: Vec<ConsolidationRow> = self
                .0
                .iter()
                .filter(|r| match r.repo.as_deref() {
                    Some(repo) if !allow.is_empty() => allow.contains(repo),
                    _ => allow.is_empty(),
                })
                .filter(|r| match g.as_deref() {
                    Some(g) => r.grain == g,
                    None => true,
                })
                .cloned()
                .collect();
            Ok(out)
        }
    }

    #[tokio::test]
    async fn source_trait_is_object_safe_and_projects_filter() {
        let src: Box<dyn ConsolidationReportSource> = Box::new(StubSource(vec![
            sample_row("01A", "session"),
            sample_row("01B", "topic"),
        ]));
        let filter = ConsolidationFilter {
            repos: vec!["Cortex".into()],
            grain: Some("topic".into()),
        };
        let rows = src.list(&filter).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].grain, "topic");
    }
}
