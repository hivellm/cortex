## 1. P0 — Design ADRs (lock open questions)
- [x] 1.1 ADR-018 — `phase18 §1.1 — Bitemporal time precision: UTC RFC3339 with second precision; day-precision for ADR-facing tooling`. Storage second-precision; CLI day-precision input expands to range probes. `.rulebook/decisions/018-phase18-1-1-...md`.
- [x] 1.2 ADR-019 — `phase18 §1.2 — Branch identifier scheme: (project_id, branch_name) unique pair with strict regex; main reserved`. Regex `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$`, composite global id `<project>:<branch>`. `.rulebook/decisions/019-phase18-1-2-...md`.
- [x] 1.3 ADR-020 — `phase18 §1.3 — Cross-project retrieval default: opt-in until eval evidence justifies opt-out`. Default OFF; `--projects` flag flips it. Reassess after CDC MRR@10 evidence. `.rulebook/decisions/020-phase18-1-3-...md`.
- [x] 1.4 ADR-021 — `phase18 §1.4 — Branch merge representation: keep-and-link with classifier-driven fold-in`. No fact rewrite; `MERGED_INTO` edge + classifier-time strategy switch. `.rulebook/decisions/021-phase18-1-4-...md`.
- [x] 1.5 ADR-022 — `phase18 §1.5 — Temporal retention: never delete; archive superseded/expired after 5 years`. Hot slice drops; cold Vectorizer/Meili archive slice keeps audit window; purge operator-controlled. `.rulebook/decisions/022-phase18-1-5-...md`.
- [x] 1.6 ADR-023 — `phase18 §1.6 — Consolidation vs supersession edge semantics: SUPERSEDES / OBSOLETES / EVOLVES_FROM rules`. SUPERSEDES = replace (stamp superseded_at); EVOLVES_FROM = non-replacing precursor; OBSOLETES = deprecate without replace. `.rulebook/decisions/023-phase18-1-6-...md`.

## 2. P1 — Schema, migration, backfill
- [x] 2.1 Nexus writer extension shipped via `crates/cortex-workers/src/graph/bitemporal.rs`. New helper `stamp_bitemporal_props_on_patch(event, patch)` runs at the END of `map_event_to_patch` and stamps 5 columns on every `NodeOp`: `project_id` (lower-cased `context_repo`, falls back to `"unknown"`), `branch_id` (`"main"` until §2.12), `valid_from` (RFC3339 second-precision from `occurred_at_ms`, falls back to `now()` when sentinel-zero per phase20 §5), `recorded_at` (= `valid_from` at first write), `lifecycle` (`"active"` default; never overwrites a value the emitter already set). `valid_to` + `superseded_at` default to ABSENT (NULL = still valid per ADR-018). Idempotent: every key uses `entry().or_insert()` so a Decision emitter setting `lifecycle = superseded` survives. Nexus has no schema-bootstrap step — ADR-004 makes node properties dynamic per `_id`-keyed writer; the column set is implicit in the writer output. 5 new unit tests in `bitemporal::tests` cover: stamp sets all 5 columns; emitter-set lifecycle survives; ms=0 falls back to now; absent repo → `"unknown"`; multi-node patch walks every node. cortex-workers 841 lib tests pass (was 836; +5); clippy clean.
- [x] 2.2 `Branch` node + its 12 fields land in `crates/cortex-workers/src/graph/branch.rs`. `BranchStatus { Active, Merged, Abandoned }` + `MergeStrategy { Accept, Partial, Discard }` enums encode ADR-021 §1.4 values; `Branch::main_for(project, created_at)` factories the reserved `cortex:main` row; `as_node_op()` emits a `NodeOp` with composite-id natural key + every set field projected onto `props` (unset Optionals omitted so docs stay compact). Branch is graph-only (no envelope `Kind` variant); the writer runs from the `cortex-ops branch` CLI in §4.2, not from the Synap pipeline.
- [x] 2.3 `TimelineEvent` node + its 11 fields land alongside Branch in `graph/branch.rs`. `TimelineKind` enum encodes the 12 design.md §1.3 discriminators (`commit`, `adr`, `decision`, `release`, `incident`, `learning`, `task_start`, `task_archive`, `branch_fork`, `branch_merge`, `branch_abandon`, `cross_project_link`). Same graph-only writer pattern as Branch. `validate_branch_name()` enforces ADR-019 §1.2 regex shared between the Branch constructor and the §4.2 CLI; 6 new unit tests cover the factory + NodeOp projection + ADR-019 regex (positive + negative cases). cortex-workers 848 tests pass (was 842; +6); clippy clean.
- [x] 2.4 7 edge kinds registered in `crates/cortex-workers/src/graph/temporal_edges.rs`. `SUPERSEDES` already lived in `mapper::emit_decision`; the constant is now centralised so future writers reference the same identifier (drift between writer + classifier would silently break retrieval). 6 builder helpers (`build_obsoletes`, `build_evolves_from`, `build_conflicts_with`, `build_forked_from`, `build_merged_into`, `build_cross_project_ref`) encode the per-edge prop semantics from ADR-021/023: `MERGED_INTO` carries `strategy` + `merge_point_event_id`; `CROSS_PROJECT_REF` carries `version_constraint` + `valid_from` + optional `valid_to`; the others ship empty-props. `ALL_TEMPORAL_EDGES` const slice powers the §7.2 dashboard breakdown + the §3 classifier walk. 6 new unit tests cover registry length, prop projection per builder, branch-label-on-both-ends invariant for FORKED_FROM, optional `valid_to` projection on CROSS_PROJECT_REF. cortex-workers 854 lib tests pass (was 848; +6); clippy clean.
- [x] 2.5 Schema bootstrap extended in `crates/cortex-workers/src/graph/schema.rs::SCHEMA_STATEMENTS`. 2 new identity constraints (`b:Branch` + `t:TimelineEvent`); 11 new bitemporal-axis composite indexes: `decision_project_valid_to` / `memory_project_valid_to` / `analysis_project_valid_to` / `law_violation_project_valid_to` (drop EXPIRED), `decision_project_branch_valid_from` / `memory_project_branch_valid_from` (scope branch retrievals), `decision_superseded_at` / `memory_superseded_at` / `analysis_superseded_at` (drop SUPERSEDED), `timeline_event_project_branch_valid_time`, `branch_project`. Every statement carries `IF NOT EXISTS` so the bootstrap stays idempotent on re-run. 2 new pin tests (`phase18_branch_and_timeline_event_constraints_landed`, `phase18_bitemporal_indexes_landed`) guard against a future drop. Statement counts: was 14 → now 27.
- [x] 2.6 Meili schema bumped `v6 → v7` in `crates/cortex-workers/settings/settings.v1.json`. New filterable attrs: `project_id`, `branch_id`, `lifecycle`, `valid_from_unix`, `valid_to_unix`, `superseded_at_unix`. New sortable attrs: `valid_from_unix`, `valid_to_unix`, `superseded_at_unix` (second-precision epoch ints per ADR-018). Writer-side wiring in `crates/cortex-workers/src/fulltext/document.rs` (Document struct extended with the 6 new Option-typed fields, `skip_serializing_if = Option::is_none` so docs whose kind doesn't apply stay compact) + `crates/cortex-workers/src/fulltext/builders.rs::apply_bitemporal_projection` (idempotent — runs after `apply_top_level_projection` so a payload-derived `decision_status = superseded` lands on `lifecycle` before the default `active` would). New `bitemporal_axis_fields_are_filterable_and_sortable` test pins the contract. 842 cortex-workers lib tests pass; workspace `cargo check` clean.
- [x] 2.7 `ChunkMetadata` (`crates/cortex-workers/src/embedder/chunker.rs`) extended with the 6 bitemporal payload fields (`project_id`, `branch_id`, `lifecycle`, `valid_from_unix`, `valid_to_unix`, `superseded_at_unix`), all `Option`-typed with `skip_serializing_if = Option::is_none` so Vectorizer payloads stay compact on rows whose kind doesn't apply. `ChunkMetadata::stamp_bitemporal(event)` mirrors `graph::bitemporal::stamp_one_node` + `fulltext::builders::apply_bitemporal_projection` so the three writers (graph, Meili, Vectorizer) carry the same values per event. Three live chunker emit sites (`chunker_fallback.rs`, `chunker_doc.rs` ×2, `chunker_code.rs`) call `stamp_bitemporal` after struct construction. Embedder summary-substitution path inherits the bitemporal payload through `..chunk.metadata.clone()`. 6 production + 35 test fixture sites updated. 856 cortex-workers lib tests pass (was 854; +2 mod-internal coverage); clippy clean.
- [ ] 2.8 Reindex into versioned aliases `cortex-meili-bitemporal-v1` and `cortex-vector-bitemporal-v1`.
- [ ] 2.9 New CLI `crates/cortex-cli/src/bin/cortex-ops/migrate_bitemporal.rs` — idempotent, dry-run capable.
- [ ] 2.10 Backfill rules: `valid_from = recorded_at OR created_at OR 1970-01-01`; `valid_to = NULL`; `branch_id = "main"`; `lifecycle` derived from existing status fields.
- [ ] 2.11 For ADRs with `status = superseded`: set `valid_to = next_adr.valid_from`, `superseded_at = now`, write `SUPERSEDES` edge.
- [ ] 2.12 Create `branch:main` per project.
- [ ] 2.13 Migration report: per-project counts of nodes updated, edges created, anomalies flagged.
- [ ] 2.14 New spec `docs/specs/30-bitemporal-schema.md`.

## 3. P2 — Temporal classifier + branch filters in retrieval
- [ ] 3.1 New module `crates/cortex-workers/src/temporal/classifier.rs` implementing the state machine per `docs/analysis/timeline-branching/design.md §2.2`.
- [ ] 3.2 States: `EXPIRED | SUPERSEDED | VALID | TEMPORAL | NOT_YET_VALID | ABANDONED`. Per-state action per design.
- [ ] 3.3 Wire classifier into the fusion lane after BM25+dense+graph fusion, before the cross-encoder reranker (from `phase17_cdc-code-doc-correlation` P2).
- [ ] 3.4 Extend `cortex-config` with `TemporalConfig { enabled, include_history_default, temporal_window_days, temporal_boost }`. Defaults: `enabled = true`, `temporal_window_days = 30`, `temporal_boost = 1.10`.
- [ ] 3.5 Branch filter at every retrieval lane: Meili filter, Vectorizer pre-filter, Nexus graph walk.
- [ ] 3.6 Helper `branch_ancestry_chain(project, branch) -> Vec<branch_id>` shared across lanes.
- [ ] 3.7 Integration tests in `crates/cortex-api/tests/temporal_it.rs` and `branch_filter_it.rs`: synthetic fork/merge/abandon fixtures.
- [ ] 3.8 Eval: CDC harness time-sensitive subset MRR@10 ≥ +10%; no regression on time-insensitive subset.
- [ ] 3.9 Audit events `temporal_classification(entity_id, state, action, reason, as_of, branch)` and `branch_resolution(query, branch, ancestry_chain)`.
- [ ] 3.10 Mark `phase17_cdc-code-doc-correlation` §4 (supersession weighting) as superseded with pointer here.
- [ ] 3.11 New specs `docs/specs/31-temporal-classifier.md` and `docs/specs/32-branches.md`.

## 4. P3 — Public surfaces (CLI / HTTP / MCP)
- [ ] 4.1 New CLI `crates/cortex-cli/src/bin/cortex-ops/timeline.rs`: `cortex timeline <project> [--as-of] [--branch] [--kind] [--from] [--to] [--limit]`.
- [ ] 4.2 New CLI `crates/cortex-cli/src/bin/cortex-ops/branch_cmd.rs`: `cortex branch list|show|create|merge|abandon`.
- [ ] 4.3 New CLI: `cortex query "X" [--as-of] [--branch] [--projects]`, `cortex history <entity-id>`, `cortex supersession <entity-id>`.
- [ ] 4.4 HTTP routes: `GET /v1/timeline/{project}`, `GET/POST /v1/branch/{project}[/{branch}]`, `POST /v1/branch/{project}/{branch}/{merge|abandon}`, `GET /v1/entity/{id}/{history|supersession}`, extended `POST /v1/query` body.
- [ ] 4.5 Backward compatibility: missing `as_of` defaults to `now`; missing `branch` defaults to `"main"`; missing `projects` derives from scope. Existing `/v1/query` callers unchanged.
- [ ] 4.6 MCP tools registered in `crates/cortex-mcp-server/src/tools.rs`: `cortex_timeline`, `cortex_branch_list`, `cortex_branch_show`, `cortex_history`, `cortex_supersession`; extend `cortex_query` schema with optional `as_of`, `branch`, `projects`.
- [ ] 4.7 Integration tests in `crates/cortex-api/tests/timeline_it.rs` and `branch_it.rs`.
- [ ] 4.8 OpenAPI regenerated; MCP schemas pass `mcp validate`.
- [ ] 4.9 New spec `docs/specs/33-timeline-api.md`.

## 5. P4 — Cross-project axis activation
- [ ] 5.1 Backfill `CROSS_PROJECT_REF` edges from `Cargo.toml`, `package.json`, lockfiles, and explicit version mentions in ADRs/docs.
- [ ] 5.2 Cross-project propagation in fusion pipeline per design §2.4: walk edges from top-K, apply temporal classifier with the constraint's `valid_from/valid_to`, fuse with provenance preserved (`source_project`).
- [ ] 5.3 Config: `query.cross_project.enabled` (default false), `query.cross_project.max_hops` (default 1).
- [ ] 5.4 Eval: CDC harness cross-project query subset shows positive delta; provenance per candidate verified.
- [ ] 5.5 New spec `docs/specs/34-cross-project-axis.md`.

## 6. P5 — Pre-thinking bundle additions
- [ ] 6.1 Extend `crates/cortex-pre-thinking/src/bundle.rs` schema with `timeline_window`, `supersession_overlay`, `branch_context` per design §3.4.
- [ ] 6.2 Update section caps + budget logic to include the new sections; audit envelope counts each section's contribution.
- [ ] 6.3 Cross-link from `docs/analysis/prethinking/README.md` to the new sections.
- [ ] 6.4 New spec `docs/specs/35-temporal-pre-thinking.md`.

## 7. P6 — Observability + dashboard
- [ ] 7.1 Audit-event schema additions for `temporal_classification`, `branch_resolution`, `cross_project_propagation`.
- [ ] 7.2 Grafana/observability panels: % of queries with non-`now` `as_of`; % of candidates filtered per classifier state; branch usage distribution; cross-project hit ratio.
- [ ] 7.3 Weekly digest exported from the dashboard.

## 8. Cross-cutting
- [ ] 8.1 Knowledge capture after each phase: `rulebook_knowledge_add pattern` for observed metric deltas and load-bearing config values; `rulebook_knowledge_add anti-pattern` for any approach that regressed.
- [ ] 8.2 Memory: `rulebook_memory_save` migration counts (P1), classifier deltas (P2), cross-project hit ratios (P4) for next-session continuity.
- [ ] 8.3 Coordinate with `phase17_cdc-code-doc-correlation` P2 (cross-encoder rerank): shared fusion-pipeline refactor; sequence so rerank lands first, then classifier wedges in front of rerank.

## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation. (Specs 30–35 + CHANGELOG entries per phase + update `docs/analysis/prethinking/README.md` and `docs/analysis/timeline-branching/README.md` with shipped status.)
- [ ] 99.2 Write tests covering the new behavior. (Integration tests per phase; classifier state-machine unit tests; migration dry-run tests.)
- [ ] 99.3 Run tests and confirm they pass. (`cargo check --workspace && cargo clippy --workspace -- -D warnings && cargo test --workspace` clean; `cortex-eval --suite retrieval` meets gates with classifier enabled.)
