# cortex-ingestion

> Spec: [`docs/specs/04-cortex-core.md`](../../docs/specs/04-cortex-core.md)

The Cortex ingestion service. Accepts envelope-compliant events over HTTP,
runs a defense-in-depth redaction pass, archives a durable record to disk,
and publishes the event onto the appropriate Synap stream.

This is the **front door** of Cortex. Adapters (Claude Code, Cursor,
Codex, Gemini, …) and the bootstrap CLI are the only legitimate clients.

## HTTP API

Default bind: `0.0.0.0:8081`. All routes accept and return JSON.

| Method | Path                  | Purpose                                  |
|--------|-----------------------|------------------------------------------|
| POST   | `/v1/events`          | Submit a single envelope.                |
| POST   | `/v1/events/batch`    | Submit `{ "events": [ ... ] }`.          |
| GET    | `/healthz`            | Liveness probe.                          |
| GET    | `/metrics`            | Prometheus metrics.                      |

Every accepted event is:

1. **Validated** against the JSON Schemas in `cortex-core`.
2. **Redacted** with `cortex_core::redact` before any payload is persisted.
3. **Archived** as zstd-compressed NDJSON under the configured archive root.
4. **Published** to `cortex.events.raw` (live) or `cortex.events.bootstrap`
   depending on the envelope's `source.mode`.

Rejection is fail-closed: if redaction errors, schema validation fails, or
the archive write fails, the event is **not** published and the call
returns a 4xx/5xx with a structured error body.

## Configuration

All config keys can be set via environment variables (`CORTEX_INGEST_*`)
or a TOML file passed with `--config`.

| Variable                            | Default                       | Notes                                |
|-------------------------------------|-------------------------------|--------------------------------------|
| `CORTEX_INGEST_BIND`                | `0.0.0.0:8081`                | HTTP listen address.                 |
| `CORTEX_INGEST_ARCHIVE_DIR`         | `./data/archive`              | Root of the NDJSON+zstd archive.     |
| `CORTEX_INGEST_ARCHIVE_ROTATE_MB`   | `64`                          | Per-file rotation threshold.         |
| `CORTEX_INGEST_SYNAP_URL`           | `http://localhost:18443`      | Synap base URL.                      |
| `CORTEX_INGEST_LIVE_STREAM`         | `cortex.events.raw`           |                                      |
| `CORTEX_INGEST_BOOTSTRAP_STREAM`    | `cortex.events.bootstrap`     |                                      |
| `RUST_LOG`                          | `info`                        | Standard `tracing-subscriber` env.   |

## Run locally

```bash
docker compose up -d synap                                # ensure Synap is reachable
cargo run -p cortex-ingestion --release
```

Smoke test:

```bash
curl -s -X POST http://localhost:8081/v1/events \
  -H 'content-type: application/json' \
  --data @docs/specs/fixtures/sample-turn.json | jq
```

## Library

```toml
[dependencies]
cortex-ingestion = { path = "../cortex-ingestion" }
```

The crate exposes `build_router(state)` so embedders (tests, alternative
front-ends, Tauri shells) can mount the ingestion routes inside a larger
Axum application.

```rust
use cortex_ingestion::{build_router, AppState};

let state = AppState::from_env().await?;
let app   = build_router(state);
axum::serve(listener, app).await?;
```

## Testing

```bash
cargo test -p cortex-ingestion
```

The integration tests spin up the router with `MemoryPublisher` and exercise
the full validate → redact → archive → publish pipeline.
