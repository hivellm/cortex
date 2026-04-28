# Proposal: phase8b_pipeline_stage_metrics

## Why

A `/healthz` endpoint (phase8a) tells you each component is alive, but the
2026-04-28 incident proved that "all green" can still mean "no tool_calls
flowing for 6 hours". Each pipeline stage was alive; the data simply wasn't
moving. The user needs per-kind freshness — "when did the last tool_call
land in the archive?" — at every stage, so a stalled pipeline is detected
the moment it stalls instead of hours later.

This is the metric that would have flagged the JSON-truncation bug in
seconds: hooks invoked = N, frames parsed = N - 1000, envelopes published =
N - 1000, archive writes for kind=tool_call = 0. The divergence between
"hooks invoked" and "envelopes published" is the smoking gun.

## What Changes

1. Each pipeline stage emits a per-kind `last_event_ts` and a per-kind
   `events_total` counter, exposed via `/metrics` (Prometheus text) AND
   embedded in the stage's `/healthz` `extras`.

2. Stages instrumented:
   - **adapter.ipc** — `frames_received_total{hook=...}`, `frames_parsed_total{hook=...}`, `last_frame_ts{hook=...}`
   - **adapter.dispatcher** — `envelopes_built_total{kind=...}`, `last_envelope_ts{kind=...}`
   - **adapter.publisher** — `batches_sent_total`, `events_sent_total{kind=...}`, `last_publish_ok_ts`, `wal_bytes`
   - **ingestion.http** — `events_received_total{kind=...}`, `events_archived_total{kind=...}`, `last_archive_write_ts{kind=...}`
   - **cortex-api.archive_loader** — `last_refresh_ts`, `envelopes_seeded_total{kind=...}`
   - **cortex-api.meili_loader** — `last_refresh_ts`, `docs_seeded_total{family=...}`
   - **classifier-worker** — `jobs_processed_total`, `last_job_ts`, `queue_lag`
   - **embedder/fulltext/graph workers** — same shape

3. NEW endpoint `cortex-api /v1/health/freshness` aggregates all
   `last_event_ts` values across stages and returns a flat table:
   ```json
   { "adapter.ipc.PostToolUse": "2026-04-28T20:00:00Z",
     "adapter.publisher.tool_call": "2026-04-28T19:55:00Z",
     "ingestion.archived.tool_call": "2026-04-28T19:55:01Z",
     ... }
   ```
   With a derived `gap_seconds` per row so the GUI can colour-code stale
   ones (>60s = warn, >300s = critical).

4. NEW `cortex-api /v1/health/divergence` flags counters that should
   match but don't. E.g. `adapter.ipc.PostToolUse_count` vs
   `adapter.publisher.tool_call_count` should track within ~2s; a
   sustained 100-event gap means silent drops.

## Impact

- Affected specs: NEW `specs/freshness/spec.md`.
- Affected code:
  - `crates/cortex-adapter-claude-code/src/metrics.rs` (extend)
  - `crates/cortex-adapter-claude-code/src/ipc.rs`, `dispatcher.rs`,
    `publisher.rs` (instrument)
  - `crates/cortex-ingestion/src/metrics.rs`, `archive.rs`, `router.rs`
  - `crates/cortex-api/src/main.rs` + new `health/freshness.rs`
    + new `health/divergence.rs`
  - workers: `crates/cortex-classifier-worker/src/`,
    `crates/cortex-embedder/src/`, `crates/cortex-fulltext/src/`,
    `crates/cortex-graph/src/`
- Breaking change: NO (additive metrics).
- User benefit: stalls are visible within seconds of happening instead of
  hours; divergence between adjacent stages immediately localises which
  stage is dropping events.
