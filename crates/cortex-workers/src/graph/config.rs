//! Runtime configuration for the graph-writer worker.
//!
//! ADR-016 §3.4c — `GraphConfig::from_env()` delegates to
//! `cortex_config::Config::load()`. Struct shape preserved.

/// Transport selector for the Nexus client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphTransport {
    /// Let the SDK pick based on the URL scheme (`nexus://` → RPC,
    /// `http(s)://` → HTTP).
    Auto,
    /// Force RPC (`nexus://`) — equivalent of "Bolt" in spec 07.
    Rpc,
    /// Force HTTP — fallback selected by `CORTEX_GRAPH_TRANSPORT=http`.
    Http,
}

impl GraphTransport {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "rpc" | "bolt" | "nexus" => Some(Self::Rpc),
            "http" | "https" => Some(Self::Http),
            _ => None,
        }
    }
}

/// Graph-writer worker configuration.
#[derive(Debug, Clone)]
pub struct GraphConfig {
    /// Nexus base URL. Default `http://127.0.0.1:17002`. Scheme drives the
    /// transport choice when [`GraphConfig::transport`] is `Auto`.
    pub nexus_url: String,
    /// Forced transport selector.
    pub transport: GraphTransport,
    /// Synap base URL.
    pub synap_url: String,
    /// Synap consumer-group label (carried through metadata; Synap 0.11
    /// does not yet expose durable groups).
    pub synap_group: String,
    /// Number of concurrent worker tasks.
    pub workers: usize,
    /// Maximum graph-patch entries (nodes + edges combined) per Cypher tx.
    pub patch_batch: usize,
    /// Flush interval in milliseconds — coalesced micro-batches are
    /// written at most every `flush_ms`.
    pub flush_ms: u64,
    /// Maximum retry attempts for transient Nexus errors.
    pub max_retry: u32,
    /// Optional Nexus username (prod auth; dev mode ignores it).
    pub nexus_user: Option<String>,
    /// Optional Nexus password / token (prod auth).
    pub nexus_password: Option<String>,
    /// Out-of-order tolerance: how long to buffer a `tool_call` waiting
    /// for its `turn.start`, in seconds, before fabricating an orphan
    /// Turn.
    pub out_of_order_buffer_secs: u64,
    /// phase15c — when `false`, the worker skips the semantic-edge
    /// projection pass (batch step 3b) and runs structural-mapping
    /// only. Kill-switch for the projection's write amplification that
    /// trips nexus#12. Default `true`.
    pub projection_enabled: bool,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self::from_typed(cortex_config::NexusConfig::default())
    }
}

impl GraphConfig {
    /// Read configuration via `cortex_config::Config::load()`.
    pub fn from_env() -> Self {
        match cortex_config::Config::load() {
            Ok(cfg) => Self::from_typed(cfg.nexus),
            Err(_) => Self::default(),
        }
    }

    fn from_typed(t: cortex_config::NexusConfig) -> Self {
        Self {
            nexus_url: t
                .nexus_url
                .unwrap_or_else(|| "http://127.0.0.1:17002".to_string()),
            transport: GraphTransport::parse(&t.transport).unwrap_or(GraphTransport::Auto),
            synap_url: t.synap_url,
            synap_group: t.synap_group,
            workers: t.workers,
            patch_batch: t.patch_batch,
            flush_ms: t.flush_ms,
            max_retry: t.max_retry,
            nexus_user: t.nexus_user,
            nexus_password: t.nexus_password,
            out_of_order_buffer_secs: t.out_of_order_buffer_secs,
            projection_enabled: t.projection_enabled,
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
        "CORTEX_GRAPH_NEXUS_URL",
        "CORTEX_GRAPH_TRANSPORT",
        "CORTEX_GRAPH_SYNAP_URL",
        "CORTEX_GRAPH_SYNAP_GROUP",
        "CORTEX_GRAPH_WORKERS",
        "CORTEX_GRAPH_PATCH_BATCH",
        "CORTEX_GRAPH_FLUSH_MS",
        "CORTEX_GRAPH_MAX_RETRY",
        "CORTEX_GRAPH_NEXUS_USER",
        "CORTEX_GRAPH_NEXUS_PASSWORD",
        "CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS",
    ];
    fn clear_all() {
        for k in ALL_KEYS {
            env::remove_var(k);
        }
    }

    #[test]
    fn defaults_match_spec() {
        let d = GraphConfig::default();
        assert_eq!(d.nexus_url, "http://127.0.0.1:17002");
        assert_eq!(d.transport, GraphTransport::Auto);
        assert_eq!(d.synap_group, "cortex-graph");
        assert_eq!(d.workers, 4);
        assert_eq!(d.patch_batch, 256);
        assert_eq!(d.flush_ms, 500);
        assert_eq!(d.max_retry, 3);
        assert!(d.nexus_user.is_none());
        assert!(d.nexus_password.is_none());
        assert_eq!(d.out_of_order_buffer_secs, 30);
    }

    #[test]
    fn transport_parses_each_alias() {
        assert_eq!(GraphTransport::parse("auto"), Some(GraphTransport::Auto));
        assert_eq!(GraphTransport::parse("AUTO"), Some(GraphTransport::Auto));
        assert_eq!(GraphTransport::parse("rpc"), Some(GraphTransport::Rpc));
        assert_eq!(GraphTransport::parse("bolt"), Some(GraphTransport::Rpc));
        assert_eq!(GraphTransport::parse("nexus"), Some(GraphTransport::Rpc));
        assert_eq!(GraphTransport::parse("http"), Some(GraphTransport::Http));
        assert_eq!(GraphTransport::parse("HTTPS"), Some(GraphTransport::Http));
        assert_eq!(GraphTransport::parse("garbage"), None);
        assert_eq!(GraphTransport::parse(""), None);
    }

    #[test]
    fn from_env_returns_defaults_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        let cfg = GraphConfig::from_env();
        let def = GraphConfig::default();
        assert_eq!(cfg.nexus_url, def.nexus_url);
        assert_eq!(cfg.transport, def.transport);
    }

    #[test]
    fn from_env_overrides_each_field() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("CORTEX_GRAPH_NEXUS_URL", "http://nx:1");
        env::set_var("CORTEX_GRAPH_TRANSPORT", "http");
        env::set_var("CORTEX_GRAPH_SYNAP_URL", "http://sy:2");
        env::set_var("CORTEX_GRAPH_SYNAP_GROUP", "g7");
        env::set_var("CORTEX_GRAPH_WORKERS", "11");
        env::set_var("CORTEX_GRAPH_PATCH_BATCH", "128");
        env::set_var("CORTEX_GRAPH_FLUSH_MS", "1500");
        env::set_var("CORTEX_GRAPH_MAX_RETRY", "9");
        env::set_var("CORTEX_GRAPH_NEXUS_USER", "u");
        env::set_var("CORTEX_GRAPH_NEXUS_PASSWORD", "p");
        env::set_var("CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS", "10");

        let cfg = GraphConfig::from_env();
        assert_eq!(cfg.nexus_url, "http://nx:1");
        assert_eq!(cfg.transport, GraphTransport::Http);
        assert_eq!(cfg.synap_group, "g7");
        assert_eq!(cfg.workers, 11);
        assert_eq!(cfg.patch_batch, 128);
        assert_eq!(cfg.flush_ms, 1500);
        assert_eq!(cfg.max_retry, 9);
        assert_eq!(cfg.nexus_user.as_deref(), Some("u"));
        assert_eq!(cfg.nexus_password.as_deref(), Some("p"));
        assert_eq!(cfg.out_of_order_buffer_secs, 10);

        clear_all();
    }

    #[test]
    fn from_env_unknown_transport_falls_back_to_default() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("CORTEX_GRAPH_TRANSPORT", "carrier-pigeon");
        let cfg = GraphConfig::from_env();
        assert_eq!(cfg.transport, GraphTransport::Auto);
        clear_all();
    }
}
