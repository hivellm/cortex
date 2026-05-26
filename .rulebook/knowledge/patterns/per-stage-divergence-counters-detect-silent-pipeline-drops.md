# Per-stage divergence counters detect silent pipeline drops

**Category**: observability
**Tags**: phase8b, metrics, health, divergence, cortex

## Description

Each pipeline stage exports a `last_*_ts_ms` gauge plus per-kind/per-hook counters via `/healthz` extras AND a parallel Prometheus-text `/metrics` endpoint mounted on the same listener. A central aggregator (cortex-api `/v1/health/freshness` + `/v1/health/divergence`) probes the same target list once and pivots adjacent-stage counter pairs to localise silent drops in seconds — instead of relying on per-component liveness.

## Example

// adapter side
metrics.incr_envelopes_built(env.kind.schema_stem());
publisher.publish(env).await;
// publisher.flush_locked → on success → metrics.incr_envelopes_publish_ok(kind)
// /v1/health/divergence pairs adapter.envelopes_built ↔ adapter.envelopes_publish_ok
// → built=100, publish_ok=20 over 60s ⇒ delta_growth=80 ⇒ severity=critical

## When to Use

Multi-stage pipelines where individual `/healthz` returning Ok masks a stalled data flow (e.g. JSON-truncation incident 2026-04-28). Pair every counter that bumps at the upstream side with the symmetric counter at the immediate downstream side; a sustained `delta_growth_60s > 10` is the smoking gun.

## When NOT to Use

Single-stage services (no adjacent boundary to pair). Stages where individual events naturally fan-out / fan-in (per-kind divergence becomes meaningless when the cardinality intentionally shifts between stages).
