# cortex-health

Shared health-report types + server + aggregator helpers for the
Cortex stack. Lives in its own crate so per-binary `/healthz`
handlers and the `cortex-api` `/v1/health` aggregator speak the
same wire shape without dragging in cortex-api's full dependency
surface.

Phase8a of the Cortex relevance / observability roadmap. Closes
the 2026-04-28 incident class where every component looked
individually healthy but the stack was silently degraded.

## Wire shape

```jsonc
// SubsystemStatus (one row in the report)
{
  "name": "cortex-api",
  "state": "ok",            // ok | degraded | down
  "latency_ms": 0,
  "last_error": null,        // string when state != ok
  "version": "0.1.0",
  "since": "2026-04-29T00:00:00Z",
  "extras": { "uptime_ms": 12345 }   // free-form per-subsystem signals
}

// HealthReport (what /v1/health returns)
{
  "overall": "degraded",      // worst state across subsystems
  "subsystems": [ /* SubsystemStatus rows, sorted by name */ ],
  "checked_at": "2026-04-29T00:00:01Z"
}
```

Aggregation rule: `Down` wins over `Degraded`, which wins over
`Ok`. Empty `subsystems` → `Ok`.

## Features

| Feature   | What it pulls in        | Who uses it |
|-----------|--------------------------|-------------|
| (default) | serde + chrono types only | every crate that consumes the wire shape |
| `server`  | axum + tokio              | every long-running binary that exposes `/healthz` (workers, adapter) |
| `client`  | reqwest + tokio           | the cortex-api aggregator at `/v1/health` |

## Endpoints

| Subsystem                  | Default port | Path         | Selector env                         |
|----------------------------|--------------|--------------|--------------------------------------|
| `cortex-api`               | 17000        | `/healthz`   | (built into the api router)          |
| `cortex-api` aggregator    | 17000        | `/v1/health` | (built into the api router)          |
| `cortex-ingestion`         | 17010        | `/v1/healthz`| (built into the ingestion router)    |
| `cortex-adapter-claude`    | 17011        | `/healthz`   | `CORTEX_ADAPTER_ADMIN_PORT`          |
| `cortex-classifier-worker` | 17021        | `/healthz`   | `CORTEX_CLASSIFIER_HEALTH_PORT`      |
| `cortex-embedder-worker`   | 17022        | `/healthz`   | `CORTEX_EMBEDDER_HEALTH_PORT`        |
| `cortex-fulltext-worker`   | 17023        | `/healthz`   | `CORTEX_FULLTEXT_HEALTH_PORT`        |
| `cortex-graph-worker`      | 17024        | `/healthz`   | `CORTEX_GRAPH_HEALTH_PORT`           |

## Aggregator URL overrides

The aggregator on `cortex-api` discovers each subsystem via env
vars (default = the localhost addresses above). Set these to
remote hosts on multi-machine deployments:

- `CORTEX_ADAPTER_ADMIN_URL`
- `CORTEX_INGESTION_URL`
- `CORTEX_CLASSIFIER_WORKER_URL`
- `CORTEX_EMBEDDER_WORKER_URL`
- `CORTEX_FULLTEXT_WORKER_URL`
- `CORTEX_GRAPH_WORKER_URL`

Per-probe budget: 1.5s. Failed probes mark the subsystem `Down`
with a `last_error` reason — they NEVER fail the aggregator call,
so a single dead worker doesn't take down the whole report.

## Operator scripts

```sh
# Bash / WSL
scripts/health.sh
# Windows
scripts\health.bat
```

Both pretty-print the report and exit `0` (ok) / `1` (degraded) /
`2` (down) / `3` (could not reach the aggregator). Wire into your
CI smoke job to catch silent stack degradation in <2s.
