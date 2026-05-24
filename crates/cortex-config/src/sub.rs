//! ADR-016 domain sub-structs.
//!
//! Each field carries:
//! - a `Default` covering the production-sane value,
//! - a doc comment for the operator-facing knob,
//! - and an entry in [`crate::env_map::KNOWN_ENV_NAMES`] so the
//!   env-overlay path knows which `CORTEX_*` name maps to it.
//!
//! Serde's own `#[serde(rename / alias)]` is NOT used for the
//! env layer — env names do not map cleanly to nested TOML
//! paths (`CORTEX_EMBEDDER_VECTORIZER_URL` →
//! `embedder.vectorizer_url`). The env overlay (see
//! [`crate::load`]) walks [`crate::env_map::KNOWN_ENV_NAMES`]
//! and stitches a JSON value with the right nesting before
//! handing it to serde for merge.

use serde::{Deserialize, Serialize};

// -------------------------------------------------------------
// Retention
// -------------------------------------------------------------

/// Retention sweep knobs. Mirrors the existing
/// `cortex-workers::retention` runtime config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetentionConfig {
    /// Override "now" for the sweep window calculation. RFC-3339;
    /// `None` means current wall-clock. Env: `CORTEX_RETENTION_NOW`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_override: Option<String>,
    /// Days before fp32 → pq tier transition.
    /// Env: `CORTEX_RETENTION_FP32_TO_PQ_DAYS`.
    #[serde(default = "default_fp32_to_pq_days")]
    pub fp32_to_pq_days: i64,
    /// Days before pq → binary tier transition.
    /// Env: `CORTEX_RETENTION_PQ_TO_BINARY_DAYS`.
    #[serde(default = "default_pq_to_binary_days")]
    pub pq_to_binary_days: i64,
    /// Batch size per sweep iteration.
    /// Env: `CORTEX_RETENTION_BATCH_SIZE`.
    #[serde(default = "default_retention_batch")]
    pub batch_size: u32,
}

fn default_fp32_to_pq_days() -> i64 {
    30
}
fn default_pq_to_binary_days() -> i64 {
    365
}
fn default_retention_batch() -> u32 {
    256
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            now_override: None,
            fp32_to_pq_days: default_fp32_to_pq_days(),
            pq_to_binary_days: default_pq_to_binary_days(),
            batch_size: default_retention_batch(),
        }
    }
}

// -------------------------------------------------------------
// Embedder
// -------------------------------------------------------------

/// Embedder worker knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbedderConfig {
    /// Vectorizer base URL. Env: `CORTEX_EMBEDDER_VECTORIZER_URL`.
    #[serde(default = "default_vectorizer_url")]
    pub vectorizer_url: String,
    /// Vectorizer admin user.
    /// Env: `CORTEX_EMBEDDER_VECTORIZER_USER`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vectorizer_user: Option<String>,
    /// Vectorizer admin password.
    /// Env: `CORTEX_EMBEDDER_VECTORIZER_PASSWORD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vectorizer_password: Option<String>,
    /// Synap base URL the embedder consumes from.
    /// Env: `CORTEX_EMBEDDER_SYNAP_URL`.
    #[serde(default = "default_embedder_synap_url")]
    pub synap_url: String,
    /// Collection prefix (deployment namespace).
    /// Env: `CORTEX_EMBEDDER_COLLECTION_PREFIX`.
    #[serde(default = "default_collection_prefix")]
    pub collection_prefix: String,
}

fn default_vectorizer_url() -> String {
    "http://127.0.0.1:17001".to_string()
}
fn default_embedder_synap_url() -> String {
    "http://127.0.0.1:15500".to_string()
}
fn default_collection_prefix() -> String {
    "cortex.".to_string()
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            vectorizer_url: default_vectorizer_url(),
            vectorizer_user: None,
            vectorizer_password: None,
            synap_url: default_embedder_synap_url(),
            collection_prefix: default_collection_prefix(),
        }
    }
}

// -------------------------------------------------------------
// Meili (fulltext)
// -------------------------------------------------------------

/// Meilisearch full-text indexer knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeiliConfig {
    /// Meili base URL. Env: `CORTEX_FULLTEXT_MEILI_URL`.
    #[serde(default = "default_meili_url")]
    pub meili_url: String,
    /// Meili master / API key. Env: `CORTEX_FULLTEXT_MEILI_API_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meili_api_key: Option<String>,
    /// Synap base URL the fulltext worker consumes from.
    /// Env: `CORTEX_FULLTEXT_SYNAP_URL`.
    #[serde(default = "default_fulltext_synap_url")]
    pub synap_url: String,
    /// Synap consumer group.
    /// Env: `CORTEX_FULLTEXT_SYNAP_GROUP`.
    #[serde(default = "default_fulltext_synap_group")]
    pub synap_group: String,
    /// Index name prefix (deployment namespace).
    /// Env: `CORTEX_FULLTEXT_INDEX_PREFIX`.
    #[serde(default = "default_index_prefix")]
    pub index_prefix: String,
}

fn default_meili_url() -> String {
    "http://127.0.0.1:17004".to_string()
}
fn default_fulltext_synap_url() -> String {
    "http://127.0.0.1:15500".to_string()
}
fn default_fulltext_synap_group() -> String {
    "cortex-fulltext".to_string()
}
fn default_index_prefix() -> String {
    "cortex-".to_string()
}

impl Default for MeiliConfig {
    fn default() -> Self {
        Self {
            meili_url: default_meili_url(),
            meili_api_key: None,
            synap_url: default_fulltext_synap_url(),
            synap_group: default_fulltext_synap_group(),
            index_prefix: default_index_prefix(),
        }
    }
}

// -------------------------------------------------------------
// Nexus (graph)
// -------------------------------------------------------------

/// Nexus graph store knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NexusConfig {
    /// Nexus base URL. Env: `CORTEX_NEXUS_URL` (also accepts
    /// `CORTEX_GRAPH_NEXUS_URL` for the graph worker's legacy
    /// per-subsystem override).
    #[serde(default = "default_nexus_url")]
    pub nexus_url: String,
    /// Nexus auth user. Env: `CORTEX_GRAPH_NEXUS_USER`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_user: Option<String>,
    /// Nexus auth password. Env: `CORTEX_GRAPH_NEXUS_PASSWORD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_password: Option<String>,
}

fn default_nexus_url() -> String {
    "http://127.0.0.1:17002".to_string()
}

impl Default for NexusConfig {
    fn default() -> Self {
        Self {
            nexus_url: default_nexus_url(),
            nexus_user: None,
            nexus_password: None,
        }
    }
}

// -------------------------------------------------------------
// Ingestion
// -------------------------------------------------------------

/// Ingestion router knobs (cortex-ingestion bin).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngestionConfig {
    /// Bind address. Env: `CORTEX_INGESTION_BIND` (also accepts
    /// `CORTEX_BIND`+`CORTEX_API_PORT` for legacy split form).
    #[serde(default = "default_ingestion_bind")]
    pub bind: String,
    /// Parquet archive root.
    /// Env: `CORTEX_ARCHIVE_ROOT` (also `CORTEX_HOME` + "/archive").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_root: Option<String>,
    /// Synap publisher URL.
    /// Env: `CORTEX_SYNAP_URL` (also `CORTEX_INGESTION_SYNAP_URL`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synap_url: Option<String>,
    /// zstd compression level for archive writes.
    /// Env: `CORTEX_ARCHIVE_ZSTD`.
    #[serde(default = "default_zstd")]
    pub archive_zstd_level: i32,
    /// Metadata SQLite DB path.
    /// Env: `CORTEX_METADATA_DB` (also `CORTEX_HOME` + "/metadata.sqlite").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_db: Option<String>,
}

fn default_ingestion_bind() -> String {
    "127.0.0.1:17010".to_string()
}
fn default_zstd() -> i32 {
    3
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            bind: default_ingestion_bind(),
            archive_root: None,
            synap_url: None,
            archive_zstd_level: default_zstd(),
            metadata_db: None,
        }
    }
}

// -------------------------------------------------------------
// Dashboard / /v1 surface
// -------------------------------------------------------------

/// Dashboard / cortex-api surface knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardConfig {
    /// API bind address. Env: `CORTEX_API_BIND`.
    #[serde(default = "default_api_bind")]
    pub api_bind: String,
    /// Synap base URL the API queries.
    /// Env: `CORTEX_API_SYNAP_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synap_url: Option<String>,
    /// Boot-time archive refresh interval (seconds).
    /// Env: `CORTEX_ARCHIVE_REFRESH_SECS`.
    #[serde(default = "default_refresh_secs")]
    pub archive_refresh_secs: u64,
    /// Meili refresh interval (seconds).
    /// Env: `CORTEX_MEILI_REFRESH_SECS`.
    #[serde(default = "default_refresh_secs")]
    pub meili_refresh_secs: u64,
}

fn default_api_bind() -> String {
    "127.0.0.1:17000".to_string()
}
fn default_refresh_secs() -> u64 {
    30
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            api_bind: default_api_bind(),
            synap_url: None,
            archive_refresh_secs: default_refresh_secs(),
            meili_refresh_secs: default_refresh_secs(),
        }
    }
}

// -------------------------------------------------------------
// Pre-thinking
// -------------------------------------------------------------

/// Pre-thinking bundle knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreThinkingConfig {
    /// Bundle size cap in KiB.
    /// Env: `CORTEX_PRE_THINKING_KB`.
    #[serde(default = "default_pre_thinking_kb")]
    pub bundle_kb: u32,
    /// Bundle assembly timeout in ms.
    /// Env: `CORTEX_PRE_THINKING_TIMEOUT_MS`.
    #[serde(default = "default_pre_thinking_timeout")]
    pub timeout_ms: u32,
}

fn default_pre_thinking_kb() -> u32 {
    16
}
fn default_pre_thinking_timeout() -> u32 {
    1500
}

impl Default for PreThinkingConfig {
    fn default() -> Self {
        Self {
            bundle_kb: default_pre_thinking_kb(),
            timeout_ms: default_pre_thinking_timeout(),
        }
    }
}
