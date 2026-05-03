//! Deterministic Markdown bundle formatter. Spec 12 §Output +
//! §Deterministic formatting.
//!
//! Pure-Rust string assembly — no template engine. Sections are
//! emitted in a fixed order (laws → decisions → similar turns →
//! snippets → graph neighbours), with a leading comment that carries
//! the `query_id` for audit correlation and a trailing
//! `<!-- end cortex -->` marker.
//!
//! Sections with zero entries are dropped entirely. The empty-bundle
//! contract (spec 12 §Decisions §4) returns an empty string when no
//! section produced anything.

use std::fmt::Write;

use chrono::{TimeZone, Utc};
use cortex_api::QueryResponse;

/// Per-section caps used by the budget clipper. Spec 12 §Budget-aware
/// section caps.
pub mod section_caps {
    /// Max law entries.
    pub const LAWS: usize = 10;
    /// Max decision entries.
    pub const DECISIONS: usize = 5;
    /// Max similar-turn entries.
    pub const SIMILAR_TURNS: usize = 5;
    /// Max snippet entries.
    pub const SNIPPETS: usize = 5;
    /// Max graph-neighbour entries (off by default — emitted only
    /// when the budget can absorb them).
    pub const GRAPH_NEIGHBORS: usize = 0;
    /// Phase11i §4.1 — max past-session entries. Spec calls for
    /// "top-3 by centroid similarity"; the renderer takes the
    /// first N entries the orchestrator surfaces (already ranked).
    pub const PAST_SESSIONS: usize = 3;
    /// Phase11i §4.1 — max bytes for the clipped first-prompt
    /// preview. Spec calls for an 80-char clip; we cap at the byte
    /// equivalent of 80 ASCII chars to keep the section compact.
    pub const PAST_SESSION_PROMPT_BYTES: usize = 80;
    /// Phase11j §4.2 — max consolidation entries the
    /// "Consolidated context" section renders. Spec calls for top-3
    /// by similarity; the consolidations replace the past-sessions
    /// section when ≥ 1 hit lands.
    pub const CONSOLIDATIONS: usize = 3;
    /// Max bytes per snippet text.
    pub const SNIPPET_BYTES: usize = 1024;
    /// Max bytes per decision body.
    pub const DECISION_BYTES: usize = 512;
}

/// Trim mode applied by the budget clipper to snippets. Spec 12
/// §Budget-aware section caps step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetTrim {
    /// Full snippet text (subject to `SNIPPET_BYTES`).
    Full,
    /// Only the `why` blurb plus the first 3 lines of `text`.
    SlimWhyPlusThree,
}

/// Sectioning toggles handed to the formatter. Spec 12 budget clipper
/// flips these as it climbs the trim ladder.
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Maximum number of laws to render.
    pub laws_cap: usize,
    /// Maximum number of decisions to render.
    pub decisions_cap: usize,
    /// Maximum number of similar turns to render.
    pub similar_turns_cap: usize,
    /// Maximum number of past sessions to render (phase11i §4.1).
    pub past_sessions_cap: usize,
    /// Per-prompt byte cap for the past-sessions section
    /// (phase11i §4.1).
    pub past_session_prompt_byte_cap: usize,
    /// Maximum number of consolidations to render (phase11j §4.2).
    pub consolidations_cap: usize,
    /// Maximum number of snippets to render.
    pub snippets_cap: usize,
    /// Maximum number of graph neighbours to render.
    pub graph_cap: usize,
    /// How aggressively to trim each snippet's text.
    pub snippet_trim: SnippetTrim,
    /// Per-decision byte cap.
    pub decision_byte_cap: usize,
    /// Per-snippet byte cap.
    pub snippet_byte_cap: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            laws_cap: section_caps::LAWS,
            decisions_cap: section_caps::DECISIONS,
            similar_turns_cap: section_caps::SIMILAR_TURNS,
            past_sessions_cap: section_caps::PAST_SESSIONS,
            past_session_prompt_byte_cap: section_caps::PAST_SESSION_PROMPT_BYTES,
            consolidations_cap: section_caps::CONSOLIDATIONS,
            snippets_cap: section_caps::SNIPPETS,
            graph_cap: section_caps::GRAPH_NEIGHBORS,
            snippet_trim: SnippetTrim::Full,
            decision_byte_cap: section_caps::DECISION_BYTES,
            snippet_byte_cap: section_caps::SNIPPET_BYTES,
        }
    }
}

/// Build the bundle. Returns an empty string when every section
/// would be empty (spec 12 Decision 4).
pub fn format_bundle(intent: &str, response: &QueryResponse, opts: &FormatOptions) -> String {
    let laws_count = response.laws_active.len().min(opts.laws_cap);
    let decisions_count = response.results.decisions.len().min(opts.decisions_cap);
    let turns_count = response
        .results
        .similar_turns
        .len()
        .min(opts.similar_turns_cap);
    let consolidations_count = response
        .results
        .consolidations
        .len()
        .min(opts.consolidations_cap);
    // Phase11j §4.3 — `Past sessions` falls back when zero
    // consolidations match. Computing both counts up-front so the
    // empty-bundle short-circuit accounts for either section
    // landing.
    let past_sessions_count = if consolidations_count > 0 {
        0
    } else {
        response
            .results
            .past_sessions
            .len()
            .min(opts.past_sessions_cap)
    };
    let snippets_count = response.results.snippets.len().min(opts.snippets_cap);
    let graph_count = response.results.graph_neighbors.len().min(opts.graph_cap);

    if laws_count == 0
        && decisions_count == 0
        && turns_count == 0
        && consolidations_count == 0
        && past_sessions_count == 0
        && snippets_count == 0
        && graph_count == 0
    {
        return String::new();
    }

    let mut out = String::with_capacity(2048);
    write!(
        out,
        "<!-- cortex: {intent} · query_id={query_id} · budget=section_caps -->\n\n",
        query_id = response.query_id,
    )
    .ok();

    if laws_count > 0 {
        out.push_str("## Active laws in this scope\n");
        for law in response.laws_active.iter().take(laws_count) {
            writeln!(
                out,
                "- **{id}** ({severity}) — {title}.",
                id = law.id,
                severity = law.severity,
                title = trim_one_line(&law.title),
            )
            .ok();
        }
        out.push('\n');
    }

    if decisions_count > 0 {
        out.push_str("## Recent decisions you should know about\n");
        for d in response.results.decisions.iter().take(decisions_count) {
            let date = format_ts_date(d.ts);
            writeln!(
                out,
                "- {glyph} **{id} ({status}{date_suffix})** — {title}",
                glyph = decision_glyph(&d.status),
                id = d.id,
                status = d.status,
                date_suffix = if date.is_empty() {
                    String::new()
                } else {
                    format!(", {date}")
                },
                title = trim_one_line(&d.title),
            )
            .ok();
            if !d.links.is_empty() {
                let body = d
                    .links
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ");
                writeln!(out, "  Links: {body}").ok();
            }
        }
        // Truncate the section's tail if any single decision title
        // overflowed the per-decision byte cap (rare but possible
        // for very long titles).
        let _ = opts.decision_byte_cap;
        out.push('\n');
    }

    if turns_count > 0 {
        out.push_str("## Similar past turns\n");
        for (i, t) in response
            .results
            .similar_turns
            .iter()
            .take(turns_count)
            .enumerate()
        {
            let date = format_ts_date(t.ts);
            writeln!(
                out,
                "{}. {glyph} {date} — {model} {summary}",
                i + 1,
                glyph = outcome_glyph(t.outcome.as_deref()),
                date = date,
                model = t.model,
                summary = trim_one_line(&t.summary),
            )
            .ok();
        }
        out.push('\n');
    }

    // Phase11j §4.2 — "Consolidated context": one line per
    // consolidation, ordered by upstream similarity. Format pins
    // `grain/id · date · ✓|✗|⚠ · title` so the agent gets the
    // distilled lesson without reading every turn that fed it.
    // When ≥ 1 consolidation matches, this section replaces
    // "Past sessions" (§4.3 fallback rule); otherwise it is
    // skipped and "Past sessions" runs as before.
    if consolidations_count > 0 {
        writeln!(out, "## Consolidated context ({consolidations_count})").ok();
        for (i, c) in response
            .results
            .consolidations
            .iter()
            .take(consolidations_count)
            .enumerate()
        {
            let date = format_ts_date(c.ts);
            writeln!(
                out,
                "{}. {grain}/{id} · {date_field} · {glyph} · {title}",
                i + 1,
                grain = c.grain,
                id = c.consolidation_id,
                date_field = if date.is_empty() {
                    "—".to_string()
                } else {
                    date
                },
                glyph = outcome_glyph(c.outcome.as_deref()),
                title = trim_one_line(&c.title),
            )
            .ok();
        }
        out.push('\n');
    }

    // Phase11i §4.1 — "Past sessions": one line per session,
    // ordered by upstream centroid similarity. Format pins
    // `id · date · "first user prompt (≤ 80 chars)" · turn_count`
    // so the agent can recognise prior sessions touching the same
    // problem space without reading every turn back.
    if past_sessions_count > 0 {
        writeln!(out, "## Past sessions ({past_sessions_count})").ok();
        for (i, s) in response
            .results
            .past_sessions
            .iter()
            .take(past_sessions_count)
            .enumerate()
        {
            let date = format_ts_date(s.ts);
            let prompt = clip_utf8(
                &trim_one_line(&s.first_prompt),
                opts.past_session_prompt_byte_cap,
            );
            writeln!(
                out,
                "{}. {id} — {date_field} · \"{prompt}\" · {turns} turn{plural}",
                i + 1,
                id = s.session_id,
                date_field = if date.is_empty() {
                    "—".to_string()
                } else {
                    date
                },
                prompt = prompt,
                turns = s.turn_count,
                plural = if s.turn_count == 1 { "" } else { "s" },
            )
            .ok();
        }
        out.push('\n');
    }

    if snippets_count > 0 {
        writeln!(out, "## Relevant snippets ({snippets_count})").ok();
        for (i, s) in response
            .results
            .snippets
            .iter()
            .take(snippets_count)
            .enumerate()
        {
            let header = render_snippet_header(s);
            let body = match opts.snippet_trim {
                SnippetTrim::SlimWhyPlusThree => render_snippet_slim(s, opts.snippet_byte_cap),
                SnippetTrim::Full => render_snippet_full(s, opts.snippet_byte_cap),
            };
            writeln!(out, "{}. {header}{body}", i + 1).ok();
        }
        out.push('\n');
    }

    if graph_count > 0 {
        // Phase11k §6.2 — split graph neighbours into three named
        // sub-blocks for the relations the renderer specifically
        // wants to surface (`Connected files (via IMPORTS_FILE)`,
        // `Documented under (via DOCUMENTED_BY)`, `Cited from (via
        // CITES)`); everything else falls back to the generic
        // `Graph neighbours` heading. Sub-block ordering matches
        // the spec-12 update under phase11k §6.3.
        let neighbours: Vec<&cortex_api::GraphNeighbor> = response
            .results
            .graph_neighbors
            .iter()
            .take(graph_count)
            .collect();
        let imports: Vec<&cortex_api::GraphNeighbor> = neighbours
            .iter()
            .copied()
            .filter(|n| n.relation == "IMPORTS_FILE")
            .collect();
        let documented: Vec<&cortex_api::GraphNeighbor> = neighbours
            .iter()
            .copied()
            .filter(|n| n.relation == "DOCUMENTED_BY")
            .collect();
        let cites: Vec<&cortex_api::GraphNeighbor> = neighbours
            .iter()
            .copied()
            .filter(|n| n.relation == "CITES")
            .collect();
        let other: Vec<&cortex_api::GraphNeighbor> = neighbours
            .iter()
            .copied()
            .filter(|n| {
                n.relation != "IMPORTS_FILE"
                    && n.relation != "DOCUMENTED_BY"
                    && n.relation != "CITES"
            })
            .collect();

        if !imports.is_empty() {
            out.push_str("## Connected files (via IMPORTS_FILE)\n");
            for n in &imports {
                writeln!(
                    out,
                    "- {from} -> {to} (hops={hops})",
                    from = n.from,
                    to = n.to,
                    hops = n.hops,
                )
                .ok();
            }
            out.push('\n');
        }
        if !documented.is_empty() {
            out.push_str("## Documented under (via DOCUMENTED_BY)\n");
            for n in &documented {
                writeln!(
                    out,
                    "- {from} -> {to} (hops={hops})",
                    from = n.from,
                    to = n.to,
                    hops = n.hops,
                )
                .ok();
            }
            out.push('\n');
        }
        if !cites.is_empty() {
            out.push_str("## Cited from (via CITES)\n");
            for n in &cites {
                writeln!(
                    out,
                    "- {from} -> {to} (hops={hops})",
                    from = n.from,
                    to = n.to,
                    hops = n.hops,
                )
                .ok();
            }
            out.push('\n');
        }
        if !other.is_empty() {
            out.push_str("## Graph neighbours\n");
            for n in &other {
                writeln!(
                    out,
                    "- {from} -[{relation}]-> {to} (hops={hops})",
                    from = n.from,
                    relation = n.relation,
                    to = n.to,
                    hops = n.hops,
                )
                .ok();
            }
            out.push('\n');
        }
    }

    out.push_str("<!-- end cortex -->\n");
    out
}

fn render_snippet_header(s: &cortex_api::Snippet) -> String {
    // phase10b §3.1 — header is `repo/path:symbol` when a real
    // symbol is present (Tree-sitter for code, H1 for docs).
    // `Snippet.symbol` no longer carries kind labels (the orchestrator
    // strips `artifact` / `turn` / etc. in `snippet_from_hit`); when
    // the upstream lacks a real symbol the header degrades to
    // `repo/path` instead of producing the audit-flagged
    // `Cortex/.../types.rs:artifact` rendering.
    let repo = s.repo.as_deref().unwrap_or("");
    let path = s.path.as_deref().unwrap_or("");
    let symbol = s.symbol.as_deref().filter(|sym| !sym.is_empty());
    let prefix = match (repo.is_empty(), path.is_empty(), symbol) {
        (false, false, Some(sym)) => format!("`{repo}/{path}:{sym}` — "),
        (false, false, None) => format!("`{repo}/{path}` — "),
        (false, true, _) => format!("`{repo}` — "),
        (true, false, _) => format!("`{path}` — "),
        _ => String::new(),
    };
    // phase10b §3.1 — when the keyword lane could not project a
    // body, surface that in the header so the agent does not
    // assume the renderer accidentally truncated it.
    let why_text = s
        .why
        .as_deref()
        .map(trim_one_line)
        .filter(|s| !s.is_empty());
    let truncation_note = if s.body_truncated && s.text.is_empty() {
        Some("(body not indexed inline)".to_string())
    } else {
        None
    };
    match (why_text, truncation_note) {
        (Some(why), Some(note)) => format!("{prefix}{why} — {note}"),
        (Some(why), None) => format!("{prefix}{why}"),
        (None, Some(note)) => format!("{prefix}{note}"),
        (None, None) => prefix,
    }
}

fn render_snippet_slim(s: &cortex_api::Snippet, byte_cap: usize) -> String {
    let body: String = s.text.lines().take(3).collect::<Vec<_>>().join("\n");
    if body.is_empty() {
        return String::new();
    }
    let truncated = clip_utf8(&body, byte_cap.min(384));
    format!("\n   {}", truncated.replace('\n', "\n   "))
}

fn render_snippet_full(s: &cortex_api::Snippet, byte_cap: usize) -> String {
    if s.text.is_empty() {
        return String::new();
    }
    let truncated = clip_utf8(&s.text, byte_cap);
    format!("\n   {}", truncated.replace('\n', "\n   "))
}

fn trim_one_line(s: &str) -> String {
    let trimmed = s.lines().next().unwrap_or(s).trim();
    trimmed.to_string()
}

/// Phase11i §4.2 — render an outcome label as a single glyph.
///
/// Mapping:
/// - `success` → `✓`
/// - `error` / `failed` / `failure` → `✗`
/// - `partial` / `blocked_by_law` / unknown / `None` → `⚠`
///
/// The neutral fallback (`⚠`) keeps the column shape consistent
/// when the upstream classifier did not stamp an outcome — a row
/// always carries exactly one glyph.
pub fn outcome_glyph(outcome: Option<&str>) -> &'static str {
    match outcome.map(str::trim).filter(|s| !s.is_empty()) {
        Some("success") => "✓",
        Some("error") | Some("failed") | Some("failure") => "✗",
        _ => "⚠",
    }
}

/// Phase11i §4.2 — render an ADR status as a single glyph.
///
/// Mapping:
/// - `accepted` → `✓`
/// - `superseded` / `deprecated` / `rejected` → `✗`
/// - `proposed` / `draft` / unknown → `⚠`
pub fn decision_glyph(status: &str) -> &'static str {
    match status.trim() {
        "accepted" => "✓",
        "superseded" | "deprecated" | "rejected" => "✗",
        _ => "⚠",
    }
}

fn format_ts_date(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let dt = match Utc.timestamp_opt(secs, 0) {
        chrono::offset::LocalResult::Single(d) => d,
        _ => return String::new(),
    };
    dt.format("%Y-%m-%d").to_string()
}

/// UTF-8-safe clip — preserves the first `n` bytes ending on a char
/// boundary.
pub fn clip_utf8(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut cut = n;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_api::{
        BudgetReport, DebugInfo, DecisionRef, GraphNeighbor, LaneTimings, LawRef, PastSession,
        QueryResponse, ResultsBag, Scope, SimilarTurn, Snippet, ViolationRef,
    };

    fn populated_response() -> QueryResponse {
        QueryResponse {
            intent: "pre_change_context".into(),
            query_id: "01HFIXED".into(),
            scope_resolved: Scope {
                repo: Some("Vectorizer".into()),
                ..Default::default()
            },
            results: ResultsBag {
                snippets: vec![Snippet {
                    rank: 1,
                    source: "vector".into(),
                    collection: None,
                    repo: Some("Vectorizer".into()),
                    path: Some("src/index/hnsw/mod.rs".into()),
                    symbol: Some("hnsw_search".into()),
                    content_hash: None,
                    text: "pub fn hnsw_search() {}".into(),
                    body_truncated: false,
                    score: 0.9,
                    why: Some("vector match to ef_search tuning".into()),
                }],
                decisions: vec![DecisionRef {
                    rank: 1,
                    id: "DEC-0042".into(),
                    title: "Adopt Meilisearch".into(),
                    rationale_excerpt: None,
                    status: "accepted".into(),
                    ts: 1_715_000_000_000,
                    score: 0.7,
                    links: vec![],
                }],
                violations: vec![ViolationRef {
                    id: "VIO-1".into(),
                    law_id: "LAW-007".into(),
                    severity: "critical".into(),
                    message: "no --no-verify".into(),
                    observed_in: "turn:01HX".into(),
                }],
                graph_neighbors: vec![GraphNeighbor {
                    from: "ToolCall:01H".into(),
                    relation: "TOUCHED".into(),
                    to: "Artifact:V|src/lib.rs|sha".into(),
                    hops: 1,
                }],
                similar_turns: vec![SimilarTurn {
                    turn_id: "01HXZ".into(),
                    ts: 1_715_000_000_000,
                    model: "claude-sonnet".into(),
                    summary: "refactored hnsw_search".into(),
                    score: 0.6,
                    outcome: Some("success".into()),
                }],
                past_sessions: Vec::new(),
                consolidations: Vec::new(),
            },
            laws_active: vec![LawRef {
                id: "LAW-012".into(),
                severity: "notable".into(),
                title: "HNSW recall benchmarks must run".into(),
            }],
            budget: BudgetReport {
                used_ms: 0,
                cap_ms: 500,
                cache: "miss".into(),
            },
            debug: DebugInfo {
                lanes: LaneTimings::default(),
                errors: Default::default(),
                truncated: false,
                notes: Vec::new(),
            },
            notice: None,
            clipped: None,
        }
    }

    #[test]
    fn fixed_section_order_renders_laws_first() {
        let opts = FormatOptions::default();
        let bundle = format_bundle("pre_change_context", &populated_response(), &opts);
        let laws = bundle.find("Active laws in this scope").unwrap();
        let decisions = bundle.find("Recent decisions").unwrap();
        let turns = bundle.find("Similar past turns").unwrap();
        let snippets = bundle.find("Relevant snippets").unwrap();
        assert!(laws < decisions);
        assert!(decisions < turns);
        assert!(turns < snippets);
        assert!(bundle.contains("<!-- end cortex -->"));
        assert!(bundle.contains("query_id=01HFIXED"));
    }

    #[test]
    fn empty_response_returns_empty_string() {
        let mut resp = populated_response();
        resp.results = ResultsBag::default();
        resp.laws_active.clear();
        let bundle = format_bundle("free_search", &resp, &FormatOptions::default());
        assert!(bundle.is_empty());
    }

    #[test]
    fn graph_section_omitted_when_cap_zero() {
        let bundle = format_bundle(
            "pre_change_context",
            &populated_response(),
            &FormatOptions::default(),
        );
        assert!(!bundle.contains("Graph neighbours"));
    }

    #[test]
    fn graph_section_renders_when_cap_lifted() {
        let opts = FormatOptions {
            graph_cap: 5,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &populated_response(), &opts);
        assert!(bundle.contains("Graph neighbours"));
    }

    fn neighbour(from: &str, relation: &str, to: &str, hops: u8) -> GraphNeighbor {
        GraphNeighbor {
            from: from.into(),
            relation: relation.into(),
            to: to.into(),
            hops,
        }
    }

    #[test]
    fn imports_file_neighbours_render_under_connected_files_block() {
        // Phase11k §6.2 — IMPORTS_FILE neighbours land under
        // `## Connected files (via IMPORTS_FILE)`.
        let mut resp = populated_response();
        resp.results.graph_neighbors = vec![neighbour(
            "Artifact:src/lib.rs",
            "IMPORTS_FILE",
            "Artifact:src/foo.rs",
            1,
        )];
        let opts = FormatOptions {
            graph_cap: 5,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(
            bundle.contains("## Connected files (via IMPORTS_FILE)"),
            "missing Connected files block in:\n{bundle}"
        );
        assert!(
            !bundle.contains("Documented under"),
            "Documented under block must stay absent when no DOCUMENTED_BY hits"
        );
    }

    #[test]
    fn documented_by_neighbours_render_under_documented_under_block() {
        let mut resp = populated_response();
        resp.results.graph_neighbors = vec![neighbour(
            "Artifact:docs/spec.md",
            "DOCUMENTED_BY",
            "Symbol:foo::bar",
            1,
        )];
        let opts = FormatOptions {
            graph_cap: 5,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(
            bundle.contains("## Documented under (via DOCUMENTED_BY)"),
            "missing Documented under block in:\n{bundle}"
        );
    }

    #[test]
    fn cites_neighbours_render_under_cited_from_block() {
        let mut resp = populated_response();
        resp.results.graph_neighbors = vec![neighbour(
            "Decision:DEC-0042",
            "CITES",
            "Spec:docs/specs/07.md",
            2,
        )];
        let opts = FormatOptions {
            graph_cap: 5,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(
            bundle.contains("## Cited from (via CITES)"),
            "missing Cited from block in:\n{bundle}"
        );
    }

    #[test]
    fn unknown_relation_falls_through_to_generic_block() {
        // TOUCHED stays in the legacy `## Graph neighbours`
        // catch-all so nothing the orchestrator surfaces gets
        // dropped silently.
        let mut resp = populated_response();
        resp.results.graph_neighbors = vec![neighbour(
            "ToolCall:tc-1",
            "TOUCHED",
            "Artifact:src/x.rs",
            1,
        )];
        let opts = FormatOptions {
            graph_cap: 5,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(bundle.contains("## Graph neighbours"));
        assert!(!bundle.contains("Connected files"));
    }

    #[test]
    fn deterministic_byte_for_byte_output() {
        let opts = FormatOptions::default();
        let resp = populated_response();
        let a = format_bundle("pre_change_context", &resp, &opts);
        let b = format_bundle("pre_change_context", &resp, &opts);
        assert_eq!(a, b);
    }

    #[test]
    fn clip_utf8_respects_char_boundaries() {
        let raw = "ééé"; // 6 bytes
        assert_eq!(clip_utf8(raw, 5), "éé");
        assert_eq!(clip_utf8(raw, 100), "ééé");
    }

    fn past_session(id: &str, ts: i64, prompt: &str, turn_count: u32, score: f64) -> PastSession {
        PastSession {
            session_id: id.into(),
            ts,
            first_prompt: prompt.into(),
            turn_count,
            score,
        }
    }

    fn consolidation(
        id: &str,
        grain: &str,
        ts: i64,
        title: &str,
        outcome: Option<&str>,
        score: f64,
    ) -> cortex_api::ConsolidationRef {
        cortex_api::ConsolidationRef {
            consolidation_id: id.into(),
            grain: grain.into(),
            ts,
            title: title.into(),
            outcome: outcome.map(|s| s.to_string()),
            score,
        }
    }

    #[test]
    fn consolidated_context_section_renders_when_present() {
        // Phase11j §4.2 — `Consolidated context` section format pin:
        // `grain/id · YYYY-MM-DD · ✓|✗|⚠ · title`. One line per
        // consolidation, top-3 by similarity (cap at
        // `consolidations_cap`).
        let mut resp = populated_response();
        resp.results.consolidations = vec![
            consolidation(
                "cons-ses-aaa",
                "session",
                1_715_000_000_000,
                "Auth refactor session",
                Some("success"),
                0.92,
            ),
            consolidation(
                "cons-top-bbb",
                "topic",
                1_716_000_000_000,
                "JWT rotation pattern",
                Some("partial"),
                0.85,
            ),
        ];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(
            bundle.contains("## Consolidated context (2)"),
            "missing header in:\n{bundle}"
        );
        assert!(bundle.contains("session/cons-ses-aaa"));
        assert!(bundle.contains("Auth refactor session"));
        assert!(bundle.contains("topic/cons-top-bbb"));
    }

    #[test]
    fn consolidations_replace_past_sessions_when_at_least_one_matches() {
        // Phase11j §4.3 — fallback rule: when a consolidation
        // matches, the past-sessions section is suppressed; when
        // none match, past-sessions runs as before.
        let mut resp = populated_response();
        resp.results.consolidations = vec![consolidation(
            "cons-ses-x",
            "session",
            1_715_000_000_000,
            "single match",
            None,
            0.7,
        )];
        resp.results.past_sessions = vec![past_session(
            "sess-Y",
            1_715_000_000_000,
            "prompt",
            3,
            0.6,
        )];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("Consolidated context"));
        assert!(
            !bundle.contains("Past sessions"),
            "past sessions must be suppressed when consolidations are present:\n{bundle}"
        );
    }

    #[test]
    fn past_sessions_falls_back_when_no_consolidations_match() {
        // Phase11j §4.3 — empty consolidations → renderer falls back
        // to the original past-sessions block.
        let mut resp = populated_response();
        resp.results.consolidations = Vec::new();
        resp.results.past_sessions = vec![past_session(
            "sess-fallback",
            1_715_000_000_000,
            "prompt",
            2,
            0.5,
        )];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(!bundle.contains("Consolidated context"));
        assert!(bundle.contains("## Past sessions (1)"));
        assert!(bundle.contains("sess-fallback"));
    }

    #[test]
    fn consolidations_cap_caps_section_size() {
        // Spec calls for top-3; the cap is configurable and the
        // renderer takes the first N entries the upstream surfaced.
        let mut resp = populated_response();
        resp.results.consolidations = (0..6)
            .map(|i| {
                consolidation(
                    &format!("cons-ses-{i}"),
                    "session",
                    1_715_000_000_000,
                    "title",
                    None,
                    0.5,
                )
            })
            .collect();
        let opts = FormatOptions {
            consolidations_cap: 2,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(bundle.contains("## Consolidated context (2)"));
        assert!(bundle.contains("cons-ses-0"));
        assert!(bundle.contains("cons-ses-1"));
        assert!(!bundle.contains("cons-ses-2"));
    }

    #[test]
    fn past_sessions_section_renders_one_line_per_session() {
        let mut resp = populated_response();
        resp.results.past_sessions = vec![
            past_session(
                "sess-A",
                1_715_000_000_000,
                "How do I tune ef_search for HNSW recall?",
                12,
                0.92,
            ),
            past_session(
                "sess-B",
                1_715_086_400_000,
                "wire meili filter grammar",
                5,
                0.81,
            ),
            past_session(
                "sess-C",
                1_715_172_800_000,
                "audit envelope shape regression",
                1,
                0.74,
            ),
        ];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("## Past sessions (3)"));
        assert!(bundle.contains(
            "1. sess-A — 2024-05-06 · \"How do I tune ef_search for HNSW recall?\" · 12 turns"
        ));
        assert!(bundle.contains("2. sess-B — "));
        assert!(bundle.contains("3. sess-C — "));
        assert!(bundle.contains("· 1 turn\n"));
        assert!(bundle.contains("· 5 turns\n"));
    }

    #[test]
    fn past_sessions_clip_first_prompt_to_eighty_bytes() {
        let mut resp = populated_response();
        let long = "x".repeat(160);
        resp.results.past_sessions = vec![past_session("sess-X", 1_715_000_000_000, &long, 3, 0.5)];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        // Prompt segment lives between the first `"` and the
        // closing `"` on the session line. The clipped body must
        // be exactly 80 ASCII bytes long.
        let after = bundle
            .find("sess-X — ")
            .expect("session line present in bundle");
        let line: &str = bundle[after..]
            .lines()
            .next()
            .expect("session line terminates with newline");
        let q1 = line.find('"').unwrap();
        let q2 = line[q1 + 1..].find('"').unwrap() + q1 + 1;
        let clipped = &line[q1 + 1..q2];
        assert_eq!(clipped.len(), 80, "first prompt must clip to 80 bytes");
        assert!(clipped.chars().all(|c| c == 'x'));
    }

    #[test]
    fn past_sessions_renders_after_similar_turns_before_snippets() {
        let mut resp = populated_response();
        resp.results.past_sessions = vec![past_session(
            "sess-A",
            1_715_000_000_000,
            "first prompt",
            2,
            0.6,
        )];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        let turns = bundle.find("Similar past turns").unwrap();
        let past = bundle.find("Past sessions").unwrap();
        let snippets = bundle.find("Relevant snippets").unwrap();
        assert!(turns < past);
        assert!(past < snippets);
    }

    #[test]
    fn past_sessions_cap_caps_section_size() {
        let mut resp = populated_response();
        resp.results.past_sessions = (0..10)
            .map(|i| past_session(&format!("sess-{i}"), 1_715_000_000_000, "prompt", 1, 0.5))
            .collect();
        let opts = FormatOptions {
            past_sessions_cap: 2,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(bundle.contains("## Past sessions (2)"));
        assert!(bundle.contains("sess-0"));
        assert!(bundle.contains("sess-1"));
        assert!(!bundle.contains("sess-2"));
    }

    #[test]
    fn past_sessions_section_omitted_when_empty() {
        let resp = populated_response();
        // Default populated_response leaves past_sessions empty.
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(!bundle.contains("Past sessions"));
    }

    #[test]
    fn past_sessions_handles_unset_ts_gracefully() {
        let mut resp = populated_response();
        resp.results.past_sessions = vec![past_session("sess-Z", 0, "no timestamp", 1, 0.4)];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        // The em-dash fills the date column when the upstream
        // couldn't supply a timestamp, keeping the line shape
        // consistent across rows.
        assert!(bundle.contains("sess-Z — — · \"no timestamp\" · 1 turn"));
    }

    #[test]
    fn outcome_glyph_table_resolves_known_and_unknown() {
        assert_eq!(outcome_glyph(Some("success")), "✓");
        assert_eq!(outcome_glyph(Some("error")), "✗");
        assert_eq!(outcome_glyph(Some("failed")), "✗");
        assert_eq!(outcome_glyph(Some("failure")), "✗");
        assert_eq!(outcome_glyph(Some("partial")), "⚠");
        assert_eq!(outcome_glyph(Some("blocked_by_law")), "⚠");
        assert_eq!(outcome_glyph(Some("unknown")), "⚠");
        assert_eq!(outcome_glyph(Some("")), "⚠");
        assert_eq!(outcome_glyph(None), "⚠");
    }

    #[test]
    fn decision_glyph_table_resolves_known_and_unknown() {
        assert_eq!(decision_glyph("accepted"), "✓");
        assert_eq!(decision_glyph("superseded"), "✗");
        assert_eq!(decision_glyph("deprecated"), "✗");
        assert_eq!(decision_glyph("rejected"), "✗");
        assert_eq!(decision_glyph("proposed"), "⚠");
        assert_eq!(decision_glyph("draft"), "⚠");
        assert_eq!(decision_glyph(""), "⚠");
    }

    #[test]
    fn similar_turn_line_carries_outcome_glyph() {
        // Default fixture stamps `outcome=Some("success")`, so the
        // turn line must render the ✓ glyph between the index and
        // the date.
        let bundle = format_bundle(
            "pre_change_context",
            &populated_response(),
            &FormatOptions::default(),
        );
        assert!(bundle.contains("1. ✓ 2024-05-06 — claude-sonnet refactored hnsw_search"));
    }

    #[test]
    fn similar_turn_unknown_outcome_falls_back_to_warning_glyph() {
        let mut resp = populated_response();
        resp.results.similar_turns[0].outcome = None;
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("1. ⚠ 2024-05-06 — claude-sonnet"));
    }

    #[test]
    fn similar_turn_error_outcome_renders_cross_glyph() {
        let mut resp = populated_response();
        resp.results.similar_turns[0].outcome = Some("error".into());
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("1. ✗ 2024-05-06 — claude-sonnet"));
    }

    #[test]
    fn decision_line_carries_outcome_glyph() {
        // Default fixture status is `accepted`, so the decision
        // line must render the ✓ glyph between the bullet and the
        // bolded id block.
        let bundle = format_bundle(
            "pre_change_context",
            &populated_response(),
            &FormatOptions::default(),
        );
        assert!(bundle.contains("- ✓ **DEC-0042 (accepted, 2024-05-06)** — Adopt Meilisearch"));
    }

    #[test]
    fn decision_line_superseded_renders_cross_glyph() {
        let mut resp = populated_response();
        resp.results.decisions[0].status = "superseded".into();
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("- ✗ **DEC-0042 (superseded"));
    }

    #[test]
    fn decision_line_proposed_renders_warning_glyph() {
        let mut resp = populated_response();
        resp.results.decisions[0].status = "proposed".into();
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("- ⚠ **DEC-0042 (proposed"));
    }

    #[test]
    fn slim_snippet_only_renders_three_lines() {
        let mut resp = populated_response();
        resp.results.snippets[0].text = "1\n2\n3\n4\n5".into();
        let opts = FormatOptions {
            snippet_trim: SnippetTrim::SlimWhyPlusThree,
            ..Default::default()
        };
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        // Snippet body is indented 3 spaces; check we kept only
        // the first three lines of text.
        assert!(bundle.contains("   1\n   2\n   3"));
        assert!(!bundle.contains("   4"));
    }
}
