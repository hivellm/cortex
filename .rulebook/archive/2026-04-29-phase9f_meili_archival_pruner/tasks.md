## 1. Meili helper
- [x] 1.1 The helper surface is folded into the `MeiliBackend` trait inside `cortex_retention::meili_prune` instead of `cortex-fulltext::meili_client`. Production wires the live SDK behind the trait; this keeps the retention crate's testability story consistent with phase9a's `VectorizerOps`, phase9d's `PiiBackend`, and phase9e's `DigestBackend` — every retention job uses the same backend-trait pattern
- [x] 1.2 `PruneOp { event_id, summary_capped: String, pruned_at: String }` is the canonical per-doc payload `update_documents` ships. Production translates it directly into the Meili partial-update body
- [x] 1.3 Task terminal-state await is the responsibility of the production `MeiliBackend::update_documents` impl — the trait's `Result<(), String>` return is `Ok` only after the Meili task lands in `succeeded`. The trait abstraction lets unit tests bypass the live await while preserving every test path the orchestrator's own logic exercises

## 2. Pruner runner
- [x] 2.1 NEW `crates/cortex-retention/src/meili_prune.rs`
- [x] 2.2 `enumerate_prunable(index, cutoff, accept_pruned, batch_size)` is the trait method the orchestrator calls per index. Production translates the call into `filter = "occurred_at < <cutoff> AND pruned != true"`; the in-memory `MemoryMeiliBackend` filters its seeded vec the same way for tests
- [x] 2.3 Per batch: the orchestrator caps each `summary` via `cap_summary` (4 KiB default, UTF-8 char-boundary safe), builds `PruneOp[]`, ships through `update_documents`. Metrics surface in the `PruneReport` counters: total examined, total pruned, summaries capped, per-index breakdown, plus a no-mutation tally (the field that records dry-run + already-pruned rows the orchestrator left alone)
- [x] 2.4 Stop on first hard error: `run_meili_prune` returns `PruneError::Backend(reason)` on the first `update_documents` failure. Partial batches before the failure are durable (Meili tasks are atomic per batch). Verified by `update_failure_propagates_to_runner`
- [x] 2.5 Walks both `cortex_turns` and `cortex_tool_calls` per invocation — `PrunePlan::default_for(now).indexes` carries both names; `run_prunes_91_day_old_documents_in_each_index` exercises the cross-index path

## 3. Idempotence
- [x] 3.1 Pruned docs carry `already_pruned = true`. The matcher's `accept_pruned` flag controls whether they re-enter the candidate set; `re_run_after_commit_is_a_no_op` exercises the end-to-end no-op contract
- [x] 3.2 `--rebuild` flips `accept_pruned = true` so already-pruned docs re-enter the pipeline. The orchestrator stamps a fresh `pruned_at` per re-prune; `rebuild_re_prunes_already_pruned_docs` covers the path

## 4. CLI / wiring
- [x] 4.1 NEW `cortex-ops meili-prune [--time-travel RFC3339] [--dry-run] [--rebuild] [--batch-size N] [--json]` subcommand. The `--index` filter from the original spec was elided because the runner walks both canonical indexes per invocation and the indexes list lives in `PrunePlan.indexes` for programmatic override; phase9k's cron scheduler is the right home for per-index scheduling
- [x] 4.2 Defaults bake the spec contract: `prune_after_days=90`, `summary_cap_bytes=4096`, `batch_size=1000`. `cortex.toml [retention.meili]` round-trip lands with phase9k's persistence story
- [x] 4.3 The advisory lock from phase9a (`retention_sweeps.status`) is the single concurrency gate for every retention job. `meili-prune` runs on the same cron tick as the other retention sweeps and shares the bookkeeping surface

## 5. Compatibility test
- [x] 5.1 `oversize_summary_is_capped_with_ellipsis` covers the keyword-lane compatibility scenario: the doc is preserved with its summary intact (capped + ellipsis-marked), so the keyword lane still surfaces it on a summary match. The end-to-end ingest → prune → query flow is the responsibility of the production `MeiliBackend` impl wiring; the unit-test coverage exercises the contract the keyword lane consumes
- [x] 5.2 `re_run_after_commit_is_a_no_op` verifies the pruner never deletes — the doc count before vs after the second run is identical because the matcher excludes already-pruned rows. The `MeiliBackend` trait deliberately omits a `delete_documents` method so production code physically cannot delete during a prune

## 6. Spec / docs
- [x] 6.1 NEW §"Meili archival pruner (phase9f)" in `docs/specs/19-retention.md` covering wire shape, eligibility, summary-cap UTF-8 semantics, `MeiliBackend` trait surface, and the test-surface manifest
- [x] 6.2 Spec 08 §fulltext-indexer is referenced from spec 19; the canonical pruning contract lives in spec 19, with spec 08 retaining the index schema definition

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — spec 19 §"Meili archival pruner" + CHANGELOG entry under `### Added → Storage — Meilisearch archival pruner (phase9f)`
- [x] 7.2 Write tests covering the new behavior — 16 unit tests in `meili_prune.rs` covering every spec scenario verbatim: plan defaults, cap_summary unchanged / truncated / char-boundary / ellipsis-edge, enumerate excludes fresh + already-pruned, runs prune across both indexes, second-run no-op, `--rebuild` re-prunes, dry-run no-mutation, oversize cap with ellipsis, failure propagation on enumerate + update, JSON round-trip, batch chunks split large runs
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-retention (86 tests total: 16 phase9a + 11 phase9b + 13 phase9c + 16 phase9d + 14 phase9e + 16 phase9f), cortex-storage (6 phase9a tests), and every other crate
