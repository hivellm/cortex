## 1. Workspace scaffold
- [ ] 1.1 Root `Cargo.toml` as virtual workspace; members include `cortex-core`
- [ ] 1.2 `cortex-core/Cargo.toml` with axum, tokio, serde, tracing, opentelemetry, parquet, reqwest
- [ ] 1.3 `lib.rs` re-exports + `main.rs` binary target `cortex-core-server`

## 2. Redactor
- [ ] 2.1 Pattern catalog v1 under `cortex-core/src/redact/patterns.rs` (aws keys, github tokens, api keys, JWT, private keys, `.env=VALUE`, common PII)
- [ ] 2.2 Redaction engine operating on `serde_json::Value` (visits strings; replaces with `[REDACTED:<pattern_class>]`)
- [ ] 2.3 Per-pattern unit tests with positive + negative samples

## 3. Ingestion router
- [ ] 3.1 Axum app with `POST /v1/events`, `POST /v1/events/batch`, `GET /healthz`, `GET /metrics`
- [ ] 3.2 Request validation against generated schemas from phase0_event-schema
- [ ] 3.3 Route to `cortex.events.raw` or `cortex.events.bootstrap` based on `X-Cortex-Stream` header
- [ ] 3.4 Write durable Parquet record before ack (archive-first semantics)
- [ ] 3.5 ULID generation on-missing for `event_id`; accept caller-supplied IDs otherwise

## 4. Synap publisher
- [ ] 4.1 Reusable Synap client wrapper with retry + exponential backoff
- [ ] 4.2 Fire event to declared stream; bounded in-memory buffer as overflow guard

## 5. Telemetry
- [ ] 5.1 Counters: `cortex.core.events.received`, `cortex.core.events.redacted_fields`, `cortex.core.router.errors`
- [ ] 5.2 Histogram: `cortex.core.router.latency_ms`
- [ ] 5.3 `/metrics` endpoint exposes Prometheus text format

## 6. Tail (mandatory)
- [ ] 6.1 Update `docs/specs/04-cortex-core.md` status flag to 🟢 + index row
- [ ] 6.2 Integration tests: POST envelope → observe on Synap + Parquet; redactor unit tests; validator rejection tests
- [ ] 6.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
