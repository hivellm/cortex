//! Classifier cache — keyed by content hash + prompt version.
//!
//! In-memory default is enough for unit tests and single-process dev. A
//! Synap-backed impl lives behind the same trait and plugs in under the
//! full worker wiring.

use crate::errors::ClassifierError;
use crate::types::{Classifier, ClassifierOutput, ClassifierSource, EnrichmentInput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// Cache errors.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// Backend failure surfaced to the caller.
    #[error("backend: {0}")]
    Backend(String),
}

/// Abstract cache over `(content_hash, prompt_version) → ClassifierOutput`.
#[async_trait]
pub trait ClassifierCache: Send + Sync + 'static {
    /// Look up a cached record. Returns `None` on miss.
    async fn get(
        &self,
        key: &str,
    ) -> Result<Option<ClassifierOutput>, CacheError>;
    /// Store a classifier output under the given key.
    async fn put(&self, key: &str, value: ClassifierOutput) -> Result<(), CacheError>;
}

/// In-memory cache (default for tests + single-process dev).
#[derive(Default)]
pub struct InMemoryCache {
    inner: Mutex<HashMap<String, ClassifierOutput>>,
}

impl InMemoryCache {
    /// Number of currently-cached entries.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl ClassifierCache for InMemoryCache {
    async fn get(&self, key: &str) -> Result<Option<ClassifierOutput>, CacheError> {
        Ok(self.inner.lock().map(|m| m.get(key).cloned()).unwrap_or(None))
    }

    async fn put(&self, key: &str, value: ClassifierOutput) -> Result<(), CacheError> {
        if let Ok(mut m) = self.inner.lock() {
            m.insert(key.to_string(), value);
        }
        Ok(())
    }
}

#[async_trait]
impl<T: ClassifierCache + ?Sized> ClassifierCache for Box<T> {
    async fn get(&self, key: &str) -> Result<Option<ClassifierOutput>, CacheError> {
        (**self).get(key).await
    }
    async fn put(&self, key: &str, value: ClassifierOutput) -> Result<(), CacheError> {
        (**self).put(key, value).await
    }
}

/// Wraps any [`Classifier`] with a pre/post-cache.
///
/// On `classify_batch`:
/// 1. For each input, compute `key = content_hash + ':' + prompt_version`
///    and ask the cache.
/// 2. Hits are turned into records with `source=cache` and zero token counts.
/// 3. Misses are forwarded to `inner`; results are written back to the cache.
/// 4. Results are returned in the original input order.
pub struct CachedClassifier<C, K> {
    inner: C,
    cache: K,
    prompt_version: String,
}

impl<C, K> CachedClassifier<C, K> {
    /// Build a new cached classifier.
    pub fn new(inner: C, cache: K, prompt_version: impl Into<String>) -> Self {
        Self {
            inner,
            cache,
            prompt_version: prompt_version.into(),
        }
    }

    fn key(&self, content_hash: &str) -> String {
        format!("{content_hash}:{}", self.prompt_version)
    }
}

#[async_trait]
impl<C, K> Classifier for CachedClassifier<C, K>
where
    C: Classifier,
    K: ClassifierCache,
{
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, ClassifierError> {
        let mut results: Vec<Option<ClassifierOutput>> = vec![None; events.len()];
        let mut misses: Vec<(usize, EnrichmentInput)> = Vec::new();

        for (i, input) in events.iter().enumerate() {
            let key = self.key(&input.content_hash);
            match self.cache.get(&key).await {
                Ok(Some(mut hit)) => {
                    hit.source = ClassifierSource::Cache;
                    hit.latency_ms = 0;
                    hit.tokens_in = 0;
                    hit.tokens_out = 0;
                    hit.event_id = input.event_id.clone();
                    results[i] = Some(hit);
                }
                Ok(None) => misses.push((i, input.clone())),
                Err(e) => return Err(ClassifierError::Cache(e.to_string())),
            }
        }

        if !misses.is_empty() {
            let miss_inputs: Vec<EnrichmentInput> =
                misses.iter().map(|(_, input)| input.clone()).collect();
            let miss_out = self.inner.classify_batch(&miss_inputs).await?;
            if miss_out.len() != miss_inputs.len() {
                return Err(ClassifierError::LengthMismatch {
                    expected: miss_inputs.len(),
                    actual: miss_out.len(),
                });
            }
            for ((slot, input), output) in misses.iter().zip(miss_out.into_iter()) {
                let key = self.key(&input.content_hash);
                self.cache
                    .put(&key, output.clone())
                    .await
                    .map_err(|e| ClassifierError::Cache(e.to_string()))?;
                results[*slot] = Some(output);
            }
        }

        Ok(results.into_iter().map(|o| o.expect("all slots filled")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statics::StaticClassifier;
    use cortex_core::events::Kind;
    use serde_json::json;

    fn input(event_id: &str, content_hash: &str) -> EnrichmentInput {
        EnrichmentInput {
            event_id: event_id.into(),
            kind: Kind::Turn,
            content_hash: content_hash.into(),
            redacted_payload: json!({ "user_message": "hi" }),
            context_repo: None,
        }
    }

    #[tokio::test]
    async fn first_call_populates_cache() {
        let cache = InMemoryCache::default();
        let clf = CachedClassifier::new(StaticClassifier::new(), cache, "v1");
        let _ = clf
            .classify_batch(&[input("a", "sha256:aa")])
            .await
            .unwrap();
        assert_eq!(clf.cache.len(), 1);
    }

    #[tokio::test]
    async fn second_call_with_same_hash_hits_cache() {
        let cache = InMemoryCache::default();
        let clf = CachedClassifier::new(StaticClassifier::new(), cache, "v1");
        let out1 = clf
            .classify_batch(&[input("a", "sha256:xx")])
            .await
            .unwrap();
        let out2 = clf
            .classify_batch(&[input("b", "sha256:xx")])
            .await
            .unwrap();
        assert_eq!(out1.len(), 1);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].source, ClassifierSource::Cache);
        assert_eq!(out2[0].event_id, "b");
    }

    #[tokio::test]
    async fn mixed_hits_and_misses_preserve_order() {
        let cache = InMemoryCache::default();
        let clf = CachedClassifier::new(StaticClassifier::new(), cache, "v1");
        let _ = clf
            .classify_batch(&[input("seed", "sha256:cached")])
            .await
            .unwrap();
        let out = clf
            .classify_batch(&[
                input("a", "sha256:cached"),
                input("b", "sha256:new"),
                input("c", "sha256:cached"),
            ])
            .await
            .unwrap();
        assert_eq!(out[0].event_id, "a");
        assert_eq!(out[1].event_id, "b");
        assert_eq!(out[2].event_id, "c");
        assert_eq!(out[0].source, ClassifierSource::Cache);
        assert_eq!(out[1].source, ClassifierSource::StaticFallback);
        assert_eq!(out[2].source, ClassifierSource::Cache);
    }
}
