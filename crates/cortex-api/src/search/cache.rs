//! Whole-response cache. Spec 11 §Caching: TTL 10 min, key
//! `hash(intent || scope || query || schema_version)`. Synap-backed
//! storage is documented in the spec but the daemon owns a single
//! process today, so an in-memory `tokio::sync::RwLock`-protected
//! map is correct and trivially testable. Swapping backends later is
//! a matter of swapping the trait implementation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::types::{QueryRequest, QueryResponse};

/// Schema-version tag — every cache key includes it so deploys with
/// a migration-worthy event/graph schema change automatically miss
/// every previously-cached entry.
pub const SCHEMA_VERSION: &str = "v1";

/// Default TTL — spec 11 §Caching.
pub const DEFAULT_TTL: Duration = Duration::from_secs(600);

/// Bounds retained responses when archive revisions generate new cache keys.
const MAX_ENTRIES: usize = 1024;

/// Cache trait.
#[async_trait]
pub trait Cache: Send + Sync {
    /// Look up the cached response for `key`. Implementations
    /// honour their own TTL — `None` means miss.
    async fn get(&self, key: &str) -> Option<QueryResponse>;
    /// Store `value` under `key`.
    async fn put(&self, key: &str, value: QueryResponse);
    /// Drop entries for `repo`. Spec 11 §Caching: invalidation by
    /// scope on every `severity=critical` ingestion event.
    async fn invalidate_repo(&self, repo: &str);
    /// Drop every cached entry. Test helper.
    async fn clear(&self);
}

/// In-memory cache with TTL and at most 1024 responses. Inserts prune expired
/// responses, then evict the oldest insertion when full. Lookups use a read-lock.
pub struct InMemoryCache {
    inner: RwLock<HashMap<String, Entry>>,
    ttl: Duration,
}

struct Entry {
    response: QueryResponse,
    inserted: Instant,
    repo: Option<String>,
}

impl InMemoryCache {
    /// Build a cache with the spec-default TTL.
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }
    /// Build a cache with a custom TTL (used by tests).
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    fn fresh(&self, e: &Entry) -> bool {
        e.inserted.elapsed() <= self.ttl
    }
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Cache for InMemoryCache {
    async fn get(&self, key: &str) -> Option<QueryResponse> {
        let g = self.inner.read().await;
        g.get(key).filter(|e| self.fresh(e)).map(|e| {
            let mut r = e.response.clone();
            r.budget.cache = "hit".to_string();
            r
        })
    }
    async fn put(&self, key: &str, value: QueryResponse) {
        let repo = value.scope_resolved.repo.clone();
        let mut g = self.inner.write().await;
        g.retain(|_, entry| self.fresh(entry));
        if g.len() >= MAX_ENTRIES && !g.contains_key(key) {
            let oldest = g
                .iter()
                .min_by(|(left_key, left), (right_key, right)| {
                    left.inserted
                        .cmp(&right.inserted)
                        .then_with(|| left_key.cmp(right_key))
                })
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                g.remove(&oldest);
            }
        }
        g.insert(
            key.to_string(),
            Entry {
                response: value,
                inserted: Instant::now(),
                repo,
            },
        );
    }
    async fn invalidate_repo(&self, repo: &str) {
        let mut g = self.inner.write().await;
        g.retain(|_, e| e.repo.as_deref() != Some(repo));
    }
    async fn clear(&self) {
        let mut g = self.inner.write().await;
        g.clear();
    }
}

/// Compute the cache key for a request. Mirrors spec 11 §Caching.
pub fn cache_key(req: &QueryRequest) -> String {
    #[derive(Serialize)]
    struct KeyInput<'a> {
        intent: &'a str,
        scope: &'a crate::types::Scope,
        query: &'a str,
        schema: &'static str,
    }
    let input = KeyInput {
        intent: req.intent.label(),
        scope: &req.scope,
        query: &req.query,
        schema: SCHEMA_VERSION,
    };
    let bytes = serde_json::to_vec(&input).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Convenience wrapper: trait object cache.
pub type CacheHandle = Arc<dyn Cache>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{empty_response, IncludeField, Intent, QueryRequest, Scope};

    fn req(intent: Intent, query: &str, repo: Option<&str>) -> QueryRequest {
        QueryRequest {
            intent,
            scope: Scope {
                repo: repo.map(String::from),
                ..Default::default()
            },
            query: query.to_string(),
            limit: 20,
            k: 50,
            include: vec![IncludeField::Snippets],
            budget_ms: 500,
            budget_bytes: None,
            as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
            principal: None,
        }
    }

    #[tokio::test]
    async fn put_then_get_returns_a_hit_with_cache_label() {
        let cache = InMemoryCache::new();
        let r = req(Intent::PreChangeContext, "x", Some("R"));
        let key = cache_key(&r);
        cache.put(&key, empty_response(&r)).await;
        let hit = cache.get(&key).await.expect("hit");
        assert_eq!(hit.budget.cache, "hit");
    }

    #[tokio::test]
    async fn ttl_expires_entries_after_the_window() {
        let cache = InMemoryCache::with_ttl(Duration::from_millis(20));
        let r = req(Intent::PreChangeContext, "x", None);
        let key = cache_key(&r);
        cache.put(&key, empty_response(&r)).await;
        assert!(cache.get(&key).await.is_some());
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(cache.get(&key).await.is_none(), "entry expired");
    }

    #[tokio::test]
    async fn revision_churn_evicts_oldest_response_and_keeps_recent_responses() {
        let cache = InMemoryCache::new();
        let request = req(Intent::PreChangeContext, "archive revision", Some("Cortex"));
        for revision in 0..MAX_ENTRIES {
            cache
                .put(&format!("revision-{revision}"), empty_response(&request))
                .await;
        }
        cache
            .inner
            .write()
            .await
            .get_mut("revision-0")
            .unwrap()
            .inserted = Instant::now() - Duration::from_secs(1);

        // Updating an existing response at capacity must not evict another key.
        cache.put("revision-1", empty_response(&request)).await;
        assert!(cache.get("revision-0").await.is_some());
        cache.put("newest-revision", empty_response(&request)).await;

        assert_eq!(cache.inner.read().await.len(), MAX_ENTRIES);
        assert!(cache.get("revision-0").await.is_none());
        assert!(cache.get("revision-1").await.is_some());
        assert!(cache.get("newest-revision").await.is_some());
    }

    #[tokio::test]
    async fn inserting_fresh_response_reclaims_expired_response_storage() {
        let cache = InMemoryCache::with_ttl(Duration::from_secs(1));
        let request = req(Intent::PreChangeContext, "archive revision", None);
        cache
            .put("expired-revision", empty_response(&request))
            .await;
        cache
            .inner
            .write()
            .await
            .get_mut("expired-revision")
            .unwrap()
            .inserted = Instant::now() - Duration::from_secs(2);
        assert!(cache.get("expired-revision").await.is_none());

        cache.put("fresh-revision", empty_response(&request)).await;

        assert_eq!(cache.inner.read().await.len(), 1);
        assert!(!cache.inner.read().await.contains_key("expired-revision"));
        assert_eq!(
            cache.get("fresh-revision").await.unwrap().budget.cache,
            "hit"
        );
    }

    #[tokio::test]
    async fn invalidate_repo_drops_only_matching_entries() {
        let cache = InMemoryCache::new();
        let r1 = req(Intent::PreChangeContext, "x", Some("Vectorizer"));
        let r2 = req(Intent::PreChangeContext, "x", Some("Nexus"));
        cache.put(&cache_key(&r1), empty_response(&r1)).await;
        cache.put(&cache_key(&r2), empty_response(&r2)).await;
        cache.invalidate_repo("Vectorizer").await;
        assert!(cache.get(&cache_key(&r1)).await.is_none());
        assert!(cache.get(&cache_key(&r2)).await.is_some());
    }

    #[test]
    fn cache_key_is_deterministic_across_runs() {
        let r1 = req(Intent::PreChangeContext, "tune ef_search", Some("R"));
        let r2 = req(Intent::PreChangeContext, "tune ef_search", Some("R"));
        assert_eq!(cache_key(&r1), cache_key(&r2));
    }

    #[test]
    fn cache_key_changes_with_intent_and_query() {
        let a = req(Intent::PreChangeContext, "x", None);
        let b = req(Intent::PreChangeContext, "y", None);
        let c = req(Intent::FreeSearch, "x", None);
        assert_ne!(cache_key(&a), cache_key(&b));
        assert_ne!(cache_key(&a), cache_key(&c));
    }
}
