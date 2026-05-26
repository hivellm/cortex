# Proposal: phase13b_envelope-producer-trait-adr-010

Source: `docs/analysis/rework/04-architecture.md` §A.2; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.2.

## Why

Bootstrap, claude-archive ingest, topic-cards emit, and consolidator emit each construct envelopes ad-hoc. There is no common interface for "I am a thing that produces envelopes" — so checkpointing is ad-hoc, kill-resume is broken (bootstrap overwrites its checkpoint), and every new adapter (OpenCode/Cursor/Codex/Gemini) duplicates the construction logic. The 4-doc analysis names this the second largest abstraction debt after `Sweep`.

## What Changes

- New ADR-010 — "`EnvelopeProducer` trait + accumulating checkpoint table".
- New trait `cortex_workers::producer::EnvelopeProducer`:
  ```rust
  #[async_trait]
  pub trait EnvelopeProducer: Send + Sync {
      fn name(&self) -> &'static str;
      async fn produce(&self, ctx: ProducerCtx) -> Result<BoxStream<'static, Envelope>>;
      async fn checkpoint(&self, ctx: ProducerCtx) -> Result<ProducerCheckpoint>;
  }
  ```
- New SQLite table `producer_checkpoints { producer_name, scope, last_event_id, last_occurred_at, accumulated_at }` — append-only, never overwritten.
- Migrate `bootstrap`, `claude-archive`, `topic-cards-emit`, `consolidator-emit` to the trait.
- `kill -9` then resume IT proves bootstrap continues from the checkpoint instead of restarting.

## Impact

- Affected specs: `docs/specs/03-bootstrap.md` § Resume contract; new `docs/specs/24-producer-trait.md`.
- Affected code: `crates/cortex-workers/src/producer/{trait.rs,ctx.rs,checkpoint.rs}` (new), `crates/cortex-storage/src/metadata.rs` (new table), `crates/cortex-cli/src/bootstrap/walker.rs` (migrate), `crates/cortex-workers/src/{topic_cards,consolidator}/*.rs` (migrate).
- Breaking change: NO at the envelope wire format; INTERNAL refactor.
- User benefit: bootstrap survives kill; every new adapter ships as `impl EnvelopeProducer` in <1 day.
