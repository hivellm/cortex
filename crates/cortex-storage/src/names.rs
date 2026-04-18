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
];

// ---------- Nexus graph ----------
/// Nexus database / keyspace name.
pub const NEXUS_DB: &str = "cortex";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_unique() {
        let mut v = ALL_STREAMS.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), ALL_STREAMS.len());
    }

    #[test]
    fn indexes_are_unique() {
        let mut v = ALL_INDEXES.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), ALL_INDEXES.len());
    }

    #[test]
    fn all_streams_prefixed() {
        for s in ALL_STREAMS {
            assert!(s.starts_with("cortex."), "stream `{s}` missing cortex. prefix");
        }
    }

    #[test]
    fn all_indexes_prefixed() {
        for i in ALL_INDEXES {
            assert!(i.starts_with("cortex_"), "index `{i}` missing cortex_ prefix");
        }
    }
}
