//! Default composition that layers `Budgeted ← Cached ← backend`.

use super::budget::{BudgetTracker, BudgetedClassifier};
use super::cache::{CachedClassifier, ClassifierCache};
use super::statics::StaticClassifier;
use super::types::Classifier;
use std::sync::Arc;

/// Fully-composed classifier ready to consume from a worker loop.
pub type ClassifierStack =
    BudgetedClassifier<CachedClassifier<Box<dyn Classifier>, Box<dyn ClassifierCache>>>;

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
    build_stack(
        Box::new(StaticClassifier::default()),
        cache,
        budget,
        "static-v1",
    )
}
