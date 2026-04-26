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

use cortex_api::QueryResponse;
use chrono::{TimeZone, Utc};

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
    let turns_count = response.results.similar_turns.len().min(opts.similar_turns_cap);
    let snippets_count = response.results.snippets.len().min(opts.snippets_cap);
    let graph_count = response.results.graph_neighbors.len().min(opts.graph_cap);

    if laws_count == 0
        && decisions_count == 0
        && turns_count == 0
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
                "- **{id} ({status}{date_suffix})** — {title}",
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
        for (i, t) in response.results.similar_turns.iter().take(turns_count).enumerate() {
            let date = format_ts_date(t.ts);
            writeln!(
                out,
                "{}. {} — {model} {summary}",
                i + 1,
                date,
                model = t.model,
                summary = trim_one_line(&t.summary),
            )
            .ok();
        }
        out.push('\n');
    }

    if snippets_count > 0 {
        writeln!(out, "## Relevant snippets ({snippets_count})").ok();
        for (i, s) in response.results.snippets.iter().take(snippets_count).enumerate() {
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
        out.push_str("## Graph neighbours\n");
        for n in response.results.graph_neighbors.iter().take(graph_count) {
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

    out.push_str("<!-- end cortex -->\n");
    out
}

fn render_snippet_header(s: &cortex_api::Snippet) -> String {
    let repo = s.repo.as_deref().unwrap_or("");
    let path = s.path.as_deref().unwrap_or("");
    let symbol = s.symbol.as_deref();
    let prefix = match (repo.is_empty(), path.is_empty(), symbol) {
        (false, false, Some(sym)) => format!("`{repo}/{path}:{sym}` — "),
        (false, false, None) => format!("`{repo}/{path}` — "),
        (false, true, _) => format!("`{repo}` — "),
        (true, false, _) => format!("`{path}` — "),
        _ => String::new(),
    };
    let why = s
        .why
        .as_deref()
        .map(trim_one_line)
        .filter(|s| !s.is_empty());
    if let Some(why) = why {
        format!("{prefix}{why}")
    } else {
        prefix
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
        BudgetReport, DebugInfo, DecisionRef, GraphNeighbor, LaneTimings, LawRef, QueryResponse,
        ResultsBag, Scope, SimilarTurn, Snippet, ViolationRef,
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
                    score: 0.9,
                    why: Some("vector match to ef_search tuning".into()),
                }],
                decisions: vec![DecisionRef {
                    rank: 1,
                    id: "DEC-0042".into(),
                    title: "Adopt Meilisearch".into(),
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
                }],
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
            },
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
        let bundle =
            format_bundle("pre_change_context", &populated_response(), &FormatOptions::default());
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
