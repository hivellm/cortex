//! Integration tests for `cortex_workers::classifier::cache`.

use cortex_core::events::Kind;
use cortex_workers::classifier::cache::{CachedClassifier, ClassifierCache, InMemoryCache};
use cortex_workers::classifier::statics::StaticClassifier;
use cortex_workers::classifier::types::{Classifier, ClassifierSource, EnrichmentInput};
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
    assert_eq!(clf.cache().len(), 1);
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

#[tokio::test]
async fn classifier_cache_trait_is_object_safe() {
    // Smoke test that ClassifierCache can be boxed (used by the composer
    // stack). If this stops compiling, the trait lost object safety.
    let _: Box<dyn ClassifierCache> = Box::new(InMemoryCache::default());
}
