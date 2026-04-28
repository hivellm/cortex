## 1. Adapter instrumentation
- [ ] 1.1 Extend `crates/cortex-adapter-claude-code/src/metrics.rs` with `frames_received_total: AtomicU64` per hook kind (HashMap behind Mutex)
- [ ] 1.2 Add `frames_parsed_total` and `frames_parse_error_total` counters
- [ ] 1.3 Add `envelopes_built_total{kind}`, `envelopes_publish_ok_total{kind}`, `envelopes_publish_fail_total{kind}`
- [ ] 1.4 Stamp `last_frame_ts{hook}` (RwLock<HashMap<HookKind, Instant>>) updated by `ipc::handle_pipe`/`handle_unix`
- [ ] 1.5 Stamp `last_publish_ok_ts` updated by `publisher::flush_locked` after successful POST
- [ ] 1.6 Wire metrics into `/healthz` extras (depends on phase8a)

## 2. Ingestion instrumentation
- [ ] 2.1 Extend `crates/cortex-ingestion/src/metrics.rs` with `events_received_total{kind}`, `events_archived_total{kind}`, `last_archive_write_ts{kind}`
- [ ] 2.2 Increment in `archive::write_envelope` after successful flush
- [ ] 2.3 Add `events_rejected_total{reason}` to surface validation failures (e.g. malformed ULID, schema)
- [ ] 2.4 Wire into `/v1/healthz` extras

## 3. cortex-api stage observers
- [ ] 3.1 In `archive_loader.rs`, stamp `last_refresh_ts` and `envelopes_seeded_total{kind}` on each loader pass
- [ ] 3.2 Same for `meili_loader.rs`: `last_refresh_ts` + `docs_seeded_total{family}`
- [ ] 3.3 Make these readable from the dashboard `AppState` (add Arc<RwLock<LoaderMetrics>>)

## 4. Worker instrumentation
- [ ] 4.1 cortex-classifier-worker: `jobs_processed_total`, `last_job_ts`, `queue_lag`
- [ ] 4.2 cortex-embedder-worker: same shape
- [ ] 4.3 cortex-fulltext-worker: same shape
- [ ] 4.4 cortex-graph-worker: same shape
- [ ] 4.5 Each worker exposes the metrics on `/healthz` extras + new `/metrics` Prometheus text endpoint

## 5. Aggregator endpoints in cortex-api
- [ ] 5.1 NEW `crates/cortex-api/src/health/freshness.rs` with `freshness_handler` returning `HashMap<String, FreshnessRow { last_event_ts, gap_seconds }>`
- [ ] 5.2 Probe each subsystem's `/healthz` and pull `extras.last_*_ts`; flatten into `<stage>.<kind>` keys
- [ ] 5.3 NEW `health/divergence.rs` with handler `GET /v1/health/divergence` that compares adjacent-stage counters and returns `{ pair, delta, ratio, alert }` rows
- [ ] 5.4 Define divergence pairs in a static table: `(adapter.ipc.PostToolUse, adapter.publisher.tool_call)`, `(adapter.publisher.tool_call, ingestion.archived.tool_call)`, etc.
- [ ] 5.5 Alert threshold: `delta > 50` AND `delta_growth_60s > 10` → `severity: warn`; `delta_growth_60s > 50` → `severity: critical`
- [ ] 5.6 Wire `/v1/health/freshness` and `/v1/health/divergence` routes in `dashboard.rs`

## 6. /metrics Prometheus text exposure
- [ ] 6.1 Each crate exposes `/metrics` with the standard Prometheus text encoding
- [ ] 6.2 Document in `docs/metrics.md` the canonical metric names and labels

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update `docs/architecture.md` (pipeline section) + `docs/metrics.md` + CHANGELOG entries on every touched crate
- [ ] 7.2 Tests: unit tests for counter increments (per crate); integration test that drives a fake hook and asserts adapter/ingestion counters move in lockstep; cortex-api integration test for `/v1/health/freshness` and `/v1/health/divergence`
- [ ] 7.3 Run `cargo test --workspace` and confirm all pass with ≥95% coverage on new code
