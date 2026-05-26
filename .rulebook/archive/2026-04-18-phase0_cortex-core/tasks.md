## 1. Workspace scaffold
- [x] 1.1 Root `Cargo.toml` already a virtual workspace; `cortex-ingestion` added as a dedicated crate so heavy deps (axum / tokio / synap-sdk) do not leak into `cortex-core` consumers
- [x] 1.2 `cortex-ingestion/Cargo.toml` with axum, tokio, tracing, async-trait, reqwest, zstd, synap-sdk (path dep)
- [x] 1.3 `lib.rs` re-exports + `main.rs` binary target `cortex-ingestion`

## 2. Redactor (in cortex-core)
- [x] 2.1 Pattern catalog v1 under `cortex-core/src/redact.rs` (aws access keys, aws secret, github, slack, openai, anthropic, google, stripe live, bearer, private-key PEM, JWT, generic env secrets)
- [x] 2.2 Redaction engine walks `serde_json::Value`; replaces with `[REDACTED:<class>]` and emits `secret:<class>:<path>:offset=:length=` tokens
- [x] 2.3 Per-pattern unit tests + idempotence + nested-walk coverage (8 tests passing)

## 3. Ingestion router
- [x] 3.1 Axum app with `POST /v1/events`, `POST /v1/events/batch`, `GET /healthz`, `GET /metrics`
- [x] 3.2 Request validation via `cortex_core::validate_event` after redaction
- [x] 3.3 Route to `cortex.events.raw` or `cortex.events.bootstrap` based on `X-Cortex-Stream` header (fallback to envelope.stream)
- [x] 3.4 Durable archive write (NdJsonZstdArchive) before publishing — archive-first semantics
- [x] 3.5 `event_id` + `ingested_at` stamped server-side when absent

## 4. Synap publisher
- [x] 4.1 `Publisher` trait with `MemoryPublisher` + `SynapPublisher` impls
- [x] 4.2 `SynapPublisher` wraps `synap-sdk`'s `StreamManager::publish`; falls back to `MemoryPublisher` when `SYNAP_URL` is unset (dev ergonomics)
- [x] 4.3 HTTP handlers surface publisher errors with structured JSON body + metrics counter

## 5. Telemetry
- [x] 5.1 Counters: `cortex_events_received`, `cortex_events_rejected`, `cortex_events_routed{stream=raw|bootstrap}`, `cortex_redaction_hits`, `cortex_publisher_errors`, `cortex_archive_errors`
- [x] 5.2 `/metrics` endpoint emits Prometheus text format

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation (spec 04 flipped to 🟢 in [docs/specs/00-index.md](../../../docs/specs/00-index.md) and [04-cortex-core.md](../../../docs/specs/04-cortex-core.md))
- [x] 6.2 Write tests covering the new behavior (8 redactor tests in cortex-core + 12 ingestion tests: healthz, accept, batch, bootstrap header routing, invalid rejection, redaction leak, /metrics, archive separation, NDJSON round-trip, memory publisher)
- [x] 6.3 Run tests and confirm they pass (`cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — 80 tests pass, 0 warnings)
