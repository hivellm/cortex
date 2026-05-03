//! Namespace constants. Change these only via a new spec.

/// Deployment prefix applied to external services.
pub const NS_PREFIX: &str = "cortex";

// ---------- Synap streams ----------
/// Live ingestion bus.
pub const STREAM_EVENTS_RAW: &str = "cortex.events.raw";
/// Backfill bus (lower priority, pausable).
pub const STREAM_EVENTS_BOOTSTRAP: &str = "cortex.events.bootstrap";
/// Post-processing fan-out for dashboard, hooks, governance.
pub const STREAM_EVENTS_ENRICHED: &str = "cortex.events.enriched";
/// Downstream after embedding.
pub const STREAM_EVENTS_EMBEDDED: &str = "cortex.events.embedded";
/// Downstream after graph write.
pub const STREAM_EVENTS_GRAPHED: &str = "cortex.events.graphed";
/// Downstream after full-text index.
pub const STREAM_EVENTS_FULLTEXT_INDEXED: &str = "cortex.events.fulltext_indexed";
/// Dead-letter for schema-invalid events.
pub const STREAM_EVENTS_INVALID: &str = "cortex.events.invalid";
/// Governance engine output.
pub const STREAM_VIOLATIONS: &str = "cortex.violations";
/// Worker telemetry.
pub const STREAM_METRICS: &str = "cortex.metrics";
/// Query audit trail.
pub const STREAM_QUERY_AUDIT: &str = "cortex.events.query_audit";
/// Cache invalidation fan-out.
pub const STREAM_CACHE_INVALIDATE: &str = "cortex.cache.invalidate";

/// Every known Synap stream (useful for bootstrap + validation tests).
pub const ALL_STREAMS: &[&str] = &[
    STREAM_EVENTS_RAW,
    STREAM_EVENTS_BOOTSTRAP,
    STREAM_EVENTS_ENRICHED,
    STREAM_EVENTS_EMBEDDED,
    STREAM_EVENTS_GRAPHED,
    STREAM_EVENTS_FULLTEXT_INDEXED,
    STREAM_EVENTS_INVALID,
    STREAM_VIOLATIONS,
    STREAM_METRICS,
    STREAM_QUERY_AUDIT,
    STREAM_CACHE_INVALIDATE,
];

// ---------- Synap pub/sub topic prefixes ----------
/// Per-repo live dashboard SSE topic prefix (append the repo name).
pub const TOPIC_LIVE_PREFIX: &str = "cortex.live.";
/// Law-violation topic prefix (append `<law_id>.fired`).
pub const TOPIC_LAW_PREFIX: &str = "cortex.law.";

// ---------- Synap KV namespaces ----------
/// Query result cache; TTL 5 min.
pub const KV_CACHE_QUERY: &str = "cache:query";
/// Classifier cache keyed by content hash; TTL 24 h.
pub const KV_CACHE_CLASSIFY: &str = "cache:classify";
/// Embedding retry-safety cache; TTL 1 h.
pub const KV_CACHE_EMBED: &str = "cache:embed";
/// Daily classifier budget counter; TTL 25 h.
pub const KV_BUDGET_CLASSIFIER: &str = "budget:classifier";
/// Per-session governance reminders.
pub const KV_GOV_REMINDERS: &str = "governance:reminders";

// ---------- Vectorizer collections ----------
/// Turn summaries, hot tier.
pub const COLLECTION_TURN_FP32: &str = "cortex.turn.fp32";
/// Turn summaries, warm tier.
pub const COLLECTION_TURN_PQ: &str = "cortex.turn.pq";
/// Tool call summaries, hot tier.
pub const COLLECTION_TOOL_CALL_FP32: &str = "cortex.tool_call.fp32";
/// Tool call summaries, warm tier.
pub const COLLECTION_TOOL_CALL_PQ: &str = "cortex.tool_call.pq";
/// Code chunks, hot tier.
pub const COLLECTION_CODE_CHUNK_FP32: &str = "cortex.code_chunk.fp32";
/// Code chunks, warm tier.
pub const COLLECTION_CODE_CHUNK_PQ: &str = "cortex.code_chunk.pq";
/// Doc chunks, hot tier.
pub const COLLECTION_DOC_CHUNK_FP32: &str = "cortex.doc_chunk.fp32";
/// Decisions, recall-tuned.
pub const COLLECTION_DECISION_FP32: &str = "cortex.decision.fp32";
/// Analyses, recall-tuned.
pub const COLLECTION_ANALYSIS_FP32: &str = "cortex.analysis.fp32";
/// Memory entries.
pub const COLLECTION_MEMORY_FP32: &str = "cortex.memory.fp32";
/// Law definitions.
pub const COLLECTION_LAW_FP32: &str = "cortex.law.fp32";
/// phase10e — pattern / anti-pattern entries (Rulebook
/// `rulebook_knowledge_add`). Single hot-tier; no warm tier
/// because the corpus is small + dense + worth keeping at full
/// precision.
pub const COLLECTION_KNOWLEDGE_FP32: &str = "cortex.knowledge.fp32";
/// phase10e — implementation insights (Rulebook
/// `rulebook_learn_capture`). Same shape as
/// [`COLLECTION_KNOWLEDGE_FP32`].
pub const COLLECTION_LEARNING_FP32: &str = "cortex.learning.fp32";
/// phase11j — `Kind::Consolidation` summaries, hot tier (FP32 vectors).
/// Holds the recent + Deep-depth consolidations the dashboard's
/// "Consolidated context" lane reads. PQ-compressed sibling lives at
/// [`COLLECTION_CONSOLIDATION_PQ`] for older Shallow consolidations.
pub const COLLECTION_CONSOLIDATION_FP32: &str = "cortex.consolidation.fp32";
/// phase11j — `Kind::Consolidation` summaries, warm tier
/// (PQ-compressed). Demotion target for Shallow consolidations
/// older than the consolidation-tier retention window.
pub const COLLECTION_CONSOLIDATION_PQ: &str = "cortex.consolidation.pq";
/// Cold fallback (binary-quantized).
pub const COLLECTION_COLD_BINARY: &str = "cortex.cold.binary";

// ---------- Meilisearch indexes ----------
/// Turns index.
pub const INDEX_TURNS: &str = "cortex_turns";
/// Tool-calls index.
pub const INDEX_TOOL_CALLS: &str = "cortex_tool_calls";
/// Code-chunks index.
pub const INDEX_CODE_CHUNKS: &str = "cortex_code_chunks";
/// Doc-chunks index.
pub const INDEX_DOCS: &str = "cortex_docs";
/// Decisions index.
pub const INDEX_DECISIONS: &str = "cortex_decisions";
/// Analyses index.
pub const INDEX_ANALYSES: &str = "cortex_analyses";
/// Memories index.
pub const INDEX_MEMORIES: &str = "cortex_memories";
/// Laws index.
pub const INDEX_LAWS: &str = "cortex_laws";
/// phase10e — knowledge entries (pattern / anti-pattern).
pub const INDEX_KNOWLEDGE: &str = "cortex_knowledge";
/// phase10e — learning entries (implementation insights).
pub const INDEX_LEARNINGS: &str = "cortex_learnings";
/// phase11j — `Kind::Consolidation` summaries, single global index
/// the dashboard's consolidations lane filters on. Per-repo scoping
/// is handled by the `repo` filter inside the documents themselves.
pub const INDEX_CONSOLIDATIONS: &str = "cortex_consolidations";

/// Every known Meilisearch index.
pub const ALL_INDEXES: &[&str] = &[
    INDEX_TURNS,
    INDEX_TOOL_CALLS,
    INDEX_CODE_CHUNKS,
    INDEX_DOCS,
    INDEX_DECISIONS,
    INDEX_ANALYSES,
    INDEX_MEMORIES,
    INDEX_LAWS,
    INDEX_KNOWLEDGE,
    INDEX_LEARNINGS,
    INDEX_CONSOLIDATIONS,
];

// ---------- Nexus graph ----------
/// Nexus database / keyspace name.
pub const NEXUS_DB: &str = "cortex";

// ---------- Per-repo collection / index naming ----------

/// Slug used when a repo id is missing or empty. Keeps every
/// downstream collection / index name well-formed instead of producing
/// strings like `cortex--docs`.
pub const UNKNOWN_REPO_SLUG: &str = "unknown";

/// Canonicalise a `cortex.id` (or git-root basename) into an
/// identifier safe for use as part of a Vectorizer collection name or
/// a Meilisearch index uid. Output matches `[a-z0-9-]+` with no
/// leading or trailing `-`.
///
/// Empty input maps to [`UNKNOWN_REPO_SLUG`] so callers can rely on
/// `format!("cortex-{slug}-{family}")` always producing a well-formed
/// name.
pub fn slug_for_repo(repo_id: &str) -> String {
    let lowered: String = repo_id
        .chars()
        .map(|c| {
            let lc = c.to_ascii_lowercase();
            if lc.is_ascii_alphanumeric() || lc == '-' {
                lc
            } else {
                '-'
            }
        })
        .collect();
    let mut out = String::with_capacity(lowered.len());
    let mut last_dash = true;
    for c in lowered.chars() {
        if c == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(c);
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        UNKNOWN_REPO_SLUG.to_string()
    } else {
        out
    }
}

/// Compose the per-repo Vectorizer collection / Meilisearch index
/// name. `prefix` is the deployment namespace (default `"cortex"`),
/// `family` is the kind suffix (`docs`, `code`, `turns`, …).
pub fn repo_scoped_name(prefix: &str, repo_slug: &str, family: &str) -> String {
    format!("{prefix}-{repo_slug}-{family}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_ascii() {
        assert_eq!(slug_for_repo("Cortex"), "cortex");
        assert_eq!(slug_for_repo("CompressionPrompt"), "compressionprompt");
    }

    #[test]
    fn slug_collapses_special_chars() {
        assert_eq!(slug_for_repo("Hive Hub/cloud"), "hive-hub-cloud");
        assert_eq!(slug_for_repo("foo___bar"), "foo-bar");
        assert_eq!(slug_for_repo("a..b..c"), "a-b-c");
    }

    #[test]
    fn slug_strips_leading_trailing_dashes() {
        assert_eq!(slug_for_repo("--foo--"), "foo");
        assert_eq!(slug_for_repo("/proj/"), "proj");
    }

    #[test]
    fn slug_empty_falls_back_to_unknown() {
        assert_eq!(slug_for_repo(""), UNKNOWN_REPO_SLUG);
        assert_eq!(slug_for_repo("///"), UNKNOWN_REPO_SLUG);
    }

    #[test]
    fn slug_keeps_existing_dashes() {
        assert_eq!(slug_for_repo("hive-llm"), "hive-llm");
    }

    #[test]
    fn repo_scoped_name_format() {
        assert_eq!(
            repo_scoped_name("cortex", "cortex", "docs"),
            "cortex-cortex-docs"
        );
        assert_eq!(
            repo_scoped_name("cortex", "tml", "code"),
            "cortex-tml-code"
        );
    }
}

