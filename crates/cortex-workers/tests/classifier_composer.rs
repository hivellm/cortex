//! Integration tests for `cortex_workers::classifier::composer`.

use cortex_core::events::Kind;
use cortex_workers::classifier::budget::BudgetTracker;
use cortex_workers::classifier::cache::InMemoryCache;
use cortex_workers::classifier::composer::build_offline_stack;
use cortex_workers::classifier::stats::PricingTable;
use cortex_workers::classifier::types::{Classifier, EnrichmentInput};
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
