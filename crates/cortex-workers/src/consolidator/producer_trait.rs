//! Phase13b §3.4 — `ConsolidatorProducer` wraps the existing
//! consolidator orchestrator's three grain entrypoints
//! ([`Orchestrator::run_session`], `run_topic`, `run_decision`)
//! behind the [`EnvelopeProducer`] trait.
//!
//! The wrapper holds an `Arc<Orchestrator>` plus a
//! [`ConsolidatorInputProvider`] that returns the per-grain input
//! lists. Each `produce` invocation drives every input in
//! declaration order (sessions → topics → decisions) and writes
//! one `producer_checkpoints` row per grain scope. The
//! existing `~/.cortex/consolidator-cursor.json` file-store keeps
//! its per-grain counters in parallel; the trait surface adds the
//! cross-grain audit the file-store cannot answer.
//!
//! Scope policy:
//! - `session_id` for the Session grain.
//! - `topic:<label>` for the Topic grain.
//! - `decision:<event_id>` for the DecisionTrace grain.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

use super::orchestrator::Orchestrator;
use super::producer::decision_trace::DecisionTraceInput;
use super::producer::session::SessionInput;
use super::producer::topic::TopicCluster;

/// Canonical producer name.
pub const CONSOLIDATOR_PRODUCER_NAME: &str = "consolidator";

/// Provider returning the per-grain inputs the producer should
/// process at `now`. Production wires the live source layer
/// (`LiveSessionSource`, `LiveTopicSource`,
/// `LiveDecisionTraceSource`); tests author static slices via
/// [`StaticConsolidatorInputs`].
#[async_trait]
pub trait ConsolidatorInputProvider: Send + Sync {
    /// Session inputs at `now`.
    async fn sessions(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<SessionInput>>;
    /// Topic clusters at `now`.
    async fn topics(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<TopicCluster>>;
    /// Decision-trace inputs at `now`.
    async fn decisions(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<DecisionTraceInput>>;
}

/// Static provider — returns fixed slices. Useful for tests and
/// for the nightly-bin path that has already materialised the
/// per-grain inputs.
pub struct StaticConsolidatorInputs {
    sessions: Vec<SessionInput>,
    topics: Vec<TopicCluster>,
    decisions: Vec<DecisionTraceInput>,
}

impl StaticConsolidatorInputs {
    /// Build a static provider with the supplied per-grain slices.
    pub fn new(
        sessions: Vec<SessionInput>,
        topics: Vec<TopicCluster>,
        decisions: Vec<DecisionTraceInput>,
    ) -> Self {
        Self {
            sessions,
            topics,
            decisions,
        }
    }

    /// Empty provider — every grain returns `Vec::new()`. Tests
    /// that need exactly one grain build via
    /// [`Self::new(...)`] directly.
    pub fn empty() -> Self {
        Self::new(Vec::new(), Vec::new(), Vec::new())
    }
}

#[async_trait]
impl ConsolidatorInputProvider for StaticConsolidatorInputs {
    async fn sessions(&self, _now: DateTime<Utc>) -> anyhow::Result<Vec<SessionInput>> {
        Ok(self.sessions.clone())
    }
    async fn topics(&self, _now: DateTime<Utc>) -> anyhow::Result<Vec<TopicCluster>> {
        Ok(self.topics.clone())
    }
    async fn decisions(&self, _now: DateTime<Utc>) -> anyhow::Result<Vec<DecisionTraceInput>> {
        Ok(self.decisions.clone())
    }
}

/// Consolidator producer wrapped behind the [`EnvelopeProducer`]
/// trait.
pub struct ConsolidatorProducer {
    orchestrator: Arc<Orchestrator>,
    provider: Arc<dyn ConsolidatorInputProvider>,
}

impl ConsolidatorProducer {
    /// Build the producer over the supplied orchestrator + input
    /// provider.
    pub fn new(
        orchestrator: Arc<Orchestrator>,
        provider: Arc<dyn ConsolidatorInputProvider>,
    ) -> Self {
        Self {
            orchestrator,
            provider,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for ConsolidatorProducer {
    fn name(&self) -> &'static str {
        CONSOLIDATOR_PRODUCER_NAME
    }

    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        let sessions = self.provider.sessions(ctx.now).await?;
        let topics = self.provider.topics(ctx.now).await?;
        let decisions = self.provider.decisions(ctx.now).await?;

        let mut total = 0u64;
        let mut batches = 0u64;
        let mut last_event_id = String::new();
        let mut offset_us = 0i64;

        for input in &sessions {
            let scope = input.session_id.clone();
            match self.orchestrator.run_session(input).await {
                Ok(produced) => {
                    total += 1;
                    last_event_id = produced.payload.consolidation_id.clone();
                    let store = ctx.metadata.lock().await;
                    store.record_producer_checkpoint(
                        CONSOLIDATOR_PRODUCER_NAME,
                        &scope,
                        &last_event_id,
                        ctx.now,
                        Utc::now() + chrono::Duration::microseconds(offset_us),
                    )?;
                    offset_us += 1;
                    batches += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        session = %scope,
                        "consolidator producer: session run failed"
                    );
                    continue;
                }
            }
        }

        for cluster in &topics {
            let scope = format!("topic:{}", cluster.label);
            match self.orchestrator.run_topic(cluster).await {
                Ok(produced) => {
                    total += 1;
                    last_event_id = produced.payload.consolidation_id.clone();
                    let store = ctx.metadata.lock().await;
                    store.record_producer_checkpoint(
                        CONSOLIDATOR_PRODUCER_NAME,
                        &scope,
                        &last_event_id,
                        ctx.now,
                        Utc::now() + chrono::Duration::microseconds(offset_us),
                    )?;
                    offset_us += 1;
                    batches += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        topic = %scope,
                        "consolidator producer: topic run failed"
                    );
                    continue;
                }
            }
        }

        for input in &decisions {
            let scope = format!("decision:{}", input.decision.event_id);
            match self.orchestrator.run_decision_trace(input).await {
                Ok(produced) => {
                    total += 1;
                    last_event_id = produced.payload.consolidation_id.clone();
                    let store = ctx.metadata.lock().await;
                    store.record_producer_checkpoint(
                        CONSOLIDATOR_PRODUCER_NAME,
                        &scope,
                        &last_event_id,
                        ctx.now,
                        Utc::now() + chrono::Duration::microseconds(offset_us),
                    )?;
                    offset_us += 1;
                    batches += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        decision = %scope,
                        "consolidator producer: decision run failed"
                    );
                    continue;
                }
            }
        }

        Ok(ProducerReport {
            producer_name: CONSOLIDATOR_PRODUCER_NAME.to_string(),
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
        let row = store.latest_producer_checkpoint(CONSOLIDATOR_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_storage::MetadataStore;
    use tokio::sync::Mutex;

    fn make_ctx() -> (ProducerCtx, Arc<Mutex<MetadataStore>>) {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        let ctx = ProducerCtx::new(handle.clone(), "cortex.producer.consolidator");
        (ctx, handle)
    }

    // Use the existing dummy summariser the consolidator tests
    // already ship — keeps the unit fast and matches the
    // production trait shape exactly.
    use crate::consolidator::summariser::{
        Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
    };
    use async_trait::async_trait;

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
            Ok(SummariserResult {
                text: "Consolidation body adequate for test path.\n\nFinal paragraph clears the floor.".repeat(4),
                cost_cents: 1,
                kind: SummariserKind::Haiku45,
                input_tokens: 0,
                output_tokens: 0,
            })
        }
    }

    #[tokio::test]
    async fn consolidator_empty_input_emits_zero_rows() {
        let (ctx, handle) = make_ctx();
        let orchestrator = Arc::new(Orchestrator::new(
            Arc::new(FixedSummariser),
            Arc::new(FixedSummariser),
        ));
        let provider = Arc::new(StaticConsolidatorInputs::empty());
        let producer = ConsolidatorProducer::new(orchestrator, provider);
        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.batches_emitted, 0);
        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for(CONSOLIDATOR_PRODUCER_NAME, 50)
            .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn consolidator_resume_from_returns_none_on_fresh_corpus() {
        let (ctx, _handle) = make_ctx();
        let orchestrator = Arc::new(Orchestrator::new(
            Arc::new(FixedSummariser),
            Arc::new(FixedSummariser),
        ));
        let provider = Arc::new(StaticConsolidatorInputs::empty());
        let producer = ConsolidatorProducer::new(orchestrator, provider);
        let resume = producer.resume_from(&ctx, "01ANYSESSION").await.unwrap();
        assert!(resume.is_none());
    }

    #[test]
    fn consolidator_producer_name_is_canonical() {
        let orchestrator = Arc::new(Orchestrator::new(
            Arc::new(FixedSummariser),
            Arc::new(FixedSummariser),
        ));
        let provider = Arc::new(StaticConsolidatorInputs::empty());
        let producer = ConsolidatorProducer::new(orchestrator, provider);
        assert_eq!(producer.name(), "consolidator");
    }
}
