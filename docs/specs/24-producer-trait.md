# Spec 24 — `EnvelopeProducer` trait + accumulating checkpoint

**Status:** 🟢 Phase13b shipped (ADR-010 accepted 2026-05-19).

## Why

Bootstrap, claude-archive ingest, topic-cards emit, and consolidator
emit each constructed envelopes ad-hoc. Every producer carried its
own checkpoint store (`.cortex-bootstrap.state.json` for bootstrap,
per-project files for claude-archive, no persistence at all for
topic-cards, `~/.cortex/consolidator-cursor.json` for the
consolidator), and every new adapter (OpenCode, Cursor, Codex,
Gemini) duplicated the same shape. The 4-doc rework analysis
(`docs/analysis/rework/04-architecture.md` §A.2) named this the
second-largest abstraction debt after `Sweep`. Bootstrap's
`.cortex-bootstrap.state.json` was overwritten on every invocation
— multi-repo accumulation was structurally impossible.

## What

Single trait in `crates/cortex-workers/src/producer/`:

```rust
#[async_trait]
pub trait EnvelopeProducer: Send + Sync {
    fn name(&self) -> &'static str;
    async fn produce(&self, ctx: &ProducerCtx) -> Result<ProducerReport>;
    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> Result<Option<ProducerCheckpoint>>;
}
```

Layered support types:

- `ProducerCtx` — shared environment (metadata handle, reference
  clock, logger target). Per-backend handles (Synap, Meili,
  Nexus, Vectorizer) live on each `impl EnvelopeProducer` struct.
- `ProducerCheckpoint { producer_name, scope, last_event_id,
  last_occurred_at, accumulated_at }` — per-(producer, scope)
  cursor row, append-only.
- `ProducerReport { producer_name, envelopes_emitted,
  batches_emitted, last_event_id, last_occurred_at }` — per-run
  summary the supervisor logs / persists.

## SQLite schema

New table in
`crates/cortex-storage/schemas/sqlite/schema.sql` +
`apply_phase13b_schema`:

```sql
CREATE TABLE IF NOT EXISTS producer_checkpoints (
    producer_name    TEXT NOT NULL,
    scope            TEXT NOT NULL,
    last_event_id    TEXT NOT NULL,
    last_occurred_at TEXT NOT NULL,
    accumulated_at   TEXT NOT NULL,
    PRIMARY KEY (producer_name, scope, accumulated_at)
);

CREATE INDEX IF NOT EXISTS producer_checkpoints_latest
    ON producer_checkpoints (producer_name, scope, accumulated_at DESC);
```

Append-only — every emit batch from every producer writes one
row. The composite primary key includes `accumulated_at` so two
invocations never collide; resume reads
`latest_producer_checkpoint(name, scope)` which returns the row
with the maximum `accumulated_at`.

Helper API on `MetadataStore`:

- `record_producer_checkpoint(name, scope, last_event_id,
  last_occurred_at, accumulated_at) -> Result<()>`.
- `latest_producer_checkpoint(name, scope) ->
  Result<Option<ProducerCheckpointRow>>`.
- `list_producer_checkpoints_for(name, limit) ->
  Result<Vec<ProducerCheckpointRow>>` — newest first.

## Scope policy

| Producer | Scope | Cursor token (`last_event_id`) |
|---|---|---|
| `bootstrap` | `repo_id` (lowercase) | last walked file (relative path) |
| `claude_archive` | `project_dir` or `__sidecars__` | last jsonl file path |
| `topic_cards` | `topic_slug` | rewritten card's `topic_slug` |
| `consolidator` | `session_id` \| `topic:<label>` \| `decision:<event_id>` | `consolidation_id` of the produced envelope |

Phase14 promotes per-emit envelope ids into `last_event_id`
uniformly; today the cursor tokens are the producer-specific
identifiers above.

## Migrated producers

- `crates/cortex-cli/src/bootstrap/producer.rs` —
  `BootstrapProducer` wraps `run_repo_with_dedup`.
- `crates/cortex-workers/src/claude_archive/producer_trait.rs` —
  `ClaudeArchiveProducer` wraps the walker, one row per
  `project_dir`.
- `crates/cortex-workers/src/topic_cards/producer_trait.rs` —
  `TopicCardsProducer` wraps `Orchestrator::run`.
- `crates/cortex-workers/src/consolidator/producer_trait.rs` —
  `ConsolidatorProducer` walks the three grains
  (`run_session`, `run_topic`, `run_decision_trace`).

## Resume-after-kill contract

The load-bearing IT lives at
`crates/cortex-workers/tests/producer_resume_after_kill_it.rs`. A
synthetic producer walks a 10 000-event corpus, panics after 30%
(simulating `kill -9` between the final pre-kill checkpoint and
the next batch), and a fresh producer instance reads the cursor
back to finish the corpus with:

- **No duplicates** — verified via `BTreeSet` over the union of
  pre-kill + post-resume emits.
- **No gaps** — final emit count equals the input corpus exactly.
- **Forward cursor** — the final `producer_checkpoints` row sits
  at `corpus.last()`.

ADR-010 promotion to `accepted` was gated on this IT.

## Legacy file stores

The legacy per-producer checkpoint files (`.cortex-bootstrap.state.
json`, `~/.cortex/consolidator-cursor.json`, claude-archive's
per-project files) remain readable for one release as a
compatibility bridge while phase14 retires them. The trait writes
to `producer_checkpoints` going forward; resume reads from the
SQLite table only.

## Operator query surface

`SELECT * FROM producer_checkpoints WHERE producer_name = ?` is
the single audit query operators run for any producer. Replaces
four ad-hoc file formats.
