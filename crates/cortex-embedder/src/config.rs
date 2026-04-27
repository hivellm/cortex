//! Runtime configuration for the embedder worker, parsed from `CORTEX_EMBEDDER_*`
//! environment variables.

use std::env;

/// Embedder worker configuration.
#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Number of concurrent Synap-consumer worker tasks.
    pub workers: usize,
    /// Chunker parallelism per worker task.
    pub chunker_concurrency: usize,
    /// Maximum chunks per Vectorizer upsert call.
    pub upsert_batch: usize,
    /// Maximum retry attempts for Vectorizer requests.
    pub max_retry: u32,
    /// Vectorizer base URL (HTTP).
    pub vectorizer_url: String,
    /// Synap base URL (HTTP).
    pub synap_url: String,
    /// Vectorizer admin username.
    pub vectorizer_user: String,
    /// Vectorizer admin password (optional; may be supplied via token).
    pub vectorizer_password: Option<String>,
    /// Collection-name prefix (deployment namespace).
    pub collection_prefix: String,
    /// Vector dimension to provision new collections with. Must match
    /// what the Vectorizer server's embedding provider produces — 512
    /// for the BM25 fallback, 768 for the production FastEmbed
    /// configuration. Mismatched values lead to every insert failing
    /// `invalid_dimension` at the server.
    pub vector_dim: u32,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            workers: 6,
            chunker_concurrency: 4,
            upsert_batch: 64,
            max_retry: 3,
            vectorizer_url: "http://127.0.0.1:15001".to_string(),
            synap_url: "http://127.0.0.1:15003".to_string(),
            vectorizer_user: "admin".to_string(),
            vectorizer_password: None,
            collection_prefix: "cortex".to_string(),
            vector_dim: 768,
        }
    }
}

impl EmbedderConfig {
    /// Read the configuration from `CORTEX_EMBEDDER_*` environment variables.
    ///
    /// Missing variables fall back to [`EmbedderConfig::default`].
    pub fn from_env() -> Self {
        let def = Self::default();
        Self {
            workers: parse_usize("CORTEX_EMBEDDER_WORKERS", def.workers),
            chunker_concurrency: parse_usize(
                "CORTEX_EMBEDDER_CHUNKER_CONCURRENCY",
                def.chunker_concurrency,
            ),
            upsert_batch: parse_usize("CORTEX_EMBEDDER_UPSERT_BATCH", def.upsert_batch),
            max_retry: parse_u32("CORTEX_EMBEDDER_MAX_RETRY", def.max_retry),
            vectorizer_url: env::var("CORTEX_EMBEDDER_VECTORIZER_URL")
                .unwrap_or(def.vectorizer_url),
            synap_url: env::var("CORTEX_EMBEDDER_SYNAP_URL").unwrap_or(def.synap_url),
            vectorizer_user: env::var("CORTEX_EMBEDDER_VECTORIZER_USER")
                .unwrap_or(def.vectorizer_user),
            vectorizer_password: env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD").ok(),
            collection_prefix: env::var("CORTEX_EMBEDDER_COLLECTION_PREFIX")
                .unwrap_or(def.collection_prefix),
            vector_dim: parse_u32("CORTEX_EMBEDDER_DIM", def.vector_dim),
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
