//! The [`EnvelopeProducer`] trait — the single contract every
//! envelope source implements.
//!
//! Defined in its own file per ADR-010 §2.1; the parent module
//! `producer/mod.rs` mounts it via `#[path = "trait.rs"] pub mod
//! producer_trait;` because `trait` is a reserved keyword.

use async_trait::async_trait;

use super::{ProducerCheckpoint, ProducerCtx, ProducerReport};

/// The single contract every envelope-producing subsystem
/// implements. Phase13b §2.2.
///
/// Invariants every implementor SHALL uphold:
///
/// 1. `name()` returns a stable `'static` identifier used as the
///    `producer_checkpoints.producer_name` key. Two producers
///    MUST NOT share a name.
/// 2. `produce(ctx)` is responsible for batching its own emit
///    stream. At the end of every emit batch the implementation
///    MUST call
///    `ctx.metadata.lock().await.record_producer_checkpoint(...)`
///    so a `kill -9` mid-walk surfaces the prior batch's cursor on
///    resume. The trait does not enforce batch size — bootstrap
///    uses 256, claude-archive uses per-file boundaries, etc.
/// 3. `resume_from(ctx, scope)` reads the latest `(producer_name,
///    scope)` checkpoint. Producers consult this on entry and
///    skip envelopes whose `event_id` is older than the persisted
///    cursor.
/// 4. The implementation MUST be idempotent under crash-replay.
///    Reading the checkpoint, emitting a batch, then writing the
///    new checkpoint MUST converge to the same `event_id` set
///    across runs.
///
/// Object-safety: the trait MUST stay object-safe so a registry
/// can own a `Vec<Box<dyn EnvelopeProducer>>` (phase14 supervisor).
/// The unit test `producer_trait_is_object_safe` in
/// `producer/mod.rs` is the regression guard.
#[async_trait]
pub trait EnvelopeProducer: Send + Sync {
    /// Stable `'static` identifier (`bootstrap`, `claude_archive`,
    /// `topic_cards`, `consolidator`, plus future adapters).
    fn name(&self) -> &'static str;

    /// Run one production pass. The implementation owns batching,
    /// emit-to-bus, and the per-batch
    /// `record_producer_checkpoint` write. Returns the per-run
    /// `ProducerReport` summary the supervisor logs / persists.
    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport>;

    /// Read the latest checkpoint for `(self.name(), scope)`.
    /// Returns `None` when no checkpoint exists (fresh-corpus
    /// walk).
    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> anyhow::Result<Option<ProducerCheckpoint>>;
}
