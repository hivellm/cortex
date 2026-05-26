//! Cortex query API — hybrid retrieval (vector + keyword + graph)
//! with RRF fusion (spec 11).
//!
//! The crate exposes both an Axum HTTP service (`POST /v1/query`)
//! and an MCP tool binding (`cortex.query`) that share the same
//! request/response types so behaviour stays identical between
//! transports. Lane traits are defined here; live wiring against
//! Vectorizer / Meilisearch / Nexus drops in behind the same
//! traits and is selected at daemon startup.
//!
//! ## Module layout (2026-05-25 reorg)
//!
//! The crate is bucketed into thematic dirs:
//!
//! - [`lanes`] — vector / keyword / graph clients + their loaders.
//! - [`search`] — orchestrator, intent rewriter, fusion / RRF,
//!   analyzer, lane strategies, response cache, byte budget,
//!   search proxy.
//! - [`security`] — auth, ACL, audit, rate limit, redaction.
//! - [`admin`] — operator endpoints (forget, list-events,
//!   config-audit, canary, silent-drop).
//! - [`mcp`] — MCP tool bindings (`cortex_query`, topic-card).
//! - [`health`] — health / freshness / divergence / coverage.
//! - [`ingest`] — ingestion proxy surface.
//! - [`dashboard`] — `/v1/dashboard/*` handlers + their loaders.
//! - [`storage`] — SQLite-backed persistence (API keys today).
//!
//! Root-only modules (`http`, `service`, `types`, `main`,
//! `retention_daemon`) host the crate's transport / lifecycle.
//!
//! Pre-bucket module paths (e.g. `cortex_api::meili_loader`) are
//! preserved via `pub use` re-exports below so external crates
//! (cortex-cli, integration tests, etc.) keep compiling unchanged.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// -- buckets ----------------------------------------------------------------
pub mod active_work;
pub mod admin;
pub mod dashboard;
pub mod decision_chain;
pub mod feedback;
pub mod health;
pub mod ingest;
pub mod lanes;
pub mod mcp;
pub mod search;
pub mod security;
pub mod similar_sessions;

// -- root modules -----------------------------------------------------------
pub mod http;
pub mod retention_daemon;
pub mod service;
pub mod storage;
#[cfg(feature = "ts-export")]
pub mod ts_export;
pub mod types;

// -- pre-bucket path preservation ------------------------------------------
//
// Every `pub use bucket::child;` line below keeps an external import
// like `use cortex_api::meili_loader;` resolving after the
// 2026-05-25 reorg. Adding a new module: either drop it in its
// natural bucket and add the corresponding `pub use` here, or skip
// the re-export when no external crate references the old path.

pub use admin::canary;
pub use admin::config_audit;
pub use admin::forget as admin_forget;
pub use admin::list_events as admin_list_events;
pub use admin::silent_drop;
pub use dashboard::consumer as dashboard_consumer;
pub use dashboard::memory_tail;
pub use dashboard::series as dashboard_series;
pub use dashboard::tasks_loader;
pub use dashboard::watcher as dashboard_watcher;
pub use health::coverage;
pub use ingest::proxy as ingest_proxy;
pub use lanes::archive_loader;
pub use lanes::loader_metrics;
pub use lanes::meili_lane;
pub use lanes::meili_loader;
pub use lanes::nexus_graph_lane;
pub use lanes::vectorizer_lane;
pub use mcp::topic_card as mcp_topic_card;
pub use search::analyzer;
pub use search::budget;
pub use search::cache;
pub use search::consolidation_get;
pub use search::events_by_kind;
pub use search::files_touched;
pub use search::fusion;
pub use search::orchestrator;
pub use search::query_rewrite;
pub use search::relevance_config;
pub use search::search_proxy;
pub use search::strategies;
pub use search::tool_calls;
pub use search::topic_search;
pub use security::acl;
pub use security::audit;
pub use security::audit_store;
pub use security::auth;
pub use security::rate_limit;
pub use security::redaction;

// -- crate-root symbol re-exports (kept verbatim from the
// pre-reorg lib.rs so call sites like `cortex_api::LaneHit`
// continue to resolve). -----------------------------------------------------
pub use acl::{AclDecision, AclStore};
pub use archive_loader::{
    load_into_keyword_lane, load_lane_hits, LoadError, LoadReport, DEFAULT_INDEX,
};
pub use audit::{build_envelope, AuditPublisher, MemoryAuditPublisher, STREAM_QUERY_AUDIT};
pub use audit_store::{AuditStore, DEFAULT_STORE_CAPACITY as AUDIT_STORE_CAPACITY};
pub use cache::{cache_key, Cache, CacheHandle, InMemoryCache, DEFAULT_TTL, SCHEMA_VERSION};
pub use dashboard::{build_dashboard_router, DashboardState};
pub use fusion::{rrf_fuse, FusionConfig, DEFAULT_RRF_ALPHA, RRF_K};
pub use http::{
    build_router, build_router_with, build_router_with_auth, build_router_with_auth_and_cfg,
    CALLER_HEADER,
};
pub use lanes::{
    GraphLane, GraphRequest, KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, VectorLane, VectorRequest,
};
pub use loader_metrics::LoaderMetrics;
pub use mcp::{invoke as mcp_invoke, tool_descriptor, McpError, TOOL_NAME};
pub use mcp_topic_card::{
    invoke_synthesize, invoke_topic_diff, invoke_topic_drill, invoke_topic_get,
    invoke_topic_neighbors, is_valid_topic_slug, synthesize_descriptor, topic_diff_descriptor,
    topic_drill_descriptor, topic_get_descriptor, topic_neighbors_descriptor, DrillDimension,
    DrillResult, HydratedEvidenceItem, NeighborEdge, NeighborGraph, NeighborNode,
    SynthesizeRequest, SynthesizeResult, TopicCardDiff, TopicCardDiffer, TopicCardDrill,
    TopicCardLookup, TopicCardMcpError, TopicCardNeighbors, TopicCardRevision,
    TopicCardSynthesizer, TOOL_NAME_SYNTHESIZE, TOOL_NAME_TOPIC_DIFF, TOOL_NAME_TOPIC_DRILL,
    TOOL_NAME_TOPIC_GET, TOOL_NAME_TOPIC_NEIGHBORS, TOPIC_GET_CONFIDENCE_FLOOR,
    TOPIC_NEIGHBORS_DEFAULT_DEPTH, TOPIC_NEIGHBORS_NODE_CAP,
};
pub use meili_lane::MeiliKeywordLane;
pub use meili_loader::{load_meili_into_keyword_lane, MeiliLoadError, MeiliLoadReport};
pub use nexus_graph_lane::NexusGraphLane;
pub use orchestrator::Orchestrator;
pub use rate_limit::{RateConfig, RateDecision, RateLimiter};
pub use redaction::redact_response;
pub use relevance_config::{
    default_config_path as default_relevance_config_path, RelevanceConfig, RelevanceConfigError,
};
pub use service::{ErrorBody, QueryService, ServiceOutcome};
pub use strategies::{build_plan, BudgetSplit, Overlay, Plan};
pub use tasks_loader::{
    CachedRowSnapshot, ListQuery, MultiTaskLoader, PhaseBreakdown, ProgressCounts, SortField,
    SortOrder, SpecFile, TaskChecklistItem, TaskChecklistSection, TaskDetail, TaskListResponse,
    TaskLoader, TaskRow, TaskSummary,
};
pub use types::{
    empty_response, BudgetReport, ConsolidationRef, DebugInfo, DecisionRef, GraphNeighbor,
    IncludeField, Intent, LaneTimings, LawRef, Notice, PastSession, Props, QueryRequest,
    QueryResponse, ResultsBag, Scope, SimilarTurn, Snippet, TopicCardRef, ViolationRef,
};
pub use vectorizer_lane::VectorizerLane;

/// Phase8c — capture cortex-api's own compile-time version block.
/// Wraps the [`cortex_build::version_info!`] macro so the `health`
/// module (and tests) can read it without expanding the macro at
/// every call site.
pub fn self_version_info() -> cortex_build::VersionInfo {
    cortex_build::version_info!()
}
