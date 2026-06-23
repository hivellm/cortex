//! Budget clipper — six-step trim ladder. Spec 12 §Budget-aware
//! section caps:
//!
//! 1. Drop graph neighbours (if present).
//! 2. Trim snippets to their `why` + first 3 lines of `text`.
//! 3. Halve the snippets count.
//! 4. Halve the similar-turns count.
//! 5. Truncate decision bodies to 160 chars.
//! 6. As a last resort, drop snippets entirely.
//!
//! The laws section is **never** trimmed — load-bearing per spec 12
//! Decision 2.

use cortex_api::QueryResponse;
use serde::{Deserialize, Serialize};

use crate::formatter::{self, format_bundle, FormatOptions, SnippetTrim};

/// One step of the trim ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimStep {
    /// Step 1: drop graph neighbours.
    DropGraph,
    /// Step 2: trim snippets to `why` + 3 lines of body.
    SlimSnippets,
    /// Step 3: halve the snippets count.
    HalveSnippets,
    /// Step 4: halve the similar-turns count.
    HalveTurns,
    /// Step 5: truncate decision bodies.
    TruncateDecisions,
    /// Step 6: drop snippets entirely.
    DropSnippets,
}

/// Outcome of [`clip_to_budget`]. Carries the final bundle plus the
/// list of trim steps the clipper had to apply, so the audit pass
/// can record how aggressive the truncation was.
#[derive(Debug, Clone)]
pub struct ClippedBundle {
    /// Final Markdown bundle.
    pub bundle: String,
    /// Steps applied, in order.
    pub steps: Vec<TrimStep>,
    /// Final byte length of `bundle`.
    pub bytes: usize,
    /// Phase18 §6.2 — per-section rendered counts reflecting the
    /// FINAL (post-trim) caps. Only sections with count > 0 are
    /// present. Built from [`formatter::count_sections`] after every
    /// return point so the counts always match the opts actually used.
    pub section_counts: std::collections::BTreeMap<String, u32>,
}

/// Clip `bundle_bytes`-bound bundle. Returns the result plus the
/// trim ladder it walked. Empty input → empty output.
pub fn clip_to_budget(
    intent: &str,
    response: &QueryResponse,
    bundle_bytes: usize,
) -> ClippedBundle {
    // Build a `ClippedBundle` with section_counts reflecting `opts` at
    // whichever point in the trim ladder we stop. Extracted so every
    // return site uses identical construction.
    let finish = |bundle: String, steps: Vec<TrimStep>, opts: &FormatOptions| -> ClippedBundle {
        let bytes = bundle.len();
        let section_counts = formatter::count_sections(response, opts);
        ClippedBundle {
            bundle,
            steps,
            bytes,
            section_counts,
        }
    };

    let mut opts = FormatOptions::default();
    let mut steps: Vec<TrimStep> = Vec::new();
    let mut bundle = format_bundle(intent, response, &opts);
    if bundle.len() <= bundle_bytes {
        return finish(bundle, steps, &opts);
    }

    // Step 1 — drop graph neighbours.
    if opts.graph_cap > 0 {
        opts.graph_cap = 0;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::DropGraph);
        if bundle.len() <= bundle_bytes {
            return finish(bundle, steps, &opts);
        }
    }

    // Step 2 — slim snippets.
    if opts.snippet_trim != SnippetTrim::SlimWhyPlusThree {
        opts.snippet_trim = SnippetTrim::SlimWhyPlusThree;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::SlimSnippets);
        if bundle.len() <= bundle_bytes {
            return finish(bundle, steps, &opts);
        }
    }

    // Step 3 — halve snippets.
    if opts.snippets_cap > 1 {
        opts.snippets_cap = (opts.snippets_cap / 2).max(1);
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::HalveSnippets);
        if bundle.len() <= bundle_bytes {
            return finish(bundle, steps, &opts);
        }
    }

    // Step 4 — halve similar turns.
    if opts.similar_turns_cap > 1 {
        opts.similar_turns_cap = (opts.similar_turns_cap / 2).max(1);
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::HalveTurns);
        if bundle.len() <= bundle_bytes {
            return finish(bundle, steps, &opts);
        }
    }

    // Step 5 — truncate decision bodies.
    if opts.decision_byte_cap > 160 {
        opts.decision_byte_cap = 160;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::TruncateDecisions);
        if bundle.len() <= bundle_bytes {
            return finish(bundle, steps, &opts);
        }
    }

    // Step 6 — drop snippets entirely.
    if opts.snippets_cap > 0 {
        opts.snippets_cap = 0;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::DropSnippets);
    }

    finish(bundle, steps, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_api::{
        BudgetReport, DebugInfo, DecisionRef, GraphNeighbor, LaneTimings, LawRef, QueryResponse,
        ResultsBag, Scope, SimilarTurn, Snippet,
    };

    fn fat_response() -> QueryResponse {
        // 5 snippets, each carrying 8 KB of text → ~40 KB pre-clip.
        let snippets: Vec<Snippet> = (0..5)
            .map(|i| Snippet {
                rank: i + 1,
                source: "vector".into(),
                collection: None,
                repo: Some("R".into()),
                path: Some(format!("src/{i}.rs")),
                symbol: Some(format!("fn_{i}")),
                content_hash: None,
                text: "x".repeat(8 * 1024),
                body_truncated: false,
                score: 0.5,
                why: Some("why".into()),
                verified: None,
                verdict: None,
                class_level: None,
                class_compartments: vec![],
            })
            .collect();
        let decisions: Vec<DecisionRef> = (0..5)
            .map(|i| DecisionRef {
                rank: i + 1,
                id: format!("DEC-{i:04}"),
                title: "T".repeat(800),
                rationale_excerpt: None,
                status: "accepted".into(),
                ts: 1_700_000_000_000,
                score: 0.5,
                links: vec![],
            })
            .collect();
        let turns: Vec<SimilarTurn> = (0..5)
            .map(|i| SimilarTurn {
                turn_id: format!("01HX{i:02}"),
                ts: 1_700_000_000_000,
                model: "claude".into(),
                summary: "summary".repeat(60),
                score: 0.5,
                outcome: None,
            })
            .collect();
        let neighbours: Vec<GraphNeighbor> = (0..3)
            .map(|i| GraphNeighbor {
                from: format!("ToolCall:{i}"),
                relation: "TOUCHED".into(),
                to: format!("Artifact:{i}"),
                hops: 1,
            })
            .collect();
        QueryResponse {
            intent: "pre_change_context".into(),
            query_id: "01HFAT".into(),
            scope_resolved: Scope::default(),
            results: ResultsBag {
                snippets,
                decisions,
                violations: vec![],
                graph_neighbors: neighbours,
                similar_turns: turns,
                past_sessions: Vec::new(),
                consolidations: Vec::new(),
                topic_cards: Vec::new(),
            },
            laws_active: vec![LawRef {
                id: "LAW-007".into(),
                severity: "critical".into(),
                title: "no skip hooks".into(),
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
    fn fits_within_budget_short_circuits_with_no_steps() {
        let mut resp = fat_response();
        for s in resp.results.snippets.iter_mut() {
            s.text = "small".into();
        }
        for d in resp.results.decisions.iter_mut() {
            d.title = "T".into();
        }
        let clipped = clip_to_budget("pre_change_context", &resp, 32 * 1024);
        assert!(clipped.steps.is_empty());
        assert!(clipped.bytes <= 32 * 1024);
    }

    #[test]
    fn eighty_kb_payload_clips_to_under_32_kb() {
        let mut resp = fat_response();
        // Make graph render — bundle starts above the cap.
        resp.results
            .graph_neighbors
            .extend((0..20).map(|i| GraphNeighbor {
                from: format!("X{i}"),
                relation: "TOUCHED".into(),
                to: format!("Y{i}"),
                hops: 1,
            }));
        let clipped = clip_to_budget("pre_change_context", &resp, 32 * 1024);
        assert!(clipped.bytes <= 32 * 1024, "got {} bytes", clipped.bytes);
        // The clipper must keep the laws section intact.
        assert!(clipped.bundle.contains("Active laws in this scope"));
    }

    #[test]
    fn step_order_is_documented_drop_graph_first() {
        let mut resp = fat_response();
        // Graph rendering off by default; force-cap to enable.
        resp.results
            .graph_neighbors
            .extend((0..40).map(|i| GraphNeighbor {
                from: format!("X{i}"),
                relation: "TOUCHED".into(),
                to: format!("Y{i}"),
                hops: 1,
            }));
        // Tight budget so we walk the ladder.
        let clipped = clip_to_budget("pre_change_context", &resp, 4 * 1024);
        assert!(
            clipped
                .steps
                .first()
                .copied()
                .unwrap_or(TrimStep::DropSnippets)
                != TrimStep::DropSnippets
        );
        // The first applied step must be DropGraph (which is a
        // no-op in v1 since graph_cap defaults to 0); for that
        // case the next applied step is SlimSnippets.
        let first = clipped.steps[0];
        assert!(matches!(
            first,
            TrimStep::DropGraph | TrimStep::SlimSnippets
        ));
    }

    #[test]
    fn laws_are_never_dropped() {
        // Tighten budget to a few hundred bytes — even a tiny budget
        // must keep the law header.
        let resp = fat_response();
        let clipped = clip_to_budget("pre_change_context", &resp, 600);
        assert!(clipped.bundle.contains("LAW-007"));
        assert!(clipped.bundle.contains("Active laws in this scope"));
    }

    #[test]
    fn empty_response_returns_empty_bundle() {
        let resp = QueryResponse {
            intent: "free_search".into(),
            query_id: "q".into(),
            scope_resolved: Scope::default(),
            results: ResultsBag::default(),
            laws_active: Vec::new(),
            budget: Default::default(),
            debug: Default::default(),
            notice: None,
            clipped: None,
        };
        let clipped = clip_to_budget("free_search", &resp, 32 * 1024);
        assert!(clipped.bundle.is_empty());
        assert_eq!(clipped.bytes, 0);
        assert!(clipped.steps.is_empty());
    }

    #[test]
    fn section_counts_snippets_reflect_post_trim_cap() {
        // Phase18 §6.2 — after a trim step that drops snippets, the
        // section_counts map must NOT contain a "snippets" key (or must
        // contain 0), reflecting the final opts.snippets_cap == 0.
        //
        // Build a response large enough that the clipper walks all the
        // way to DropSnippets (step 6). A sub-100-byte budget with 5
        // fat snippets guarantees that path.
        let resp = fat_response();
        let clipped = clip_to_budget("pre_change_context", &resp, 200);

        // The DropSnippets step must have been applied.
        assert!(
            clipped.steps.contains(&TrimStep::DropSnippets),
            "expected DropSnippets to be applied; steps: {:?}",
            clipped.steps
        );
        // After dropping snippets, the count must be absent or zero.
        let snippet_count = clipped.section_counts.get("snippets").copied().unwrap_or(0);
        assert_eq!(
            snippet_count, 0,
            "section_counts[\"snippets\"] must be 0 after DropSnippets step"
        );
    }

    #[test]
    fn section_counts_present_when_no_trim_needed() {
        // When the bundle fits without trimming, section_counts should
        // reflect the default caps.
        let mut resp = fat_response();
        // Shrink all content so it fits easily.
        for s in resp.results.snippets.iter_mut() {
            s.text = "small".into();
        }
        for d in resp.results.decisions.iter_mut() {
            d.title = "T".into();
        }
        let clipped = clip_to_budget("pre_change_context", &resp, 32 * 1024);
        assert!(clipped.steps.is_empty());
        // Laws, decisions, turns, snippets should all be present in counts.
        assert!(
            clipped.section_counts.contains_key("laws"),
            "laws must appear in section_counts"
        );
        assert!(
            clipped.section_counts.contains_key("snippets"),
            "snippets must appear in section_counts"
        );
        // graph_neighbors is 0 by default (graph_cap == 0) → key must be absent.
        assert!(
            !clipped.section_counts.contains_key("graph_neighbors"),
            "graph_neighbors must be absent when graph_cap is 0"
        );
    }
}
