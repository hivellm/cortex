//! Runtime configuration for the graph-writer worker, parsed from
//! `CORTEX_GRAPH_*` environment variables.

use std::env;

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
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            nexus_url: "http://127.0.0.1:17002".to_string(),
            transport: GraphTransport::Auto,
            synap_url: "http://127.0.0.1:17003".to_string(),
            synap_group: "cortex-graph".to_string(),
            workers: 4,
            patch_batch: 256,
            flush_ms: 500,
            max_retry: 3,
            nexus_user: None,
            nexus_password: None,
            out_of_order_buffer_secs: 30,
        }
    }
}

impl GraphConfig {
    /// Read configuration from `CORTEX_GRAPH_*` environment variables.
    /// Missing variables fall back to [`GraphConfig::default`].
    pub fn from_env() -> Self {
        let def = Self::default();
        Self {
            nexus_url: env::var("CORTEX_GRAPH_NEXUS_URL").unwrap_or(def.nexus_url),
            transport: env::var("CORTEX_GRAPH_TRANSPORT")
                .ok()
                .and_then(|raw| GraphTransport::parse(&raw))
                .unwrap_or(def.transport),
            synap_url: env::var("CORTEX_GRAPH_SYNAP_URL").unwrap_or(def.synap_url),
            synap_group: env::var("CORTEX_GRAPH_SYNAP_GROUP").unwrap_or(def.synap_group),
            workers: parse_usize("CORTEX_GRAPH_WORKERS", def.workers),
            patch_batch: parse_usize("CORTEX_GRAPH_PATCH_BATCH", def.patch_batch),
            flush_ms: parse_u64("CORTEX_GRAPH_FLUSH_MS", def.flush_ms),
            max_retry: parse_u32("CORTEX_GRAPH_MAX_RETRY", def.max_retry),
            nexus_user: env::var("CORTEX_GRAPH_NEXUS_USER").ok(),
            nexus_password: env::var("CORTEX_GRAPH_NEXUS_PASSWORD").ok(),
            out_of_order_buffer_secs: parse_u64(
                "CORTEX_GRAPH_OUT_OF_ORDER_BUFFER_SECS",
                def.out_of_order_buffer_secs,
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
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
