//! Search subsystem — orchestrator, intent rewriter, fusion / RRF,
//! analyzer, lane strategies, response cache, byte budget, and the
//! HTTP search proxy.
//!
//! Every file in this bucket previously lived at
//! `crates/cortex-api/src/<name>.rs`. The bucketing is a 1:1 rename
//! to give the module tree a thematic home; external paths are
//! preserved via `pub use search::<child>` re-exports in
//! [`crate`] (see `lib.rs`).

pub mod analyzer;
pub mod architecture_route;
pub mod budget;
pub mod cache;
pub mod consolidation_costs;
pub mod consolidation_get;
pub mod consolidation_lineage;
pub mod consolidations_by_entity;
pub mod consolidations_diff;
pub mod consolidations_recent;
pub mod consolidations_search;
pub mod decision_search;
pub mod events_by_kind;
pub mod files_touched;
pub mod fusion;
pub mod graph_seeds;
pub mod law_violations;
pub mod orchestrator;
pub mod query_explain;
pub mod query_rewrite;
pub mod relevance_config;
pub mod search_proxy;
pub mod strategies;
pub mod temporal_metrics;
pub mod timeline_routes;
pub mod tool_calls;
pub mod topic_search;

/// Map a canonical envelope kind discriminator (snake_case) to the
/// per-repo Meili family the fulltext worker writes to. Mirrors
/// `cortex_workers::graph::routing::family_for` so the read-side
/// uses the same family the writer used. Returns `None` for unknown
/// kinds.
pub(crate) fn kind_to_family(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "turn" | "agent_call" => "turns",
        "tool_call" => "code",
        "consolidation" => "consolidations",
        "decision" => "decisions",
        "analysis" => "analyses",
        // phase0_laws-index-routing: law DEFINITIONS live in `governance`
        // per-repo (family_for(Kind::Law) == "governance"). Violations
        // also land in `governance`. Both are per-repo; the global
        // `cortex_laws` dual-write is for definitions only and is handled
        // by `index_for_event_global`, not the per-repo family routing.
        "law_violation" | "violation" | "law" => "governance",
        "memory" | "artifact" => "misc",
        "knowledge" => "knowledge",
        "learning" | "learnings" => "learnings",
        "topic_card" => "topic_cards",
        _ => return None,
    })
}

/// Resolve the Meilisearch index uid for an envelope family,
/// preferring the per-repo `cortex-<lowercase-repo>-<family>` form
/// when `repo` is set. Falls back to the supplied global uid when
/// no repo is supplied. Phase15b post-test: live Meili only carries
/// the per-repo `cortex-<slug>-*` family for non-decision/non-law
/// kinds, so the global uids (e.g. `cortex_consolidations`,
/// `cortex_turns`) are not addressable; routing per-repo is the
/// only path that returns hits without a schema bump + reindex.
pub(crate) fn resolve_family_index(repo: Option<&str>, family: &str, global: &str) -> String {
    match repo.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => format!("cortex-{}-{}", r.to_ascii_lowercase(), family),
        None => global.to_string(),
    }
}

/// True when a non-2xx Meili search response means the target index
/// does not exist — a recoverable empty state, not a gateway failure
/// (issue #4: `cortex_tool_calls` / `cortex_turns` 502s). Meili
/// signals a missing index with HTTP 404 + a JSON body carrying
/// `code: "index_not_found"`. The body is the canonical, version-
/// stable signal; the 404 status is accepted as a belt-and-suspenders
/// fallback for the same condition. Callers map `true` to an empty
/// `200` result set instead of propagating a `502`.
pub(crate) fn is_meili_index_missing(status: u16, body: &serde_json::Value) -> bool {
    if body.get("code").and_then(|v| v.as_str()) == Some("index_not_found") {
        return true;
    }
    // No `index_not_found` code, but a 404 whose message names a
    // missing index still means the same thing on the search path.
    status == 404
        && body
            .get("message")
            .and_then(|v| v.as_str())
            .map(|m| m.contains("not found"))
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn index_missing_detects_meili_code() {
        let body = json!({
            "message": "Index `cortex_tool_calls` not found.",
            "code": "index_not_found",
            "type": "invalid_request"
        });
        assert!(is_meili_index_missing(404, &body));
        // The code is authoritative regardless of the status line.
        assert!(is_meili_index_missing(400, &body));
    }

    #[test]
    fn index_missing_accepts_404_with_not_found_message() {
        let body = json!({ "message": "Index `cortex_turns` not found." });
        assert!(is_meili_index_missing(404, &body));
    }

    #[test]
    fn index_missing_rejects_other_errors() {
        // A genuine Meili error (bad filter) must NOT be treated as
        // an empty index — it should still surface as a 502.
        let bad_filter = json!({
            "message": "Attribute `nope` is not filterable.",
            "code": "invalid_search_filter"
        });
        assert!(!is_meili_index_missing(400, &bad_filter));
        // A 404 without a "not found" message is not an index miss.
        let other_404 = json!({ "message": "route does not exist" });
        assert!(!is_meili_index_missing(404, &other_404));
        // No body fields at all.
        assert!(!is_meili_index_missing(500, &json!({})));
    }
}
