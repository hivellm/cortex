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

/// Embedder worker knobs. Mirrors the legacy
/// `cortex_workers::embedder::EmbedderConfig` field-for-field
/// so the worker's `from_env()` can route through
/// `cortex_config::Config::load()` without changing its public
/// struct shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbedderConfig {
    /// Number of concurrent Synap-consumer worker tasks.
    /// Env: `CORTEX_EMBEDDER_WORKERS`.
    #[serde(default = "default_embedder_workers")]
    pub workers: usize,
    /// Chunker parallelism per worker task.
    /// Env: `CORTEX_EMBEDDER_CHUNKER_CONCURRENCY`.
    #[serde(default = "default_embedder_chunker_concurrency")]
    pub chunker_concurrency: usize,
    /// Maximum chunks per Vectorizer upsert call.
    /// Env: `CORTEX_EMBEDDER_UPSERT_BATCH`.
    #[serde(default = "default_embedder_upsert_batch")]
    pub upsert_batch: usize,
    /// Maximum retry attempts for Vectorizer requests.
    /// Env: `CORTEX_EMBEDDER_MAX_RETRY`.
    #[serde(default = "default_embedder_max_retry")]
    pub max_retry: u32,
    /// Vectorizer base URL. `None` means "not configured" — the API
    /// live-lane gate stays on MemoryVectorLane when absent. Workers
    /// that require a URL fall back to a hardcoded default.
    /// Env: `CORTEX_EMBEDDER_VECTORIZER_URL` (also
    /// `CORTEX_VECTORIZER_URL` legacy alias).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vectorizer_url: Option<String>,
    /// Synap base URL the embedder consumes from.
    /// Env: `CORTEX_EMBEDDER_SYNAP_URL`.
    #[serde(default = "default_embedder_synap_url")]
    pub synap_url: String,
    /// Vectorizer admin user.
    /// Env: `CORTEX_EMBEDDER_VECTORIZER_USER`.
    #[serde(default = "default_embedder_vectorizer_user")]
    pub vectorizer_user: String,
    /// Vectorizer admin password.
    /// Env: `CORTEX_EMBEDDER_VECTORIZER_PASSWORD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vectorizer_password: Option<String>,
    /// Vectorizer API key (bearer token). Takes precedence over
    /// `vectorizer_user` + `vectorizer_password` when set.
    /// Env: `CORTEX_VECTORIZER_API_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vectorizer_api_key: Option<String>,
    /// Proactive JWT warmup interval in seconds (0 = disabled).
    /// When login credentials are configured and `warmup_secs > 0`,
    /// the API spawns a background loop that calls `/auth/refresh`
    /// every N seconds to keep the cached token fresh.
    /// Env: `CORTEX_VECTORIZER_JWT_WARMUP_SECS`.
    #[serde(default)]
    pub jwt_warmup_secs: u64,
    /// Pre-minted Vectorizer JWT for the API's search proxy. When
    /// set, the proxy presents this bearer instead of running the
    /// `/auth/login` flow itself. Env:
    /// `CORTEX_EMBEDDER_VECTORIZER_JWT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vectorizer_jwt: Option<String>,
    /// Collection prefix (deployment namespace).
    /// Env: `CORTEX_EMBEDDER_COLLECTION_PREFIX`.
    #[serde(default = "default_collection_prefix")]
    pub collection_prefix: String,
    /// Vector dimension to provision new collections with.
    /// Env: `CORTEX_EMBEDDER_DIM`.
    #[serde(default = "default_embedder_vector_dim")]
    pub vector_dim: u32,
}

fn default_embedder_synap_url() -> String {
    "http://127.0.0.1:17003".to_string()
}
fn default_collection_prefix() -> String {
    "cortex".to_string()
}
fn default_embedder_workers() -> usize {
    6
}
fn default_embedder_chunker_concurrency() -> usize {
    4
}
fn default_embedder_upsert_batch() -> usize {
    64
}
fn default_embedder_max_retry() -> u32 {
    3
}
fn default_embedder_vectorizer_user() -> String {
    "admin".to_string()
}
fn default_embedder_vector_dim() -> u32 {
    768
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            workers: default_embedder_workers(),
            chunker_concurrency: default_embedder_chunker_concurrency(),
            upsert_batch: default_embedder_upsert_batch(),
            max_retry: default_embedder_max_retry(),
            vectorizer_url: None,
            synap_url: default_embedder_synap_url(),
            vectorizer_user: default_embedder_vectorizer_user(),
            vectorizer_password: None,
            vectorizer_api_key: None,
            jwt_warmup_secs: 0,
            vectorizer_jwt: None,
            collection_prefix: default_collection_prefix(),
            vector_dim: default_embedder_vector_dim(),
        }
    }
}

// -------------------------------------------------------------
// Meili (fulltext)
// -------------------------------------------------------------

/// Meilisearch full-text indexer knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeiliConfig {
    /// Meili base URL. `None` means "not configured" — the API
    /// live keyword-lane gate stays on MemoryKeywordLane when absent.
    /// Workers that require a URL fall back to `http://127.0.0.1:7700`.
    /// Env: `CORTEX_FULLTEXT_MEILI_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meili_url: Option<String>,
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
    /// Number of concurrent worker tasks.
    /// Env: `CORTEX_FULLTEXT_WORKERS`.
    #[serde(default = "default_fulltext_workers")]
    pub workers: usize,
    /// Maximum documents per Meili upsert call.
    /// Env: `CORTEX_FULLTEXT_BATCH`.
    #[serde(default = "default_fulltext_batch")]
    pub upsert_batch: usize,
    /// Coalesced micro-batch flush interval in milliseconds.
    /// Env: `CORTEX_FULLTEXT_FLUSH_MS`.
    #[serde(default = "default_fulltext_flush_ms")]
    pub flush_ms: u64,
    /// Maximum retry attempts for transient Meili errors.
    /// Env: `CORTEX_FULLTEXT_MAX_RETRY`.
    #[serde(default = "default_fulltext_max_retry")]
    pub max_retry: u32,
    /// When true, `upsert_documents` waits on the returned task.
    /// Env: `CORTEX_FULLTEXT_AWAIT_TASK`.
    #[serde(default)]
    pub await_task: bool,
    /// Maximum body size in bytes before truncation kicks in.
    /// Env: `CORTEX_FULLTEXT_MAX_BODY_BYTES`.
    #[serde(default = "default_fulltext_max_body_bytes")]
    pub max_body_bytes: usize,
    /// When true, the fulltext-indexer bin walks the archive on boot
    /// and replays missing partitions before consuming Synap.
    /// Env: `CORTEX_FULLTEXT_REPLAY_MISSING`.
    #[serde(default)]
    pub replay_missing: bool,
    /// When true, the fulltext routing layer indexes low-signal
    /// `tool_call` envelopes (otherwise they are dropped at routing).
    /// Env: `CORTEX_INDEX_LOW_SIGNAL_TOOL_CALLS`.
    #[serde(default)]
    pub index_low_signal_tool_calls: bool,
}

fn default_fulltext_synap_url() -> String {
    "http://127.0.0.1:17003".to_string()
}
fn default_fulltext_synap_group() -> String {
    "cortex-fulltext".to_string()
}
fn default_index_prefix() -> String {
    "cortex-".to_string()
}
fn default_fulltext_workers() -> usize {
    4
}
fn default_fulltext_batch() -> usize {
    1_000
}
fn default_fulltext_flush_ms() -> u64 {
    1_000
}
fn default_fulltext_max_retry() -> u32 {
    3
}
fn default_fulltext_max_body_bytes() -> usize {
    10 * 1024 * 1024
}

impl Default for MeiliConfig {
    fn default() -> Self {
        Self {
            meili_url: None,
            meili_api_key: None,
            synap_url: default_fulltext_synap_url(),
            synap_group: default_fulltext_synap_group(),
            index_prefix: default_index_prefix(),
            workers: default_fulltext_workers(),
            upsert_batch: default_fulltext_batch(),
            flush_ms: default_fulltext_flush_ms(),
            max_retry: default_fulltext_max_retry(),
            await_task: false,
            max_body_bytes: default_fulltext_max_body_bytes(),
            replay_missing: false,
            index_low_signal_tool_calls: false,
        }
    }
}

// -------------------------------------------------------------
// Nexus (graph)
// -------------------------------------------------------------

/// Nexus graph store knobs + graph-worker tuning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NexusConfig {
    /// Nexus base URL. `None` means "not configured" — the API graph
    /// lane and dashboard graph endpoint degrade to synthetic fallback.
    /// Workers fall back to `http://127.0.0.1:17002`. Env:
    /// `CORTEX_NEXUS_URL` (also `CORTEX_GRAPH_NEXUS_URL` legacy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_url: Option<String>,
    /// Transport selector — string form `"auto"` / `"rpc"` /
    /// `"http"` / aliases (`"bolt"`, `"nexus"`, `"https"`).
    /// Workers parse to their typed `GraphTransport` enum.
    /// Env: `CORTEX_GRAPH_TRANSPORT`.
    #[serde(default = "default_graph_transport")]
    pub transport: String,
    /// Synap base URL the graph worker consumes from.
    /// Env: `CORTEX_GRAPH_SYNAP_URL`.
    #[serde(default = "default_graph_synap_url")]
    pub synap_url: String,
    /// Synap consumer group.
    /// Env: `CORTEX_GRAPH_SYNAP_GROUP`.
    #[serde(default = "default_graph_synap_group")]
    pub synap_group: String,
    /// Concurrent worker tasks.
    /// Env: `CORTEX_GRAPH_WORKERS`.
    #[serde(default = "default_graph_workers")]
    pub workers: usize,
    /// Max nodes + edges per Cypher tx.
    /// Env: `CORTEX_GRAPH_PATCH_BATCH`.
    #[serde(default = "default_graph_patch_batch")]
    pub patch_batch: usize,
    /// Micro-batch flush interval (ms).
    /// Env: `CORTEX_GRAPH_FLUSH_MS`.
    #[serde(default = "default_graph_flush_ms")]
    pub flush_ms: u64,
    /// Max retry attempts for transient Nexus errors.
    /// Env: `CORTEX_GRAPH_MAX_RETRY`.
    #[serde(default = "default_graph_max_retry")]
    pub max_retry: u32,
    /// Nexus auth user. Env: `CORTEX_GRAPH_NEXUS_USER`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_user: Option<String>,
    /// Nexus auth password. Env: `CORTEX_GRAPH_NEXUS_PASSWORD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_password: Option<String>,
    /// Nexus API key (bearer token). Distinct from `nexus_password` —
    /// used by the cortex-api dashboard graph endpoint rather than
    /// the graph-writer worker. Env: `CORTEX_NEXUS_API_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_api_key: Option<String>,
    /// Out-of-order tolerance window (seconds) for buffering a
    /// `tool_call` waiting for its `turn.start` before
    /// fabricating an orphan Turn.
    /// Env: `CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS`.
    #[serde(default = "default_graph_out_of_order_secs")]
    pub out_of_order_buffer_secs: u64,
    /// Graph worker metadata DB path override.
    /// Env: `CORTEX_GRAPH_METADATA_DB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_db: Option<String>,
    /// Directory holding Cypher templates the graph-writer +
    /// graph-backfill bins load on boot.
    /// Env: `CORTEX_GRAPH_CYPHER_DIR`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cypher_dir: Option<String>,
    /// Synap consumer id stamped on commits.
    /// Env: `CORTEX_GRAPH_CONSUMER_ID`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer_id: Option<String>,
    /// When true, the MCP `cortex_graph_query` surface accepts raw
    /// Cypher (kill-switch off in production unless signed).
    /// Env: `CORTEX_GRAPH_CYPHER_ENABLED`.
    #[serde(default)]
    pub cypher_enabled: bool,
    /// Stale-edge sweeper tick interval in seconds. `None` uses the
    /// compile-time default. Env: `CORTEX_GRAPH_SWEEPER_INTERVAL_SECS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweeper_interval_secs: Option<u64>,
}

fn default_graph_transport() -> String {
    "auto".to_string()
}
fn default_graph_synap_url() -> String {
    "http://127.0.0.1:17003".to_string()
}
fn default_graph_synap_group() -> String {
    "cortex-graph".to_string()
}
fn default_graph_workers() -> usize {
    4
}
fn default_graph_patch_batch() -> usize {
    256
}
fn default_graph_flush_ms() -> u64 {
    500
}
fn default_graph_max_retry() -> u32 {
    3
}
fn default_graph_out_of_order_secs() -> u64 {
    30
}

impl Default for NexusConfig {
    fn default() -> Self {
        Self {
            nexus_url: None,
            transport: default_graph_transport(),
            synap_url: default_graph_synap_url(),
            synap_group: default_graph_synap_group(),
            workers: default_graph_workers(),
            patch_batch: default_graph_patch_batch(),
            flush_ms: default_graph_flush_ms(),
            max_retry: default_graph_max_retry(),
            nexus_user: None,
            nexus_password: None,
            nexus_api_key: None,
            out_of_order_buffer_secs: default_graph_out_of_order_secs(),
            metadata_db: None,
            cypher_dir: None,
            consumer_id: None,
            cypher_enabled: false,
            sweeper_interval_secs: None,
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
    /// Cortex home directory override. When set, `archive_root` and
    /// `metadata_db` use it as a base when they are not individually
    /// overridden. Env: `CORTEX_HOME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Comma-separated list of upstream archive-watcher URLs the
    /// coverage audit probes for archive-side keepers.
    /// Env: `CORTEX_ARCHIVE_WATCHER_URLS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_watcher_urls: Option<String>,
    /// Path the pruner writes its post-run status JSON to. `None`
    /// falls back to `<home>/.cortex/pruner-status.json`.
    /// Env: `CORTEX_PRUNER_STATUS_FILE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruner_status_file: Option<String>,
    /// Client-side URL of the ingestion router (cortex-ingestion or
    /// cortex-ops replay). Distinct from `bind` which is the
    /// listen-side address. Env: `CORTEX_INGESTION_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingestion_url: Option<String>,
    /// CAS SQLite database path (adjacent to the archive).
    /// Env: `CORTEX_CAS_DB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cas_db: Option<String>,
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
            home: None,
            ingestion_url: None,
            cas_db: None,
            archive_watcher_urls: None,
            pruner_status_file: None,
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
    /// Path to the `api_keys.sqlite` file. When `None`, the daemon
    /// falls back to `~/.cortex/api_keys.sqlite`.
    /// Env: `CORTEX_API_KEYS_DB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_keys_db: Option<String>,
    /// Enable the dashboard filesystem watcher (default `true`).
    /// Set to `false` to disable SSE push events for cold-stack tests.
    /// Env: `CORTEX_DASHBOARD_WATCH`.
    #[serde(default = "default_true")]
    pub watch: bool,
    /// Enable the SQLite memory tail loop (default `true`).
    /// Gated independently from `watch` so operators can disable
    /// only the DB polling without disabling FS watchers.
    /// Env: `CORTEX_DASHBOARD_MEMORY_TAIL`.
    #[serde(default = "default_true")]
    pub memory_tail: bool,
    /// Comma-separated repo slugs to include in the boot-time
    /// coverage audit. `None` falls back to slugs the keyword
    /// lane has already learned. Env: `CORTEX_COVERAGE_SLUGS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_slugs: Option<String>,
    /// Comma-separated allow-list — when set, the on-demand
    /// `/v1/health/coverage` audit restricts the slug set to these.
    /// Stops classifier-stamped language tags (rust, python, …) that
    /// aren't real repos from inflating the "missing" count.
    /// Env: `CORTEX_COVERAGE_SLUGS_ONLY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_slugs_only: Option<String>,
    /// RRF alpha blend weight (`0.0`–`1.0`). `None` uses the
    /// `relevance.toml`-derived default. Env: `CORTEX_RRF_ALPHA`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrf_alpha: Option<f32>,
    /// RRF stabilisation constant (`>= 1`). `None` uses the
    /// `relevance.toml`-derived default. Env: `CORTEX_RRF_K`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrf_k: Option<u32>,
    /// Query rewriter strategy. One of `noun_phrase` (default),
    /// `sonnet`, or `passthrough`. Unknown values fall back to
    /// `noun_phrase` with a WARN log. Env: `CORTEX_QUERY_REWRITER`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_rewriter: Option<String>,
    /// Sonnet rewriter model id. `None` uses the compile-time
    /// default `claude-sonnet-4-6`. Env: `CORTEX_REWRITER_MODEL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewriter_model: Option<String>,
    /// Sonnet rewriter per-call timeout in milliseconds (minimum
    /// 100ms). `None` uses the compile-time default 1500ms.
    /// Env: `CORTEX_REWRITER_TIMEOUT_MS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewriter_timeout_ms: Option<u64>,
    /// Override path for `relevance.toml`. `None` resolves to the
    /// in-tree default via `default_relevance_config_path()`.
    /// Env: `CORTEX_RELEVANCE_CONFIG`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relevance_config_path: Option<String>,
    /// Bearer token the CLI uses when calling the cortex-api.
    /// Env: `CORTEX_API_TOKEN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
    /// Client-side URL of the cortex-api (distinct from `api_bind`
    /// which is the listen-side address). Env: `CORTEX_API_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
}

fn default_api_bind() -> String {
    "127.0.0.1:17000".to_string()
}
fn default_refresh_secs() -> u64 {
    30
}
fn default_true() -> bool {
    true
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            api_bind: default_api_bind(),
            synap_url: None,
            archive_refresh_secs: default_refresh_secs(),
            meili_refresh_secs: default_refresh_secs(),
            api_keys_db: None,
            watch: true,
            memory_tail: true,
            coverage_slugs: None,
            coverage_slugs_only: None,
            rrf_alpha: None,
            rrf_k: None,
            query_rewriter: None,
            rewriter_model: None,
            rewriter_timeout_ms: None,
            relevance_config_path: None,
            api_token: None,
            api_url: None,
        }
    }
}

// -------------------------------------------------------------
// Rulebook
// -------------------------------------------------------------

/// Rulebook task-loader knobs.
///
/// Drives the multi-project rulebook roots feature:
/// - `roots` takes a comma- or semicolon-separated list of
///   `.rulebook/` directories; one `TaskLoader` per entry,
///   slug derived from each directory's parent name.
/// - `root` is the legacy single-project path.
/// - Falls back to `<cwd>/.rulebook` when both are `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RulebookConfig {
    /// Comma- or semicolon-separated `.rulebook/` directories
    /// for multi-project deployments. Wins over `root` when set.
    /// Env: `CORTEX_RULEBOOK_ROOTS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<String>,
    /// Single `.rulebook/` directory (legacy single-project path).
    /// Env: `CORTEX_RULEBOOK_ROOT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

// -------------------------------------------------------------
// Canary
// -------------------------------------------------------------

/// Opt-in synthetic canary runner knobs.
///
/// When `enabled = true`, the API fires a fake hook frame through
/// the real IPC pipe every `interval_secs` and asserts it lands
/// in the archive within `deadline_secs`. On failure a
/// `law_violation` envelope is emitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanaryConfig {
    /// Enable the canary runner. Default `false`.
    /// Env: `CORTEX_CANARY_ENABLED`.
    #[serde(default)]
    pub enabled: bool,
    /// Fire interval in seconds (minimum 10).
    /// Env: `CORTEX_CANARY_INTERVAL_SECS`.
    #[serde(default = "default_canary_interval")]
    pub interval_secs: u64,
    /// Archive-landing deadline in seconds (minimum 1).
    /// Env: `CORTEX_CANARY_DEADLINE_SECS`.
    #[serde(default = "default_canary_deadline")]
    pub deadline_secs: u64,
}

fn default_canary_interval() -> u64 {
    300
}
fn default_canary_deadline() -> u64 {
    10
}

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_canary_interval(),
            deadline_secs: default_canary_deadline(),
        }
    }
}

// -------------------------------------------------------------
// Pre-thinking
// -------------------------------------------------------------

/// Pre-thinking bundle knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreThinkingConfig {
    /// Bundle size cap in KiB. Acts as the fallback for any
    /// intent not present in [`Self::budget_per_intent`].
    /// Env: `CORTEX_PRE_THINKING_KB`.
    #[serde(default = "default_pre_thinking_kb")]
    pub bundle_kb: u32,
    /// Bundle assembly timeout in ms.
    /// Env: `CORTEX_PRE_THINKING_TIMEOUT_MS`.
    #[serde(default = "default_pre_thinking_timeout")]
    pub timeout_ms: u32,
    /// Phase14f §2 — per-intent bundle cap (KiB). Keyed on the
    /// canonical lower-snake-case intent label
    /// (`pre_change_context`, `explain`, `decision_lookup`,
    /// `similar_problems`, `law_check`, `free_search`,
    /// `coverage`). Missing keys fall through to
    /// [`Self::bundle_kb`]. Defaults follow the F-005 spec table:
    /// `pre_change_context`=32, `similar_problems`=32, `explain`=24,
    /// `decision_lookup`=24, `coverage`=16, `law_check`=12,
    /// `free_search`=16.
    #[serde(default = "default_budget_per_intent")]
    pub budget_per_intent: std::collections::BTreeMap<String, u32>,
}

fn default_pre_thinking_kb() -> u32 {
    16
}
fn default_pre_thinking_timeout() -> u32 {
    1500
}
fn default_budget_per_intent() -> std::collections::BTreeMap<String, u32> {
    let mut m = std::collections::BTreeMap::new();
    m.insert("pre_change_context".to_string(), 32);
    m.insert("similar_problems".to_string(), 32);
    m.insert("explain".to_string(), 24);
    m.insert("decision_lookup".to_string(), 24);
    m.insert("coverage".to_string(), 16);
    m.insert("law_check".to_string(), 12);
    m.insert("free_search".to_string(), 16);
    m
}

impl PreThinkingConfig {
    /// Phase14f — resolve the per-intent cap in bytes. Returns
    /// [`Self::budget_per_intent`] entry × 1 KiB when present,
    /// else [`Self::bundle_kb`] × 1 KiB.
    pub fn budget_bytes_for(&self, intent_label: &str) -> u32 {
        let kb = self
            .budget_per_intent
            .get(intent_label)
            .copied()
            .unwrap_or(self.bundle_kb);
        kb.saturating_mul(1024)
    }
}

impl Default for PreThinkingConfig {
    fn default() -> Self {
        Self {
            bundle_kb: default_pre_thinking_kb(),
            timeout_ms: default_pre_thinking_timeout(),
            budget_per_intent: default_budget_per_intent(),
        }
    }
}

// -------------------------------------------------------------
// Doctor
// -------------------------------------------------------------

/// Doctor subcommand knobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DoctorConfig {
    /// Enable the 100k-row budget gate test (expensive; default `false`).
    /// Set to `true` in CI to enforce the §4.2 latency contract.
    /// Env: `CORTEX_DOCTOR_BENCH`.
    #[serde(default)]
    pub bench: bool,
}

// -------------------------------------------------------------
// Classifier
// -------------------------------------------------------------

/// Classifier worker knobs (both health-probe and worker config).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ClassifierConfig {
    /// HTTP URL of the classifier worker's `/healthz` endpoint.
    /// When `None`, the doctor skips the classifier probe.
    /// Env: `CORTEX_CLASSIFIER_HEALTH_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_url: Option<String>,
    /// Staleness threshold in milliseconds. When
    /// `last_consume_ts_ms` is older than this, the worker is
    /// flagged as degraded. Default `None` means 60 000 ms.
    /// Env: `CORTEX_CLASSIFIER_STALENESS_MS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staleness_ms: Option<u64>,
    /// Max consecutive consume errors before the worker self-exits.
    /// `0` disables the supervisor (pre-§1.2 behaviour).
    /// Env: `CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_consume_errors: Option<u64>,
    /// Synap base URL the classifier consumes from.
    /// Env: `CORTEX_CLASSIFIER_SYNAP_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synap_url: Option<String>,
    /// Classifier mode (`heuristic` / `claude` / `passthrough`).
    /// Env: `CORTEX_CLASSIFIER_MODE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Prompt version identifier baked into outgoing audit envelopes.
    /// Env: `CORTEX_CLASSIFIER_PROMPT_VERSION`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_version: Option<String>,
    /// Anthropic model id the classifier calls.
    /// Env: `CORTEX_CLASSIFIER_MODEL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

// -------------------------------------------------------------
// Consolidator
// -------------------------------------------------------------

/// Consolidator knobs (JSONL fallback path + cursor + rotation).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatorConfig {
    /// Path to the JSONL fallback file the consolidator writes
    /// when the ingestion router is unreachable.
    /// Env: `CORTEX_CONSOLIDATIONS_FALLBACK_FILE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_file: Option<String>,
    /// Byte ceiling that triggers JSONL rotation (per-run cap).
    /// `None` uses the compile-time default.
    /// Env: `CORTEX_CONSOLIDATIONS_FALLBACK_ROTATE_BYTES`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_rotate_bytes: Option<u64>,
    /// Path the consolidator stores its tail cursor in.
    /// Env: `CORTEX_CONSOLIDATOR_CURSOR_FILE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_file: Option<String>,
}

// -------------------------------------------------------------
// Auto-memory
// -------------------------------------------------------------

/// Auto-memory integration knobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AutoMemoryConfig {
    /// Rulebook project slug for auto-memory directory resolution.
    /// When set, `cortex-ops memory-consolidate` targets
    /// `~/.claude/projects/<slug>/memory/` without needing `--project`.
    /// Env: `CORTEX_AUTO_MEMORY_PROJECT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

// -------------------------------------------------------------
// Analyzer (Claude Code CLI rewriter / summariser)
// -------------------------------------------------------------

/// `cortex-api::analyzer::Analyzer` knobs — the Claude Code CLI
/// binding for prompt-side rewrites.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AnalyzerConfig {
    /// Claude Code CLI binary name. Falls back to `CLAUDE_CODE_BIN`
    /// (non-CORTEX env) then `"claude"`.
    /// Env: `CORTEX_ANALYZER_BIN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<String>,
    /// Anthropic model id the analyzer calls. `None` falls back to
    /// the compile-time default `claude-sonnet-4-6`.
    /// Env: `CORTEX_ANALYZER_MODEL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Anthropic API key. Falls back to `ANTHROPIC_API_KEY`
    /// (non-CORTEX env). Env: `CORTEX_ANALYZER_API_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Anthropic API base URL override (default
    /// `https://api.anthropic.com`). Env: `CORTEX_ANALYZER_API_BASE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

// -------------------------------------------------------------
// Claude archive (cortex-claude-archive bin)
// -------------------------------------------------------------

/// Bind / poll / root knobs for `cortex-claude-archive`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ClaudeArchiveConfig {
    /// Bind address for the IPC socket / pipe. Env:
    /// `CORTEX_CLAUDE_ARCHIVE_BIND`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    /// Poll interval in milliseconds. Env:
    /// `CORTEX_CLAUDE_ARCHIVE_POLL_MS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_ms: Option<u64>,
    /// Root directory the watcher tails (defaults to
    /// `~/.claude/projects` when unset). Env:
    /// `CORTEX_CLAUDE_ARCHIVE_ROOT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

// -------------------------------------------------------------
// Adapter (cortex-adapter-claude-code)
// -------------------------------------------------------------

/// Claude Code adapter knobs (hook shim + admin port).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AdapterConfig {
    /// When true, the `cortex-hook` shim forces the legacy
    /// fallback transport (named pipe / unix socket) instead of the
    /// direct daemon connection. Env: `CORTEX_HOOK_FORCE_FALLBACK`.
    #[serde(default)]
    pub hook_force_fallback: bool,
    /// When true, the adapter exits early on every hook event
    /// (kill-switch). Env: `CORTEX_ADAPTER_DISABLE`.
    #[serde(default)]
    pub adapter_disable: bool,
    /// Override path for the Windows named pipe the adapter
    /// connects through. Env: `CORTEX_ADAPTER_PIPE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_pipe: Option<String>,
    /// Override path for the Unix domain socket the adapter
    /// connects through. Env: `CORTEX_ADAPTER_SOCK`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_sock: Option<String>,
    /// TCP port for the local admin endpoint. `None` defaults
    /// inside the adapter. Env: `CORTEX_ADAPTER_ADMIN_PORT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_admin_port: Option<u16>,
}

// -------------------------------------------------------------
// MCP server (cortex-mcp-server)
// -------------------------------------------------------------

/// Phase14i §2.3 — per-tool timeout knobs for `cortex-mcp-server`.
///
/// `default_timeout_ms` applies to every tool whose name is not in
/// `tool_timeout_ms` (which holds per-tool overrides keyed by MCP
/// tool name, e.g. `cortex_query`). Unset means the in-crate
/// `MCP_TOOL_TIMEOUT_MS = 5_000` default applies.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpConfig {
    /// Default per-call timeout in milliseconds. `None` falls back
    /// to `cortex_mcp_server::MCP_TOOL_TIMEOUT_MS` (5_000 ms).
    /// Env: `CORTEX_MCP_TOOL_TIMEOUT_MS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_timeout_ms: Option<u64>,
    /// Per-tool timeout overrides keyed by MCP tool name. Env keys
    /// follow the convention
    /// `CORTEX_MCP_TOOL_TIMEOUT_MS_{TOOL_NAME_UPPER}`; the env
    /// reader uppercases the tool name and replaces non-alnum
    /// characters with `_`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tool_timeout_ms: std::collections::BTreeMap<String, u64>,
}

impl McpConfig {
    /// Resolve the timeout for `tool_name`. Returns `None` when no
    /// override is configured at any level so the caller falls back
    /// to the in-crate default.
    pub fn timeout_ms_for(&self, tool_name: &str) -> Option<u64> {
        self.tool_timeout_ms
            .get(tool_name)
            .copied()
            .or(self.default_timeout_ms)
    }
}

#[cfg(test)]
mod tests_mcp_config {
    use super::*;

    #[test]
    fn timeout_for_falls_back_to_default_when_per_tool_absent() {
        let cfg = McpConfig {
            default_timeout_ms: Some(7_000),
            ..McpConfig::default()
        };
        assert_eq!(cfg.timeout_ms_for("cortex_query"), Some(7_000));
    }

    #[test]
    fn per_tool_override_shadows_default() {
        let mut tool_timeout_ms = std::collections::BTreeMap::new();
        tool_timeout_ms.insert("cortex_query".to_string(), 12_000);
        let cfg = McpConfig {
            default_timeout_ms: Some(5_000),
            tool_timeout_ms,
        };
        assert_eq!(cfg.timeout_ms_for("cortex_query"), Some(12_000));
        assert_eq!(cfg.timeout_ms_for("cortex_status"), Some(5_000));
    }

    #[test]
    fn unset_fields_return_none_so_caller_uses_in_crate_default() {
        let cfg = McpConfig::default();
        assert_eq!(cfg.timeout_ms_for("any_tool"), None);
    }

    #[test]
    fn toml_round_trips_per_tool_map() {
        let mut tool_timeout_ms = std::collections::BTreeMap::new();
        tool_timeout_ms.insert("cortex_query".into(), 10_000u64);
        tool_timeout_ms.insert("cortex_pre_thinking".into(), 8_000u64);
        let cfg = McpConfig {
            default_timeout_ms: Some(5_000),
            tool_timeout_ms,
        };
        let s = toml::to_string(&cfg).expect("serialise");
        let back: McpConfig = toml::from_str(&s).expect("deserialise");
        assert_eq!(back, cfg);
    }
}

#[cfg(test)]
mod tests_phase14f {
    use super::*;

    #[test]
    fn default_budget_per_intent_carries_the_f005_table() {
        let cfg = PreThinkingConfig::default();
        assert_eq!(cfg.budget_per_intent.get("pre_change_context").copied(), Some(32));
        assert_eq!(cfg.budget_per_intent.get("similar_problems").copied(), Some(32));
        assert_eq!(cfg.budget_per_intent.get("explain").copied(), Some(24));
        assert_eq!(cfg.budget_per_intent.get("decision_lookup").copied(), Some(24));
        assert_eq!(cfg.budget_per_intent.get("coverage").copied(), Some(16));
        assert_eq!(cfg.budget_per_intent.get("law_check").copied(), Some(12));
        assert_eq!(cfg.budget_per_intent.get("free_search").copied(), Some(16));
    }

    #[test]
    fn budget_bytes_for_returns_per_intent_when_set() {
        let cfg = PreThinkingConfig::default();
        assert_eq!(cfg.budget_bytes_for("law_check"), 12 * 1024);
        assert_eq!(cfg.budget_bytes_for("pre_change_context"), 32 * 1024);
    }

    #[test]
    fn budget_bytes_for_falls_back_to_bundle_kb_when_intent_missing() {
        let mut cfg = PreThinkingConfig::default();
        cfg.budget_per_intent.clear();
        cfg.bundle_kb = 7;
        assert_eq!(cfg.budget_bytes_for("explain"), 7 * 1024);
    }

    #[test]
    fn toml_round_trips_per_intent_table() {
        let cfg = PreThinkingConfig::default();
        let s = toml::to_string(&cfg).expect("serialise");
        let back: PreThinkingConfig = toml::from_str(&s).expect("deserialise");
        assert_eq!(back.budget_per_intent, cfg.budget_per_intent);
        assert_eq!(back.bundle_kb, cfg.bundle_kb);
    }
}
