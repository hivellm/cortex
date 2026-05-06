# Proposal: phase14h_synap-worker-shared-module

Source: `docs/analysis/rework/glm5.1/findings.md` F-003 (HIGH).

## Why

Synap worker infrastructure is duplicated across `cortex-workers/src/{embedder,fulltext,graph,classifier}/synap_consumer.rs` (~1,500 lines of effectively identical code: consumer-group registration, retry loop, cursor checkpointing, dead-letter handling). Each copy drifts slightly. glm5.1 flags this as the largest single source of in-codebase duplication.

## What Changes

- Extract the shared scaffolding into `crates/cortex-workers/src/synap_worker/` exposing:
  - `pub trait SynapWorker { fn topic(&self) -> &str; fn consumer_group(&self) -> &str; async fn handle(&self, env: Envelope) -> Result<()>; }`
  - `pub async fn run<W: SynapWorker>(worker: W) -> !` driving the shared retry / checkpoint / dead-letter loop.
- Migrate all 4 workers to `impl SynapWorker`. Each module shrinks ~70%.
- Centralise metrics (`cortex_synap_worker_lag`, `cortex_synap_worker_dead_letter_total`).

## Impact

- Affected specs: `docs/specs/00-architecture.md` § Worker architecture.
- Affected code: `crates/cortex-workers/src/synap_worker/` (new), `crates/cortex-workers/src/{embedder,fulltext,graph,classifier}/synap_consumer.rs` (rewrites).
- Breaking change: NO.
- User benefit: worker bug fixed once applies to all 4 consumers; new workers are <100 lines.
