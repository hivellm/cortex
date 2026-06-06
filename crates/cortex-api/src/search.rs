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
