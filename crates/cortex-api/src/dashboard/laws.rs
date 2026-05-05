use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::{collect_lane_hits, DashboardState};

/// One law row — shape mirrors the prototype's `MOCK.laws`. Real
/// data lands when spec-13 ships the law catalogue. Until then this
/// list is empty by design.
#[derive(Debug, Clone, Serialize)]
pub struct LawRow {
    /// `LAW-NNN`.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// `info` | `notable` | `critical`.
    pub severity: String,
    /// Whether the law actively blocks tool calls.
    pub blocked: bool,
    /// Free-text scope tags.
    pub scope: String,
    /// Number of contexts where the law applies.
    pub applies: u64,
    /// Violations recorded in the last 7 days.
    pub violations_7d: u64,
    /// Per-1000 violations rate.
    pub rate: f64,
    /// Detector identifier.
    pub detector: String,
    /// Suggested remediation.
    pub remediation: String,
}

pub(super) async fn laws(State(state): State<DashboardState>) -> Response {
    // Spec-13 owns the canonical law catalogue. Until it lands, the
    // `.claude/rules/*.md` files that the bootstrap walker imports as
    // `kind=law_violation` envelopes (one per rule file) double as
    // the catalogue: each rule's `law_id` is its identifier, and the
    // body Markdown is its definition. Dedupe the envelope stream by
    // `law_id` and return one row per unique rule.
    //
    // When spec-13 ships an explicit catalogue source (envelope kind
    // or static manifest), this handler stays — it just consumes the
    // richer stream instead of fanning out from the violations
    // bucket.
    let hits = collect_lane_hits(&state.lane);
    let mut by_id: std::collections::BTreeMap<String, LawRow> = std::collections::BTreeMap::new();
    for h in hits
        .iter()
        .filter(|h| h.symbol.as_deref() == Some("law_violation"))
    {
        let law_id = match h.extras.get("law_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let title = h
            .extras
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&law_id)
            .to_string();
        let severity = h.severity.clone().unwrap_or_else(|| "info".to_string());
        let blocked = severity == "critical";
        let scope = h.path.clone().unwrap_or_else(|| "all".to_string());
        let row = by_id.entry(law_id.clone()).or_insert_with(|| LawRow {
            id: law_id,
            title,
            severity,
            blocked,
            scope,
            applies: 0,
            violations_7d: 0,
            rate: 0.0,
            detector: String::new(),
            remediation: String::new(),
        });
        row.violations_7d = row.violations_7d.saturating_add(1);
        row.applies = row.applies.saturating_add(1);
    }
    let rows: Vec<LawRow> = by_id.into_values().collect();
    (StatusCode::OK, Json(rows)).into_response()
}
