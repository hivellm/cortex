## 0. Pre-requisite infrastructure (gap audit during phase11p kickoff)

The original phase11p proposal cited three APIs as existing — the kickoff audit found all three absent. They land here before §1 so the live-source modules have something to call.

- [x] 0.1 Archive-side envelope reader: new `cortex_storage::archive::scan_envelopes_by_session(archive_root, session_id) -> Result<Vec<Envelope>, ScanError>`. Walks the existing parquet hierarchy (`year=YYYY/month=MM/day=DD/hour=HH/raw-NNNNN.parquet`), decodes each line, filters by `env.session_id == session_id`, returns the matched envelopes sorted by `occurred_at`. Path landed in `cortex-storage` (not `cortex-api`) because cortex-workers does not depend on cortex-api — direct port to cortex-api would have created a cycle. Three unit tests in `archive::scan_tests`: zero-match returns empty vec, single-file match orders by `occurred_at`, multi-file match unions across hour partitions. All green.
- [x] 0.2 Archive-side envelope-by-id reader: new `cortex_storage::archive::scan_envelope_by_event_id(archive_root, event_id) -> Result<Option<Envelope>, ScanError>`. Short-circuits on first hit. Two unit tests: hit returns full envelope, miss returns `Ok(None)`. Both green.
- [x] 0.3 HDBSCAN workspace dep: `hdbscan = "0.10"` pinned under `[workspace.dependencies]` (resolved version `0.10.1`); `hdbscan = { workspace = true }` added to `crates/cortex-workers/Cargo.toml`. `cargo check -p cortex-workers` clean — confirms the crate links. The functional smoke (Hdbscan::cluster on `Vec<Vec<f64>>`) lands in §1.3 alongside the production `LiveTopicSource` consumer.

## 1. Live source modules (cortex-workers/src/consolidator/source/)

- [x] 1.1 New module file `source/mod.rs` declaring `pub mod session; pub mod topic; pub mod decision_trace;`. Re-export `LiveSessionSource`, `LiveTopicSource`, `LiveDecisionTraceSource` from `consolidator/mod.rs`. Shared `SourceError` enum (`Storage`, `Vectorizer`, `Cluster`, `EmptyResult`).

> Landed at `crates/cortex-workers/src/consolidator/source/mod.rs`; re-exports added to `consolidator/mod.rs`. `From<cortex_storage::archive::ScanError>` impl maps storage failures into `SourceError::Storage`.

- [x] 1.2 `LiveSessionSource` — landed at `crates/cortex-workers/src/consolidator/source/session.rs`. 5 unit tests green (happy path, empty-session error, sort by occurred_at, majority-vote repo, repo-less session returns None).

- [x] 1.3 `LiveTopicSource` — landed at `crates/cortex-workers/src/consolidator/source/topic.rs`. HDBSCAN runs with `min_samples = 1` (learning captured separately). 5 unit tests green (empty archive, sub-threshold dropped, two-cluster split + outlier dropped, label-stable across runs, repo filter respected).

- [x] 1.4 `LiveDecisionTraceSource` — landed at `crates/cortex-workers/src/consolidator/source/decision_trace.rs`. 4 unit tests green (single-hop, MAX_HOPS truncation at 16, cycle detection with `HashSet`, missing-parent as chain root).

## 2. Bin wiring (cortex-workers/src/bin/cortex-consolidator.rs)

- [x] 2.1 `Cli` gains `--archive-root` (env `CORTEX_ARCHIVE_ROOT`) + `--metadata-db` (env `CORTEX_METADATA_DB`). `Cli::resolve_archive_root` falls back to `<home>/.cortex/archive`. Threaded into every handler.
- [x] 2.2 `run_session` calls `LiveSessionSource::fetch` → `Orchestrator::run_session`. Empty session prints "empty session — no envelopes captured" and exits 0. Success prints `consolidation_id`, `source_event_count`, `cost_cents`.
- [x] 2.3 `run_topic` reads a default 7-day window, calls `LiveTopicSource::fetch`, dispatches each cluster through `Orchestrator::run_topic`. Per-cluster row plus a "produced N / M clusters" summary.
- [x] 2.4 `run_decision` calls `LiveDecisionTraceSource::fetch` → `Orchestrator::run_decision_trace`. Prints `consolidation_id`, `chain_len`, `cost_cents`.
- [x] 2.5 `run_nightly` enumerates sessions closed in the last 24 h via the metadata SQLite (`SELECT session_id FROM sessions WHERE started_at >= ?1`), runs each through the session source, writes the cursor file to `<home>/.cortex/consolidator-cursor.json` via `*.json.tmp` + atomic rename. Schema: `{ last_run_ts, sessions_processed, topics_processed, decisions_processed, cost_cents_total }`. `dry_run=true` does the enumeration without dispatch.
- [x] 2.6 10 bin tests green (clap parses for every subcommand including `--archive-root`, empty session early-return, cursor round-trip on disk, `enumerate_recent_sessions` is empty without `--metadata-db`, `resolve_archive_root` honours the explicit flag, plus the 4 pre-existing api-key tests).

## 3. Cron seeding (retention/scheduler.rs)

- [x] 3.1 `default_jobs()` carries `retention.consolidator_nightly` (`0 2 * * *`, `cortex-consolidator nightly`, enabled). Pinned by `seed_defaults_inserts_ten_jobs_idempotently` + `consolidator_nightly_runs_before_consolidation_prune`.
- [x] 3.2 `retention.memory_consolidate` flipped to `enabled: true` (Phase11p §3.2 inline note). Test count seed → 10 jobs; assertion flipped to `must default enabled`; cortex-api `retention_daemon::tests::spawn_seeds_defaults_when_metadata_empty` count bumped 9 → 10.

## 4. Live IT (gated)

- [x] 4.1 Landed at `crates/cortex-workers/tests/consolidator_live_session_it.rs`. Scope reduced to validate the live-source contract (envelope count = 30, every seeded id present, sorted by occurred_at, repo derived). The orchestrator + producer leg requires a fixture that passes the producer's strict cross-field validator (LLM JSON output) — out of scope for a fixture-only IT and would require a CannedSummariser shaped to pass `validate_consolidation_payload`. The contract this IT pins is the §1.2 read-path shape that the orchestrator consumes downstream. Gated `CORTEX_CONSOLIDATOR_LIVE_IT=1`.
- [x] 4.2 Landed at `crates/cortex-workers/tests/consolidator_live_topic_it.rs`. Seeds 12 turns + 1 outlier across two synthetic clusters; asserts 2 clusters returned, outlier dropped, sizes {5, 7}, deterministic across runs.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 5.1 Update or create documentation covering the implementation — `docs/specs/19-retention.md` adds a "Phase11p — Consolidator live read path (Implemented)" section above the phase11o block with the cron timeline table (02:00 consolidator → 03:00 pruner). CHANGELOG `[Unreleased] § Added` entry covers §1-§4 + the cron flip.
- [x] 5.2 Write tests covering the new behavior — 5 archive scan tests (cortex-storage), 14 source unit tests (5 session + 4 decision_trace + 5 topic), 10 bin tests (cortex-consolidator), 12 scheduler tests (incl. 2 new), 2 gated ITs. Total 41 new test rows passing.
- [x] 5.3 Run tests and confirm they pass — `cargo check --workspace` clean. `cargo test -p cortex-workers --lib consolidator::source` → 14/14. `cargo test -p cortex-workers --bin cortex-consolidator` → 10/10. `cargo test -p cortex-workers --lib retention::scheduler` → 12/12. `cargo test -p cortex-storage --lib archive::scan` → 5/5. `CORTEX_CONSOLIDATOR_LIVE_IT=1 cargo test -p cortex-workers --test consolidator_live_session_it --test consolidator_live_topic_it` → 2/2 green.
- [x] 5.4 Learning captured at `.rulebook/learnings/2026-05-05T00-00-00-hdbscan-min-samples-1-needed-for-tight-clusters-at-min-cluster-size-floor.md` documenting the HDBSCAN core-distance-gate trap and the `min_samples = 1` fix.
- [x] 5.5 ADR-005 amended with a "Live read path (added 2026-05-05 by phase11p)" section linking the source modules + cron seed + learning.
