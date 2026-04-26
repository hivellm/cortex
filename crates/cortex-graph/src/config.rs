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
    /// Nexus base URL. Default `http://127.0.0.1:15002`. Scheme drives the
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
            nexus_url: "http://127.0.0.1:15002".to_string(),
            transport: GraphTransport::Auto,
            synap_url: "http://127.0.0.1:15003".to_string(),
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
