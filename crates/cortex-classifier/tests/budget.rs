//! Integration tests for `cortex_classifier::budget`.

use async_trait::async_trait;
use cortex_classifier::budget::{BudgetState, BudgetTracker, BudgetedClassifier};
use cortex_classifier::errors::ClassifierError;
use cortex_classifier::stats::PricingTable;
use cortex_classifier::types::{
    Classifier, ClassifierOutput, ClassifierSource, EnrichmentInput, PiiRisk, Severity,
};
use cortex_core::events::Kind;
use serde_json::json;
use std::sync::Arc;

struct FakeHaiku {
    tokens_in: u32,
    tokens_out: u32,
}

#[async_trait]
impl Classifier for FakeHaiku {
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, ClassifierError> {
        Ok(events
            .iter()
            .map(|e| ClassifierOutput {
                event_id: e.event_id.clone(),
                kind_refinement: None,
                topics: vec!["code".into()],
                severity: Severity::Info,
                pii_risk: PiiRisk::Low,
                redaction_suggestions: vec![],
                summary: None,
                entities: Vec::new(),
                relations: Vec::new(),
                source: ClassifierSource::HaikuCli,
                prompt_version: "v1".into(),
                model: "claude-haiku-4-5".into(),
                latency_ms: 10,
                tokens_in: self.tokens_in,
                tokens_out: self.tokens_out,
            })
            .collect())
    }
}

fn input(i: u32) -> EnrichmentInput {
    EnrichmentInput {
        event_id: format!("e{i}"),
        kind: Kind::Turn,
        content_hash: format!("sha256:{i:02x}"),
        redacted_payload: json!({ "user_message": "hi" }),
        context_repo: None,
    }
}

#[tokio::test]
async fn state_transitions() {
    let tracker = BudgetTracker::new(100, PricingTable::HAIKU_4_5);
    assert_eq!(tracker.state(), BudgetState::Normal);
    tracker.set_spend_cents_for_test(80);
    assert_eq!(tracker.state(), BudgetState::Warn);
    tracker.set_spend_cents_for_test(95);
    assert_eq!(tracker.state(), BudgetState::Degrade);
    tracker.set_spend_cents_for_test(101);
    assert_eq!(tracker.state(), BudgetState::Halt);
}

#[tokio::test]
async fn halt_falls_back_to_static() {
    let tracker = Arc::new(BudgetTracker::new(100, PricingTable::HAIKU_4_5));
    tracker.set_spend_cents_for_test(999);
    let inner = FakeHaiku {
        tokens_in: 1_000_000_000,
        tokens_out: 1_000_000_000,
    };
    let clf = BudgetedClassifier::new(inner, tracker);
    let out = clf.classify_batch(&[input(1), input(2)]).await.unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].source, ClassifierSource::StaticFallback);
    assert_eq!(out[1].source, ClassifierSource::StaticFallback);
}

#[tokio::test]
async fn normal_path_passes_through_and_records_spend() {
    let tracker = Arc::new(BudgetTracker::new(1_000, PricingTable::HAIKU_4_5));
    let inner = FakeHaiku {
        tokens_in: 1000,
        tokens_out: 1000,
    };
    let clf = BudgetedClassifier::new(inner, tracker.clone());
    let out = clf.classify_batch(&[input(1)]).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].source, ClassifierSource::HaikuCli);
    assert!(tracker.spend_cents() > 0);
}
