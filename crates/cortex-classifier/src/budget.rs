//! Daily spend tracker + budget-gated classifier.

use crate::errors::ClassifierError;
use crate::stats::PricingTable;
use crate::statics::StaticClassifier;
use crate::types::{Classifier, ClassifierOutput, EnrichmentInput};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

/// Threshold state derived from current spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetState {
    /// Below the warn threshold (spend / limit < 0.8).
    Normal,
    /// Warn (0.8 ≤ spend/limit < 0.9).
    Warn,
    /// Degrade (0.9 ≤ spend/limit < 1.0) — callers should shrink prompts.
    Degrade,
    /// Halted (spend/limit ≥ 1.0) — fall back to the static classifier.
    Halt,
}

/// Running daily budget tracker; thread-safe via atomics.
#[derive(Debug)]
pub struct BudgetTracker {
    daily_limit_cents: u64,
    spend_today_cents: AtomicU64,
    pricing: PricingTable,
    warn: f32,
    degrade: f32,
    halt: f32,
}

impl BudgetTracker {
    /// Build a tracker with the default 0.8 / 0.9 / 1.0 thresholds.
    pub fn new(daily_limit_cents: u64, pricing: PricingTable) -> Self {
        Self {
            daily_limit_cents,
            spend_today_cents: AtomicU64::new(0),
            pricing,
            warn: 0.8,
            degrade: 0.9,
            halt: 1.0,
        }
    }

    /// Build a tracker with custom thresholds.
    pub fn with_thresholds(
        daily_limit_cents: u64,
        pricing: PricingTable,
        warn: f32,
        degrade: f32,
        halt: f32,
    ) -> Self {
        Self {
            daily_limit_cents,
            spend_today_cents: AtomicU64::new(0),
            pricing,
            warn,
            degrade,
            halt,
        }
    }

    /// Record spend from a single classifier call.
    pub fn record(&self, tokens_in: u64, tokens_out: u64) {
        let cents = self.pricing.spend_cents(tokens_in, tokens_out);
        self.spend_today_cents.fetch_add(cents, Ordering::Relaxed);
    }

    /// Current spend in US cents.
    pub fn spend_cents(&self) -> u64 {
        self.spend_today_cents.load(Ordering::Relaxed)
    }

    /// Ratio of spend to limit (0.0 if limit is 0).
    pub fn ratio(&self) -> f32 {
        if self.daily_limit_cents == 0 {
            return 0.0;
        }
        self.spend_cents() as f32 / self.daily_limit_cents as f32
    }

    /// Current state.
    pub fn state(&self) -> BudgetState {
        let r = self.ratio();
        if r >= self.halt {
            BudgetState::Halt
        } else if r >= self.degrade {
            BudgetState::Degrade
        } else if r >= self.warn {
            BudgetState::Warn
        } else {
            BudgetState::Normal
        }
    }

    /// Reset the spend counter. Usually called at UTC midnight.
    pub fn reset(&self) {
        self.spend_today_cents.store(0, Ordering::Relaxed);
    }
}

/// [`Classifier`] wrapper that shunts to `StaticClassifier` when the tracker halts.
pub struct BudgetedClassifier<C> {
    inner: C,
    tracker: std::sync::Arc<BudgetTracker>,
    fallback: StaticClassifier,
}

impl<C> BudgetedClassifier<C> {
    /// Build a new budgeted classifier.
    pub fn new(inner: C, tracker: std::sync::Arc<BudgetTracker>) -> Self {
        Self {
            inner,
            tracker,
            fallback: StaticClassifier::default(),
        }
    }

    /// Current budget state.
    pub fn state(&self) -> BudgetState {
        self.tracker.state()
    }
}

#[async_trait]
impl<C> Classifier for BudgetedClassifier<C>
where
    C: Classifier,
{
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, ClassifierError> {
        if self.tracker.state() == BudgetState::Halt {
            return self.fallback.classify_batch(events).await;
        }
        let out = self.inner.classify_batch(events).await?;
        for record in &out {
            self.tracker
                .record(record.tokens_in as u64, record.tokens_out as u64);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClassifierSource, PiiRisk, Severity};
    use async_trait::async_trait;
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
        tracker.spend_today_cents.store(80, Ordering::Relaxed);
        assert_eq!(tracker.state(), BudgetState::Warn);
        tracker.spend_today_cents.store(95, Ordering::Relaxed);
        assert_eq!(tracker.state(), BudgetState::Degrade);
        tracker.spend_today_cents.store(101, Ordering::Relaxed);
        assert_eq!(tracker.state(), BudgetState::Halt);
    }

    #[tokio::test]
    async fn halt_falls_back_to_static() {
        let tracker = Arc::new(BudgetTracker::new(100, PricingTable::HAIKU_4_5));
        tracker.spend_today_cents.store(999, Ordering::Relaxed);
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
}
