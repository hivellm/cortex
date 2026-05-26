# Silent-drop watcher: push channel for divergence alerts

**Category**: observability
**Tags**: phase8e, watcher, alerts, silent-drop, cortex

## Description

A background watcher polls the same divergence rows the `/v1/health/divergence` endpoint surfaces, runs each pair through a debounced state machine (Ok→Warn requires 2 consecutive over-threshold polls; Warn→Critical fires on 1; recovery requires 5 non-growing polls), and emits a `law_violation` envelope on every transition. The envelope lands in BOTH the durable archive (POSTed to ingestion) and the in-memory keyword lane (immediate Live Timeline visibility). Per-pair state persists to disk for restart-safe dedup; optional webhook + handoff escalation gated by config.

## Example

// transition()/step() are pure; tick() handles the side-effects:
let rows = compute_divergence_rows(aggregator).await;
for (row, severity) in watcher.step(&rows) {
    lane.seed(index, vec![alert_lane_hit(&row, severity, now)]);
    post_envelope_to_ingestion(&client, url, &build_alert_envelope(&row, severity)).await;
}

## When to Use

When a `/v1/health/divergence`-style aggregator already exposes per-pair counter deltas but operators only see them by manually polling. Wire the watcher to push high-severity transitions into the existing event lane so dashboards / log streams surface them automatically. The pattern generalises to any "the data is there but nobody looks at it" observability gap.

## When NOT to Use

When you don't have a per-pair counter model (the watcher's value is the debounced state machine, which needs a stable scalar to compare against). Also: don't fire alerts on every poll above threshold — debounce or you'll flood the lane on normal traffic spikes.
