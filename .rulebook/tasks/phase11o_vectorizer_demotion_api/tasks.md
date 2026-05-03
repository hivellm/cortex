## 1. Coordination — vectorizer + vectorizer-sdk (external)

- [ ] 1.1 Open an upstream issue / PR in `hivellm/vectorizer` requesting the `move_to_collection` + `delete_vectors` server endpoints described in this task's proposal §1.
- [ ] 1.2 Open the matching `vectorizer-sdk` (Rust) PR exposing the new client methods per proposal §2 + §3.
- [ ] 1.3 Pin the released `vectorizer-sdk` version that carries them in this repo's workspace `Cargo.toml`.

## 2. Cortex-side — pruner module + sinks

- [ ] 2.1 New module `crates/cortex-claude-archive/src/pruner.rs`. Walks the `cortex_consolidations` index for every active consolidation, resolves each `source_event_id` to its current Vectorizer collection (via the index document's `repo` + `kind` + the per-repo naming convention), and computes the demotion target per the 0-7d / 7-90d / 90-365d / >365d schedule.
- [ ] 2.2 Demotion sink: call `vectorizer::Client::move_to_collection(src, dst, ids)` per resolved `(src, dst)` pair; chunk batches at 256 ids so a single transient 5xx does not lose a whole day of work.
- [ ] 2.3 Meili demotion sink: `update_documents` to drop the high-cost fields (`body`, `summary`, `outcome_distribution`) on cold-tier rows so the keyword lane keeps a stub but the disk footprint stays bounded.
- [ ] 2.4 Hard-purge sink: callable from the new `/cortex forget <event_id>` MCP tool (cortex-mcp-server, phase 11j §5.3); requires a confirmation token. Cascades to Vectorizer (`delete_vectors`), Meili (`delete_documents`), Nexus (`DELETE node`), and rewrites the matching Parquet partition.
- [ ] 2.5 Cron schedule: pruner runs nightly at 03:00 local time; configurable via `cortex.toml [cortex.consolidation] prune_at = "03:00"`.

## 3. Health surface

- [ ] 3.1 Surface pruner status in `/v1/health/coverage` under a new `pruner` block: `last_run_ts`, `events_demoted_per_tier`, `events_purged`, `last_run_duration_ms`, `last_error`.

## 4. Integration tests

- [ ] 4.1 `crates/cortex-claude-archive/tests/pruner_it.rs` — seeds 100 raw events spanning 5 age buckets + 20 consolidations referencing them; asserts post-prune doc counts match expected per-tier targets. Gated on `CORTEX_PRUNER_IT=1`.
- [ ] 4.2 `crates/cortex-claude-archive/tests/pruner_safety_it.rs` — asserts no `source_event_id` referenced by an active consolidation is dropped before the consolidation itself expires. Same gate.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 5.1 Update or create documentation covering the implementation — flip `docs/specs/19-retention.md` to mark the Vectorizer demotion path as Implemented; document the new `/cortex forget` MCP tool in `docs/specs/20-mcp-tool-surface.md`; CHANGELOG entry under Added.
- [ ] 5.2 Write tests covering the new behavior — every IT named in §4 lands; coverage ≥ 95 % on `crates/cortex-claude-archive/src/pruner.rs`.
- [ ] 5.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p cortex-claude-archive`, plus the `CORTEX_PRUNER_IT=1` gated suite. All green before archive.
