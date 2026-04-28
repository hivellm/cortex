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
    /// `CORTEX_ARCHIVE_ROOT`, `SYNAP_URL`, `CORTEX_ARCHIVE_ZSTD`).
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
        if let Ok(url) = std::env::var("SYNAP_URL") {
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
