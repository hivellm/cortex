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

use crate::formatter::{format_bundle, FormatOptions, SnippetTrim};

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
}

/// Clip `bundle_bytes`-bound bundle. Returns the result plus the
/// trim ladder it walked. Empty input → empty output.
pub fn clip_to_budget(
    intent: &str,
    response: &QueryResponse,
    bundle_bytes: usize,
) -> ClippedBundle {
    let mut opts = FormatOptions::default();
    let mut steps: Vec<TrimStep> = Vec::new();
    let mut bundle = format_bundle(intent, response, &opts);
    if bundle.len() <= bundle_bytes {
        return ClippedBundle {
            bytes: bundle.len(),
            bundle,
            steps,
        };
    }

    // Step 1 — drop graph neighbours.
    if opts.graph_cap > 0 {
        opts.graph_cap = 0;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::DropGraph);
        if bundle.len() <= bundle_bytes {
            return ClippedBundle {
                bytes: bundle.len(),
                bundle,
                steps,
            };
        }
    }

    // Step 2 — slim snippets.
    if opts.snippet_trim != SnippetTrim::SlimWhyPlusThree {
        opts.snippet_trim = SnippetTrim::SlimWhyPlusThree;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::SlimSnippets);
        if bundle.len() <= bundle_bytes {
            return ClippedBundle {
                bytes: bundle.len(),
                bundle,
                steps,
            };
        }
    }

    // Step 3 — halve snippets.
    if opts.snippets_cap > 1 {
        opts.snippets_cap = (opts.snippets_cap / 2).max(1);
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::HalveSnippets);
        if bundle.len() <= bundle_bytes {
            return ClippedBundle {
                bytes: bundle.len(),
                bundle,
                steps,
            };
        }
    }

    // Step 4 — halve similar turns.
    if opts.similar_turns_cap > 1 {
        opts.similar_turns_cap = (opts.similar_turns_cap / 2).max(1);
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::HalveTurns);
        if bundle.len() <= bundle_bytes {
            return ClippedBundle {
                bytes: bundle.len(),
                bundle,
                steps,
            };
        }
    }

    // Step 5 — truncate decision bodies.
    if opts.decision_byte_cap > 160 {
        opts.decision_byte_cap = 160;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::TruncateDecisions);
        if bundle.len() <= bundle_bytes {
            return ClippedBundle {
                bytes: bundle.len(),
                bundle,
                steps,
            };
        }
    }

    // Step 6 — drop snippets entirely.
    if opts.snippets_cap > 0 {
        opts.snippets_cap = 0;
        bundle = format_bundle(intent, response, &opts);
        steps.push(TrimStep::DropSnippets);
    }

    ClippedBundle {
        bytes: bundle.len(),
        bundle,
        steps,
    }
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
                score: 0.5,
                why: Some("why".into()),
            })
            .collect();
        let decisions: Vec<DecisionRef> = (0..5)
            .map(|i| DecisionRef {
                rank: i + 1,
                id: format!("DEC-{i:04}"),
                title: "T".repeat(800),
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
            },
            notice: None,
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
        resp.results.graph_neighbors.extend((0..20).map(|i| GraphNeighbor {
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
        resp.results.graph_neighbors.extend((0..40).map(|i| GraphNeighbor {
            from: format!("X{i}"),
            relation: "TOUCHED".into(),
            to: format!("Y{i}"),
            hops: 1,
        }));
        // Tight budget so we walk the ladder.
        let clipped = clip_to_budget("pre_change_context", &resp, 4 * 1024);
        assert!(clipped.steps.first().copied().unwrap_or(TrimStep::DropSnippets) != TrimStep::DropSnippets);
        // The first applied step must be DropGraph (which is a
        // no-op in v1 since graph_cap defaults to 0); for that
        // case the next applied step is SlimSnippets.
        let first = clipped.steps[0];
        assert!(matches!(first, TrimStep::DropGraph | TrimStep::SlimSnippets));
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
        };
        let clipped = clip_to_budget("free_search", &resp, 32 * 1024);
        assert!(clipped.bundle.is_empty());
        assert_eq!(clipped.bytes, 0);
        assert!(clipped.steps.is_empty());
    }
}
