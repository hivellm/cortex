//! Meilisearch index metadata. Per-index settings files are embedded from
//! `schemas/meili/<index>.settings.v1.json`.

/// Descriptor for a Meilisearch index.
#[derive(Debug, Clone, Copy)]
pub struct IndexDescriptor {
    /// Index name (matches the `cortex_*` pattern).
    pub name: &'static str,
    /// Primary-key attribute.
    pub primary_key: &'static str,
    /// Embedded `settings.v1.json` contents.
    pub settings_json: &'static str,
}

const SETTINGS_TURNS: &str = include_str!("../schemas/meili/cortex_turns.settings.v1.json");
const SETTINGS_TOOL_CALLS: &str =
    include_str!("../schemas/meili/cortex_tool_calls.settings.v1.json");
const SETTINGS_CODE_CHUNKS: &str =
    include_str!("../schemas/meili/cortex_code_chunks.settings.v1.json");
const SETTINGS_DOCS: &str = include_str!("../schemas/meili/cortex_docs.settings.v1.json");
const SETTINGS_DECISIONS: &str = include_str!("../schemas/meili/cortex_decisions.settings.v1.json");
const SETTINGS_ANALYSES: &str = include_str!("../schemas/meili/cortex_analyses.settings.v1.json");
const SETTINGS_MEMORIES: &str = include_str!("../schemas/meili/cortex_memories.settings.v1.json");
const SETTINGS_LAWS: &str = include_str!("../schemas/meili/cortex_laws.settings.v1.json");
const SETTINGS_KNOWLEDGE: &str = include_str!("../schemas/meili/cortex_knowledge.settings.v1.json");
const SETTINGS_LEARNINGS: &str = include_str!("../schemas/meili/cortex_learnings.settings.v1.json");
// phase12d — consolidation + topic_card indexes were created lazily
// by Meili on first write but had no `searchableAttributes` /
// `filterableAttributes` configured, so every query against them
// ranked by document id (useless for retrieval). The bootstrap path
// now ships explicit settings so first-write reconciles to the
// expected attribute set.
const SETTINGS_CONSOLIDATIONS: &str =
    include_str!("../schemas/meili/cortex_consolidations.settings.v1.json");
const SETTINGS_TOPIC_CARDS: &str =
    include_str!("../schemas/meili/cortex_topic_cards.settings.v1.json");

/// Every Meilisearch index Cortex expects to exist.
pub const INDEXES: &[IndexDescriptor] = &[
    IndexDescriptor {
        name: crate::names::INDEX_TURNS,
        primary_key: "event_id",
        settings_json: SETTINGS_TURNS,
    },
    IndexDescriptor {
        name: crate::names::INDEX_TOOL_CALLS,
        primary_key: "event_id",
        settings_json: SETTINGS_TOOL_CALLS,
    },
    IndexDescriptor {
        name: crate::names::INDEX_CODE_CHUNKS,
        primary_key: "chunk_id",
        settings_json: SETTINGS_CODE_CHUNKS,
    },
    IndexDescriptor {
        name: crate::names::INDEX_DOCS,
        primary_key: "chunk_id",
        settings_json: SETTINGS_DOCS,
    },
    IndexDescriptor {
        name: crate::names::INDEX_DECISIONS,
        primary_key: "event_id",
        settings_json: SETTINGS_DECISIONS,
    },
    IndexDescriptor {
        name: crate::names::INDEX_ANALYSES,
        primary_key: "event_id",
        settings_json: SETTINGS_ANALYSES,
    },
    IndexDescriptor {
        name: crate::names::INDEX_MEMORIES,
        primary_key: "event_id",
        settings_json: SETTINGS_MEMORIES,
    },
    IndexDescriptor {
        name: crate::names::INDEX_LAWS,
        primary_key: "law_id",
        settings_json: SETTINGS_LAWS,
    },
    // phase10e — knowledge + learnings each get their own
    // dedicated index so the orchestrator can fan out to them
    // without diluting the catch-all `cortex_memories` index.
    IndexDescriptor {
        name: crate::names::INDEX_KNOWLEDGE,
        primary_key: "knowledge_id",
        settings_json: SETTINGS_KNOWLEDGE,
    },
    IndexDescriptor {
        name: crate::names::INDEX_LEARNINGS,
        primary_key: "learning_id",
        settings_json: SETTINGS_LEARNINGS,
    },
    // phase12d — ship explicit settings so the consolidator + topic-
    // card writers don't depend on Meili's lazy default ranking.
    IndexDescriptor {
        name: crate::names::INDEX_CONSOLIDATIONS,
        primary_key: "event_id",
        settings_json: SETTINGS_CONSOLIDATIONS,
    },
    IndexDescriptor {
        name: crate::names::INDEX_TOPIC_CARDS,
        primary_key: "event_id",
        settings_json: SETTINGS_TOPIC_CARDS,
    },
];
