//! Runtime configuration for the embedder worker.
//!
//! ADR-016 §3.4a — `EmbedderConfig::from_env()` delegates to
//! `cortex_config::Config::load()`. The struct shape stays
//! unchanged so existing call sites (`cfg.workers`,
//! `cfg.vectorizer_url`, …) compile without edits, but the
//! resolution path now flows through the typed Config crate:
//! CLI > env > `cortex.toml` > default. Every `CORTEX_EMBEDDER_*`
//! env var keeps working byte-for-byte via the env-overlay
//! pointer table in `cortex_config::env_map::KNOWN_ENV_NAMES`.

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
        // Mirror cortex_config::EmbedderConfig::default() field-for-
        // field so the two paths converge on the same defaults.
        let d = cortex_config::EmbedderConfig::default();
        Self::from_typed(d)
    }
}

impl EmbedderConfig {
    /// Read the configuration via the typed Config crate
    /// ([`cortex_config::Config::load`]). Resolves CLI > env >
    /// `cortex.toml` > default. A load failure falls back to
    /// `Default::default()` so the legacy "missing env → defaults"
    /// behaviour stays.
    pub fn from_env() -> Self {
        match cortex_config::Config::load() {
            Ok(cfg) => Self::from_typed(cfg.embedder),
            Err(_) => Self::default(),
        }
    }

    fn from_typed(t: cortex_config::EmbedderConfig) -> Self {
        Self {
            workers: t.workers,
            chunker_concurrency: t.chunker_concurrency,
            upsert_batch: t.upsert_batch,
            max_retry: t.max_retry,
            vectorizer_url: t.vectorizer_url,
            synap_url: t.synap_url,
            vectorizer_user: t.vectorizer_user,
            vectorizer_password: t.vectorizer_password,
            collection_prefix: t.collection_prefix,
            vector_dim: t.vector_dim,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    // Env-mutating tests must run sequentially. The std env table is
    // process-global; multiple tests racing on `set_var` corrupt each
    // other's reads. cortex_config::Config::load() also reads the env,
    // so this lock is shared semantics — own only inside this module's
    // tests; cortex_config has its own hermetic loader (load_from)
    // that bypasses process env entirely.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const ALL_KEYS: &[&str] = &[
        "CORTEX_EMBEDDER_WORKERS",
        "CORTEX_EMBEDDER_CHUNKER_CONCURRENCY",
        "CORTEX_EMBEDDER_UPSERT_BATCH",
        "CORTEX_EMBEDDER_MAX_RETRY",
        "CORTEX_EMBEDDER_VECTORIZER_URL",
        "CORTEX_EMBEDDER_SYNAP_URL",
        "CORTEX_EMBEDDER_VECTORIZER_USER",
        "CORTEX_EMBEDDER_VECTORIZER_PASSWORD",
        "CORTEX_EMBEDDER_COLLECTION_PREFIX",
        "CORTEX_EMBEDDER_DIM",
    ];

    fn clear_all() {
        for k in ALL_KEYS {
            env::remove_var(k);
        }
    }

    #[test]
    fn default_values_match_spec() {
        let d = EmbedderConfig::default();
        assert_eq!(d.workers, 6);
        assert_eq!(d.chunker_concurrency, 4);
        assert_eq!(d.upsert_batch, 64);
        assert_eq!(d.max_retry, 3);
        assert_eq!(d.vectorizer_url, "http://127.0.0.1:17001");
        assert_eq!(d.synap_url, "http://127.0.0.1:17003");
        assert_eq!(d.vectorizer_user, "admin");
        assert_eq!(d.vectorizer_password, None);
        assert_eq!(d.collection_prefix, "cortex");
        assert_eq!(d.vector_dim, 768);
    }

    #[test]
    fn from_env_with_no_vars_returns_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        let cfg = EmbedderConfig::from_env();
        let def = EmbedderConfig::default();
        assert_eq!(cfg.workers, def.workers);
        assert_eq!(cfg.vectorizer_url, def.vectorizer_url);
        assert_eq!(cfg.vector_dim, def.vector_dim);
    }

    #[test]
    fn from_env_overrides_each_field() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("CORTEX_EMBEDDER_WORKERS", "9");
        env::set_var("CORTEX_EMBEDDER_CHUNKER_CONCURRENCY", "2");
        env::set_var("CORTEX_EMBEDDER_UPSERT_BATCH", "32");
        env::set_var("CORTEX_EMBEDDER_MAX_RETRY", "7");
        env::set_var("CORTEX_EMBEDDER_VECTORIZER_URL", "http://vec:1234");
        env::set_var("CORTEX_EMBEDDER_SYNAP_URL", "http://syn:5678");
        env::set_var("CORTEX_EMBEDDER_VECTORIZER_USER", "ops");
        env::set_var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD", "p4ss");
        env::set_var("CORTEX_EMBEDDER_COLLECTION_PREFIX", "stage");
        env::set_var("CORTEX_EMBEDDER_DIM", "384");

        let cfg = EmbedderConfig::from_env();
        assert_eq!(cfg.workers, 9);
        assert_eq!(cfg.chunker_concurrency, 2);
        assert_eq!(cfg.upsert_batch, 32);
        assert_eq!(cfg.max_retry, 7);
        assert_eq!(cfg.vectorizer_url, "http://vec:1234");
        assert_eq!(cfg.synap_url, "http://syn:5678");
        assert_eq!(cfg.vectorizer_user, "ops");
        assert_eq!(cfg.vectorizer_password.as_deref(), Some("p4ss"));
        assert_eq!(cfg.collection_prefix, "stage");
        assert_eq!(cfg.vector_dim, 384);

        clear_all();
    }

    #[test]
    fn from_env_falls_back_on_unparseable_numbers() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("CORTEX_EMBEDDER_WORKERS", "abc");
        env::set_var("CORTEX_EMBEDDER_DIM", "not-an-int");

        // cortex_config's env-overlay coerces non-numeric strings to
        // `Value::String`, which then fails serde deserialisation
        // into `usize` / `u32`. The Config::load() call returns Err,
        // and from_env() falls back to Default. That preserves the
        // legacy "unparseable → default" behaviour byte-for-byte.
        let cfg = EmbedderConfig::from_env();
        let def = EmbedderConfig::default();
        assert_eq!(cfg.workers, def.workers);
        assert_eq!(cfg.vector_dim, def.vector_dim);

        clear_all();
    }
}
