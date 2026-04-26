## 1. Crate scaffold
- [x] 1.1 `cortex-fulltext` crate with `FulltextIndexer` trait + `Document` type
- [x] 1.2 Worker binary `cortex-fulltext-worker` consuming `cortex.events.enriched`
- [x] 1.3 Config via env (`CORTEX_FULLTEXT_*`)

## 2. Index settings
- [x] 2.1 `crates/cortex-fulltext/settings/settings.v1.json` shipped with searchable/filterable/sortable attrs, ranking rules, stop-words, synonyms, typo tolerance — baked in via `include_str!`
- [x] 2.2 `MeiliClient::ensure_index` creates index + applies settings idempotently; fail-fast on `incompatible` settings drift
- [x] 2.3 Settings bump telemetry: `Metrics::incr_settings_bump` counter ticked on every successful `ensure_index`; reindex not required

## 3. Doc builders
- [x] 3.1 Exhaustive `match` on `Kind` dispatches to one pure builder per family (`turn`, `tool_call`, `agent_call`, `memory`, `decision`, `analysis`, `law_violation`, `artifact`)
- [x] 3.2 Shared core fields (`id`, `event_id`, `kind`, `content_hash`, `ts`, `repo`, `path`, `topics`, `severity`, `pii_risk`, `summary`, `title`, `body`, `language`, `truncated`)
- [x] 3.3 `ext.<family>` sub-object populated for `tool_call`, `agent_call`, `decision`, `law_violation`, `memory`, `analysis`

## 4. Identity + body rules
- [x] 4.1 `doc_id` = `event_id` (live) or `bootstrap:<repo>:<path>:<content_hash>` (bootstrap) via `live_doc_id` / `bootstrap_doc_id`
- [x] 4.2 Body selection in `body::select_body`: summary if raw >4 KB and summary present, else raw; truncate to `max_body_bytes` (default 10 MB) and flip `truncated`
- [x] 4.3 Empty-body branch returned when raw + summary are both blank; `Metrics::incr_skipped_empty` counter ticks

## 5. Meili client + worker loop
- [x] 5.1 `LiveMeiliClient` over `reqwest` + retry policy (3 attempts, exp backoff 100/400/1600 ms); upsert batched at `upsert_batch` (default 1 000)
- [x] 5.2 Fire-and-forget on live; `CORTEX_FULLTEXT_AWAIT_TASK=1` flips `await_task` so bootstrap waits on the Meili task to fail-fast
- [x] 5.3 Backpressure gauge in `Worker::handle_batch`: `MeiliError::TransientError` arms `BackpressureState`; sustained ≥30 s halts consumption
- [x] 5.4 Successful batch publishes report on `cortex.events.fulltext_indexed`; rejection routes per-event invalid envelopes

## 6. Observability
- [x] 6.1 `Metrics` registry with `documents_total`, `batch_size`, `upsert_latency_ms`, `dedup_hits`, `task_failures`, `errors`, `skipped_empty`, `truncated`, `settings_bump`, `backpressure_active`
- [x] 6.2 Per-batch structured tracing event emitted in every outcome branch (ok / transient / rejected / task_failed / error) with `events`, `documents_upserted`, `documents_skipped`, `documents_truncated`, `latency_ms`

## 7. Tail (mandatory)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/08-fulltext-indexer.md` flipped to 🟢 Implemented; `docs/specs/00-index.md` row updated
- [x] 7.2 Write tests covering the new behavior — `tests/builders.rs` (8), `tests/indexer.rs` (4), `tests/routing.rs` (2), `tests/worker.rs` (9) covering per-kind builder shape, ext extensions, body fallback (oversize+summary, oversize+no-summary, truncation), bootstrap doc_id stability, per-kind index routing, batch chunking, malformed payload routing, transient → success backpressure recovery, paused-state pause, 10 000-event drain + idempotent replay; live-Meili probes gated by `CORTEX_FULLTEXT_IT=1` as a follow-up
- [x] 7.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy -p cortex-fulltext --all-targets -- -D warnings`, `cargo test -p cortex-fulltext` all green
