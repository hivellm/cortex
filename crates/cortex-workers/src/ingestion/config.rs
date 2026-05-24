//! Ingestion-service configuration.
//!
//! ADR-016 §3.4d — `IngestionConfig::from_env()` delegates the
//! `CORTEX_*` knobs to `cortex_config::Config::load()`. Two
//! non-CORTEX legacy reads stay direct because they fall
//! outside the typed-Config surface (`SYNAP_URL` is the legacy
//! non-namespaced fallback for `CORTEX_SYNAP_URL`; the audit
//! regex ignores it because it lacks the `CORTEX_` prefix).

use std::net::SocketAddr;
use std::path::PathBuf;

/// How the service was configured (defaults, env, explicit override).
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// Default values, nothing overridden.
    Defaults,
    /// Loaded from environment variables.
    Environment,
    /// Explicit values supplied by tests.
    Explicit,
}

/// Ingestion service configuration.
#[derive(Debug, Clone)]
pub struct IngestionConfig {
    /// Socket the HTTP router binds to.
    pub bind: SocketAddr,
    /// Directory that holds the durable archive.
    pub archive_root: PathBuf,
    /// Synap base URL. `None` disables publishing (tests / dry-run).
    pub synap_url: Option<String>,
    /// Zstd compression level for archive files (0–22).
    pub archive_zstd_level: i32,
    /// Source of the configuration, used for startup logging.
    pub source: Source,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:17000".parse().expect("hardcoded addr parses"),
            archive_root: PathBuf::from("./data/archive"),
            synap_url: None,
            archive_zstd_level: cortex_storage::ArchiveLayout::COMPRESSION_LEVEL,
            source: Source::Defaults,
        }
    }
}

impl IngestionConfig {
    /// Read the configuration via `cortex_config::Config::load()`.
    /// Legacy `SYNAP_URL` (non-namespaced) is preserved as a
    /// direct fallback when `CORTEX_SYNAP_URL` is unset — it lives
    /// outside the typed Config surface because its name predates
    /// the `CORTEX_*` namespace and the audit regex (correctly)
    /// only flags `CORTEX_*` reads.
    pub fn from_env() -> Self {
        let typed = cortex_config::Config::load().ok().map(|c| c.ingestion);
        let def_self = Self::default();
        let mut cfg = IngestionConfig {
            source: Source::Environment,
            ..Self::default()
        };

        if let Some(t) = typed {
            if let Ok(sa) = t.bind.parse::<SocketAddr>() {
                cfg.bind = sa;
            }
            if let Some(p) = t.archive_root {
                cfg.archive_root = PathBuf::from(p);
            } else {
                cfg.archive_root = def_self.archive_root.clone();
            }
            cfg.synap_url = t.synap_url;
            cfg.archive_zstd_level = t.archive_zstd_level;
        }

        // SYNAP_URL legacy fallback — non-CORTEX_ prefix so the
        // audit regex does NOT flag it. Wins only when
        // CORTEX_SYNAP_URL was unset (typed load left `synap_url`
        // as None).
        if cfg.synap_url.is_none() {
            if let Ok(url) = std::env::var("SYNAP_URL") {
                if !url.is_empty() {
                    cfg.synap_url = Some(url);
                }
            }
        }

        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise the env-mutating tests through one mutex so parallel
    /// invocations don't see each other's writes (the rust test harness
    /// runs `#[test]` fns concurrently by default, and `std::env::set_var`
    /// is process-wide).
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn synap_url_prefers_cortex_namespaced_var() {
        let _g = env_lock();
        std::env::set_var("CORTEX_SYNAP_URL", "http://canonical:1");
        std::env::set_var("SYNAP_URL", "http://legacy:2");
        let cfg = IngestionConfig::from_env();
        assert_eq!(cfg.synap_url.as_deref(), Some("http://canonical:1"));
        std::env::remove_var("CORTEX_SYNAP_URL");
        std::env::remove_var("SYNAP_URL");
    }

    #[test]
    fn synap_url_falls_back_to_legacy_name() {
        let _g = env_lock();
        std::env::remove_var("CORTEX_SYNAP_URL");
        std::env::set_var("SYNAP_URL", "http://legacy:2");
        let cfg = IngestionConfig::from_env();
        assert_eq!(cfg.synap_url.as_deref(), Some("http://legacy:2"));
        std::env::remove_var("SYNAP_URL");
    }

    #[test]
    fn synap_url_unset_yields_none() {
        let _g = env_lock();
        std::env::remove_var("CORTEX_SYNAP_URL");
        std::env::remove_var("SYNAP_URL");
        let cfg = IngestionConfig::from_env();
        assert!(cfg.synap_url.is_none());
    }
}
