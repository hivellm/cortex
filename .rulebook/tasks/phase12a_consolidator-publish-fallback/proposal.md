# Proposal: phase12a_consolidator-publish-fallback

Source: `docs/analysis/rework/01-consolidation.md` Achado 2 (P0 data-loss); `docs/analysis/rework/opus5.7/03-recommendation.md` patch #1.

## Why

`crates/cortex-workers/src/consolidator/publisher.rs::publish_consolidation()` POSTs envelopes to `http://127.0.0.1:17010` (default `CORTEX_INGESTION_URL`). When that env var is unset in production — the current state — every consolidation envelope is silently discarded with no log, no metric, no fallback. The user reports "consolidação não funciona" and 4 independent rework analyses (4-doc set + glm5.1 + minmax2.7 + opus5.7) confirm this is the largest single source of data loss in the pipeline.

## What Changes

- Add ERROR-level `tracing::error!` on every failure path of `publish_consolidation()` (network error, non-2xx response, missing env var) with the envelope's `event_id` and `query_id` in the structured fields.
- Add a JSONL fallback: when the POST fails OR `CORTEX_INGESTION_URL` is unset, append the envelope to `${CORTEX_HOME}/consolidations.jsonl` so it is at least recoverable post-hoc.
- Add a Prometheus counter `cortex_consolidator_publish_failures_total{reason}` with reasons `env_unset`, `network`, `non_2xx`, `serialise`.
- Add `cortex-ops consolidations-replay --from .cortex/consolidations.jsonl` so the operator can replay the recovered envelopes once `CORTEX_INGESTION_URL` is set.

## Impact

- Affected specs: `docs/specs/12-consolidator.md` § Publishing (new fallback contract).
- Affected code: `crates/cortex-workers/src/consolidator/publisher.rs`, `crates/cortex-workers/src/consolidator/metrics.rs` (new counter), `crates/cortex-cli/src/bin/cortex-ops.rs` (replay subcommand).
- Breaking change: NO. Additive logs + fallback path.
- User benefit: zero envelope data-loss; visible failure mode replaces silent one.
