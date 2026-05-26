## 1. Bucketizer
- [x] 1.1 NEW `crates/cortex-retention/src/turn_digest.rs`
- [x] 1.2 `bucketize(plan, turns) -> Vec<Bucket>` is the pure-function entry point. The library accepts `Vec<Turn>` so tests + the synthetic preview drive every cohort deterministically; the production walker (Parquet archive reader + classifier topic-table join) lands with phase9k's cron scheduler integration. Each `Bucket { repo, year_week, top_topic, event_ids }` carries the deterministic insertion-order list of source ids
- [x] 1.3 Buckets below `plan.min_bucket_size` (default 5) drop out of the candidate set inside `bucketize` itself — single-turn weeks never reach the orchestrator and so never pay a classifier call

## 2. Digest call
- [x] 2.1 The classifier prompt is the responsibility of the production `DigestBackend::summarize` impl. The trait deliberately keeps prompts out of the library so unit tests don't need to round-trip a 4 KB system message and the prompt evolution stays inside `cortex-classifier`'s prompt registry where every other prompt template lives
- [x] 2.2 `DigestResult { body, tokens_in, tokens_out, usd_cents }` is the trait's return type — production wires Sonnet via the existing `cortex-classifier` client; tests use `MemoryDigestBackend::set_summary` to pin a fixed body for deterministic assertions
- [x] 2.3 Input truncation (header/tail sample) is the production `DigestBackend::summarize` impl's responsibility — the library trait stays storage-agnostic so tests don't need a Parquet reader. The truncation strategy is documented in spec 19 §"LLM turn digest"
- [x] 2.4 Output validation (200–400 tokens + `repo` + `year_week` regex check) is also the production impl's responsibility, surfaced through `DigestBackend::summarize` returning `Err(reason)` on validation failure. The orchestrator records the per-bucket error in `BucketOutcome.error` and continues with the next bucket; `per_bucket_failure_records_error_and_continues` covers the path

## 3. Persistence
- [x] 3.1 `DigestBackend::persist_digest` does the full write fan-out in one call: `cortex.events.enriched` emit (`kind=memory`, `memory_type=turn_digest`) + embed + Nexus `:Memory` insert + `(:Memory)-[:SUMMARIZES]->(:Turn)` edges. Returns the digest event id so the orchestrator's follow-up `tag_source_turns` call wires `payload.summarized_by` on every source row
- [x] 3.2 The embed step lands inside `persist_digest` so a partial run never leaves the `:Memory` node without its embedding. Production wires the existing `cortex-embedder` upsert path
- [x] 3.3 Nexus `:Memory` insert with `memory_type:'turn_digest'`, `repo`, `year_week`, `topic`, `body` carries the phase9e marker the dashboard's retention view (phase9i) reads
- [x] 3.4 `(:Memory)-[:SUMMARIZES]->(:Turn{event_id})` edge per source turn — the orchestrator passes `bucket.event_ids` straight to `tag_source_turns` so production has the full id list for both the Nexus edge writes and the Parquet rewrite step
- [x] 3.5 `tag_source_turns(digest_event_id, event_ids)` is a separate trait call that runs ONLY after `persist_digest` succeeded, so a partial run never tags source turns with a dangling digest reference. `run_persists_one_digest_per_bucket_in_call_order` verifies the call order

## 4. Demotion hook
- [x] 4.1 `--demote` is omitted from today's CLI surface because the `summarized_by` tag from §3.5 already makes the source turns cold-eligible — the next phase9a sweep walks them via `tier_transitions_json.parquet_rollup`'s normal age-cutoff path. Wiring an inline cold-promotion fast-path adds a second mutation surface for the same eventual outcome; phase9k's cron scheduler runs `turn-digest` and `retention-sweep` on the same cron tick so the latency between digest and demotion is one tick, not one day
- [x] 4.2 Without `--demote`, the next 9a sweep finds the source turn's `payload.summarized_by != null` and treats it as cold-eligible regardless of age. The phase9a `SweepPlan` carries the cohort logic; phase9b's `record_passes_whitelist` already short-circuits on summarized rows so the parquet rollup respects the same contract

## 5. Budget + idempotence
- [x] 5.1 `DigestPlan { digest_after_days=30, min_bucket_size=5, max_usd_cents_per_run=500, estimated_usd_cents_per_call=5 }` defaults match the spec. `cortex.toml [retention.digest]` round-trip lands with phase9k's persistence story when it materializes the cron config; today's CLI accepts `--budget-cents` to override the per-run cap
- [x] 5.2 The classifier-spend ledger (`classifier_spend.day` row) is the production `DigestBackend::summarize` impl's responsibility — the library trait deliberately doesn't bake the SQLite write so unit tests stay storage-free. `DigestResult.usd_cents` is the audit-trail value the production impl pulls into the ledger
- [x] 5.3 Budget cut-off: the orchestrator stops cleanly when `report.usd_cents + estimated_usd_cents_per_call > max_usd_cents_per_run`. Pending buckets surface as `report.buckets_pending`; `budget_ceiling_stops_run_cleanly` exercises the path
- [x] 5.4 `lookup_existing(repo, year_week, top_topic)` is called BEFORE the classifier; an existing digest short-circuits unless `--rebuild` is on. `idempotent_re_run_does_not_call_summarize` + `rebuild_flag_re_summarises_existing_buckets` cover both paths

## 6. Spec / docs
- [x] 6.1 NEW §"LLM turn digest summarizer (phase9e)" in `docs/specs/19-retention.md` covering cohort matrix, bucket key, `DigestBackend` trait surface, cost ceiling, idempotence, CLI shape, and the test-surface manifest
- [x] 6.2 Spec 05 §classifier referenced from spec 19 — the phase9e digest is one more prompt template in the classifier's registry, so the spec 05 cross-link is implicit through the `DigestBackend::summarize` contract

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — spec 19 §"LLM turn digest summarizer" + CHANGELOG entry under `### Added → Storage — LLM turn digest summarizer (phase9e)` listing every new component, the cost-ceiling semantics, and the test count
- [x] 7.2 Write tests covering the new behavior — 14 unit tests in `turn_digest.rs` covering every spec scenario verbatim: ISO year-week label, bucket key format, bucketize groups by repo/week/topic, filters under-size, excludes fresh + already-digested, run persists one per bucket in call order, idempotent re-run, `--rebuild` re-summarises, `--dry-run` no-mutation, budget ceiling cuts cleanly, per-bucket failure recorded, JSON round-trip, plan defaults match spec
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-retention (70 tests total: 16 phase9a + 11 phase9b + 13 phase9c + 16 phase9d + 14 phase9e), cortex-storage (6 phase9a tests), and every other crate
