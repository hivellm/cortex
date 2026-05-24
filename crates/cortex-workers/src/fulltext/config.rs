//! Runtime configuration for the full-text indexer worker.
//!
//! ADR-016 §3.4b — `FulltextConfig::from_env()` delegates to
//! `cortex_config::Config::load()`. Struct shape preserved so
//! existing call sites compile unchanged.

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
        Self::from_typed(cortex_config::MeiliConfig::default())
    }
}

impl FulltextConfig {
    /// Read configuration via `cortex_config::Config::load()`.
    /// Resolves CLI > env > `cortex.toml` > default. Load failure
    /// falls back to `Default::default()` (legacy "missing env →
    /// defaults" behaviour).
    pub fn from_env() -> Self {
        match cortex_config::Config::load() {
            Ok(cfg) => Self::from_typed(cfg.meili),
            Err(_) => Self::default(),
        }
    }

    fn from_typed(t: cortex_config::MeiliConfig) -> Self {
        Self {
            meili_url: t
                .meili_url
                .unwrap_or_else(|| "http://127.0.0.1:7700".to_string()),
            meili_api_key: t.meili_api_key,
            synap_url: t.synap_url,
            synap_group: t.synap_group,
            index_prefix: t.index_prefix,
            workers: t.workers,
            upsert_batch: t.upsert_batch,
            flush_ms: t.flush_ms,
            max_retry: t.max_retry,
            await_task: t.await_task,
            max_body_bytes: t.max_body_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const ALL_KEYS: &[&str] = &[
        "CORTEX_FULLTEXT_MEILI_URL",
        "CORTEX_FULLTEXT_MEILI_API_KEY",
        "CORTEX_FULLTEXT_SYNAP_URL",
        "CORTEX_FULLTEXT_SYNAP_GROUP",
        "CORTEX_FULLTEXT_INDEX_PREFIX",
        "CORTEX_FULLTEXT_WORKERS",
        "CORTEX_FULLTEXT_BATCH",
        "CORTEX_FULLTEXT_FLUSH_MS",
        "CORTEX_FULLTEXT_MAX_RETRY",
        "CORTEX_FULLTEXT_AWAIT_TASK",
        "CORTEX_FULLTEXT_MAX_BODY_BYTES",
    ];

    fn clear_all() {
        for k in ALL_KEYS {
            env::remove_var(k);
        }
    }

    #[test]
    fn default_values_match_spec() {
        let d = FulltextConfig::default();
        assert_eq!(d.meili_url, "http://127.0.0.1:7700");
        assert!(d.meili_api_key.is_none());
        assert_eq!(d.synap_group, "cortex-fulltext");
        assert_eq!(d.index_prefix, "cortex-");
        assert_eq!(d.workers, 4);
        assert_eq!(d.upsert_batch, 1_000);
        assert_eq!(d.flush_ms, 1_000);
        assert_eq!(d.max_retry, 3);
        assert!(!d.await_task);
        assert_eq!(d.max_body_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn from_env_with_no_vars_returns_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        let cfg = FulltextConfig::from_env();
        assert_eq!(cfg.meili_url, "http://127.0.0.1:7700");
        assert_eq!(cfg.workers, 4);
    }

    #[test]
    fn from_env_overrides_each_field() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("CORTEX_FULLTEXT_MEILI_URL", "http://m:1");
        env::set_var("CORTEX_FULLTEXT_MEILI_API_KEY", "key1");
        env::set_var("CORTEX_FULLTEXT_SYNAP_URL", "http://s:2");
        env::set_var("CORTEX_FULLTEXT_SYNAP_GROUP", "g1");
        env::set_var("CORTEX_FULLTEXT_INDEX_PREFIX", "stage-");
        env::set_var("CORTEX_FULLTEXT_WORKERS", "12");
        env::set_var("CORTEX_FULLTEXT_BATCH", "500");
        env::set_var("CORTEX_FULLTEXT_FLUSH_MS", "250");
        env::set_var("CORTEX_FULLTEXT_MAX_RETRY", "5");
        env::set_var("CORTEX_FULLTEXT_AWAIT_TASK", "true");
        env::set_var("CORTEX_FULLTEXT_MAX_BODY_BYTES", "4096");

        let cfg = FulltextConfig::from_env();
        assert_eq!(cfg.meili_url, "http://m:1");
        assert_eq!(cfg.meili_api_key.as_deref(), Some("key1"));
        assert_eq!(cfg.synap_url, "http://s:2");
        assert_eq!(cfg.synap_group, "g1");
        assert_eq!(cfg.index_prefix, "stage-");
        assert_eq!(cfg.workers, 12);
        assert_eq!(cfg.upsert_batch, 500);
        assert_eq!(cfg.flush_ms, 250);
        assert_eq!(cfg.max_retry, 5);
        assert!(cfg.await_task);
        assert_eq!(cfg.max_body_bytes, 4096);

        clear_all();
    }
}
