# Per-binary /healthz + central aggregator with feature-gated server crate

**Category**: observability
**Tags**: phase8a, observability, health, aggregator, feature-gating

## Description

For multi-process backends where "is the stack healthy?" used to take 2 hours of grep, ship a tiny dedicated `health` crate with three feature gates: default (just serde wire types — every consumer pulls these), `server` (axum + a one-call `serve_standalone(port, name, version, since, provider)` helper for binaries that don't already own a router), and `client` (reqwest + a `JoinSet`-based parallel `aggregate(targets)` helper). Each long-running binary spawns the listener with a closure that reads its live metric registry per probe; the central aggregator (cortex-api here) discovers targets from env vars (with localhost defaults), fans out with a 1.5s per-probe timeout, and stamps every failed probe as `Down` with a `last_error` reason rather than failing the whole call. Aggregation rule: `Down` > `Degraded` > `Ok`. Operator scripts (`scripts/health.{sh,bat}`) exit 0/1/2/3 mapped to the report state. The pattern collapses a multi-hour incident-trace loop ("everything looks individually healthy but the stack is silently degraded") into a sub-2s probe.

## Example

// Worker bin (one-line spawn):
spawn_health_listener(
    resolve_port_from_env("CORTEX_X_HEALTH_PORT", DEFAULT_X_PORT),
    "cortex-x-worker",
    env!("CARGO_PKG_VERSION"),
    Arc::new(move || HealthSnapshot {
        state: rules::freshness_state(metrics.last_activity_ms(), None).0,
        last_error: rules::freshness_state(metrics.last_activity_ms(), None).1,
        extras: serde_json::Map::from_iter([
            ("queue_lag".into(), serde_json::json!(metrics.queue_lag())),
        ]),
    }),
);

// Aggregator (cortex-api):
let report = aggregate(&client, &targets, &AggregatorConfig::default()).await;
// → HealthReport { overall, subsystems[], checked_at }

## When to Use

Multi-process / multi-binary stacks where each binary owns its own runtime + metrics and silent degradation is a recurring incident class. Especially valuable when the binaries already differ in axum/reqwest dependency footprints — the feature gate keeps the lightweight ones lightweight.
