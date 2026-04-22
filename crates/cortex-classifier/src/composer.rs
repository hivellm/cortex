//! Default composition that layers `Budgeted ← Cached ← backend`.

use crate::budget::{BudgetTracker, BudgetedClassifier};
use crate::cache::{CachedClassifier, ClassifierCache};
use crate::statics::StaticClassifier;
use crate::types::Classifier;
use std::sync::Arc;

/// Fully-composed classifier ready to consume from a worker loop.
pub type ClassifierStack = BudgetedClassifier<CachedClassifier<Box<dyn Classifier>, Box<dyn ClassifierCache>>>;

/// Convenience builder that wraps a backend in the standard cache + budget stack.
pub fn build_stack(
    backend: Box<dyn Classifier>,
    cache: Box<dyn ClassifierCache>,
    budget: Arc<BudgetTracker>,
    prompt_version: impl Into<String>,
) -> ClassifierStack {
    let cached = CachedClassifier::new(backend, cache, prompt_version);
    BudgetedClassifier::new(cached, budget)
}

/// Compose a fully-offline stack (no network): [`StaticClassifier`] behind the cache + budget.
pub fn build_offline_stack(
    cache: Box<dyn ClassifierCache>,
    budget: Arc<BudgetTracker>,
) -> ClassifierStack {
    build_stack(Box::new(StaticClassifier::default()), cache, budget, "static-v1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;
    use crate::stats::PricingTable;
    use crate::types::EnrichmentInput;
    use cortex_core::events::Kind;
    use serde_json::json;

    fn input() -> EnrichmentInput {
        EnrichmentInput {
            event_id: "01H".into(),
            kind: Kind::Turn,
            content_hash: "sha256:aa".into(),
            redacted_payload: json!({ "user_message": "refactor please" }),
            context_repo: None,
        }
    }

    #[tokio::test]
    async fn offline_stack_runs_end_to_end() {
        let stack = build_offline_stack(
            Box::new(InMemoryCache::default()),
            Arc::new(BudgetTracker::new(100, PricingTable::HAIKU_4_5)),
        );
        let out = stack.classify_batch(&[input()]).await.unwrap();
        assert_eq!(out.len(), 1);
    }
}
