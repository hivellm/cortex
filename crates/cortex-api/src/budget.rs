//! Phase11c — byte-budget clipper for `/v1/query` responses.
//!
//! The MCP transport rejects single-tool results past a hard token
//! cap (~30k chars in the runtime today). The orchestrator's
//! `free_search` intent — and any high-recall query at scale — can
//! easily produce a 100+ KiB JSON document that the transport drops
//! to a side-file. This module keeps the wire response under a
//! caller-supplied (or default) byte budget by trimming snippet text,
//! then progressively dropping tail entries from each result section,
//! and stamps a [`ClipReport`] on the response so callers can see
//! what was removed without re-running the query.
//!
//! Design notes
//! ============
//!
//! - Pre-thinking has its own clipper in `cortex-pre-thinking::budget`,
//!   but that runs *after* the API has already produced the full
//!   response. By the time the pre-thinking pipeline trims, the wire
//!   payload has already crossed the MCP boundary. This clipper sits
//!   in `cortex-api` so the trim happens before the response leaves
//!   the daemon.
//! - The clipper is intentionally simple: per-snippet text cap, then
//!   tail-drop until the serialised size fits. No re-ranking, no
//!   fancy heuristics. Spec 11 already ranks; clipping keeps the
//!   prefix.
//! - JSON serialisation runs after every drop because that is the
//!   only way to know the actual on-the-wire size — `serde_json`'s
//!   pretty / compact modes differ enough that any heuristic would
//!   miss the true target. The clipper bounds re-serialisations by
//!   the number of removable items (worst case linear in total hits).

use crate::types::{ClipReport, QueryResponse};

/// Default byte budget when the caller omits `budget_bytes` from the
/// request. Sized to fit comfortably under the MCP transport's
/// per-tool-result cap (~30 KiB chars in the current runtime) with
/// margin for the JSON wrapper + audit envelope on the way out.
pub const DEFAULT_BUDGET_BYTES: usize = 32 * 1024;

/// Per-snippet `text` byte cap. A single oversized snippet would
/// otherwise eat the whole budget and force the tail-drop loop to
/// strip every other result. Mirrors `cortex_pre_thinking`'s
/// `SNIPPET_BYTES = 1024`.
pub const SNIPPET_TEXT_CAP: usize = 1024;

/// Per-decision rationale byte cap — analogous to the snippet cap.
pub const DECISION_TEXT_CAP: usize = 512;

/// Per-similar-turn summary byte cap.
pub const SIMILAR_TURN_TEXT_CAP: usize = 384;

/// Clip `resp` so its serialised JSON length does not exceed
/// `budget_bytes`. Returns a [`ClipReport`] describing what was
/// removed; the caller should attach it to the response when at
/// least one field is non-zero.
///
/// The clipper:
///
/// 1. Caps each snippet/decision/turn text field at the per-section
///    byte cap (UTF-8 boundary safe).
/// 2. Re-serialises and, while still over budget, drops tail entries
///    from each section in priority order (graph_neighbors,
///    similar_turns, violations, decisions, snippets).
///
/// The clipper never returns a response with zero snippets unless
/// even the bare envelope (no results) overflows the budget — in
/// that pathological case it simply returns the empty-results form
/// and lets the caller see `final_bytes > budget_bytes` in the
/// report. This keeps clipping fail-soft instead of producing an
/// error envelope.
pub fn clip_response_to_budget(resp: &mut QueryResponse, budget_bytes: usize) -> ClipReport {
    let mut report = ClipReport {
        budget_bytes,
        ..ClipReport::default()
    };

    // Step 1 — per-field text caps.
    for snippet in resp.results.snippets.iter_mut() {
        if snippet.text.len() > SNIPPET_TEXT_CAP {
            snippet.text = clip_utf8(&snippet.text, SNIPPET_TEXT_CAP);
            report.snippets_text_clipped += 1;
        }
    }
    for decision in resp.results.decisions.iter_mut() {
        if let Some(rationale) = decision.rationale_excerpt.as_mut() {
            if rationale.len() > DECISION_TEXT_CAP {
                *rationale = clip_utf8(rationale, DECISION_TEXT_CAP);
            }
        }
    }
    for turn in resp.results.similar_turns.iter_mut() {
        if turn.summary.len() > SIMILAR_TURN_TEXT_CAP {
            turn.summary = clip_utf8(&turn.summary, SIMILAR_TURN_TEXT_CAP);
        }
    }

    // Step 2 — tail-drop loop. Re-serialise after every removal so
    // we know the on-the-wire size exactly. Order: cheapest-context
    // sections first so the most informative results survive.
    while serialised_len(resp) > budget_bytes {
        if !resp.results.graph_neighbors.is_empty() {
            resp.results.graph_neighbors.pop();
            report.removed_graph_neighbors += 1;
            continue;
        }
        if !resp.results.similar_turns.is_empty() {
            resp.results.similar_turns.pop();
            report.removed_similar_turns += 1;
            continue;
        }
        if !resp.results.violations.is_empty() {
            resp.results.violations.pop();
            report.removed_violations += 1;
            continue;
        }
        if !resp.results.decisions.is_empty() {
            resp.results.decisions.pop();
            report.removed_decisions += 1;
            continue;
        }
        if !resp.results.snippets.is_empty() {
            resp.results.snippets.pop();
            report.removed_snippets += 1;
            continue;
        }
        // Nothing left to drop and we're still over budget — emit
        // the report with the over-budget final size and stop. The
        // caller learns the bare envelope is larger than the cap.
        break;
    }

    report.final_bytes = serialised_len(resp);
    report
}

/// `true` when the report carries any structural change worth
/// surfacing to the caller. Used by the service layer to decide
/// whether to attach the report to the response.
pub fn report_is_meaningful(report: &ClipReport) -> bool {
    report.removed_snippets > 0
        || report.removed_decisions > 0
        || report.removed_violations > 0
        || report.removed_similar_turns > 0
        || report.removed_graph_neighbors > 0
        || report.snippets_text_clipped > 0
}

/// UTF-8 boundary-safe byte truncation. Identical contract to the
/// `cortex_pre_thinking::formatter::clip_utf8` helper — duplicated
/// here so `cortex-api` does not pull in `cortex-pre-thinking`
/// (which depends on `cortex-api`'s types and would form a cycle).
fn clip_utf8(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut cut = n;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

fn serialised_len(resp: &QueryResponse) -> usize {
    serde_json::to_vec(resp).map(|v| v.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        DecisionRef, GraphNeighbor, Intent, QueryRequest, ResultsBag, Scope, SimilarTurn, Snippet,
        ViolationRef,
    };

    fn snippet(rank: usize, text: &str) -> Snippet {
        Snippet {
            rank,
            source: "vector".into(),
            collection: Some("cortex-cortex-code".into()),
            repo: Some("Cortex".into()),
            path: Some(format!("src/{rank}.rs")),
            symbol: None,
            content_hash: None,
            text: text.to_string(),
            body_truncated: false,
            score: 1.0,
            why: None,
        }
    }

    fn response_with_snippets(n: usize, body_bytes: usize) -> QueryResponse {
        let mut resp = QueryResponse {
            intent: "free_search".into(),
            query_id: "01TESTQUERYID00000000000000".into(),
            ..QueryResponse::default()
        };
        let body = "x".repeat(body_bytes);
        for i in 0..n {
            resp.results.snippets.push(snippet(i + 1, &body));
        }
        resp
    }

    #[test]
    fn clip_caps_snippet_text_to_per_field_limit() {
        // Phase11c spec scenario "Default budget applied" — per-snippet
        // text MUST be clipped to ≤ SNIPPET_TEXT_CAP (1 KiB).
        let mut resp = response_with_snippets(3, 4096);
        let report = clip_response_to_budget(&mut resp, DEFAULT_BUDGET_BYTES);
        for s in &resp.results.snippets {
            assert!(
                s.text.len() <= SNIPPET_TEXT_CAP,
                "snippet text len {} > cap {}",
                s.text.len(),
                SNIPPET_TEXT_CAP
            );
        }
        assert_eq!(report.snippets_text_clipped, 3);
    }

    #[test]
    fn clip_keeps_response_under_caller_supplied_budget() {
        // Phase11c spec scenario "Caller-supplied budget honoured".
        let mut resp = response_with_snippets(50, 2048);
        let report = clip_response_to_budget(&mut resp, 8192);
        assert!(
            report.final_bytes <= 8192,
            "final {} > budget 8192",
            report.final_bytes
        );
        assert!(report.removed_snippets > 0, "expected tail-drop to run");
    }

    #[test]
    fn clip_drops_graph_neighbors_before_snippets() {
        // Drop priority: graph_neighbors first because they carry the
        // least standalone context per byte. Snippets are last
        // because they carry the body the agent actually consumes.
        let mut resp = response_with_snippets(1, 64);
        for i in 0..5 {
            resp.results.graph_neighbors.push(GraphNeighbor {
                from: format!("node-{i}"),
                relation: "calls".into(),
                to: format!("target-{i}"),
                hops: 1,
            });
        }
        let initial_size = serialised_len(&resp);
        let target = initial_size - 10;
        let report = clip_response_to_budget(&mut resp, target);
        assert!(
            report.removed_graph_neighbors > 0,
            "expected graph_neighbors drop"
        );
        assert_eq!(report.removed_snippets, 0, "snippets kept");
    }

    #[test]
    fn clip_attaches_no_report_for_already_small_response() {
        // Phase11c — if the response already fits, the clipper
        // returns a zero report and `report_is_meaningful` returns
        // false so the service layer skips attaching it.
        let mut resp = response_with_snippets(2, 64);
        let report = clip_response_to_budget(&mut resp, DEFAULT_BUDGET_BYTES);
        assert_eq!(report.removed_snippets, 0);
        assert_eq!(report.removed_decisions, 0);
        assert_eq!(report.snippets_text_clipped, 0);
        assert!(!report_is_meaningful(&report));
    }

    #[test]
    fn clip_handles_empty_response_without_panic() {
        let mut resp = QueryResponse {
            intent: "free_search".into(),
            ..QueryResponse::default()
        };
        let report = clip_response_to_budget(&mut resp, DEFAULT_BUDGET_BYTES);
        assert_eq!(report.final_bytes, serialised_len(&resp));
        assert!(!report_is_meaningful(&report));
    }

    #[test]
    fn clip_caps_decision_rationale_excerpt() {
        let mut resp = QueryResponse::default();
        resp.results.decisions.push(DecisionRef {
            rank: 1,
            id: "DEC-0001".into(),
            title: "Adopt foo".into(),
            rationale_excerpt: Some("y".repeat(2048)),
            status: "accepted".into(),
            ts: 0,
            score: 1.0,
            links: vec![],
        });
        let _ = clip_response_to_budget(&mut resp, DEFAULT_BUDGET_BYTES);
        assert!(
            resp.results.decisions[0]
                .rationale_excerpt
                .as_deref()
                .unwrap_or("")
                .len()
                <= DECISION_TEXT_CAP
        );
    }

    #[test]
    fn clip_utf8_does_not_split_multibyte_codepoint() {
        // `é` is two bytes (0xC3 0xA9). Clipping at byte 1 must
        // walk back to the previous boundary and produce an empty
        // string instead of an invalid UTF-8 sequence.
        assert_eq!(clip_utf8("é", 1), "");
        assert_eq!(clip_utf8("ab", 1), "a");
    }

    #[test]
    fn report_default_carries_zero_counts_and_zero_bytes() {
        let r = ClipReport::default();
        assert_eq!(r.removed_snippets, 0);
        assert_eq!(r.final_bytes, 0);
        assert_eq!(r.budget_bytes, 0);
    }

    // Suppress unused-import warnings for items only referenced via
    // the `..QueryResponse::default()` spread above.
    #[allow(dead_code)]
    fn _types_in_scope(
        _q: QueryRequest,
        _i: Intent,
        _s: Scope,
        _r: ResultsBag,
        _v: ViolationRef,
        _t: SimilarTurn,
    ) {
    }
}
