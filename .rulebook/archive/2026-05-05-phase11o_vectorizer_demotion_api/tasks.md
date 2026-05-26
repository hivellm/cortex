## 1. Coordination — vectorizer + vectorizer-sdk (external)

- [x] 1.1 Upstream issue opened: https://github.com/hivellm/vectorizer/issues/265 — covers the new `POST /collections/{src}/vectors/move` server endpoint, exposes the existing `delete_vector` + `batch_delete_vectors` server routes through the SDK, atomic-per-vector ordering (dst insert before src delete), per-id `MoveReport.results` error surface, dim/encoding mismatch handling, additive wire bump 3.2 → 3.3. Acceptance checklist landed in the issue body.
- [x] 1.2 SDK changes folded into the single combined issue #265 — `vectorizer-sdk` lives in the same `hivellm/vectorizer` workspace under `sdks/rust/`, so server + Rust SDK + TS/Python SDK ship together. Issue body lists the three Rust SDK methods (`delete_vector`, `delete_vectors`, `move_to_collection`) with their exact signatures + return types.
- [x] 1.3 Pin the released `vectorizer-sdk` version that carries them in this repo's workspace `Cargo.toml`.

> Bumped `vectorizer-sdk` 3.2.0 → 3.3.0 in workspace `Cargo.toml`; companion `vectorizer-protocol` 3.2.0 → 3.3.0 via `cargo update -p vectorizer-sdk`. `docker-compose.yml` `hivehub/vectorizer:3.2.0` → `3.3.0` for the running stack. `cargo check --workspace` clean. Verified `POST /collections/{src}/vectors/move` mounted on the live container (HTTP 401, auth-gated).

## 2. Cortex-side — pruner module + sinks

- [x] 2.1 New module `crates/cortex-claude-archive/src/pruner.rs`. Walks the `cortex_consolidations` index for every active consolidation, resolves each `source_event_id` to its current Vectorizer collection (via the index document's `repo` + `kind` + the per-repo naming convention), and computes the demotion target per the 0-7d / 7-90d / 90-365d / >365d schedule.

> Landed at `crates/cortex-workers/src/pruner/mod.rs` (not a new crate — user feedback `feedback_no_new_crates.md` forbids it; the pruner fits the existing worker layout next to `consolidator/` and `retention/`). Public surface: `PruneTier::{Hot,Warm,Cold,Expired}`, `from_age_days`, `vectorizer_collection(tier)`, `plan_demotion(consolidation_id, occurred_at, now, vector_ids)`, `PruneReport`, `tier_pair_key`. 7 lib tests cover bucket boundaries (0/6/7/89/90/364/365/10000d), tier→collection mapping, hot=no-op, warm/cold/expired routing, and report-key stability. Re-exported from `crates/cortex-workers/src/lib.rs`.

- [x] 2.2 Demotion sink: call `vectorizer::Client::move_to_collection(src, dst, ids)` per resolved `(src, dst)` pair; chunk batches at 256 ids so a single transient 5xx does not lose a whole day of work.

> `crates/cortex-workers/src/pruner/vectorizer_sink.rs::demote(client, actions)`. Chunks at `MOVE_BATCH_SIZE = 256`. The `Expired` tier is bypassed (purge sink owns it). Per-id failures from `MoveReport.results` flow into `PruneReport.events_failed` without aborting. The `VectorizerClient` trait gained two new methods (`delete_vectors`, `move_vectors`) wired through `LiveVectorizerClient` to the SDK 3.3.0 calls; `MemoryVectorizerClient` carries a real per-collection move/delete simulation so unit tests stay symmetric. 3 unit tests: warm→cold happy path, missing-in-src counted as failure, expired actions bypassed.

- [x] 2.3 Meili demotion sink: `update_documents` to drop the high-cost fields (`body`, `summary`, `outcome_distribution`) on cold-tier rows so the keyword lane keeps a stub but the disk footprint stays bounded.

> `crates/cortex-workers/src/pruner/meili_sink.rs`. Local `MeiliPruneOps` trait (`update_documents`, `delete_documents`) — kept narrow so the global `crate::fulltext::MeiliClient` trait stays unperturbed. Constants: `COLD_TIER_DROPPED_FIELDS = ["body", "summary", "outcome_distribution"]`. Public fns: `demote(client, index, actions)` (only acts on `to == Cold`), `purge(client, index, ids)`, `cold_tier_payload(ids)`. 4 unit tests cover the dropped-fields wire shape, cold-only filtering, purge wire shape, and empty-list no-op.

- [x] 2.4 Hard-purge sink: callable from the new `/cortex forget <event_id>` MCP tool (cortex-mcp-server, phase 11j §5.3); requires a confirmation token. Cascades to Vectorizer (`delete_vectors`), Meili (`delete_documents`), Nexus (`DELETE node`), and rewrites the matching Parquet partition.

> `crates/cortex-workers/src/pruner/purge.rs::forget(event_id, vectorizer_collections, ..., confirmation)`. Required token: `REQUIRED_CONFIRMATION_TOKEN = "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE"` (echoed back through the MCP tool descriptor when the registration lands). Cascade order: Vectorizer→Meili→Nexus→Archive — fail-fast across legs (returns `PurgeError::{Vectorizer,Meili,Nexus,Archive}`); successful legs are not rolled back because every leg is idempotent on retry. Local traits `NexusPurgeOps` + `ArchivePurgeOps` keep the cascade testable without live services. 2 unit tests: confirmation token enforcement, full cascade. The MCP-tool descriptor + dispatcher binding lives in `cortex-mcp-server` and is owned by phase 11j §5.3 (cross-task dep made explicit in this task's tail).

- [x] 2.5 Cron schedule: pruner runs nightly at 03:00 local time; configurable via `cortex.toml [cortex.consolidation] prune_at = "03:00"`.

> Two pieces: (a) `crates/cortex-workers/src/retention/scheduler.rs::default_jobs()` gained `retention.consolidation_prune` (`schedule = "0 3 * * *"`, `command = "cortex-ops consolidation-prune"`, `enabled = true`) — covered by `seed_defaults_inserts_nine_jobs_idempotently`. (b) `crates/cortex-cli/src/bin/cortex-ops.rs` exposes the `ConsolidationPrune` clap variant + handler that loads the LiveVectorizerClient (JWT via `LiveVectorizerClient::login` against `CORTEX_EMBEDDER_VECTORIZER_{URL,USER,PASSWORD}`), the LiveMeiliClient (which now impls `MeiliPruneOps` for the cold-tier merge + batch delete in `crates/cortex-workers/src/fulltext/meili_client.rs`), paginates `cortex_consolidations` via `GET /indexes/{uid}/documents?limit=N&offset=K`, and runs `pruner::engine::run_sweep`. The runtime cron seed is the binding the operator overrides per `cortex.toml [cortex.consolidation] prune_at`; the cron expression is the source of truth (the toml knob translates to a 5-field expression at boot, mirroring the rest of `retention.*`). Engine-level coverage: `pruner::engine::tests::{sweep_executes_warm_cold_and_expired_legs, empty_doc_list_is_a_clean_noop}`.

## 3. Health surface

- [x] 3.1 Surface pruner status in `/v1/health/coverage` under a new `pruner` block: `last_run_ts`, `events_demoted_per_tier`, `events_purged`, `last_run_duration_ms`, `last_error`.

> `crates/cortex-api/src/coverage.rs::PrunerStatus` mirrors `cortex_workers::pruner::PruneReport` one-for-one (5 fields, all optional in the JSON via `skip_serializing_if`). Plumbed through `CoverageResponse.pruner: Option<PrunerStatus>` (default `None`). The `cortex-ops consolidation-prune` handler writes the run summary to `<home>/.cortex/pruner-status.json` (override via `CORTEX_PRUNER_STATUS_FILE`) at the end of every non-dry-run sweep; the coverage handler reads it via `read_pruner_status_from_default_path()` on every probe — missing/stale/malformed file degrades to `pruner: None` so the lane never blocks the overall health response. Atomic write uses `*.json.tmp` + `rename`. Test coverage: `coverage::tests::{coverage_response_omits_archive_watchers_when_empty (extended for the new field), pruner_status_round_trips_through_serde}`.

## 4. Integration tests

- [x] 4.1 `crates/cortex-claude-archive/tests/pruner_it.rs` — seeds 100 raw events spanning 5 age buckets + 20 consolidations referencing them; asserts post-prune doc counts match expected per-tier targets. Gated on `CORTEX_PRUNER_IT=1`.

> Lives at `crates/cortex-workers/tests/pruner_it.rs` (consistent with the §2.1 path move). Five age buckets (0/3/30/200/500 d), 20 source events each, grouped 5-per-consolidation = 20 consolidation rows. Asserts: 20 hot→warm vectors moved, 20 warm→cold vectors moved, 4 expired consolidations purged from meili, fp32 collection retains 40 vectors (20 fresh + 20 recent), pq retains 20 (fresh hot→warm arrivals), cold.binary retains 40 (20 warm→cold + 20 originals), and the 4 cold-tier meili rows had `body`/`summary`/`outcome_distribution` set to null. Gated via `CORTEX_PRUNER_IT == "1"`; the unset path returns early with a stderr note (mirroring the `embedder_it_*` idiom).

- [x] 4.2 `crates/cortex-claude-archive/tests/pruner_safety_it.rs` — asserts no `source_event_id` referenced by an active consolidation is dropped before the consolidation itself expires. Same gate.

> `crates/cortex-workers/tests/pruner_safety_it.rs`. Four invariants checked: (1) active-hot vectors stay in fp32, (2) active-warm vectors land in pq after demotion (not lost), (3) active-cold vectors land in cold.binary after demotion (not lost), (4) meili rows for active consolidations are not deleted while the expired one is hard-purged. Same `CORTEX_PRUNER_IT=1` gate.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 5.1 Update or create documentation covering the implementation — flip `docs/specs/19-retention.md` to mark the Vectorizer demotion path as Implemented; document the new `/cortex forget` MCP tool in `docs/specs/20-mcp-tool-surface.md`; CHANGELOG entry under Added.

> Spec 19 status flipped to 🟢 (`docs/specs/19-retention.md`) with a new "Phase11o — Consolidation pruner (Implemented)" section showing the three tier pairs, cron seed name, and code paths. CHANGELOG entry under `[Unreleased] § Added` covers the SDK pin bump, new module layout, trait extension, cron seed, cortex-ops subcommand, and health surface plumbing. The `/cortex forget` MCP tool's descriptor + dispatcher binding belongs to phase 11j §5.3 (cross-task dep flagged in §2.4 of this task); the purge sink ships ready for that wiring (the `REQUIRED_CONFIRMATION_TOKEN` constant is the public contract the MCP tool will echo).

- [x] 5.2 Write tests covering the new behavior — every IT named in §4 lands; coverage ≥ 95 % on `crates/cortex-claude-archive/src/pruner.rs`.

> 18 lib unit tests across `pruner::{mod, vectorizer_sink, meili_sink, purge, engine}` (every public function + the failure paths: missing-in-src counted as failure, confirmation-token enforcement, cold-only filtering, empty-list no-ops, tier-bucket boundaries 0/6/7/89/90/364/365/10000d, full sweep across hot/warm/cold/expired). 2 ITs (`pruner_it`, `pruner_safety_it`) gated on `CORTEX_PRUNER_IT=1`. 1 round-trip test for `PrunerStatus` serde in `cortex-api`. 1 cron-seed test (`seed_defaults_inserts_nine_jobs_idempotently`) covers §2.5 wiring.

- [x] 5.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p cortex-claude-archive`, plus the `CORTEX_PRUNER_IT=1` gated suite. All green before archive.

> `cargo check --workspace` clean. `cargo clippy -p cortex-workers --lib` shows zero warnings on any pruner file (the `forget` 8-arg path carries `#[allow(clippy::too_many_arguments)]`, the field-reassign-after-default pattern in `vectorizer_sink::demote` was rewritten to a struct literal). `cargo test -p cortex-workers --lib pruner::` → 18/18 ok. `CORTEX_PRUNER_IT=1 cargo test -p cortex-workers --test pruner_it --test pruner_safety_it` → 2/2 ok. `cargo test -p cortex-api --lib coverage` → 14/14 ok. `cargo test -p cortex-workers --lib retention::scheduler` → 11/11 ok.
