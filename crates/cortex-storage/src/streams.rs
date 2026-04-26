//! Synap stream topology + TTL + KV declarations.

use serde::Serialize;

/// Declarative Synap stream configuration.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct StreamConfig {
    /// Stream name.
    pub name: &'static str,
    /// Target retention in seconds; `None` means "keep forever / service default".
    pub retention_seconds: Option<u64>,
    /// Partitions recommended for the stream.
    pub partitions: u32,
    /// True for best-effort broadcast streams (telemetry, pub/sub).
    pub best_effort: bool,
}

/// Every Synap stream + recommended config.
pub const STREAMS: &[StreamConfig] = &[
    StreamConfig {
        name: crate::names::STREAM_EVENTS_RAW,
        retention_seconds: Some(7 * 24 * 3600),
        partitions: 8,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_EVENTS_BOOTSTRAP,
        retention_seconds: Some(3 * 24 * 3600),
        partitions: 4,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_EVENTS_ENRICHED,
        retention_seconds: Some(7 * 24 * 3600),
        partitions: 8,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_EVENTS_EMBEDDED,
        retention_seconds: Some(3 * 24 * 3600),
        partitions: 4,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_EVENTS_GRAPHED,
        retention_seconds: Some(3 * 24 * 3600),
        partitions: 4,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_EVENTS_FULLTEXT_INDEXED,
        retention_seconds: Some(3 * 24 * 3600),
        partitions: 4,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_EVENTS_INVALID,
        retention_seconds: Some(30 * 24 * 3600),
        partitions: 1,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_VIOLATIONS,
        retention_seconds: Some(90 * 24 * 3600),
        partitions: 2,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_METRICS,
        retention_seconds: Some(24 * 3600),
        partitions: 1,
        best_effort: true,
    },
    StreamConfig {
        name: crate::names::STREAM_QUERY_AUDIT,
        retention_seconds: Some(30 * 24 * 3600),
        partitions: 2,
        best_effort: false,
    },
    StreamConfig {
        name: crate::names::STREAM_CACHE_INVALIDATE,
        retention_seconds: Some(3600),
        partitions: 1,
        best_effort: true,
    },
];

/// KV namespace + TTL recommendation (seconds).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct KvNamespace {
    /// Namespace (prefix before `:`).
    pub namespace: &'static str,
    /// TTL for entries in this namespace (seconds).
    pub ttl_seconds: u64,
    /// Human description.
    pub purpose: &'static str,
}

/// Every known KV namespace.
pub const KV_NAMESPACES: &[KvNamespace] = &[
    KvNamespace {
        namespace: crate::names::KV_CACHE_QUERY,
        ttl_seconds: 5 * 60,
        purpose: "Query-orchestrator result bundles",
    },
    KvNamespace {
        namespace: crate::names::KV_CACHE_CLASSIFY,
        ttl_seconds: 24 * 3600,
        purpose: "Classifier output keyed by content_hash",
    },
    KvNamespace {
        namespace: crate::names::KV_CACHE_EMBED,
        ttl_seconds: 3600,
        purpose: "Embedding retry-safety cache",
    },
    KvNamespace {
        namespace: crate::names::KV_BUDGET_CLASSIFIER,
        ttl_seconds: 25 * 3600,
        purpose: "Daily classifier spend counter",
    },
    KvNamespace {
        namespace: crate::names::KV_GOV_REMINDERS,
        ttl_seconds: 30 * 60,
        purpose: "Per-session governance reminders",
    },
];

