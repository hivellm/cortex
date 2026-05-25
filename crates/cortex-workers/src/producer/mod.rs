//! Phase13b — `EnvelopeProducer` trait, the single contract for
//! bootstrap / claude-archive / topic-cards-emit / consolidator-emit
//! and every future adapter (OpenCode, Cursor, Codex, Gemini).
//!
//! Reference: ADR-010 (`adr-010-envelopeproducer-trait-
//! accumulating-checkpoint-table`),
//! `docs/analysis/rework/04-architecture.md` §A.2.
//!
//! Background. Four producers each construct envelopes ad-hoc:
//! bootstrap overwrites `.cortex-bootstrap.state.json` on every
//! invocation (no multi-repo accumulation); claude-archive keeps a
//! per-project file cursor; topic-cards re-emits the entire ladder
//! every tick because it has no persistence between runs;
//! consolidator stores its grain cursor at
//! `~/.cortex/consolidator-cursor.json`. Four cursor stores cannot
//! answer "what envelopes has any producer ever emitted?"; every new
//! adapter reimplements the same shape.
//!
//! Trait surface:
//!
//! ```ignore
//! #[async_trait]
//! pub trait EnvelopeProducer: Send + Sync {
//!     fn name(&self) -> &'static str;
//!     async fn produce(&self, ctx: &ProducerCtx) -> Result<ProducerReport>;
//!     async fn resume_from(&self, ctx: &ProducerCtx, scope: &str)
//!         -> Result<Option<ProducerCheckpoint>>;
//! }
//! ```
//!
//! The producer owns its own batching and emit-to-bus path. After
//! each batch it MUST call
//! `ctx.metadata.lock().await.record_producer_checkpoint(...)` so a
//! `kill -9` mid-walk surfaces the prior batch's cursor on resume.
//! The trait deliberately does NOT model the batching shape — the
//! four current producers paginate differently (filesystem walks vs
//! Synap streams vs in-memory accumulators).
//!
//! Layering:
//! - [`EnvelopeProducer`] (`r#trait.rs`) — the contract.
//! - [`ProducerCtx`] (`ctx.rs`) — shared environment: metadata
//!   handle, reference clock, producer-name string, logger target.
//! - [`ProducerCheckpoint`] (`checkpoint.rs`) — the persisted-cursor
//!   shape.
//! - [`ProducerReport`] / [`ProducerReportView`] (`report.rs`) — the
//!   per-run summary shape and its dashboard projection (ADR-014 /
//!   phase13f §2.4).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod checkpoint;
pub mod ctx;
#[path = "trait.rs"]
pub mod producer_trait;
pub mod report;

pub use checkpoint::ProducerCheckpoint;
pub use ctx::{ProducerCtx, ProducerMetadataHandle};
pub use producer_trait::EnvelopeProducer;
pub use report::{
    ProducerCheckpointView, ProducerCheckpointsReportView, ProducerReport, ProducerReportView,
};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use cortex_storage::MetadataStore;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// A minimal in-test producer that records two batches.
    /// Proves the trait shape compiles, is object-safe, and the
    /// checkpoint-write path drives the SQLite ledger.
    struct ScriptedProducer {
        name: &'static str,
        scope: &'static str,
        batches: Vec<Vec<&'static str>>,
    }

    #[async_trait]
    impl EnvelopeProducer for ScriptedProducer {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
            let mut total = 0u64;
            let mut last_event_id = String::new();
            for (batch_idx, batch) in self.batches.iter().enumerate() {
                for id in batch {
                    last_event_id = (*id).to_string();
                    total += 1;
                }
                let store = ctx.metadata.lock().await;
                // Per-batch `accumulated_at` offset so the
                // composite primary key never collides under a
                // pinned reference clock.
                let accumulated_at =
                    ctx.now + chrono::Duration::milliseconds((batch_idx + 1) as i64);
                store.record_producer_checkpoint(
                    self.name,
                    self.scope,
                    &last_event_id,
                    ctx.now,
                    accumulated_at,
                )?;
            }
            Ok(ProducerReport {
                producer_name: self.name.into(),
                envelopes_emitted: total,
                batches_emitted: self.batches.len() as u64,
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
            let row = store.latest_producer_checkpoint(self.name, scope)?;
            Ok(row.map(ProducerCheckpoint::from_row))
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx() -> (ProducerCtx, ProducerMetadataHandle) {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        let ctx = ProducerCtx::new(handle.clone(), "cortex.producer.test").with_now(fixed_now());
        (ctx, handle)
    }

    #[test]
    fn producer_trait_is_object_safe() {
        let _: Box<dyn EnvelopeProducer> = Box::new(ScriptedProducer {
            name: "test",
            scope: "scope",
            batches: Vec::new(),
        });
    }

    #[test]
    fn producer_ctx_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ProducerCtx>();
    }

    #[tokio::test]
    async fn produce_writes_one_checkpoint_per_batch() {
        let (ctx, handle) = make_ctx();
        let producer = ScriptedProducer {
            name: "bootstrap",
            scope: "cortex",
            batches: vec![vec!["01A", "01B"], vec!["01C"]],
        };
        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.envelopes_emitted, 3);
        assert_eq!(report.batches_emitted, 2);
        assert_eq!(report.last_event_id, "01C");

        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for("bootstrap", 50)
            .unwrap();
        assert_eq!(rows.len(), 2, "one row per batch");
        // newest first
        assert_eq!(rows[0].last_event_id, "01C");
        assert_eq!(rows[1].last_event_id, "01B");
    }

    #[tokio::test]
    async fn resume_from_returns_latest_checkpoint_after_kill() {
        let (ctx, _handle) = make_ctx();
        let producer = ScriptedProducer {
            name: "bootstrap",
            scope: "cortex",
            batches: vec![vec!["01A"], vec!["01B"]],
        };
        let _ = producer.produce(&ctx).await.unwrap();

        let resume = producer
            .resume_from(&ctx, "cortex")
            .await
            .unwrap()
            .expect("checkpoint present");
        assert_eq!(resume.producer_name, "bootstrap");
        assert_eq!(resume.scope, "cortex");
        assert_eq!(resume.last_event_id, "01B");
    }

    #[tokio::test]
    async fn resume_from_returns_none_when_no_checkpoint_exists() {
        let (ctx, _handle) = make_ctx();
        let producer = ScriptedProducer {
            name: "bootstrap",
            scope: "fresh-corpus",
            batches: Vec::new(),
        };
        let resume = producer.resume_from(&ctx, "fresh-corpus").await.unwrap();
        assert!(resume.is_none());
    }
}
