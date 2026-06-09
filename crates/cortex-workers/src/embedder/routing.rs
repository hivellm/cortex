//! Per-`Kind` collection routing — per-repo isolated.
//!
//! The table follows `docs/specs/06-embedder.md` §Collections. The
//! returned string is `"{prefix}-{repo_slug}-{suffix}"` so each repo
//! lives in its own collection (`cortex-cortex-docs`,
//! `cortex-tml-code`, …) and queries can scope deterministically to a
//! single project.

use cortex_core::events::Kind;
use cortex_storage::names::{repo_scoped_name, slug_for_repo, UNKNOWN_REPO_SLUG};

use super::chunker::ChunkSource;

/// Family suffix for the collection a `Kind` routes to.
///
/// The `prefix` + `repo_slug` are combined with this suffix to build
/// the full collection name.
fn family_for(kind: &Kind) -> &'static str {
    match kind {
        // tool_call.* — code symbols (spec: "tool_call.* (code)" → -code).
        Kind::ToolCall => "code",
        // artifact.* — defaults to docs; chunks routed via
        // `collection_for_chunk` get the precise code/docs split.
        Kind::Artifact => "docs",
        Kind::Decision => "decisions",
        Kind::Turn => "turns",
        Kind::Law | Kind::LawViolation => "governance",
        // Audit / deep-analysis reports get their own per-repo
        // collection so the dashboard's Analysis view can scope to a
        // single index instead of fishing them out of the catch-all
        // misc bucket.
        Kind::Analysis => "analyses",
        // phase10e — knowledge / learnings are high-signal
        // curated corpora (Rulebook MCP captures); each lives in
        // its own per-repo family so the orchestrator can fan
        // out to them deterministically without diluting the
        // catch-all `misc` bucket.
        Kind::Knowledge => "knowledge",
        Kind::Learning => "learnings",
        // Phase11j §3.3 — consolidations route to a dedicated
        // per-repo collection (`cortex-{slug}-consolidations`)
        // so the spec-12 renderer's `Consolidated context`
        // section can fan out against a single index.
        Kind::Consolidation => "consolidations",
        // phase11r §3.2 — topic cards route to their own per-repo
        // collection (`cortex-{slug}-topic_cards`) so the synthesis
        // lane has its own embedding space.
        Kind::TopicCard => "topic_cards",
        // Catch-all for kinds not explicitly called out in the spec table.
        Kind::AgentCall | Kind::Memory => "misc",
    }
}

/// Return the collection name for a given event kind. The optional
/// `repo_id` is normalised through [`slug_for_repo`]; `None` falls
/// back to [`UNKNOWN_REPO_SLUG`] so the resulting name is always
/// well-formed.
///
/// Note that for [`Kind::Artifact`] this returns the *doc* fallback —
/// callers that have a [`ChunkSource`] in hand should prefer
/// [`collection_for_chunk`] so code chunks land in
/// `{prefix}-{repo_slug}-code` and doc chunks land in
/// `{prefix}-{repo_slug}-docs`.
pub fn collection_for(kind: &Kind, prefix: &str, repo_id: Option<&str>) -> String {
    let slug = repo_id
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    repo_scoped_name(prefix, &slug, family_for(kind))
}

/// Route a single chunk to its destination collection. For
/// [`Kind::Artifact`] the [`ChunkSource`] discriminates between the
/// code lane (`{prefix}-{repo_slug}-code`) and the doc lane
/// (`{prefix}-{repo_slug}-docs`); other kinds delegate to
/// [`collection_for`] which is event-level.
///
/// `Summary` and `RawOversize` chunks inherit the parent event's
/// classification: they live with the original chunk they replaced or
/// shadow, which is captured here by the `parent_source` argument the
/// substitution pass already has on hand.
pub fn collection_for_chunk(
    kind: &Kind,
    source: &ChunkSource,
    prefix: &str,
    repo_id: Option<&str>,
) -> String {
    let slug = repo_id
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    match (kind, source) {
        (Kind::Artifact, ChunkSource::Code) => repo_scoped_name(prefix, &slug, "code"),
        (Kind::Artifact, ChunkSource::Doc) => repo_scoped_name(prefix, &slug, "docs"),
        (Kind::Artifact, ChunkSource::FallbackWindow) => {
            // No language hint — keep with docs so search still finds it.
            repo_scoped_name(prefix, &slug, "docs")
        }
        // Summary / RawOversize keep whatever the caller already set on
        // the chunk; we just pass through the event-level routing as a
        // safety default.
        _ => collection_for(kind, prefix, repo_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_routes_to_per_repo_docs() {
        assert_eq!(
            collection_for(&Kind::Artifact, "cortex", Some("Cortex")),
            "cortex-cortex-docs"
        );
        assert_eq!(
            collection_for(&Kind::Artifact, "cortex", Some("Tml")),
            "cortex-tml-docs"
        );
    }

    #[test]
    fn missing_repo_falls_back_to_unknown_slug() {
        assert_eq!(
            collection_for(&Kind::ToolCall, "cortex", None),
            "cortex-unknown-code"
        );
        assert_eq!(
            collection_for(&Kind::ToolCall, "cortex", Some("")),
            "cortex-unknown-code"
        );
    }

    #[test]
    fn chunk_routing_splits_artifact_by_source() {
        assert_eq!(
            collection_for_chunk(
                &Kind::Artifact,
                &ChunkSource::Code,
                "cortex",
                Some("Cortex")
            ),
            "cortex-cortex-code"
        );
        assert_eq!(
            collection_for_chunk(&Kind::Artifact, &ChunkSource::Doc, "cortex", Some("Cortex")),
            "cortex-cortex-docs"
        );
        assert_eq!(
            collection_for_chunk(
                &Kind::Artifact,
                &ChunkSource::FallbackWindow,
                "cortex",
                Some("Cortex")
            ),
            "cortex-cortex-docs"
        );
    }

    #[test]
    fn turn_routes_to_turns_per_repo() {
        assert_eq!(
            collection_for(&Kind::Turn, "cortex", Some("Cortex")),
            "cortex-cortex-turns"
        );
    }

    #[test]
    fn analysis_routes_to_dedicated_per_repo_analyses_collection() {
        assert_eq!(
            collection_for(&Kind::Analysis, "cortex", Some("Cortex")),
            "cortex-cortex-analyses"
        );
        assert_eq!(
            collection_for(&Kind::Analysis, "cortex", Some("Rulebook")),
            "cortex-rulebook-analyses"
        );
    }
}
