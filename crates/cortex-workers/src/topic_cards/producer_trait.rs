//! Phase13b §3.3 — `TopicCardsProducer` wraps the existing
//! [`Orchestrator::run`] pipeline behind the [`EnvelopeProducer`]
//! trait.
//!
//! The wrapper holds the orchestrator plus an input provider; each
//! `produce` call resolves the per-topic `ProduceInput` slice,
//! drives the orchestrator over each entry, and writes one
//! `producer_checkpoints` row per topic_slug scope carrying the
//! card's `event_id` cursor.
//!
//! Input provider trait keeps the wrapper testable without a live
//! Meili / Nexus probe. Production wires the topic-cards
//! orchestrator's existing input-resolver; tests author static
//! lists.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

use super::orchestrator::{Orchestrator, OrchestratorError};
use super::producer::ProduceInput;

/// Canonical producer name.
pub const TOPIC_CARDS_PRODUCER_NAME: &str = "topic_cards";

/// Provider for the per-run input slice. Production resolves
/// topic candidates by walking the topic-card lane; tests author
/// fixed lists.
#[async_trait]
pub trait TopicCardInputProvider: Send + Sync {
    /// Enumerate the topic-card rewrite inputs the producer
    /// should drive at `now`.
    async fn inputs(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<ProduceInput>>;
}

/// Static input provider — returns a fixed slice. Used by tests
/// and by callers that have already materialised the input set.
pub struct StaticTopicCardInputs {
    inputs: Vec<ProduceInput>,
}

impl StaticTopicCardInputs {
    /// Build a static provider with the supplied input slice.
    pub fn new(inputs: Vec<ProduceInput>) -> Self {
        Self { inputs }
    }
}

#[async_trait]
impl TopicCardInputProvider for StaticTopicCardInputs {
    async fn inputs(&self, _now: DateTime<Utc>) -> anyhow::Result<Vec<ProduceInput>> {
        Ok(self.inputs.clone())
    }
}

/// Topic-cards rewrite producer wrapped behind the
/// [`EnvelopeProducer`] trait.
pub struct TopicCardsProducer {
    orchestrator: Arc<Orchestrator>,
    provider: Arc<dyn TopicCardInputProvider>,
}

impl TopicCardsProducer {
    /// Build the producer over the supplied orchestrator + input
    /// provider.
    pub fn new(orchestrator: Arc<Orchestrator>, provider: Arc<dyn TopicCardInputProvider>) -> Self {
        Self {
            orchestrator,
            provider,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for TopicCardsProducer {
    fn name(&self) -> &'static str {
        TOPIC_CARDS_PRODUCER_NAME
    }

    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        let inputs = self.provider.inputs(ctx.now).await?;
        let mut total = 0u64;
        let mut batches = 0u64;
        let mut last_event_id = String::new();
        for (idx, input) in inputs.iter().enumerate() {
            let scope = input.topic_slug.clone();
            match self.orchestrator.run(input.clone()).await {
                Ok(produced) => {
                    total += 1;
                    last_event_id = produced.payload.topic_slug.clone();
                    let store = ctx.metadata.lock().await;
                    let accumulated_at = Utc::now() + chrono::Duration::microseconds(idx as i64);
                    store.record_producer_checkpoint(
                        TOPIC_CARDS_PRODUCER_NAME,
                        &scope,
                        &last_event_id,
                        ctx.now,
                        accumulated_at,
                    )?;
                    batches += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        topic = %scope,
                        "topic_cards producer: orchestrator run failed"
                    );
                    if matches!(err, OrchestratorError::BudgetExhausted { .. }) {
                        // Budget exhaustion is terminal — surface
                        // it so the supervisor stops the run.
                        return Err(anyhow::Error::from(err));
                    }
                    continue;
                }
            }
        }

        Ok(ProducerReport {
            producer_name: TOPIC_CARDS_PRODUCER_NAME.to_string(),
            envelopes_emitted: total,
            batches_emitted: batches,
            last_event_id,
            last_occurred_at: Some(ctx.now),
        })
    }

    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> anyhow::Result<Option<ProducerCheckpoint>> {
        let store = ctx.metadata.lock().await;
        let row = store.latest_producer_checkpoint(TOPIC_CARDS_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
    };
    use async_trait::async_trait;
    use cortex_storage::MetadataStore;
    use tokio::sync::Mutex;

    /// In-test summariser — returns a fixed valid synthesis JSON
    /// so the orchestrator's prompt → output → validation path
    /// runs end-to-end without a live model. Mirrors the shape
    /// the topic_cards synthesiser tests already use.
    struct FixedSummariser;

    #[async_trait]
    impl Summariser for FixedSummariser {
        fn kind(&self) -> SummariserKind {
            SummariserKind::Haiku45
        }
        async fn summarise(
            &self,
            _request: SummariserRequest,
        ) -> Result<SummariserResult, SummariserError> {
            // Build a synthesis body large enough to clear
            // TOPIC_CARD_SYNTHESIS_MIN_BYTES (currently 128).
            let body = format!(
                "{}{}",
                "Synthesis body for the test producer. ".repeat(8),
                "Final paragraph to clear the byte floor."
            );
            let synthesis_json = serde_json::json!({
                "synthesis_markdown": body,
                "confidence": 0.8,
                "open_questions": [],
                "contradictions": [],
            });
            Ok(SummariserResult {
                text: synthesis_json.to_string(),
                cost_cents: 1,
                kind: SummariserKind::Haiku45,
                input_tokens: 0,
                output_tokens: 0,
            })
        }
    }

    fn make_ctx() -> (ProducerCtx, Arc<Mutex<MetadataStore>>) {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        let ctx = ProducerCtx::new(handle.clone(), "cortex.producer.topic_cards");
        (ctx, handle)
    }

    fn make_input(slug: &str) -> ProduceInput {
        ProduceInput {
            topic_slug: slug.into(),
            repo_scope: "cortex".into(),
            existing_card: None,
            all_evidence: Vec::new(),
            new_evidence_text: "evidence".into(),
            superseded_evidence_text: String::new(),
            force_deep: false,
        }
    }

    #[tokio::test]
    async fn topic_cards_producer_writes_one_row_per_topic() {
        let (ctx, handle) = make_ctx();
        let orchestrator = Arc::new(Orchestrator::new(
            Arc::new(FixedSummariser),
            Arc::new(FixedSummariser),
        ));
        let provider = Arc::new(StaticTopicCardInputs::new(vec![
            make_input("auth-rewrite"),
            make_input("retention-design"),
        ]));
        let producer = TopicCardsProducer::new(orchestrator, provider);
        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.batches_emitted, 2);

        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for(TOPIC_CARDS_PRODUCER_NAME, 50)
            .unwrap();
        assert_eq!(rows.len(), 2);
        let scopes: Vec<String> = rows.iter().map(|r| r.scope.clone()).collect();
        assert!(scopes.iter().any(|s| s == "auth-rewrite"));
        assert!(scopes.iter().any(|s| s == "retention-design"));
    }

    #[tokio::test]
    async fn topic_cards_resume_from_returns_latest_per_scope() {
        let (ctx, _handle) = make_ctx();
        let orchestrator = Arc::new(Orchestrator::new(
            Arc::new(FixedSummariser),
            Arc::new(FixedSummariser),
        ));
        let provider = Arc::new(StaticTopicCardInputs::new(vec![make_input("auth-rewrite")]));
        let producer = TopicCardsProducer::new(orchestrator, provider);
        let _ = producer.produce(&ctx).await.unwrap();
        let resume = producer.resume_from(&ctx, "auth-rewrite").await.unwrap();
        assert!(resume.is_some());
    }

    #[tokio::test]
    async fn topic_cards_empty_input_emits_zero_rows() {
        let (ctx, handle) = make_ctx();
        let orchestrator = Arc::new(Orchestrator::new(
            Arc::new(FixedSummariser),
            Arc::new(FixedSummariser),
        ));
        let provider = Arc::new(StaticTopicCardInputs::new(Vec::new()));
        let producer = TopicCardsProducer::new(orchestrator, provider);
        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.batches_emitted, 0);
        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for(TOPIC_CARDS_PRODUCER_NAME, 50)
            .unwrap();
        assert!(rows.is_empty());
    }
}
