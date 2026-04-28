//! Runtime configuration for the full-text indexer worker, parsed from
//! `CORTEX_FULLTEXT_*` environment variables. Mirrors the embedder /
//! graph-writer config layout so operations work the same way across
//! the three Phase-1 worker binaries.

use std::env;

/// Full-text-worker configuration.
#[derive(Debug, Clone)]
pub struct FulltextConfig {
    /// Meilisearch base URL. Default `http://127.0.0.1:7700`.
    pub meili_url: String,
    /// Optional Meilisearch master / API key.
    pub meili_api_key: Option<String>,
    /// Synap base URL.
    pub synap_url: String,
    /// Synap consumer-group label (carried through metadata; Synap 0.11
    /// has no durable groups in the SDK).
    pub synap_group: String,
    /// Index-name prefix — every per-kind index is named
    /// `<prefix><family>` (e.g. `cortex-code`, `cortex-decisions`).
    pub index_prefix: String,
    /// Number of concurrent worker tasks.
    pub workers: usize,
    /// Maximum documents per Meilisearch upsert call.
    pub upsert_batch: usize,
    /// Flush interval in milliseconds — coalesced micro-batches are
    /// written at most every `flush_ms`.
    pub flush_ms: u64,
    /// Maximum retry attempts for transient Meili errors.
    pub max_retry: u32,
    /// When `true`, `upsert_documents` waits on the returned task to
    /// confirm completion. Defaults to `false`; bootstrap flips it on
    /// to fail fast on schema / parse errors.
    pub await_task: bool,
    /// Maximum body size in bytes before truncation kicks in.
    pub max_body_bytes: usize,
}

impl Default for FulltextConfig {
    fn default() -> Self {
        Self {
            meili_url: "http://127.0.0.1:7700".to_string(),
            meili_api_key: None,
            synap_url: "http://127.0.0.1:17003".to_string(),
            synap_group: "cortex-fulltext".to_string(),
            index_prefix: "cortex-".to_string(),
            workers: 4,
            upsert_batch: 1_000,
            flush_ms: 1_000,
            max_retry: 3,
            await_task: false,
            max_body_bytes: 10 * 1024 * 1024,
        }
    }
}

impl FulltextConfig {
    /// Read configuration from `CORTEX_FULLTEXT_*` environment variables.
    /// Missing variables fall back to [`FulltextConfig::default`].
    pub fn from_env() -> Self {
        let def = Self::default();
        Self {
            meili_url: env::var("CORTEX_FULLTEXT_MEILI_URL").unwrap_or(def.meili_url),
            meili_api_key: env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok(),
            synap_url: env::var("CORTEX_FULLTEXT_SYNAP_URL").unwrap_or(def.synap_url),
            synap_group: env::var("CORTEX_FULLTEXT_SYNAP_GROUP").unwrap_or(def.synap_group),
            index_prefix: env::var("CORTEX_FULLTEXT_INDEX_PREFIX").unwrap_or(def.index_prefix),
            workers: parse_usize("CORTEX_FULLTEXT_WORKERS", def.workers),
            upsert_batch: parse_usize("CORTEX_FULLTEXT_BATCH", def.upsert_batch),
            flush_ms: parse_u64("CORTEX_FULLTEXT_FLUSH_MS", def.flush_ms),
            max_retry: parse_u32("CORTEX_FULLTEXT_MAX_RETRY", def.max_retry),
            await_task: parse_bool("CORTEX_FULLTEXT_AWAIT_TASK", def.await_task),
            max_body_bytes: parse_usize("CORTEX_FULLTEXT_MAX_BODY_BYTES", def.max_body_bytes),
        }
    }
}

fn parse_usize(key: &str, fallback: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(fallback)
}

fn parse_u32(key: &str, fallback: u32) -> u32 {
    env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(fallback)
}

fn parse_u64(key: &str, fallback: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn parse_bool(key: &str, fallback: bool) -> bool {
    env::var(key)
        .ok()
        .map(|raw| matches!(raw.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(fallback)
}
