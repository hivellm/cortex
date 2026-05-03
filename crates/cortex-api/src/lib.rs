//! Cortex query API — hybrid retrieval (vector + keyword + graph)
//! with RRF fusion (spec 11).
//!
//! The crate exposes both an Axum HTTP service (`POST /v1/query`)
//! and an MCP tool binding (`cortex.query`) that share the same
//! request/response types so behaviour stays identical between
//! transports. Lane traits are defined here; live wiring against
//! Vectorizer / Meilisearch / Nexus drops in behind the same
//! traits and is selected at daemon startup.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod acl;
pub mod analyzer;
pub mod archive_loader;
pub mod audit;
pub mod audit_store;
pub mod auth;
pub mod budget;
pub mod cache;
pub mod canary;
pub mod config_audit;
pub mod coverage;
pub mod dashboard;
pub mod dashboard_consumer;
pub mod dashboard_series;
pub mod dashboard_watcher;
pub mod fusion;
pub mod health;
pub mod http;
pub mod ingest_proxy;
#[cfg(test)]
mod lane_contract;
pub mod lanes;
pub mod loader_metrics;
pub mod mcp;
pub mod meili_lane;
pub mod meili_loader;
pub mod nexus_graph_lane;
pub mod orchestrator;
pub mod query_rewrite;
pub mod rate_limit;
pub mod redaction;
pub mod relevance_config;
pub mod retention_daemon;
pub mod service;
pub mod silent_drop;
pub mod storage;
pub mod strategies;
pub mod tasks_loader;
pub mod types;
pub mod vectorizer_lane;

pub use acl::{AclDecision, AclStore};
pub use archive_loader::{
    load_into_keyword_lane, load_lane_hits, LoadError, LoadReport, DEFAULT_INDEX,
};
pub use audit::{build_envelope, AuditPublisher, MemoryAuditPublisher, STREAM_QUERY_AUDIT};
pub use audit_store::{AuditStore, DEFAULT_STORE_CAPACITY as AUDIT_STORE_CAPACITY};
pub use cache::{cache_key, Cache, CacheHandle, InMemoryCache, DEFAULT_TTL, SCHEMA_VERSION};
pub use dashboard::{build_dashboard_router, DashboardState};
pub use fusion::{rrf_fuse, FusionConfig, DEFAULT_RRF_ALPHA, RRF_K};
pub use http::{build_router, build_router_with, build_router_with_auth, CALLER_HEADER};
pub use lanes::{
    GraphLane, GraphRequest, KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, VectorLane, VectorRequest,
};
pub use loader_metrics::LoaderMetrics;
pub use meili_lane::MeiliKeywordLane;
pub use meili_loader::{load_meili_into_keyword_lane, MeiliLoadError, MeiliLoadReport};
pub use nexus_graph_lane::NexusGraphLane;
pub use relevance_config::{
    default_config_path as default_relevance_config_path, RelevanceConfig, RelevanceConfigError,
};
pub use tasks_loader::{
    CachedRowSnapshot, ListQuery, MultiTaskLoader, PhaseBreakdown, ProgressCounts, SortField,
    SortOrder, SpecFile, TaskChecklistItem, TaskChecklistSection, TaskDetail, TaskListResponse,
    TaskLoader, TaskRow, TaskSummary,
};
pub use vectorizer_lane::VectorizerLane;

/// Phase8c — capture cortex-api's own compile-time version block.
/// Wraps the [`cortex_build::version_info!`] macro so the `health`
/// module (and tests) can read it without expanding the macro at
/// every call site.
pub fn self_version_info() -> cortex_build::VersionInfo {
    cortex_build::version_info!()
}
pub use mcp::{invoke as mcp_invoke, tool_descriptor, McpError, TOOL_NAME};
pub use orchestrator::Orchestrator;
pub use rate_limit::{RateConfig, RateDecision, RateLimiter};
pub use redaction::redact_response;
pub use service::{ErrorBody, QueryService, ServiceOutcome};
pub use strategies::{build_plan, BudgetSplit, Overlay, Plan};
pub use types::{
    empty_response, BudgetReport, DebugInfo, DecisionRef, GraphNeighbor, IncludeField, Intent,
    ConsolidationRef, LaneTimings, LawRef, Notice, PastSession, Props, QueryRequest,
    QueryResponse, ResultsBag, Scope, SimilarTurn, Snippet, ViolationRef,
};
