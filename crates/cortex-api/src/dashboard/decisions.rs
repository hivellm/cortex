use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{clip, collect_lane_hits, normalize_repo, ts_to_relative, DashboardState};

/// One decision row — shape matches the prototype's `MOCK.decisions`
/// minimum surface so the GUI can render the list / detail view.
/// Most fields are `None` until spec-15 (deep analysis) starts
/// producing structured `decision` envelopes.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionRow {
    /// Decision id (`DEC-NNNN` or ULID).
    pub id: String,
    /// Repo this decision belongs to (the project that owns the
    /// `.rulebook/decisions/*.md` file). Multiple Hive repos ship
    /// their own ADRs — without this field the dashboard can't
    /// tell whether a decision is Cortex's, Nexus's, or another
    /// project's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Title — free text from the payload.
    pub title: String,
    /// `proposed` | `active` | `superseded` | `deprecated`.
    pub status: String,
    /// Author identifier (free text).
    pub author: Option<String>,
    /// Originating analysis id when known.
    pub source_analysis: Option<String>,
    /// Free-text rationale.
    pub rationale: Option<String>,
    /// Topic tags.
    pub tags: Vec<String>,
    /// Cross-references (specs / analyses / prior decisions).
    pub cites: Vec<String>,
    /// Id of the decision this one supersedes, if any.
    pub supersedes: Option<String>,
    /// Reverse pointer of `supersedes` — id of the decision that
    /// superseded this one. Computed by the dashboard from the
    /// captured set; not stamped on the envelope itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Linear supersession chain — oldest at index 0, current at the
    /// tail. Each node carries the decision id, its title, and its
    /// state (`current` or `old`). When the decision has no chain,
    /// this is an empty vec (length 0 or 1 means "no chain to draw"
    /// — the renderer hides the element).
    #[serde(default)]
    pub chain: Vec<DecisionChainNode>,
    /// Number of envelopes captured in the last 7 days that
    /// reference this decision id by `\bDEC-\d{4}-\d{3}\b` regex.
    pub cites_7d: u64,
    /// Relative time label.
    pub occurred_at: String,
}

/// One node of a [`DecisionRow::chain`].
#[derive(Debug, Clone, Serialize)]
pub struct DecisionChainNode {
    /// Decision id at this position.
    pub id: String,
    /// Title clipped at 80 chars.
    pub title: String,
    /// `current` (the row being rendered) or `old` (an ancestor).
    pub state: &'static str,
}

/// Optional body for the `/v1/dashboard/decisions/{id}` detail
/// endpoint — same shape as a list row plus the un-clipped Markdown
/// body.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionDetail {
    /// Spread of [`DecisionRow`].
    #[serde(flatten)]
    pub row: DecisionRow,
    /// Full envelope body.
    pub body_markdown: String,
}

/// Build the full set of [`DecisionRow`] from a snapshot of the
/// lane. Walking the supersedes pointer is local to this helper so
/// both `/decisions` and `/decisions/{id}` agree on chain layout.
fn build_decision_rows(hits: &[crate::lanes::LaneHit]) -> Vec<DecisionRow> {
    use std::collections::HashMap;

    // First pass: turn each `kind=decision` envelope into a (raw row,
    // raw body) pair so the chain pass below has a complete index
    // before walking pointers.
    let mut rows: Vec<DecisionRow> = Vec::new();
    let mut bodies: HashMap<String, String> = HashMap::new();
    for h in hits
        .iter()
        .filter(|h| h.symbol.as_deref() == Some("decision"))
    {
        let id = h
            .doc_id
            .strip_prefix("archive|")
            .unwrap_or(&h.doc_id)
            .to_string();
        // The meili_loader stamps cleaner fields on extras (parsed
        // from the JSON-encoded envelope body the fulltext worker
        // stores). Fall back to the legacy text-scraping path so
        // archive-fed envelopes — Turn / ToolCall / AgentCall paths
        // through detect_status() — still parse identically.
        let extras_title = h.extras.get("title").and_then(|v| v.as_str());
        let extras_status = h.extras.get("status").and_then(|v| v.as_str());
        let extras_supersedes = h
            .extras
            .get("supersedes")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let extras_body = h.extras.get("body_markdown").and_then(|v| v.as_str());
        let title_clean = match extras_title {
            Some(t) if !t.is_empty() => clip(t, 120),
            _ => clip(h.text.lines().next().unwrap_or(""), 120),
        };
        let rationale = match extras_body {
            Some(b) if !b.is_empty() => Some(clip(b, 600)),
            _ => Some(clip(&h.text, 600)),
        };
        let status = match extras_status {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => detect_status(&h.text),
        };
        let supersedes = extras_supersedes.or_else(|| detect_supersedes(&h.text));

        bodies.insert(id.clone(), extras_body.unwrap_or(&h.text).to_string());
        rows.push(DecisionRow {
            id,
            repo: h.repo.clone(),
            title: title_clean,
            status,
            author: None,
            source_analysis: None,
            rationale,
            tags: Vec::new(),
            cites: Vec::new(),
            supersedes,
            superseded_by: None,
            chain: Vec::new(),
            cites_7d: 0,
            occurred_at: ts_to_relative(h.ts),
        });
    }

    // Reverse-pointer pass: for every row whose `supersedes` is set,
    // stamp the matching ancestor's `superseded_by`. Two rows that
    // both supersede the same id is a writer bug — last write wins.
    let mut superseded_by_index: HashMap<String, String> = HashMap::new();
    for row in &rows {
        if let Some(prev) = row.supersedes.as_ref() {
            superseded_by_index.insert(prev.clone(), row.id.clone());
        }
    }
    for row in rows.iter_mut() {
        if let Some(next) = superseded_by_index.get(&row.id) {
            row.superseded_by = Some(next.clone());
        }
    }

    // Status reflects the reverse-pointer index: a row with a
    // `superseded_by` is `superseded`, regardless of what the
    // payload claimed.
    for row in rows.iter_mut() {
        if row.superseded_by.is_some() && row.status != "superseded" {
            row.status = "superseded".to_string();
        }
    }

    // Chain pass: walk `supersedes` backwards from the head toward
    // the oldest decision in the chain, then reverse so the oldest
    // comes first. A chain with a single element is reported as an
    // empty vec so the renderer hides the supersede element. Indexes
    // are owned String→String so the chain build doesn't borrow from
    // `rows` while we mutate it.
    let title_index: HashMap<String, String> = rows
        .iter()
        .map(|r| (r.id.clone(), r.title.clone()))
        .collect();
    let supersedes_index: HashMap<String, String> = rows
        .iter()
        .filter_map(|r| r.supersedes.clone().map(|prev| (r.id.clone(), prev)))
        .collect();
    #[allow(clippy::needless_range_loop)]
    for i in 0..rows.len() {
        let head_id = rows[i].id.clone();
        let mut nodes: Vec<DecisionChainNode> = Vec::new();
        // Tail (current).
        nodes.push(DecisionChainNode {
            id: head_id.clone(),
            title: clip(
                title_index.get(&head_id).map(String::as_str).unwrap_or(""),
                80,
            ),
            state: "current",
        });
        // Walk backward through `supersedes` until either no parent
        // is set or a cycle is hit. The `seen` guard caps the walk.
        let mut cursor = head_id.clone();
        let mut seen: std::collections::HashSet<String> = Default::default();
        seen.insert(cursor.clone());
        while let Some(prev) = supersedes_index.get(&cursor).cloned() {
            if !seen.insert(prev.clone()) {
                break;
            }
            nodes.push(DecisionChainNode {
                id: prev.clone(),
                title: clip(title_index.get(&prev).map(String::as_str).unwrap_or(""), 80),
                state: "old",
            });
            cursor = prev;
            if seen.len() > 32 {
                break;
            }
        }
        if nodes.len() <= 1 {
            // Single-element chains are the "no chain" case.
            continue;
        }
        nodes.reverse();
        rows[i].chain = nodes;
    }

    // Cites pass: count distinct lane hits within the last 7 days
    // whose body contains the decision id, excluding the decision's
    // own envelope. A regex match on the id is the cheapest signal
    // we have — the spec-12 derivation pipeline will refine this.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff_ms = now_ms - 7 * 86_400_000;
    for row in rows.iter_mut() {
        let needle = row.id.as_str();
        let mut count: u64 = 0;
        for h in hits {
            if h.ts <= 0 || h.ts < cutoff_ms {
                continue;
            }
            // Skip the decision's own envelope (the doc_id matches
            // `archive|<id>`).
            if h.doc_id
                .strip_prefix("archive|")
                .map(|s| s == needle)
                .unwrap_or(false)
            {
                continue;
            }
            if h.text.contains(needle) {
                count += 1;
            }
        }
        row.cites_7d = count;
    }

    let _ = bodies; // keep `bodies` available for the detail handler
    rows
}

/// Detect a `status:` line in a decision payload's first 32 lines.
/// Falls back to `active` when no marker is found.
fn detect_status(text: &str) -> String {
    for line in text.lines().take(32) {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("status:") {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    "active".to_string()
}

/// Detect a `supersedes: DEC-...` line. Returns the matched id.
fn detect_supersedes(text: &str) -> Option<String> {
    let re_target = "supersedes:";
    for line in text.lines().take(64) {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix(re_target) {
            let token = rest.split_whitespace().next().unwrap_or("");
            if !token.is_empty() {
                // The lower-cased prefix gave us the offset; re-derive
                // the cased token from the original line so the id
                // keeps its `DEC-` casing.
                let cased = line
                    .trim()
                    .strip_prefix(|c: char| c.is_alphabetic() || c == ':')
                    .unwrap_or(line.trim())
                    .trim_start_matches(':')
                    .trim();
                let cased_token = cased.split_whitespace().next().unwrap_or(token);
                return Some(cased_token.to_string());
            }
        }
    }
    None
}

/// Query params for `/v1/dashboard/decisions`. `repo` filters to a
/// single project (Cortex / Nexus / Vectorizer / …) so the GUI can
/// render a per-repo decisions tab. `repos` (repeatable) supports
/// multi-select. Empty → all repos.
#[derive(Debug, Default, Deserialize)]
pub struct DecisionsQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Multi-repo filter — `?repos=Cortex&repos=Nexus`.
    #[serde(default)]
    pub repos: Vec<String>,
}

pub(super) async fn decisions(
    State(state): State<DashboardState>,
    Query(params): Query<DecisionsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let mut rows = build_decision_rows(&hits);
    // Apply repo filter: union of single + multi forms. Empty union
    // means "all repos".
    let mut allow: std::collections::HashSet<String> = params
        .repos
        .into_iter()
        .map(|r| normalize_repo(&r))
        .collect();
    if let Some(r) = params.repo.filter(|s| !s.is_empty()) {
        allow.insert(normalize_repo(&r));
    }
    if !allow.is_empty() {
        rows.retain(|r| {
            r.repo
                .as_deref()
                .map(|repo| allow.contains(&normalize_repo(repo)))
                .unwrap_or(false)
        });
    }
    (StatusCode::OK, Json(rows)).into_response()
}

pub(super) async fn decision_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows = build_decision_rows(&hits);
    let row = match rows.into_iter().find(|r| r.id == id) {
        Some(r) => r,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "reason": "decision_not_found" })),
            )
                .into_response();
        }
    };
    let body_markdown = hits
        .iter()
        .find(|h| {
            h.symbol.as_deref() == Some("decision")
                && h.doc_id.strip_prefix("archive|").unwrap_or(&h.doc_id) == row.id
        })
        .map(|h| {
            // Prefer the parsed body the meili_loader stamped onto
            // extras over the lane's `text` field, since `text` may
            // be the JSON-encoded raw envelope for archive-fed
            // decisions. The meili-fed path always has the parsed
            // form available.
            h.extras
                .get("body_markdown")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| h.text.clone())
        })
        .unwrap_or_default();
    let detail = DecisionDetail { row, body_markdown };
    (StatusCode::OK, Json(detail)).into_response()
}
