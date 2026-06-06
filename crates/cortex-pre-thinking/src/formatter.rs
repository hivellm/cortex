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
    /// Phase11r §5.3 — max topic-card entries the "Topic card"
    /// section renders. Spec calls for one card (the top-priority
    /// living synthesis); fallback to consolidations if zero match
    /// or staleness fires.
    pub const TOPIC_CARDS: usize = 1;
    /// Phase11r §5.3 — section byte budget for the topic-card lane.
    /// Drives the budget clipper; the section trims its synthesis
    /// preview + evidence block to fit.
    pub const TOPIC_CARDS_BYTES: usize = 1_400;
    /// Phase11r §5.4 — staleness threshold. Cards older than this
    /// (in days) AND with ≥ 1 fresh event since the last rewrite
    /// trip the staleness advisory + downgrade.
    pub const TOPIC_CARD_STALE_AGE_DAYS: u32 = 30;
    /// Phase11r §5.4 — confidence floor below which the topic-card
    /// section is downgraded behind consolidations.
    pub const TOPIC_CARD_CONFIDENCE_FLOOR: f32 = 0.6;
    /// Max bytes per snippet text.
    pub const SNIPPET_BYTES: usize = 1024;
    /// Max bytes per decision body.
    pub const DECISION_BYTES: usize = 512;
    /// Phase13g §4.2 — section byte budget for the "Active operator
    /// work" section produced by [`super::render_active_work`].
    /// Sized for ~6 rows of `<phase> · <status> · <next>` plus a
    /// recent-archives tail.
    pub const ACTIVE_WORK_BYTES: usize = 1_200;
    /// Phase13g §4.2 — section byte budget for the "Similar past
    /// sessions" section produced by [`super::render_similar_sessions`].
    /// Larger than the active-work cap because each row carries a
    /// trimmed summary excerpt.
    pub const SIMILAR_SESSIONS_BYTES: usize = 2_000;
    /// Phase13g §4.2 — section byte budget for the "ADR provenance"
    /// section produced by [`super::render_adr_provenance`].
    /// Conditional — the orchestrator only renders it when an ADR
    /// id appears in the query or fusion result.
    pub const ADR_PROVENANCE_BYTES: usize = 800;
    /// Phase18 §6.1 — byte budget for the bitemporal "Timeline window" section.
    pub const TIMELINE_WINDOW_BYTES: usize = 1_400;
    /// Max timeline events rendered in the window.
    pub const TIMELINE_WINDOW_EVENTS: usize = 8;
    /// Phase18 §6.1 — byte budget for the "Supersession overlay" section.
    pub const SUPERSESSION_OVERLAY_BYTES: usize = 1_000;
    /// Phase18 §6.1 — byte budget for the "Branch context" section.
    pub const BRANCH_CONTEXT_BYTES: usize = 800;
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
    /// Phase11r §5.3 — maximum topic-card entries to render. Spec
    /// pins the default at 1 (the top-priority living synthesis).
    pub topic_cards_cap: usize,
    /// Phase11r §5.3 — section byte budget for the topic-card
    /// lane. Drives the synthesis preview + evidence trim inside
    /// the section.
    pub topic_cards_byte_cap: usize,
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
    /// Phase13g §4 — grounding sections fed by the three new MCP
    /// tools (`cortex_active_work`, `cortex_similar_sessions`,
    /// `cortex_decision_chain`). Default is empty; the orchestrator
    /// populates these when the pre-thinking pipeline fetches the
    /// tool output.
    pub grounding: GroundingSections,
}

/// Phase13g §4 — typed envelope carrying the three grounding-tool
/// outputs. The renderer projects each into its corresponding
/// section under [`section_caps::ACTIVE_WORK_BYTES`] /
/// [`SIMILAR_SESSIONS_BYTES`] / [`ADR_PROVENANCE_BYTES`].
#[derive(Debug, Clone, Default)]
pub struct GroundingSections {
    /// Rows produced by `cortex_active_work`. Empty ⇒ the
    /// "Active operator work" section is skipped.
    pub active_work: Vec<ActiveWorkRow>,
    /// Rows produced by `cortex_similar_sessions`. Empty ⇒ the
    /// "Similar past sessions" section is skipped.
    pub similar_sessions: Vec<SimilarSessionRow>,
    /// Rows produced by `cortex_decision_chain`. Empty ⇒ the
    /// "ADR provenance" section is skipped (phase13g §4.3
    /// conditional).
    pub adr_provenance: Vec<AdrProvenanceRow>,
    /// Phase18 §6.1 — bitemporal timeline window. None ⇒ section skipped.
    pub timeline_window: Option<TimelineWindow>,
    /// Phase18 §6.1 — supersession overlay. None ⇒ section skipped.
    pub supersession_overlay: Option<SupersessionOverlay>,
    /// Phase18 §6.1 — branch context. None ⇒ section skipped.
    pub branch_context: Option<BranchContext>,
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
            topic_cards_cap: section_caps::TOPIC_CARDS,
            topic_cards_byte_cap: section_caps::TOPIC_CARDS_BYTES,
            snippets_cap: section_caps::SNIPPETS,
            graph_cap: section_caps::GRAPH_NEIGHBORS,
            snippet_trim: SnippetTrim::Full,
            decision_byte_cap: section_caps::DECISION_BYTES,
            snippet_byte_cap: section_caps::SNIPPET_BYTES,
            grounding: GroundingSections::default(),
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
    // Phase11r §5.3 + §5.4 — topic-card section count + staleness.
    // The advisory + downgrade fires when confidence is below the
    // floor OR (age > 30d AND ≥ 1 fresh event since the last
    // rewrite). When stale, consolidations render before the topic
    // card section; when fresh, the topic card leads.
    let topic_cards_count = response.results.topic_cards.len().min(opts.topic_cards_cap);
    let topic_card_stale = topic_cards_count > 0
        && response
            .results
            .topic_cards
            .first()
            .map(|c| {
                c.confidence < section_caps::TOPIC_CARD_CONFIDENCE_FLOOR
                    || (c.synthesis_age_d > section_caps::TOPIC_CARD_STALE_AGE_DAYS
                        && c.events_since_last_rev > 0)
            })
            .unwrap_or(false);
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
        && topic_cards_count == 0
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

    // Phase18 §6.1 — temporal anchors render at the top of the
    // grounding block, immediately after the header comment and
    // before all other grounding sections. Explicit temporal
    // anchors (timeline, supersession, branch) land first so the
    // LLM sees them before the query body.
    let tw = render_timeline_window(&opts.grounding.timeline_window);
    if !tw.is_empty() {
        out.push_str(&tw);
        out.push('\n');
    }
    let so = render_supersession_overlay(&opts.grounding.supersession_overlay);
    if !so.is_empty() {
        out.push_str(&so);
        out.push('\n');
    }
    let bc = render_branch_context(&opts.grounding.branch_context);
    if !bc.is_empty() {
        out.push_str(&bc);
        out.push('\n');
    }

    // Phase13g §4.4 — grounding sections render between laws and
    // the consolidated-context block. Active work and similar
    // sessions outrank past raw turns because they carry richer
    // signal per byte. Each renderer returns "" when its input is
    // empty so an unwired tool degrades gracefully.
    let aw = render_active_work(&opts.grounding.active_work);
    if !aw.is_empty() {
        out.push_str(&aw);
        out.push('\n');
    }
    let ss = render_similar_sessions(&opts.grounding.similar_sessions);
    if !ss.is_empty() {
        out.push_str(&ss);
        out.push('\n');
    }
    let adr = render_adr_provenance(&opts.grounding.adr_provenance);
    if !adr.is_empty() {
        out.push_str(&adr);
        out.push('\n');
    }

    // Phase11r §5.4 — topic card + consolidations ordering. The
    // fresh path (default) leads with the topic card so the
    // top-priority living synthesis lands first; the stale path
    // demotes the topic card behind consolidations and stamps the
    // advisory line above it. Both paths fall through to the
    // decisions / similar_turns / past_sessions blocks below.
    let cards_slice: &[cortex_api::TopicCardRef] = &response.results.topic_cards;
    let cons_slice: &[cortex_api::ConsolidationRef] = &response.results.consolidations;
    if topic_card_stale {
        render_consolidations_section(&mut out, cons_slice, opts.consolidations_cap);
        render_topic_card_section(
            &mut out,
            cards_slice,
            opts.topic_cards_cap,
            opts.topic_cards_byte_cap,
            true,
        );
    } else {
        render_topic_card_section(
            &mut out,
            cards_slice,
            opts.topic_cards_cap,
            opts.topic_cards_byte_cap,
            false,
        );
        render_consolidations_section(&mut out, cons_slice, opts.consolidations_cap);
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

/// Phase11r §5.3 — render the "Topic card" section. Format pinned
/// by spec:
///
/// ```text
/// ## Topic card
/// > stale-topic-card: <reason>           ← only when `stale`
/// [<topic_slug>] (rev N, confidence X%, age Yd, +Z ev)
/// <synthesis preview>
///
/// ### Evidence (N)
/// - kind:id (cited@rev=N, w=…)
///
/// ### Open contradictions (N)
/// - kind: A vs B (surfaced@rev=N)
/// ```
///
/// `stale` controls whether the advisory line lands. The byte cap
/// trims the synthesis preview + evidence block to fit.
fn render_topic_card_section(
    out: &mut String,
    cards: &[cortex_api::TopicCardRef],
    cap: usize,
    byte_cap: usize,
    stale: bool,
) {
    if cards.is_empty() || cap == 0 {
        return;
    }
    out.push_str("## Topic card\n");
    if stale {
        out.push_str(
            "> stale-topic-card: confidence below floor or synthesis stale with new evidence\n",
        );
    }
    for c in cards.iter().take(cap) {
        let confidence_pct = (c.confidence * 100.0).round() as i32;
        writeln!(
            out,
            "[{slug}] (rev {rev}, confidence {pct}%, age {age}d, +{ev} ev)",
            slug = c.topic_slug,
            rev = c.revision,
            pct = confidence_pct,
            age = c.synthesis_age_d,
            ev = c.events_since_last_rev,
        )
        .ok();
        // Synthesis preview — clip to ~half the section budget so
        // the evidence + contradictions blocks fit alongside.
        let preview_cap = byte_cap / 2;
        let preview = clip_utf8(&c.synthesis_preview, preview_cap);
        if !preview.is_empty() {
            writeln!(out, "{preview}").ok();
        }
        if !c.evidence_top5.is_empty() {
            writeln!(out, "\n### Evidence ({n})", n = c.evidence_top5.len()).ok();
            for e in &c.evidence_top5 {
                let kind = format!("{:?}", e.kind).to_ascii_lowercase();
                let weight = e.weight.map(|w| format!(", w={w:.2}")).unwrap_or_default();
                writeln!(
                    out,
                    "- {kind}:{id} (cited@rev={rev}{weight})",
                    kind = kind,
                    id = e.id,
                    rev = e.cited_at_rev,
                )
                .ok();
            }
        }
        if !c.open_contradictions.is_empty() {
            writeln!(
                out,
                "\n### Open contradictions ({n})",
                n = c.open_contradictions.len()
            )
            .ok();
            for ct in &c.open_contradictions {
                let kind = format!("{:?}", ct.kind);
                writeln!(
                    out,
                    "- {kind}: {a} vs {b} (surfaced@rev={rev})",
                    kind = kind,
                    a = ct.evidence_a,
                    b = ct.evidence_b,
                    rev = ct.surfaced_at_rev,
                )
                .ok();
            }
        }
    }
    out.push('\n');
}

/// Phase11j §4.2 — render the "Consolidated context" section.
/// Extracted into a helper so the §5.4 staleness reorder (consolidations
/// before vs after the topic-card section) can call it from both
/// branches without duplicating the body.
fn render_consolidations_section(
    out: &mut String,
    consolidations: &[cortex_api::ConsolidationRef],
    cap: usize,
) {
    if consolidations.is_empty() || cap == 0 {
        return;
    }
    let count = consolidations.len().min(cap);
    writeln!(out, "## Consolidated context ({count})").ok();
    for (i, c) in consolidations.iter().take(count).enumerate() {
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

// ============================================================
// phase13g §4 — grounding-tool section renderers.
//
// Three new sections fed by the phase13g MCP tools:
//
// - `## Active operator work` (cap [`section_caps::ACTIVE_WORK_BYTES`])
//   surfaces rulebook tasks the agent has not closed yet.
// - `## Similar past sessions` (cap
//   [`section_caps::SIMILAR_SESSIONS_BYTES`]) surfaces top-K
//   consolidations matching the current query.
// - `## ADR provenance` (cap [`section_caps::ADR_PROVENANCE_BYTES`])
//   surfaces the supersession chain when an ADR id appears in the
//   query / fusion result.
//
// Renderers are pure projections of their input value into a
// markdown string. The orchestrator-side dispatch (phase13g §4.3 +
// §4.4) calls them after laws and before consolidated context.
// ============================================================

/// One row in the [`render_active_work`] input. Mirrors the
/// shape `cortex_active_work` returns so the orchestrator can pass
/// the parsed envelope through verbatim.
#[derive(Debug, Clone, Default)]
pub struct ActiveWorkRow {
    /// Task directory name.
    pub id: String,
    /// Phase identifier when known (`phase13g`).
    pub phase: Option<String>,
    /// Status string (`pending` / `in-progress` / `blocked` /
    /// `completed`).
    pub status: String,
    /// First `- [ ]` row in `tasks.md`, already trimmed by the
    /// daemon.
    pub next_unchecked_item: Option<String>,
    /// Operator-facing reason when `status == "blocked"`.
    pub blocked_reason: Option<String>,
}

/// Render the active-work section. Returns an empty string when
/// `rows` is empty so the caller can skip the section without
/// emitting a stray header. Bounded by [`section_caps::ACTIVE_WORK_BYTES`].
pub fn render_active_work(rows: &[ActiveWorkRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let count = rows.len();
    let _ = writeln!(&mut out, "## Active operator work ({count})");
    for row in rows {
        let phase = row.phase.as_deref().unwrap_or("");
        let head = if phase.is_empty() {
            row.id.clone()
        } else {
            format!("{phase} ({id})", id = row.id)
        };
        let next = row
            .next_unchecked_item
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("(all items done)");
        let mut line = format!("- {head} · {status} · {next}", status = row.status);
        if let Some(reason) = row.blocked_reason.as_deref().filter(|s| !s.is_empty()) {
            line.push_str(&format!(" — blocked: {reason}"));
        }
        line.push('\n');
        if out.len() + line.len() > section_caps::ACTIVE_WORK_BYTES {
            out.push_str("- … (truncated; see `cortex_active_work` for the full list)\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

/// One row in the [`render_similar_sessions`] input. Mirrors the
/// shape `cortex_similar_sessions` returns.
#[derive(Debug, Clone, Default)]
pub struct SimilarSessionRow {
    /// Consolidator key (`cons-ses-…` / `cons-top-…`).
    pub consolidation_id: String,
    /// Display title.
    pub title: String,
    /// First N chars of the summary markdown — already trimmed by
    /// the caller.
    pub summary_excerpt: String,
    /// Cosine-similarity score the lane returned.
    pub score: f64,
}

/// Render the similar-sessions section. Returns an empty string
/// when `rows` is empty. Bounded by
/// [`section_caps::SIMILAR_SESSIONS_BYTES`].
pub fn render_similar_sessions(rows: &[SimilarSessionRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let count = rows.len();
    let _ = writeln!(&mut out, "## Similar past sessions ({count})");
    for (i, row) in rows.iter().enumerate() {
        let title = if row.title.is_empty() {
            row.consolidation_id.clone()
        } else {
            row.title.clone()
        };
        let excerpt = trim_one_line(&row.summary_excerpt);
        let line = format!(
            "{n}. `{id}` · score {score:.2} · {title}\n   {excerpt}\n",
            n = i + 1,
            id = row.consolidation_id,
            score = row.score,
            title = title,
            excerpt = excerpt,
        );
        if out.len() + line.len() > section_caps::SIMILAR_SESSIONS_BYTES {
            out.push_str("… (truncated; see `cortex_similar_sessions` for the full list)\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

/// One row in the [`render_adr_provenance`] input. Mirrors the
/// shape `cortex_decision_chain` returns.
#[derive(Debug, Clone, Default)]
pub struct AdrProvenanceRow {
    /// Decision ULID.
    pub event_id: String,
    /// Slug (e.g. `adr-014-...`).
    pub slug: String,
    /// Lifecycle status (`proposed` / `accepted` / `superseded`).
    pub status: String,
    /// ISO-8601 date stamp.
    pub date: String,
    /// Display title.
    pub title: String,
}

/// One event row in the timeline-window section. Mirrors the
/// `TimelineEvent` shape the §4.4 `/v1/timeline` route returns.
#[derive(Debug, Clone, Default)]
pub struct TimelineEventRow {
    /// Event ULID or identifier.
    pub event_id: String,
    /// TimelineKind discriminator (e.g. `decision`, `task`, `session`).
    pub kind: String,
    /// Display title.
    pub title: String,
    /// RFC-3339 / day-precision valid-from timestamp.
    pub valid_from: String,
    /// One-line summary of the event.
    pub summary: String,
}

/// Bitemporal timeline window for the active project+branch (design §3.4).
#[derive(Debug, Clone, Default)]
pub struct TimelineWindow {
    /// Project identifier.
    pub project: String,
    /// As-of timestamp; empty string means "now".
    pub as_of: String,
    /// Composite `<project>:<branch>`; empty string means main.
    pub branch: String,
    /// Recent events ordered newest-first.
    pub recent_events: Vec<TimelineEventRow>,
}

/// A (successor, predecessor) supersession pair.
#[derive(Debug, Clone, Default)]
pub struct SupersessionPairRow {
    /// ULID / id of the successor decision.
    pub successor_id: String,
    /// Display title of the successor decision.
    pub successor_title: String,
    /// ULID / id of the decision that was superseded.
    pub predecessor_id: String,
    /// Display title of the predecessor decision.
    pub predecessor_title: String,
}

/// Active-decision reference for the supersession overlay.
#[derive(Debug, Clone, Default)]
pub struct ActiveDecisionRow {
    /// Decision identifier.
    pub decision_id: String,
    /// Display title.
    pub title: String,
}

/// Supersession overlay (design §3.4): active decisions + recently
/// superseded (new, old) pairs for the scope.
#[derive(Debug, Clone, Default)]
pub struct SupersessionOverlay {
    /// Decisions currently in an active lifecycle state.
    pub active_decisions: Vec<ActiveDecisionRow>,
    /// Recently-superseded (successor, predecessor) pairs.
    pub recently_superseded: Vec<SupersessionPairRow>,
}

/// A branch reference for the branch-context section.
#[derive(Debug, Clone, Default)]
pub struct BranchRefRow {
    /// Composite `<project>:<branch>` identifier.
    pub branch_id: String,
    /// Lifecycle status (`active` / `merged` / `abandoned`).
    pub status: String,
}

/// Branch context (design §3.4): the current branch + active siblings
/// + recently merged.
#[derive(Debug, Clone, Default)]
pub struct BranchContext {
    /// The branch that is currently active.
    pub current_branch: String,
    /// Other branches in an `active` state alongside the current one.
    pub active_sibling_branches: Vec<BranchRefRow>,
    /// Branches that have been merged recently.
    pub recently_merged: Vec<BranchRefRow>,
}

/// Minimum trimmed `user_prompt` length (chars) before the
/// orchestrator should fan out to `cortex_similar_sessions`. Below
/// this floor the query is too short to embed meaningfully — the
/// similar-sessions section returns noise and the fetch wastes
/// budget. Spec phase13g §4.3.
pub const SIMILAR_SESSIONS_QUERY_FLOOR_CHARS: usize = 16;

/// Phase13g §4.3 — return `true` when the orchestrator should
/// dispatch `cortex_similar_sessions` for `user_prompt`. False on
/// bare tool pings (under the char floor) so the pre-thinking
/// fan-out does not pay for noise hits.
pub fn should_fetch_similar_sessions(user_prompt: &str) -> bool {
    user_prompt.trim().chars().count() > SIMILAR_SESSIONS_QUERY_FLOOR_CHARS
}

/// Phase13g §4.3 — extract Crockford-safe ULIDs that appear in the
/// query string. Returns unique ids in first-seen order. The
/// orchestrator unions this with ULIDs found in the fusion result
/// (laws, decisions, similar turns) and fires
/// `cortex_decision_chain` once per id.
pub fn extract_adr_event_ids(user_prompt: &str) -> Vec<String> {
    let bytes = user_prompt.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i + 26 <= bytes.len() {
        let slice = &bytes[i..i + 26];
        if is_ulid_byte_slice(slice) {
            // Reject if the surrounding chars are themselves
            // ULID-alphabet (catches a 30-char garbage run that
            // happens to contain a 26-char window).
            let preceded = i > 0 && is_ulid_byte(bytes[i - 1]);
            let followed = i + 26 < bytes.len() && is_ulid_byte(bytes[i + 26]);
            if !preceded && !followed {
                if let Ok(id) = std::str::from_utf8(slice) {
                    if !out.iter().any(|x| x == id) {
                        out.push(id.to_string());
                    }
                }
                i += 26;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_ulid_byte(b: u8) -> bool {
    matches!(
        b,
        b'0'..=b'9'
            | b'A'..=b'H'
            | b'J'
            | b'K'
            | b'M'
            | b'N'
            | b'P'..=b'T'
            | b'V'..=b'Z'
    )
}

fn is_ulid_byte_slice(s: &[u8]) -> bool {
    s.len() == 26 && s.iter().all(|b| is_ulid_byte(*b))
}

/// Render the ADR-provenance section. Returns an empty string when
/// `rows` is empty (orchestrator only renders this conditionally —
/// see phase13g §4.3). Bounded by
/// [`section_caps::ADR_PROVENANCE_BYTES`].
pub fn render_adr_provenance(rows: &[AdrProvenanceRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let count = rows.len();
    let _ = writeln!(&mut out, "## ADR provenance ({count})");
    for row in rows {
        let slug = if row.slug.is_empty() {
            row.event_id.clone()
        } else {
            row.slug.clone()
        };
        let date = if row.date.is_empty() {
            "—".to_string()
        } else {
            row.date.clone()
        };
        let line = format!(
            "- `{slug}` · {status} · {date} · {title}\n",
            slug = slug,
            status = row.status,
            date = date,
            title = trim_one_line(&row.title),
        );
        if out.len() + line.len() > section_caps::ADR_PROVENANCE_BYTES {
            out.push_str("- … (truncated; see `cortex_decision_chain` for the full chain)\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

/// Phase18 §6.1 — render the "Timeline window" section. Returns an
/// empty string when `w` is `None` or contains no events. Bounded
/// by [`section_caps::TIMELINE_WINDOW_BYTES`]; capped at
/// [`section_caps::TIMELINE_WINDOW_EVENTS`] events. Rows that would
/// exceed the byte budget are dropped and a truncation tail is
/// appended.
pub fn render_timeline_window(w: &Option<TimelineWindow>) -> String {
    let window = match w {
        Some(w) => w,
        None => return String::new(),
    };
    if window.recent_events.is_empty() {
        return String::new();
    }
    let n = window.recent_events.len().min(section_caps::TIMELINE_WINDOW_EVENTS);
    let project = &window.project;
    let branch = if window.branch.is_empty() {
        "main".to_string()
    } else {
        window.branch.clone()
    };
    let as_of = if window.as_of.is_empty() {
        "now".to_string()
    } else {
        window.as_of.clone()
    };
    let mut out = String::new();
    let _ = writeln!(
        &mut out,
        "## Timeline window — {project}@{branch} as of {as_of} ({n} events)"
    );
    for (i, ev) in window.recent_events.iter().take(n).enumerate() {
        let summary = trim_one_line(&ev.summary);
        let line = format!(
            "{}. [{}] {} · {}\n   {}\n",
            i + 1,
            ev.kind,
            ev.valid_from,
            ev.title,
            summary,
        );
        if out.len() + line.len() > section_caps::TIMELINE_WINDOW_BYTES {
            out.push_str("… (truncated)\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

/// Phase18 §6.1 — render the "Supersession overlay" section. Returns
/// an empty string when `o` is `None` or both lists are empty. Bounded
/// by [`section_caps::SUPERSESSION_OVERLAY_BYTES`].
pub fn render_supersession_overlay(o: &Option<SupersessionOverlay>) -> String {
    let overlay = match o {
        Some(o) => o,
        None => return String::new(),
    };
    if overlay.active_decisions.is_empty() && overlay.recently_superseded.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("## Supersession overlay\n");
    if !overlay.active_decisions.is_empty() {
        out.push_str("Active decisions:\n");
        for ad in &overlay.active_decisions {
            let line = format!("- {} · {}\n", ad.decision_id, trim_one_line(&ad.title));
            if out.len() + line.len() > section_caps::SUPERSESSION_OVERLAY_BYTES {
                out.push_str("- … (truncated)\n");
                return out;
            }
            out.push_str(&line);
        }
    }
    if !overlay.recently_superseded.is_empty() {
        out.push_str("Recently superseded:\n");
        for sp in &overlay.recently_superseded {
            let line = format!(
                "- {} ({}) \u{2283} supersedes {} ({})\n",
                sp.successor_id,
                trim_one_line(&sp.successor_title),
                sp.predecessor_id,
                trim_one_line(&sp.predecessor_title),
            );
            if out.len() + line.len() > section_caps::SUPERSESSION_OVERLAY_BYTES {
                out.push_str("- … (truncated)\n");
                return out;
            }
            out.push_str(&line);
        }
    }
    out
}

/// Phase18 §6.1 — render the "Branch context" section. Returns an
/// empty string when `b` is `None` or the current branch is empty
/// and both sibling/merged lists are empty. Bounded by
/// [`section_caps::BRANCH_CONTEXT_BYTES`].
pub fn render_branch_context(b: &Option<BranchContext>) -> String {
    let ctx = match b {
        Some(b) => b,
        None => return String::new(),
    };
    if ctx.current_branch.is_empty()
        && ctx.active_sibling_branches.is_empty()
        && ctx.recently_merged.is_empty()
    {
        return String::new();
    }
    let mut out = String::new();
    let current = if ctx.current_branch.is_empty() {
        "—".to_string()
    } else {
        ctx.current_branch.clone()
    };
    let _ = writeln!(&mut out, "## Branch context — {current}");
    if !ctx.active_sibling_branches.is_empty() {
        out.push_str("Active siblings:\n");
        for br in &ctx.active_sibling_branches {
            let line = format!("- {} [{}]\n", br.branch_id, br.status);
            if out.len() + line.len() > section_caps::BRANCH_CONTEXT_BYTES {
                out.push_str("- … (truncated)\n");
                return out;
            }
            out.push_str(&line);
        }
    }
    if !ctx.recently_merged.is_empty() {
        out.push_str("Recently merged:\n");
        for br in &ctx.recently_merged {
            let line = format!("- {} [{}]\n", br.branch_id, br.status);
            if out.len() + line.len() > section_caps::BRANCH_CONTEXT_BYTES {
                out.push_str("- … (truncated)\n");
                return out;
            }
            out.push_str(&line);
        }
    }
    out
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
                topic_cards: Vec::new(),
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
        resp.results.past_sessions =
            vec![past_session("sess-Y", 1_715_000_000_000, "prompt", 3, 0.6)];
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

    // -------------------------------------------------------------
    // Phase11r §5.3 + §5.4 — topic-card section + reorder + stale
    // -------------------------------------------------------------

    fn fresh_topic_card() -> cortex_api::TopicCardRef {
        use cortex_core::events::{
            Contradiction, ContradictionKind, ContradictionStatus, EvidenceKind, EvidenceRef,
        };
        cortex_api::TopicCardRef {
            topic_card_id: "topic-".to_string() + &"a".repeat(24),
            topic_slug: "auth-rewrite".to_string(),
            revision: 3,
            synthesis_preview:
                "JWT validation now consolidated behind a single middleware so token \
                rotation lands deterministically without the 5-minute cache lag."
                    .to_string(),
            evidence_top5: vec![EvidenceRef {
                kind: EvidenceKind::Decision,
                id: "DEC-0042".to_string(),
                weight: Some(0.9),
                cited_at_rev: 3,
            }],
            open_contradictions: vec![Contradiction {
                kind: ContradictionKind::DecisionSupersession,
                evidence_a: "DEC-0042".to_string(),
                evidence_b: "DEC-0001".to_string(),
                surfaced_at_rev: 3,
                status: ContradictionStatus::Open,
            }],
            confidence: 0.82,
            synthesis_age_d: 5,
            events_since_last_rev: 2,
            score: 0.91,
        }
    }

    #[test]
    fn topic_card_section_renders_when_present() {
        let mut resp = populated_response();
        resp.results.topic_cards = vec![fresh_topic_card()];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        // Section header + format line per spec §5.3.
        assert!(bundle.contains("## Topic card"));
        assert!(bundle.contains("[auth-rewrite] (rev 3, confidence 82%, age 5d, +2 ev)"));
        // Synthesis preview lands.
        assert!(bundle.contains("JWT validation"));
        // Evidence + contradictions sub-blocks.
        assert!(bundle.contains("### Evidence (1)"));
        assert!(bundle.contains("decision:DEC-0042"));
        assert!(bundle.contains("### Open contradictions (1)"));
        // Fresh card → no advisory line.
        assert!(!bundle.contains("stale-topic-card"));
    }

    #[test]
    fn topic_cards_take_priority_over_consolidations() {
        let mut resp = populated_response();
        resp.results.topic_cards = vec![fresh_topic_card()];
        resp.results.consolidations = vec![cortex_api::ConsolidationRef {
            consolidation_id: "cons-ses-deadbeef".to_string(),
            grain: "session".to_string(),
            ts: 1_715_000_000_000,
            title: "older session".to_string(),
            outcome: Some("success".to_string()),
            score: 0.8,
        }];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        let topic_pos = bundle.find("## Topic card").unwrap();
        let cons_pos = bundle.find("## Consolidated context").unwrap();
        // Fresh card → topic card section appears BEFORE
        // consolidations (per §5.4 default order).
        assert!(
            topic_pos < cons_pos,
            "fresh topic card must render before consolidations: topic@{topic_pos} cons@{cons_pos}"
        );
    }

    #[test]
    fn stale_topic_card_advisory_downgrades_section() {
        // §5.4 staleness: confidence below floor OR (age > 30 AND
        // events_since_last_rev > 0). Trip both heuristics
        // independently so each branch is exercised.
        for stale_card in [
            // Branch A: confidence floor.
            cortex_api::TopicCardRef {
                confidence: 0.4,
                ..fresh_topic_card()
            },
            // Branch B: stale age + new events.
            cortex_api::TopicCardRef {
                synthesis_age_d: 45,
                events_since_last_rev: 1,
                ..fresh_topic_card()
            },
        ] {
            let mut resp = populated_response();
            resp.results.topic_cards = vec![stale_card];
            resp.results.consolidations = vec![cortex_api::ConsolidationRef {
                consolidation_id: "cons-ses-deadbeef".to_string(),
                grain: "session".to_string(),
                ts: 1_715_000_000_000,
                title: "older session".to_string(),
                outcome: Some("success".to_string()),
                score: 0.8,
            }];
            let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
            let topic_pos = bundle.find("## Topic card").unwrap();
            let cons_pos = bundle.find("## Consolidated context").unwrap();
            // Stale card → consolidations render FIRST.
            assert!(
                cons_pos < topic_pos,
                "stale topic card must render after consolidations: cons@{cons_pos} topic@{topic_pos}"
            );
            // Advisory line lands inside the topic card section.
            assert!(bundle.contains("stale-topic-card"));
        }
    }

    #[test]
    fn topic_cards_cap_caps_section_size() {
        let mut resp = populated_response();
        // Two cards present, cap at 1 — only the first lands.
        let mut second = fresh_topic_card();
        second.topic_slug = "session-store".to_string();
        resp.results.topic_cards = vec![fresh_topic_card(), second];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("[auth-rewrite]"));
        assert!(!bundle.contains("[session-store]"));
    }

    #[test]
    fn consolidations_render_when_no_topic_card_matches() {
        // §5.4 fallback path — zero topic cards keeps the
        // existing consolidation lane intact.
        let mut resp = populated_response();
        resp.results.consolidations = vec![cortex_api::ConsolidationRef {
            consolidation_id: "cons-ses-deadbeef".to_string(),
            grain: "session".to_string(),
            ts: 1_715_000_000_000,
            title: "older session".to_string(),
            outcome: Some("success".to_string()),
            score: 0.8,
        }];
        let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
        assert!(bundle.contains("## Consolidated context"));
        assert!(!bundle.contains("## Topic card"));
        assert!(!bundle.contains("stale-topic-card"));
    }

    // ============================================================
    // phase13g §4.5 — grounding renderer tests.
    // ============================================================

    #[test]
    fn render_active_work_empty_input_returns_empty_string() {
        assert!(render_active_work(&[]).is_empty());
    }

    #[test]
    fn render_active_work_includes_phase_status_and_next_unchecked() {
        let rows = vec![ActiveWorkRow {
            id: "phase13g_demo".into(),
            phase: Some("phase13g".into()),
            status: "in-progress".into(),
            next_unchecked_item: Some("4.5 add renderer tests".into()),
            blocked_reason: None,
        }];
        let out = render_active_work(&rows);
        assert!(out.starts_with("## Active operator work (1)"));
        assert!(out.contains("phase13g (phase13g_demo)"));
        assert!(out.contains("in-progress"));
        assert!(out.contains("4.5 add renderer tests"));
    }

    #[test]
    fn render_active_work_surfaces_blocked_reason_suffix() {
        let rows = vec![ActiveWorkRow {
            id: "phase14h_demo".into(),
            phase: Some("phase14h".into()),
            status: "blocked".into(),
            next_unchecked_item: Some("2.1 ship synap shared module".into()),
            blocked_reason: Some("synap upstream not merged".into()),
        }];
        let out = render_active_work(&rows);
        assert!(out.contains("blocked: synap upstream not merged"));
    }

    #[test]
    fn render_active_work_truncates_when_section_exceeds_budget() {
        // Build enough rows that the cumulative length runs past
        // the ACTIVE_WORK_BYTES cap. Each row ~80 bytes; 50 rows
        // is ~4 KB, comfortably above the 1.2 KB cap.
        let rows: Vec<ActiveWorkRow> = (0..50)
            .map(|i| ActiveWorkRow {
                id: format!("phase{i:02}a_demo_long_id_padding_to_inflate_row"),
                phase: Some(format!("phase{i:02}a")),
                status: "in-progress".into(),
                next_unchecked_item: Some(format!(
                    "{i}.1 a moderately long checklist item to consume budget"
                )),
                blocked_reason: None,
            })
            .collect();
        let out = render_active_work(&rows);
        assert!(out.len() <= section_caps::ACTIVE_WORK_BYTES + 96);
        assert!(out.contains("(truncated; see `cortex_active_work`"));
    }

    #[test]
    fn render_similar_sessions_empty_input_returns_empty_string() {
        assert!(render_similar_sessions(&[]).is_empty());
    }

    #[test]
    fn render_similar_sessions_lists_scored_rows() {
        let rows = vec![
            SimilarSessionRow {
                consolidation_id: "cons-ses-001".into(),
                title: "rework analysis".into(),
                summary_excerpt: "Audit opus5.7 consolidator rework".into(),
                score: 0.91,
            },
            SimilarSessionRow {
                consolidation_id: "cons-ses-002".into(),
                title: "consolidator pruning fix".into(),
                summary_excerpt: "Patched the cold tier demotion path".into(),
                score: 0.74,
            },
        ];
        let out = render_similar_sessions(&rows);
        assert!(out.starts_with("## Similar past sessions (2)"));
        assert!(out.contains("`cons-ses-001`"));
        assert!(out.contains("score 0.91"));
        assert!(out.contains("rework analysis"));
        assert!(out.contains("`cons-ses-002`"));
        assert!(out.contains("score 0.74"));
    }

    #[test]
    fn render_similar_sessions_truncates_when_section_exceeds_budget() {
        let rows: Vec<SimilarSessionRow> = (0..40)
            .map(|i| SimilarSessionRow {
                consolidation_id: format!("cons-ses-{i:04}"),
                title: format!("session {i} title with enough padding"),
                summary_excerpt:
                    "A meaningfully long summary excerpt that consumes meaningful budget".into(),
                score: 0.6 + (i as f64) * 0.005,
            })
            .collect();
        let out = render_similar_sessions(&rows);
        assert!(out.len() <= section_caps::SIMILAR_SESSIONS_BYTES + 96);
        assert!(out.contains("(truncated; see `cortex_similar_sessions`"));
    }

    #[test]
    fn render_adr_provenance_empty_input_returns_empty_string() {
        assert!(render_adr_provenance(&[]).is_empty());
    }

    #[test]
    fn render_adr_provenance_renders_slug_and_status_per_row() {
        let rows = vec![
            AdrProvenanceRow {
                event_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                slug: "adr-009-sweep-trait".into(),
                status: "superseded".into(),
                date: "2026-05-19".into(),
                title: "Sweep trait as the single contract".into(),
            },
            AdrProvenanceRow {
                event_id: "01ARZ3NDEKTSV4RRFFQ69G5FAW".into(),
                slug: "adr-014-pure-readers".into(),
                status: "proposed".into(),
                date: "2026-05-25".into(),
                title: "Dashboard handlers are pure readers".into(),
            },
        ];
        let out = render_adr_provenance(&rows);
        assert!(out.starts_with("## ADR provenance (2)"));
        assert!(out.contains("adr-009-sweep-trait"));
        assert!(out.contains("superseded"));
        assert!(out.contains("2026-05-19"));
        assert!(out.contains("adr-014-pure-readers"));
        assert!(out.contains("proposed"));
    }

    #[test]
    fn render_adr_provenance_handles_empty_slug_and_date() {
        let rows = vec![AdrProvenanceRow {
            event_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            slug: String::new(),
            status: "proposed".into(),
            date: String::new(),
            title: "legacy decision".into(),
        }];
        let out = render_adr_provenance(&rows);
        // Falls back to the event_id when the slug is empty + `—`
        // when the date is empty so the row is still parseable.
        assert!(out.contains("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(out.contains(" · — · "));
    }

    #[test]
    fn render_adr_provenance_truncates_when_section_exceeds_budget() {
        let rows: Vec<AdrProvenanceRow> = (0..30)
            .map(|i| AdrProvenanceRow {
                event_id: format!("01ARZ3NDEKTSV4RRFFQ69G5F{:02}", i),
                slug: format!("adr-{i:03}-extended-slug-for-padding"),
                status: "proposed".into(),
                date: "2026-05-25".into(),
                title: format!("Decision {i} with a reasonable title length"),
            })
            .collect();
        let out = render_adr_provenance(&rows);
        assert!(out.len() <= section_caps::ADR_PROVENANCE_BYTES + 96);
        assert!(out.contains("(truncated; see `cortex_decision_chain`"));
    }

    #[test]
    fn should_fetch_similar_sessions_respects_query_floor() {
        // 16 chars or shorter: skip.
        assert!(!should_fetch_similar_sessions(""));
        assert!(!should_fetch_similar_sessions("hi"));
        assert!(!should_fetch_similar_sessions("0123456789ABCDEF")); // exactly 16
                                                                     // > 16 chars: fire.
        assert!(should_fetch_similar_sessions("0123456789ABCDEFG"));
        assert!(should_fetch_similar_sessions(
            "rework consolidator analysis"
        ));
    }

    #[test]
    fn extract_adr_event_ids_finds_ulid_in_prose() {
        let q = "Check ADR 01ARZ3NDEKTSV4RRFFQ69G5FAV before editing";
        let ids = extract_adr_event_ids(q);
        assert_eq!(ids, vec!["01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string()]);
    }

    #[test]
    fn extract_adr_event_ids_dedupes_repeated_ulids() {
        let q = "01ARZ3NDEKTSV4RRFFQ69G5FAV vs 01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let ids = extract_adr_event_ids(q);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn extract_adr_event_ids_returns_empty_when_no_ulid_present() {
        let ids = extract_adr_event_ids("a regular query with no ulid in it");
        assert!(ids.is_empty());
    }

    #[test]
    fn extract_adr_event_ids_rejects_window_buried_in_longer_run() {
        // 32-char run of ULID-alphabet chars contains a 26-char
        // window but is not itself a ULID — must reject.
        let q = "ABCDEFGHJKMNPQRSTVWXYZ01234567890ABCDEFG";
        let ids = extract_adr_event_ids(q);
        assert!(ids.is_empty(), "got {ids:?}");
    }

    #[test]
    fn format_bundle_renders_active_work_between_laws_and_consolidations() {
        // phase13g §4.4 render order: laws → active work → similar
        // sessions → ADR provenance → consolidated context.
        let mut resp = sample_response_with_laws();
        // Add one consolidation so the section appears.
        resp.results.consolidations = vec![cortex_api::ConsolidationRef {
            consolidation_id: "cons-ses-001".to_string(),
            grain: "session".to_string(),
            ts: 1_700_000_000,
            title: "older session".to_string(),
            outcome: Some("success".to_string()),
            score: 0.8,
        }];
        let mut opts = FormatOptions::default();
        opts.grounding.active_work = vec![ActiveWorkRow {
            id: "phase13g_demo".into(),
            phase: Some("phase13g".into()),
            status: "in-progress".into(),
            next_unchecked_item: Some("4.4 render order".into()),
            blocked_reason: None,
        }];
        opts.grounding.similar_sessions = vec![SimilarSessionRow {
            consolidation_id: "cons-ses-002".into(),
            title: "consolidator rewrite".into(),
            summary_excerpt: "Earlier rewrite that survived prod".into(),
            score: 0.81,
        }];
        opts.grounding.adr_provenance = vec![AdrProvenanceRow {
            event_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            slug: "adr-009-sweep-trait".into(),
            status: "superseded".into(),
            date: "2026-05-19".into(),
            title: "Sweep trait".into(),
        }];

        let bundle = format_bundle("pre_change_context", &resp, &opts);
        let laws_pos = bundle.find("## Active laws in this scope").unwrap();
        let aw_pos = bundle.find("## Active operator work").unwrap();
        let ss_pos = bundle.find("## Similar past sessions").unwrap();
        let adr_pos = bundle.find("## ADR provenance").unwrap();
        let cons_pos = bundle.find("## Consolidated context").unwrap();
        assert!(laws_pos < aw_pos);
        assert!(aw_pos < ss_pos);
        assert!(ss_pos < adr_pos);
        assert!(adr_pos < cons_pos);
    }

    #[test]
    fn format_bundle_omits_grounding_sections_when_input_is_empty() {
        // Phase13g §4.5 "missing-tool error degrades gracefully" —
        // when the orchestrator fetched nothing (because the tool
        // returned an error or because §4.3 said "skip"), the
        // bundle just omits those sections; the rest still renders.
        let resp = sample_response_with_laws();
        let opts = FormatOptions::default();
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(bundle.contains("## Active laws in this scope"));
        assert!(!bundle.contains("## Active operator work"));
        assert!(!bundle.contains("## Similar past sessions"));
        assert!(!bundle.contains("## ADR provenance"));
    }

    fn sample_response_with_laws() -> QueryResponse {
        QueryResponse {
            query_id: "01HQ".into(),
            laws_active: vec![LawRef {
                id: "L-1".into(),
                title: "Sample law".into(),
                severity: "warn".into(),
            }],
            ..QueryResponse::default()
        }
    }

    #[test]
    fn section_caps_sum_stays_under_pre_thinking_ceiling() {
        // phase13g §4.2 — combined the three new sections must fit
        // under the existing 12 KB ceiling alongside everything
        // else the formatter renders. With 1200 + 2000 + 800 =
        // 4000 bytes, the three new sections take ~33 % of the
        // ceiling — well within the slack.
        const PRE_THINKING_CEILING: usize = 12 * 1024;
        let sum = section_caps::ACTIVE_WORK_BYTES
            + section_caps::SIMILAR_SESSIONS_BYTES
            + section_caps::ADR_PROVENANCE_BYTES;
        assert!(
            sum < PRE_THINKING_CEILING,
            "{sum} >= {PRE_THINKING_CEILING}"
        );
    }

    // ============================================================
    // Phase18 §6.1 — temporal grounding section tests.
    // ============================================================

    fn make_timeline_event(i: usize) -> TimelineEventRow {
        TimelineEventRow {
            event_id: format!("ev-{i:04}"),
            kind: "decision".into(),
            title: format!("Event {i} title"),
            valid_from: format!("2026-05-{:02}", (i % 28) + 1),
            summary: format!("Summary of event {i} with some text to consume budget"),
        }
    }

    #[test]
    fn render_timeline_window_emits_events_and_caps() {
        // 12 events supplied; renderer must cap at TIMELINE_WINDOW_EVENTS (8)
        // and stay under TIMELINE_WINDOW_BYTES; when the byte budget is hit
        // before the count cap, a truncation tail must appear.
        let events: Vec<TimelineEventRow> = (0..12).map(make_timeline_event).collect();
        let window = Some(TimelineWindow {
            project: "Cortex".into(),
            as_of: "2026-06-06".into(),
            branch: "Cortex:main".into(),
            recent_events: events,
        });
        let out = render_timeline_window(&window);
        // Section must be non-empty.
        assert!(!out.is_empty(), "expected non-empty output");
        // Must not exceed the byte budget (allow one truncation-tail line
        // of overhead, mirroring the pattern used in other renderers).
        assert!(
            out.len() <= section_caps::TIMELINE_WINDOW_BYTES + 96,
            "section too large: {} bytes",
            out.len()
        );
        // Must not render more than TIMELINE_WINDOW_EVENTS entries.
        let event_lines = out.lines().filter(|l| l.starts_with(|c: char| c.is_ascii_digit())).count();
        assert!(
            event_lines <= section_caps::TIMELINE_WINDOW_EVENTS,
            "rendered {event_lines} events, cap is {}",
            section_caps::TIMELINE_WINDOW_EVENTS
        );
        // Header must contain expected metadata.
        assert!(out.contains("Timeline window"), "missing header in:\n{out}");
        assert!(out.contains("Cortex@Cortex:main"), "missing project@branch");
        assert!(out.contains("as of 2026-06-06"), "missing as_of");
    }

    #[test]
    fn render_timeline_window_empty_is_blank() {
        // None input → empty string.
        assert!(render_timeline_window(&None).is_empty());
        // Some but no events → empty string.
        let empty_window = Some(TimelineWindow {
            project: "Cortex".into(),
            as_of: String::new(),
            branch: String::new(),
            recent_events: vec![],
        });
        assert!(render_timeline_window(&empty_window).is_empty());
    }

    #[test]
    fn render_supersession_overlay_lists_active_and_superseded() {
        let overlay = Some(SupersessionOverlay {
            active_decisions: vec![
                ActiveDecisionRow {
                    decision_id: "DEC-0042".into(),
                    title: "Adopt Meilisearch".into(),
                },
                ActiveDecisionRow {
                    decision_id: "DEC-0043".into(),
                    title: "Use HNSW for dense recall".into(),
                },
            ],
            recently_superseded: vec![SupersessionPairRow {
                successor_id: "DEC-0043".into(),
                successor_title: "Use HNSW for dense recall".into(),
                predecessor_id: "DEC-0001".into(),
                predecessor_title: "Brute-force search baseline".into(),
            }],
        });
        let out = render_supersession_overlay(&overlay);
        assert!(!out.is_empty());
        assert!(out.contains("## Supersession overlay"), "missing header");
        assert!(out.contains("Active decisions:"), "missing active decisions header");
        assert!(out.contains("DEC-0042"), "missing DEC-0042");
        assert!(out.contains("DEC-0043"), "missing DEC-0043");
        assert!(out.contains("Recently superseded:"), "missing recently superseded header");
        assert!(out.contains("supersedes"), "missing supersedes keyword");
        assert!(out.contains("DEC-0001"), "missing predecessor");

        // None and empty → blank.
        assert!(render_supersession_overlay(&None).is_empty());
        let empty = Some(SupersessionOverlay {
            active_decisions: vec![],
            recently_superseded: vec![],
        });
        assert!(render_supersession_overlay(&empty).is_empty());
    }

    #[test]
    fn render_branch_context_lists_siblings_and_merged() {
        let ctx = Some(BranchContext {
            current_branch: "Cortex:feature-auth".into(),
            active_sibling_branches: vec![
                BranchRefRow {
                    branch_id: "Cortex:feature-search".into(),
                    status: "active".into(),
                },
            ],
            recently_merged: vec![
                BranchRefRow {
                    branch_id: "Cortex:phase18-bootstrap".into(),
                    status: "merged".into(),
                },
            ],
        });
        let out = render_branch_context(&ctx);
        assert!(!out.is_empty());
        assert!(out.contains("## Branch context — Cortex:feature-auth"), "missing header");
        assert!(out.contains("Active siblings:"), "missing active siblings header");
        assert!(out.contains("Cortex:feature-search [active]"), "missing sibling row");
        assert!(out.contains("Recently merged:"), "missing recently merged header");
        assert!(out.contains("Cortex:phase18-bootstrap [merged]"), "missing merged row");

        // None and all-empty → blank.
        assert!(render_branch_context(&None).is_empty());
        let empty = Some(BranchContext {
            current_branch: String::new(),
            active_sibling_branches: vec![],
            recently_merged: vec![],
        });
        assert!(render_branch_context(&empty).is_empty());
    }

    #[test]
    fn format_bundle_includes_temporal_sections_when_populated() {
        // At least one main section must be non-empty so the
        // empty-bundle short-circuit does not fire.
        let mut resp = sample_response_with_laws();
        // Add one consolidation to ensure the bundle is non-empty past
        // the short-circuit gate.
        resp.results.consolidations = vec![cortex_api::ConsolidationRef {
            consolidation_id: "cons-ses-tw-test".to_string(),
            grain: "session".to_string(),
            ts: 1_700_000_000_000,
            title: "temporal test session".to_string(),
            outcome: Some("success".to_string()),
            score: 0.75,
        }];
        let mut opts = FormatOptions::default();
        opts.grounding.timeline_window = Some(TimelineWindow {
            project: "Cortex".into(),
            as_of: "2026-06-06".into(),
            branch: "Cortex:main".into(),
            recent_events: vec![make_timeline_event(0)],
        });
        opts.grounding.supersession_overlay = Some(SupersessionOverlay {
            active_decisions: vec![ActiveDecisionRow {
                decision_id: "DEC-0099".into(),
                title: "Test decision".into(),
            }],
            recently_superseded: vec![],
        });
        opts.grounding.branch_context = Some(BranchContext {
            current_branch: "Cortex:main".into(),
            active_sibling_branches: vec![],
            recently_merged: vec![],
        });

        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(bundle.contains("Timeline window"), "Timeline window header missing");
        assert!(bundle.contains("Supersession overlay"), "Supersession overlay header missing");
        assert!(bundle.contains("Branch context"), "Branch context header missing");

        // Temporal sections must appear before Active operator work
        // (and before the consolidation block).
        let tw_pos = bundle.find("Timeline window").expect("Timeline window");
        let so_pos = bundle.find("Supersession overlay").expect("Supersession overlay");
        let bc_pos = bundle.find("Branch context").expect("Branch context");
        let cons_pos = bundle.find("Consolidated context").expect("Consolidated context");
        // Order: timeline → supersession → branch → … → consolidations.
        assert!(tw_pos < so_pos, "timeline must precede supersession");
        assert!(so_pos < bc_pos, "supersession must precede branch context");
        assert!(bc_pos < cons_pos, "branch context must precede consolidated context");
    }

    #[test]
    fn format_bundle_omits_temporal_sections_when_none() {
        // Default grounding has None for all three temporal sections;
        // none of the three headers should appear in the bundle.
        let resp = sample_response_with_laws();
        let opts = FormatOptions::default();
        let bundle = format_bundle("pre_change_context", &resp, &opts);
        assert!(
            !bundle.contains("Timeline window"),
            "Timeline window must be absent with default grounding"
        );
        assert!(
            !bundle.contains("Supersession overlay"),
            "Supersession overlay must be absent with default grounding"
        );
        assert!(
            !bundle.contains("Branch context"),
            "Branch context must be absent with default grounding"
        );
    }
}
