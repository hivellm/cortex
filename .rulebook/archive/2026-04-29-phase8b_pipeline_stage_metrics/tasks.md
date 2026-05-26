## 1. Adapter instrumentation
- [x] 1.1 Extend `crates/cortex-adapter-claude-code/src/metrics.rs` with `frames_received_total: AtomicU64` per hook kind (HashMap behind Mutex)
- [x] 1.2 Add `frames_parsed_total` and `frames_parse_error_total` counters
- [x] 1.3 Add `envelopes_built_total{kind}`, `envelopes_publish_ok_total{kind}`, `envelopes_publish_fail_total{kind}`
- [x] 1.4 Stamp `last_frame_ts{hook}` (RwLock<HashMap<HookKind, Instant>>) updated by `ipc::handle_pipe`/`handle_unix`
- [x] 1.5 Stamp `last_publish_ok_ts` updated by `publisher::flush_locked` after successful POST
- [x] 1.6 Wire metrics into `/healthz` extras (depends on phase8a)

## 2. Ingestion instrumentation
- [x] 2.1 Extend `crates/cortex-ingestion/src/metrics.rs` with `events_received_total{kind}`, `events_archived_total{kind}`, `last_archive_write_ts{kind}`
- [x] 2.2 Increment in `archive::write_envelope` after successful flush
- [x] 2.3 Add `events_rejected_total{reason}` to surface validation failures (e.g. malformed ULID, schema)
- [x] 2.4 Wire into `/v1/healthz` extras

## 3. cortex-api stage observers
- [x] 3.1 In `archive_loader.rs`, stamp `last_refresh_ts` and `envelopes_seeded_total{kind}` on each loader pass
- [x] 3.2 Same for `meili_loader.rs`: `last_refresh_ts` + `docs_seeded_total{family}`
- [x] 3.3 Make these readable from the dashboard `AppState` (add Arc<RwLock<LoaderMetrics>>)

## 4. Worker instrumentation
- [x] 4.1 cortex-classifier-worker: `jobs_processed_total`, `last_job_ts`, `queue_lag`
- [x] 4.2 cortex-embedder-worker: same shape
- [x] 4.3 cortex-fulltext-worker: same shape
- [x] 4.4 cortex-graph-worker: same shape
- [x] 4.5 Each worker exposes the metrics on `/healthz` extras + new `/metrics` Prometheus text endpoint

## 5. Aggregator endpoints in cortex-api
- [x] 5.1 NEW `crates/cortex-api/src/health/freshness.rs` with `freshness_handler` returning `HashMap<String, FreshnessRow { last_event_ts, gap_seconds }>`
- [x] 5.2 Probe each subsystem's `/healthz` and pull `extras.last_*_ts`; flatten into `<stage>.<kind>` keys
- [x] 5.3 NEW `health/divergence.rs` with handler `GET /v1/health/divergence` that compares adjacent-stage counters and returns `{ pair, delta, ratio, alert }` rows
- [x] 5.4 Define divergence pairs in a static table: `(adapter.ipc.PostToolUse, adapter.publisher.tool_call)`, `(adapter.publisher.tool_call, ingestion.archived.tool_call)`, etc.
- [x] 5.5 Alert threshold: `delta > 50` AND `delta_growth_60s > 10` → `severity: warn`; `delta_growth_60s > 50` → `severity: critical`
- [x] 5.6 Wire `/v1/health/freshness` and `/v1/health/divergence` routes in `dashboard.rs`

## 6. /metrics Prometheus text exposure
- [x] 6.1 Each crate exposes `/metrics` with the standard Prometheus text encoding
- [x] 6.2 Document in `docs/metrics.md` the canonical metric names and labels

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/architecture.md §13.6 Observability — pipeline stage metrics & freshness (phase8b)` + new `docs/metrics.md` (canonical metric catalogue + severity buckets + canonical divergence pairs + operator workflow) + CHANGELOG entry under `### Added → Observability — pipeline stage metrics & freshness (phase8b)` covering every touched crate
- [x] 7.2 Write tests covering the new behavior — adapter `Metrics` unit tests (per-hook + per-kind counters, last-ts stamps, parse-error counter, render_prom shape), ingestion `Metrics` unit tests (per-kind received/archived, per-reason rejected, last_archive_write_ts, render shape), cortex-api `LoaderMetrics` unit tests, freshness/divergence handler unit tests in `crates/cortex-api/src/health.rs` (severity buckets, freshness rows, per-label timestamp expansion, divergence pair derivation, missing-subsystem fallback, per-kind silent-drop localisation), `cortex_health::server` route tests (metrics renderer present + absent), and integration coverage in `crates/cortex-api/tests/health_freshness.rs` (`/v1/health/freshness`, `/v1/health/divergence`, `/metrics` mount-on-router)
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across all touched crates (cortex-health, cortex-adapter-claude-code, cortex-ingestion, cortex-api, cortex-workers); per-stage unit tests + integration tests + adapter/ingestion/api lib tests all green
