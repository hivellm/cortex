# Proposal: phase8a_health_endpoints_per_crate

## Why

The 2026-04-28 incident took ~2 hours to trace because no component exposed
a self-describing health endpoint. cortex-api was alive on :17000, the GUI
loaded, the timeline rendered turns — but tool_calls were silently missing
and nothing in the system surfaced the gap. The adapter log stopped writing,
the publisher WAL stayed at 0 bytes, ingestion accepted requests, archive
grew (with turns) — every component looked individually healthy.

A uniform `/healthz` per crate plus a `/v1/health` aggregator on cortex-api
gives the operator a single command that says "the stack is degraded
because adapter publisher hasn't published in 5 min" instead of a 2-hour
debug session.

## What Changes

1. NEW crate `cortex-health` with shared types `HealthReport` /
   `SubsystemStatus { name, state: ok|degraded|down, latency_ms,
   last_error, version, since }`.

2. Every long-running binary exposes `GET /healthz` returning its own
   `SubsystemStatus`:
   - `cortex-api` (:17000) — lane snapshot ts, last archive_loader /
     meili_loader refresh.
   - `cortex-adapter-claude-code` — NEW admin port (default :17011) with
     publisher queue depth, WAL bytes, last successful publish ts,
     ipc pipe alive.
   - `cortex-ingestion` (:17010) — `/v1/healthz` with archive root
     writability, synap room registered, last batch accepted ts.
   - workers (classifier/embedder/fulltext/graph) — `/healthz` reporting
     last claimed job, queue lag.

3. cortex-api aggregator `GET /v1/health` fans out to every `/healthz`
   in parallel (1.5s budget per probe), caches 2s, returns
   `{ overall, subsystems[], checked_at }`.

4. CLI / script: `scripts/health.bat` hits `/v1/health`, renders a table,
   exits non-zero on any `down`.

## Impact

- Affected specs: NEW `.rulebook/tasks/phase8a_health_endpoints_per_crate/specs/health/spec.md`.
- Affected code:
  - NEW `crates/cortex-health/`
  - `crates/cortex-api/src/main.rs` + `dashboard.rs` (aggregator route)
  - `crates/cortex-adapter-claude-code/src/main.rs` (admin HTTP listener)
  - `crates/cortex-ingestion/src/main.rs`
  - `crates/cortex-classifier-worker/src/main.rs`
  - `crates/cortex-embedder-worker/src/main.rs`
  - `crates/cortex-fulltext-worker/src/main.rs`
  - `crates/cortex-graph-worker/src/main.rs`
  - NEW `scripts/health.bat`
- Breaking change: NO (additive).
- User benefit: one command tells the user whether the stack is healthy
  in <2s, replacing the 2-hour investigation loop.
