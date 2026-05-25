## 1. P0 — Design ADRs (lock open questions)
- [ ] 1.1 ADR: bitemporal time precision (UTC seconds storage; day-precision defaults for ADR/decision tooling). `rulebook_decision_create`.
- [ ] 1.2 ADR: branch identifier scheme (`(project_id, branch_name)` unique pair; regex constraint).
- [ ] 1.3 ADR: cross-project retrieval default scope (opt-in until eval evidence justifies opt-out).
- [ ] 1.4 ADR: branch-merge representation in retrieval (keep-and-link; classifier handles fold-in).
- [ ] 1.5 ADR: temporal retention and archival (never delete; archive `superseded`/`expired` after 5 years).
- [ ] 1.6 ADR: consolidation vs supersession edge semantics (`EVOLVES_FROM` vs `SUPERSEDES` rules).

## 2. P1 — Schema, migration, backfill
- [ ] 2.1 Nexus schema migration: add bitemporal columns + `project_id` + `branch_id` + `lifecycle` to every entity node label.
- [ ] 2.2 New `branch` node label with `id, project_id, name, parent_branch_id, fork_point_event_id, fork_valid_time, status, merge_strategy, merge_point_event_id, abandonment_reason, created_at, created_by`.
- [ ] 2.3 New `timeline_event` entity with `id, project_id, branch_id, valid_time, recorded_at, kind, title, summary, ref_entity_id, ref_entity_kind, tags`.
- [ ] 2.4 New edge kinds: `SUPERSEDES`, `OBSOLETES`, `CONFLICTS_WITH`, `FORKED_FROM`, `MERGED_INTO`, `EVOLVES_FROM`, `CROSS_PROJECT_REF`.
- [ ] 2.5 Indexes on `(project_id, valid_to)`, `(project_id, branch_id, valid_from)`, `(superseded_at)`.
- [ ] 2.6 Meili schema update in `crates/cortex-workers/src/fulltext/meili_loader.rs`: add filterable attrs `project_id`, `branch_id`, `valid_from_unix`, `valid_to_unix`, `superseded_at_unix`, `lifecycle`.
- [ ] 2.7 Vectorizer payload extension to carry the same fields.
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
