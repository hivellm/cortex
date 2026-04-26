//! Integration tests for `cortex_classifier::composer`.

use cortex_classifier::budget::BudgetTracker;
use cortex_classifier::cache::InMemoryCache;
use cortex_classifier::composer::build_offline_stack;
use cortex_classifier::stats::PricingTable;
use cortex_classifier::types::{Classifier, EnrichmentInput};
use cortex_core::events::Kind;
use serde_json::json;
use std::sync::Arc;

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
