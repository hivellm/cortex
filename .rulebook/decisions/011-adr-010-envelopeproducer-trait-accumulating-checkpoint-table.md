# 11. ADR-010 — EnvelopeProducer trait + accumulating checkpoint table

**Status**: proposed
**Date**: 2026-05-19
**Related Tasks**: phase13b_envelope-producer-trait-adr-010, phase13a_sweep-trait-adr-009, phase16a_opencode-adapter-via-envelope-producer, phase16b_cursor-adapter, phase16c_codex-adapter, phase16d_gemini-adapter

## Context

Bootstrap (`crates/cortex-cli/src/bootstrap/walker.rs`), claude-archive ingest (`crates/cortex-workers/src/claude_archive/`), topic-cards emit (`crates/cortex-workers/src/topic_cards/producer.rs`), and consolidator emit (`crates/cortex-workers/src/consolidator/producer/`) each construct envelopes ad-hoc. Every producer reimplements:

- A loop that walks a source surface (filesystem, Synap stream, in-memory accumulator).
- A producer-specific checkpoint file or SQLite row that records "where we were when we last paused".
- Resume logic that reads back the checkpoint on next boot.

The 4-doc analysis (`docs/analysis/rework/04-architecture.md` §A.2) names this the second-largest abstraction debt after `Sweep`:

- `bootstrap` overwrites `.cortex-bootstrap.state.json` on every invocation — no multi-repo accumulation. The single-repo singleton has only ever tracked "the last repo I walked", not "what's the org's coverage state".
- `claude-archive/checkpoint.rs` keeps its own per-project cursor on disk.
- `topic-cards/producer.rs` re-emits the entire ladder on every tick because it has no persistence between runs.
- `consolidator/producer/*` keeps a per-grain cursor file at `~/.cortex/consolidator-cursor.json`.

The four cursor stores cannot answer "what envelopes have any producer ever emitted?" — a basic operator question that today requires reading four file formats.

Every new adapter the Phase C plan calls for (OpenCode, Cursor, Codex, Gemini) will reimplement the same shape unless the contract is uniform.

Reference: `docs/analysis/rework/04-architecture.md` §A.2; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.2. Builds on ADR-009 (the `Sweep` trait sibling).

## Decision

Introduce `cortex_workers::producer::EnvelopeProducer` as the single trait every envelope source implements.

```rust
#[async_trait]
pub trait EnvelopeProducer: Send + Sync {
    fn name(&self) -> &'static str;
    async fn produce(
        &self,
        ctx: &ProducerCtx,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<Envelope>>>;
    async fn checkpoint(
        &self,
        ctx: &ProducerCtx,
    ) -> anyhow::Result<ProducerCheckpoint>;
}
```

Supporting types:
- `ProducerCtx` — handles for `MetadataStore`, optional Synap / Meili / Nexus / Vectorizer clients, reference clock, producer-name `&'static str`.
- `ProducerCheckpoint { producer_name, scope, last_event_id, last_occurred_at, accumulated_at }`.
- `ProducerScope` — `&str` newtype keyed per (producer, sub-source). Bootstrap uses `repo_slug` as scope; consolidator uses `grain` (`session` / `topic` / `decision`); claude-archive uses `project_path`; topic-cards uses `topic_slug`.

New SQLite table `producer_checkpoints { producer_name TEXT, scope TEXT, last_event_id TEXT, last_occurred_at TEXT, accumulated_at TEXT, PRIMARY KEY (producer_name, scope, accumulated_at) }`. Append-only; the primary key includes `accumulated_at` so two invocations never collide. `latest_checkpoint(name, scope)` returns the row with the maximum `accumulated_at`.

Schema migration: `cortex_storage::metadata::apply_phase13b_schema(&conn)` creates the table idempotently.

The four existing producers migrate to `impl EnvelopeProducer`. Each `produce` invocation calls `record_checkpoint(producer_name, scope, last_event_id, last_occurred_at)` at the end of every emit batch (not just on graceful shutdown). Bootstrap, the canonical user, must survive `kill -9` and resume from the most recent checkpoint without re-emitting earlier envelopes — see the §4 IT.

The legacy file-based checkpoint stores (`.cortex-bootstrap.state.json`, `~/.cortex/consolidator-cursor.json`, claude-archive's per-project files) remain readable for one release as a compatibility bridge; the trait writes to `producer_checkpoints` going forward. Phase 14 retires the legacy stores.

Status: `proposed`. Promote to `accepted` once §3.1 (bootstrap migration) lands and the resume-after-kill IT in §4 is green.

## Alternatives Considered

- Keep ad-hoc per-producer checkpoint stores and add a thin adapter layer that copies their contents into a shared SQLite table on every tick — rejected: solves the operator-query problem but not the checkpoint-overwrite bug in bootstrap. The bug lives in the per-producer write logic; an adapter that mirrors it carries the same bug.
- Define EnvelopeProducer at the cortex-core layer so non-worker crates can implement it — deferred: only the four worker-style producers need it today. Pulling the trait into cortex-core would force every consumer crate to grow a futures-stream dep. Promote when the fifth producer lands.
- Use a single global cursor (`max(last_event_id) across all producers`) instead of per-(producer, scope) cursors — rejected: bootstrap and claude-archive can run concurrently against different sources; a global cursor would force serialisation. Per-scope cursors preserve parallelism.
- Append-only table vs upsert-by-PK — chose append-only with a composite PK on `(producer_name, scope, accumulated_at)` so the audit trail survives kill-restart. Upsert would lose the prior cursor on resume, which is the exact bug the bootstrap state-file already suffers from.

## Consequences

**Cost** (~3 days × 4 producers):
- Each producer migrates to the trait. The migrations are mechanical (loop body shape is unchanged; the checkpoint-write call is the only mandatory addition).
- One new SQLite table + helpers (`record_checkpoint`, `latest_checkpoint`, `list_checkpoints_for`) plus the schema migration.
- Trait surface in `cortex-workers::producer` plus a `BoxStream<Envelope>` return type pull in the `futures` crate boundary (already a transitive dep).
- One extra SQLite write per emit batch. Negligible: bootstrap batches are ~256 envelopes; one write per batch is ~thousands of writes per repo walk, the same order as today's bookkeeping.

**Gain** (kill-resume + adapter onboarding):
- Bootstrap survives `kill -9` and resumes from the last checkpointed `event_id` — no duplicates, no gaps. The 30%-of-corpus then SIGKILL IT in §4 is the load-bearing gate.
- Every new adapter (OpenCode, Cursor, Codex, Gemini — see phase16a–phase16d) ships as `impl EnvelopeProducer` in <1 day. The per-adapter cost drops from "build a checkpoint store, build a resumer, build a CLI" to "write the produce body".
- One operator-facing surface: `SELECT * FROM producer_checkpoints WHERE producer_name=?` answers "where did claude-archive stop?" / "what topics did topic-cards emit last week?" without four file-format readers.
- Sets up ADR-014 (dashboard as pure reader): producer state becomes a `retention_sweeps`-style queryable table.

**Risk** (medium): the chosen abstraction may need a `Stream<Envelope>` variant that the four current producers cannot satisfy cleanly. Mitigation: the trait returns `BoxStream<Result<Envelope>>` so producers can stream lazily or buffer + replay; the `produce` method is async so file walks can chunk naturally. Trait deliberately omits backpressure / rate-limit hooks until a concrete consumer asks for them.

**Reversibility**: reversible. If the trait proves wrong, the producers can return to ad-hoc impls without a data migration (the `producer_checkpoints` table is append-only audit and remains valid as a historical log).
