//! Phase14g §3 — query rewriter cascade.
//!
//! Deterministic noun-phrase stripping in the legacy rewriter
//! loses intent signal: `"Refactor the HNSW configurator"` collapses
//! to `"HNSW configurator"`, dropping the `refactor` action verb the
//! intent selector + retrieval relevance ladder both rely on. Sonnet
//! rewrite was opt-in via `CORTEX_QUERY_REWRITER=sonnet` but never
//! adopted because a Sonnet timeout returned an error instead of
//! falling back to the deterministic path.
//!
//! The cascade fixes both. Per call:
//!
//! 1. SHA256-key the `(query, intent)` pair against a per-process
//!    response cache (TTL 24h, capped at 10 000 entries).
//! 2. **Cache hit** → return the cached rewrite tagged
//!    `sonnet_cache_hit`.
//! 3. **Cache miss** → invoke the supplied [`SonnetRewriter`] under
//!    [`SONNET_TIMEOUT`] (default 800 ms). Success tags
//!    `sonnet_hit` and caches; timeout tags `sonnet_timeout` and
//!    falls through; any other error tags `sonnet_error` and falls
//!    through.
//! 4. Fallback path runs `deterministic_rewrite(query)` and tags
//!    `deterministic_fallback`. That function is intentionally a
//!    near-identity pass-through (lowercases + collapses runs of
//!    whitespace) so callers without a Sonnet backend still get
//!    stable behaviour; the legacy noun-phrase strip lives in the
//!    adapter layer and is not in scope for this cascade.
//!
//! Telemetry: every dispatch bumps
//! `Metrics::rewriter_path_total{path}` so the doctor + dashboard
//! can read the per-path counts without scraping logs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::metrics::Metrics;

/// Default Sonnet upstream timeout. Operator-tunable via the
/// `CascadeConfig` builder.
pub const SONNET_TIMEOUT: Duration = Duration::from_millis(800);

/// Cache TTL — how long a rewrite stays warm before the cascade
/// re-asks Sonnet.
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

/// Hard cap on cache size. Beyond this the oldest entry is
/// evicted on insert (least-recently-inserted, since we only
/// stamp the insertion time).
pub const CACHE_MAX_ENTRIES: usize = 10_000;

/// Stable lower-case path labels reported in
/// `cortex_pre_thinking_rewriter_path_total{path}` + carried on
/// the [`RewrittenQuery::path`] field.
pub mod path_labels {
    /// Sonnet returned a fresh rewrite.
    pub const SONNET_HIT: &str = "sonnet_hit";
    /// Cache served a prior Sonnet rewrite.
    pub const SONNET_CACHE_HIT: &str = "sonnet_cache_hit";
    /// Sonnet exceeded `SONNET_TIMEOUT`; cascade fell through.
    pub const SONNET_TIMEOUT: &str = "sonnet_timeout";
    /// Sonnet returned a non-timeout error; cascade fell through.
    pub const SONNET_ERROR: &str = "sonnet_error";
    /// Deterministic path served the rewrite (either by direct
    /// caller request or as a Sonnet fallback).
    pub const DETERMINISTIC_FALLBACK: &str = "deterministic_fallback";
}

/// Outcome of [`cascade`]. Carries the rewrite, the path label,
/// and a `cache_hit` shortcut so callers don't have to compare
/// strings to detect cache hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenQuery {
    /// Rewritten query string.
    pub query: String,
    /// Stable path label (one of `path_labels::*`).
    pub path: &'static str,
    /// `true` when the cascade returned without touching Sonnet
    /// (cache hit OR deterministic-only call).
    pub cache_hit: bool,
}

/// Sonnet backend the cascade calls. Production wires a thin
/// HTTP client; tests inject a fixture that returns canned
/// responses or simulates timeouts. The trait is intentionally
/// minimal so callers can compose without pulling reqwest into
/// the pre-thinking crate.
#[async_trait]
pub trait SonnetRewriter: Send + Sync {
    /// Issue a single rewrite request. The implementation MUST
    /// honour `total_budget` and return `Err(SonnetError::Timeout)`
    /// rather than returning late.
    async fn rewrite(
        &self,
        query: &str,
        intent: &str,
        total_budget: Duration,
    ) -> Result<String, SonnetError>;
}

/// Sonnet-side error surface. Each variant maps to a distinct
/// path label so the cascade telemetry stays meaningful.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SonnetError {
    /// Upstream exceeded the supplied `total_budget`.
    #[error("sonnet rewrite timed out")]
    Timeout,
    /// Anything else (HTTP 5xx, network, schema mismatch, etc.).
    #[error("sonnet rewrite failed: {0}")]
    Other(String),
}

/// Operator-tunable cascade thresholds.
#[derive(Debug, Clone, Copy)]
pub struct CascadeConfig {
    /// Per-call Sonnet timeout.
    pub sonnet_timeout: Duration,
    /// Cache TTL.
    pub cache_ttl: Duration,
    /// Hard cap on cache size.
    pub cache_max_entries: usize,
}

impl Default for CascadeConfig {
    fn default() -> Self {
        Self {
            sonnet_timeout: SONNET_TIMEOUT,
            cache_ttl: CACHE_TTL,
            cache_max_entries: CACHE_MAX_ENTRIES,
        }
    }
}

/// Per-process rewrite cache. Cheap `Arc<Mutex<...>>` so the
/// cascade + the doctor probe can share one instance.
#[derive(Debug, Default)]
pub struct RewriteCache {
    inner: Mutex<RewriteCacheInner>,
}

#[derive(Debug, Default)]
struct RewriteCacheInner {
    entries: HashMap<String, CacheEntry>,
    insertion_order: std::collections::VecDeque<String>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    rewrite: String,
    inserted_at: Instant,
}

impl RewriteCache {
    /// Fresh empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Hash helper — exposed so tests can pre-seed the cache with
    /// the same key the cascade uses at runtime.
    pub fn key(query: &str, intent: &str) -> String {
        let mut h = Sha256::new();
        h.update(query.as_bytes());
        h.update(b"\x00");
        h.update(intent.as_bytes());
        let raw = h.finalize();
        raw.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Read `(query, intent)` from the cache. Returns `None` when
    /// the entry is missing OR has expired past `ttl`.
    pub async fn get(&self, query: &str, intent: &str, ttl: Duration) -> Option<String> {
        let key = Self::key(query, intent);
        let inner = self.inner.lock().await;
        let entry = inner.entries.get(&key)?;
        if entry.inserted_at.elapsed() > ttl {
            return None;
        }
        Some(entry.rewrite.clone())
    }

    /// Insert + enforce the size cap by evicting the oldest entry.
    pub async fn insert(&self, query: &str, intent: &str, rewrite: String, max_entries: usize) {
        let key = Self::key(query, intent);
        let mut inner = self.inner.lock().await;
        if inner.entries.insert(
            key.clone(),
            CacheEntry {
                rewrite,
                inserted_at: Instant::now(),
            },
        ).is_none() {
            inner.insertion_order.push_back(key);
        }
        while inner.entries.len() > max_entries {
            if let Some(oldest) = inner.insertion_order.pop_front() {
                inner.entries.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

/// Phase14g §3.1 — main cascade entry point.
pub async fn cascade(
    query: &str,
    intent: &str,
    sonnet: Option<Arc<dyn SonnetRewriter>>,
    cache: Arc<RewriteCache>,
    metrics: Arc<Metrics>,
    config: CascadeConfig,
) -> RewrittenQuery {
    // No Sonnet backend wired → deterministic-only path.
    let Some(sonnet) = sonnet else {
        metrics.incr_rewriter_path(path_labels::DETERMINISTIC_FALLBACK);
        return RewrittenQuery {
            query: deterministic_rewrite(query),
            path: path_labels::DETERMINISTIC_FALLBACK,
            cache_hit: true,
        };
    };

    if let Some(hit) = cache.get(query, intent, config.cache_ttl).await {
        metrics.incr_rewriter_path(path_labels::SONNET_CACHE_HIT);
        return RewrittenQuery {
            query: hit,
            path: path_labels::SONNET_CACHE_HIT,
            cache_hit: true,
        };
    }

    match sonnet.rewrite(query, intent, config.sonnet_timeout).await {
        Ok(rewrite) => {
            cache
                .insert(query, intent, rewrite.clone(), config.cache_max_entries)
                .await;
            metrics.incr_rewriter_path(path_labels::SONNET_HIT);
            RewrittenQuery {
                query: rewrite,
                path: path_labels::SONNET_HIT,
                cache_hit: false,
            }
        }
        Err(SonnetError::Timeout) => {
            metrics.incr_rewriter_path(path_labels::SONNET_TIMEOUT);
            tracing::warn!(intent, "rewriter cascade: sonnet timeout, falling through");
            metrics.incr_rewriter_path(path_labels::DETERMINISTIC_FALLBACK);
            RewrittenQuery {
                query: deterministic_rewrite(query),
                path: path_labels::DETERMINISTIC_FALLBACK,
                cache_hit: false,
            }
        }
        Err(SonnetError::Other(err)) => {
            metrics.incr_rewriter_path(path_labels::SONNET_ERROR);
            tracing::warn!(intent, %err, "rewriter cascade: sonnet error, falling through");
            metrics.incr_rewriter_path(path_labels::DETERMINISTIC_FALLBACK);
            RewrittenQuery {
                query: deterministic_rewrite(query),
                path: path_labels::DETERMINISTIC_FALLBACK,
                cache_hit: false,
            }
        }
    }
}

/// Phase14g §3.1 — deterministic fallback. Intentionally a thin
/// pass-through (lowercase + whitespace normalisation) so the
/// cascade's behaviour without Sonnet is predictable. The legacy
/// noun-phrase strip remains in the adapter layer for callers
/// that opt in explicitly.
pub fn deterministic_rewrite(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut last_was_space = false;
    for c in query.trim().chars() {
        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    out.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct OkSonnet {
        calls: AtomicU32,
    }
    impl OkSonnet {
        fn new() -> Self {
            Self {
                calls: AtomicU32::new(0),
            }
        }
    }
    #[async_trait]
    impl SonnetRewriter for OkSonnet {
        async fn rewrite(
            &self,
            query: &str,
            intent: &str,
            _budget: Duration,
        ) -> Result<String, SonnetError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("sonnet({intent}): {query}"))
        }
    }

    struct TimeoutSonnet;
    #[async_trait]
    impl SonnetRewriter for TimeoutSonnet {
        async fn rewrite(
            &self,
            _query: &str,
            _intent: &str,
            _budget: Duration,
        ) -> Result<String, SonnetError> {
            Err(SonnetError::Timeout)
        }
    }

    struct ErrorSonnet;
    #[async_trait]
    impl SonnetRewriter for ErrorSonnet {
        async fn rewrite(
            &self,
            _query: &str,
            _intent: &str,
            _budget: Duration,
        ) -> Result<String, SonnetError> {
            Err(SonnetError::Other("502 upstream".into()))
        }
    }

    fn cfg() -> CascadeConfig {
        CascadeConfig {
            sonnet_timeout: Duration::from_millis(800),
            cache_ttl: Duration::from_secs(3600),
            cache_max_entries: 10_000,
        }
    }

    #[tokio::test]
    async fn sonnet_hit_caches_and_reports_path() {
        let sonnet = Arc::new(OkSonnet::new());
        let cache = Arc::new(RewriteCache::new());
        let metrics = Arc::new(Metrics::new());
        let r = cascade(
            "Refactor the HNSW configurator",
            "pre_change_context",
            Some(sonnet.clone()),
            cache.clone(),
            metrics.clone(),
            cfg(),
        )
        .await;
        assert_eq!(r.path, path_labels::SONNET_HIT);
        assert!(!r.cache_hit);
        assert!(r.query.starts_with("sonnet(pre_change_context):"));
        // Second call: cache short-circuits.
        let r2 = cascade(
            "Refactor the HNSW configurator",
            "pre_change_context",
            Some(sonnet.clone()),
            cache,
            metrics.clone(),
            cfg(),
        )
        .await;
        assert_eq!(r2.path, path_labels::SONNET_CACHE_HIT);
        assert!(r2.cache_hit);
        assert_eq!(sonnet.calls.load(Ordering::SeqCst), 1);
        let snap = metrics.rewriter_path_snapshot();
        assert_eq!(snap.get(path_labels::SONNET_HIT).copied(), Some(1));
        assert_eq!(snap.get(path_labels::SONNET_CACHE_HIT).copied(), Some(1));
    }

    #[tokio::test]
    async fn sonnet_timeout_falls_through_to_deterministic() {
        let metrics = Arc::new(Metrics::new());
        let r = cascade(
            "  Refactor   the   HNSW   thing  ",
            "explain",
            Some(Arc::new(TimeoutSonnet)),
            Arc::new(RewriteCache::new()),
            metrics.clone(),
            cfg(),
        )
        .await;
        assert_eq!(r.path, path_labels::DETERMINISTIC_FALLBACK);
        assert_eq!(r.query, "refactor the hnsw thing");
        let snap = metrics.rewriter_path_snapshot();
        assert_eq!(snap.get(path_labels::SONNET_TIMEOUT).copied(), Some(1));
        assert_eq!(snap.get(path_labels::DETERMINISTIC_FALLBACK).copied(), Some(1));
    }

    #[tokio::test]
    async fn sonnet_error_falls_through_with_distinct_label() {
        let metrics = Arc::new(Metrics::new());
        let r = cascade(
            "Tune ef_search",
            "decision_lookup",
            Some(Arc::new(ErrorSonnet)),
            Arc::new(RewriteCache::new()),
            metrics.clone(),
            cfg(),
        )
        .await;
        assert_eq!(r.path, path_labels::DETERMINISTIC_FALLBACK);
        let snap = metrics.rewriter_path_snapshot();
        assert_eq!(snap.get(path_labels::SONNET_ERROR).copied(), Some(1));
    }

    #[tokio::test]
    async fn no_sonnet_backend_skips_cache_and_uses_deterministic() {
        let metrics = Arc::new(Metrics::new());
        let r = cascade(
            "Some query",
            "explain",
            None,
            Arc::new(RewriteCache::new()),
            metrics.clone(),
            cfg(),
        )
        .await;
        assert_eq!(r.path, path_labels::DETERMINISTIC_FALLBACK);
        assert!(r.cache_hit);
        assert_eq!(r.query, "some query");
    }

    #[tokio::test]
    async fn cache_evicts_oldest_past_cap() {
        let cache = Arc::new(RewriteCache::new());
        for i in 0..5 {
            cache
                .insert(&format!("q{i}"), "explain", format!("r{i}"), 3)
                .await;
        }
        // cap=3 → keys 0,1 evicted
        assert!(cache
            .get("q0", "explain", Duration::from_secs(3600))
            .await
            .is_none());
        assert!(cache
            .get("q1", "explain", Duration::from_secs(3600))
            .await
            .is_none());
        assert_eq!(
            cache.get("q4", "explain", Duration::from_secs(3600)).await,
            Some("r4".to_string())
        );
    }

    #[tokio::test]
    async fn cache_respects_ttl() {
        let cache = Arc::new(RewriteCache::new());
        cache.insert("q", "explain", "r".into(), 100).await;
        // Past-TTL lookup misses.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let miss = cache.get("q", "explain", Duration::from_millis(1)).await;
        assert!(miss.is_none());
        // Within-TTL lookup hits.
        let hit = cache.get("q", "explain", Duration::from_secs(3600)).await;
        assert_eq!(hit, Some("r".into()));
    }

    #[test]
    fn deterministic_rewrite_collapses_whitespace_and_lowercases() {
        assert_eq!(deterministic_rewrite("  HELLO   WORLD  "), "hello world");
        assert_eq!(deterministic_rewrite("Refactor\tHNSW"), "refactor hnsw");
        assert_eq!(deterministic_rewrite(""), "");
    }

    #[test]
    fn cache_key_is_stable_per_input() {
        let k1 = RewriteCache::key("hello", "explain");
        let k2 = RewriteCache::key("hello", "explain");
        let k3 = RewriteCache::key("hello", "decision_lookup");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }
}
