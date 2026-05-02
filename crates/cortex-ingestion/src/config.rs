//! Ingestion-service configuration.

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
    /// Read the usual env vars (`CORTEX_BIND`, `CORTEX_API_PORT`,
    /// `CORTEX_ARCHIVE_ROOT`, `CORTEX_SYNAP_URL` / `SYNAP_URL`,
    /// `CORTEX_ARCHIVE_ZSTD`). `CORTEX_SYNAP_URL` is the
    /// project-namespaced canonical form and wins when both are set;
    /// `SYNAP_URL` stays accepted for backwards compatibility with
    /// shell scripts and CI workflows that pre-date the rename.
    pub fn from_env() -> Self {
        let mut cfg = IngestionConfig {
            source: Source::Environment,
            ..Self::default()
        };
        if let Ok(addr) = std::env::var("CORTEX_INGESTION_BIND") {
            if let Ok(sa) = addr.parse::<SocketAddr>() {
                cfg.bind = sa;
            }
        }
        if let (Ok(host), Ok(port)) = (std::env::var("CORTEX_BIND"), std::env::var("CORTEX_API_PORT")) {
            if let Ok(sa) = format!("{host}:{port}").parse::<SocketAddr>() {
                cfg.bind = sa;
            }
        }
        if let Ok(path) = std::env::var("CORTEX_ARCHIVE_ROOT") {
            cfg.archive_root = PathBuf::from(path);
        }
        // CORTEX_SYNAP_URL is the canonical project-namespaced form;
        // SYNAP_URL is the legacy name shell scripts + the docker-
        // compose env block historically used. Read both, prefer
        // CORTEX_SYNAP_URL when both are set so a future cleanup of
        // the duplicate compose env var is structurally safe.
        if let Ok(url) = std::env::var("CORTEX_SYNAP_URL") {
            cfg.synap_url = Some(url);
        } else if let Ok(url) = std::env::var("SYNAP_URL") {
            cfg.synap_url = Some(url);
        }
        if let Ok(level) = std::env::var("CORTEX_ARCHIVE_ZSTD") {
            if let Ok(l) = level.parse::<i32>() {
                cfg.archive_zstd_level = l;
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
