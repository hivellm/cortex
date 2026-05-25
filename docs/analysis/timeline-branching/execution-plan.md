# Execution Plan — Timeline & Branching

> **Analysis ID:** TLB-001 / Execution Plan
> **Date:** 2026-05-24
> **Scope:** Phased rollout of timeline + branching across Nexus / Meili / Vectorizer / pre-thinking / API / CLI / MCP. Pending user approval before promotion to Rulebook tasks.
> **Dependency on CDC-001:** TLB Phase 1 should land after CDC Phase 1 (eval harness exists), because temporal-classifier deltas must be measurable.

---

## Sequencing rationale

Order = (1) data model + migration before any retrieval change; (2) classifier + filters before public APIs; (3) public surfaces last because they are the most expensive to change later.

```
Phase 0 — ADRs that fix design questions     (1 week)
Phase 1 — Schema, migration, backfill        (2–3 weeks)
Phase 2 — Temporal classifier in retrieval   (1–2 weeks, parallel: branch graph wiring)
Phase 3 — Public surfaces (CLI, API, MCP)    (2–3 weeks)
Phase 4 — Cross-project axis activation      (1–2 weeks)
Phase 5 — Pre-thinking bundle additions      (1 week)
Phase 6 — Observability + dashboard          (1 week)
```

Total estimated calendar: 9–13 weeks across phases. Phases 2 and 3 can overlap once Phase 1 lands.

---

## Phase 0 — Resolve open design questions (ADRs)

**Goal.** Lock the answers to the six open design questions in `design.md §6` before code is written.

**Deliverables.**
- ADR `Bitemporal time precision in Cortex` — confirm second precision, document tooling defaults.
- ADR `Branch identifier scheme` — `(project_id, branch_name)` unique pair, regex constraint.
- ADR `Cross-project retrieval default scope` — opt-in until eval evidence justifies opt-out.
- ADR `Branch merge representation in retrieval` — keep-and-link; classifier handles fold-in.
- ADR `Temporal retention and archival` — never delete; archive after 5 years.
- ADR `Consolidation vs. supersession` — `EVOLVES_FROM` vs `SUPERSEDES` rules.

**Acceptance criteria.** All six ADRs in `accepted` status; cross-referenced from `design.md`.

**Effort.** 1 week. **Tier.** Foundation. **Depends on.** — (paper design only).

---

## Phase 1 — Schema, migration, backfill

### Step 1.1 — Nexus schema migration

**Deliverables.**
- Migration script adding bitemporal columns + `project_id` + `branch_id` + `lifecycle` to every entity node label.
- New `branch` node label with the schema from `design.md §1.2`.
- New edge kinds: `SUPERSEDES`, `OBSOLETES`, `CONFLICTS_WITH`, `FORKED_FROM`, `MERGED_INTO`, `EVOLVES_FROM`, `CROSS_PROJECT_REF`.
- Indexes per `design.md §4.1`.
- Idempotent re-runnable migration in `crates/cortex-cli/src/bin/cortex-ops/migrate_bitemporal.rs`.

**Acceptance criteria.**
- Migration runs on a snapshot of production Nexus without data loss.
- Every existing entity has `valid_from`, `valid_to = NULL`, `branch_id = "main"`, `lifecycle = active`.
- Every project has a `branch:main` node.
- Schema diff validated against `design.md`.

**Effort.** 1 week. **Tier.** Foundation. **Depends on.** Phase 0.

### Step 1.2 — Meili and Vectorizer payload migration

**Deliverables.**
- Meili: add `project_id`, `branch_id`, `valid_from_unix`, `valid_to_unix`, `superseded_at_unix`, `lifecycle` as filterable attributes via `crates/cortex-workers/src/fulltext/meili_loader.rs` schema update.
- Vectorizer: extend payload struct in the dense embedding writer to carry the same fields.
- Re-ingest existing corpus in versioned indexes (`cortex-meili-bitemporal-v1`, `cortex-vector-bitemporal-v1`) to allow rollback.

**Acceptance criteria.**
- New indexes contain bitemporal payloads for every document.
- Old indexes preserved as `*-prev` until cutover.
- Cutover documented; alias swap is the rollback unit.

**Effort.** 1 week. **Tier.** Foundation. **Depends on.** 1.1.

### Step 1.3 — Bitemporal backfill of existing entities

**Deliverables.**
- `cortex-ops backfill-bitemporal --project <p>` command.
- For ADRs with explicit supersession metadata: emit `SUPERSEDES` edges and `valid_to` cuts per `design.md §4.4`.
- Dry-run mode that prints diffs before writing.
- Backfill report: per-project counts of nodes updated, edges created, anomalies flagged.

**Acceptance criteria.**
- Dry-run on production data produces a report with zero unexplained anomalies.
- Full run reduces ADR retrieval staleness by a measurable amount on the CDC eval harness.

**Effort.** 1 week. **Tier.** Foundation. **Depends on.** 1.1, 1.2, CDC-001 Phase 1.

---

## Phase 2 — Temporal classifier + branch filters in retrieval

### Step 2.1 — Temporal classifier as a fusion-pipeline stage

**Deliverables.**
- New module `crates/cortex-workers/src/temporal/classifier.rs` implementing the state machine in `design.md §2.2`.
- Wired into the fusion lane after BM25 + dense + graph fusion, before the cross-encoder reranker (from CDC-001 Step 2.1 if landed; otherwise before scoring).
- Config schema additions in `crates/cortex-config/`:
  - `temporal.enabled: bool` (default true)
  - `temporal.include_history_default: bool` (default false)
  - `temporal.temporal_window_days: u32` (default 30, controls TEMPORAL state boost)
  - `temporal.temporal_boost: f32` (default 1.10)

**Acceptance criteria.**
- CDC eval harness shows MRR@10 ≥ +10% on the time-sensitive subset (queries where ground truth is the *current* version of an evolving entity).
- No regression on the time-insensitive subset.
- Audit events emitted per classified candidate.

**Effort.** 1 week. **Tier.** A. **Depends on.** 1.1, 1.2, CDC-001 Phase 1.

### Step 2.2 — Branch filter at every retrieval lane

**Deliverables.**
- Meili filters honor `branch_id` and walk branch ancestry when requested.
- Vectorizer pre-filter narrows candidates by `(project_id, branch_id)` before ANN.
- Nexus graph walks respect branch when traversing.
- Helper `branch_ancestry_chain(project, branch) -> Vec<branch_id>` shared across lanes.

**Acceptance criteria.**
- Query on `branch = "feat/X"` returns main facts up to fork point + feat/X facts after.
- Query on `branch = "main"` excludes branch-specific facts.
- Tested with synthetic fork + merge + abandon fixtures.

**Effort.** 1 week. **Tier.** A. **Depends on.** 1.1, 1.2, 2.1.

---

## Phase 3 — Public surfaces (CLI, HTTP API, MCP)

### Step 3.1 — CLI timeline + branch commands

**Deliverables.**
- New CLI subtree `crates/cortex-cli/src/bin/cortex-ops/timeline.rs` + `branch_cmd.rs` implementing the commands in `design.md §3.1`.
- Pretty-print and `--json` output modes.
- Tests covering each command on the synthetic fixtures.

**Acceptance criteria.**
- `cortex timeline cortex --as-of 2026-03-01` returns the expected events.
- `cortex branch create cortex --from main@2026-03-01 --name feat/spec-12` creates a branch correctly.
- All commands documented in CLI help.

**Effort.** 1 week. **Tier.** B. **Depends on.** Phase 2.

### Step 3.2 — HTTP API additions

**Deliverables.**
- New routes in `crates/cortex-api/src/http.rs` per `design.md §3.2`.
- OpenAPI documentation regenerated.
- Integration tests in `crates/cortex-api/tests/timeline_it.rs` and `branch_it.rs`.

**Acceptance criteria.**
- All routes return the documented payloads.
- Backward compatibility: existing `/v1/query` calls without `as_of` / `branch` continue to work exactly as before.

**Effort.** 1 week. **Tier.** B. **Depends on.** Phase 2.

### Step 3.3 — MCP tools

**Deliverables.**
- New MCP tools registered in `crates/cortex-mcp-server/src/tools.rs`:
  - `cortex_timeline`, `cortex_branch_list`, `cortex_branch_show`, `cortex_history`, `cortex_supersession`.
- Extended `cortex_query` with optional `as_of`, `branch`, `projects` args.
- Tool JSON schemas documented.

**Acceptance criteria.**
- MCP tools callable from a Claude Code session against a local Cortex stack.
- Schemas pass `mcp validate`.
- Example traces of agent queries showing temporal grounding.

**Effort.** 1 week. **Tier.** B. **Depends on.** 3.2.

---

## Phase 4 — Cross-project axis activation

**Goal.** Enable `CROSS_PROJECT_REF` traversal in retrieval.

**Deliverables.**
- Backfill `CROSS_PROJECT_REF` edges from existing `Cargo.toml`, `package.json`, lockfiles, and explicit version mentions in ADRs.
- Cross-project propagation in fusion pipeline per `design.md §2.4`.
- Opt-in config: `query.cross_project.enabled` (default false), `query.cross_project.max_hops` (default 1).

**Acceptance criteria.**
- Query against `cortex` with `projects=[cortex, nexus]` surfaces the Nexus 2.1 external-IDs change when relevant.
- Cross-project hits show `source_project` in provenance.
- CDC eval harness shows positive delta on cross-project query subset.

**Effort.** 1–2 weeks. **Tier.** B. **Depends on.** Phase 2.

---

## Phase 5 — Pre-thinking bundle additions

**Goal.** Surface temporal/branch context in the bundle so the LLM sees grounding before the first token.

**Deliverables.**
- Extend `crates/cortex-pre-thinking/src/bundle.rs` schema with `timeline_window`, `supersession_overlay`, `branch_context` per `design.md §3.4`.
- Update section caps and budget logic to include the new sections.
- Document the new sections in `docs/analysis/prethinking/` (cross-link from there).

**Acceptance criteria.**
- Bundle includes the new sections by default for scopes that have meaningful temporal data.
- Section caps prevent unbounded growth.
- Audit envelope counts each new section's contribution.

**Effort.** 1 week. **Tier.** B. **Depends on.** Phase 2.

---

## Phase 6 — Observability + dashboard

**Goal.** Make adoption and effectiveness of temporal/branch features visible.

**Deliverables.**
- Audit-event schema additions for `temporal_classification`, `branch_resolution`, `cross_project_propagation`.
- Grafana / observability dashboard panels:
  - % of queries with non-`now` `as_of`.
  - % of candidates filtered per state (SUPERSEDED, EXPIRED, …).
  - Branch usage distribution.
  - Cross-project hit ratio.
- Weekly digest exported from the dashboard for retro review.

**Acceptance criteria.**
- Dashboard live and populated within 24h of deployment.
- Weekly digest delivered.

**Effort.** 1 week. **Tier.** B. **Depends on.** Phases 2–5.

---

## Cross-cutting concerns

### Migration safety

- Every schema change is **additive**. No destructive column drops.
- Reindex steps use versioned index aliases (`*-bitemporal-v1`) for instant rollback.
- Backfill commands are idempotent and dry-run capable.

### Coordination with CDC-001

- CDC-001 Phase 1 (eval harness) **must** be in place before TLB-001 Phase 2 lands.
- CDC-001 Phase 2 (cross-encoder rerank) and TLB-001 Phase 2 (temporal classifier) should be in the same fusion-pipeline refactor; coordinate via shared module.
- The CDC supersession-weighting Tier-A fix becomes **redundant** once TLB temporal classifier is live — explicitly mark it as superseded in CDC's plan.

### Knowledge capture per phase

After each step:
- `rulebook_knowledge_add pattern` — what worked (e.g., "bitemporal indexes on `(project_id, valid_to)` made point-in-time queries sub-100ms").
- `rulebook_decision_create` — ADRs for architecturally significant choices (already enumerated in Phase 0).
- `rulebook_learn_capture` — implementation insights.
- `rulebook_memory_save` — for the user-facing summary the next session needs.

### Audit and rollback

- Every phase ships behind a feature flag so partial rollouts are safe.
- Roll back = flip flag + (if indexed) point alias to previous index.
- `cortex-audit` is the single source of truth for "what did the classifier decide on this query".

---

## Tasks to create now

Once approved, create two Rulebook task trees:

```
rulebook_task_create
  title: "CDC-001: Code↔Doc Correlation"
  proposal_source: "docs/analysis/code-doc-correlation/"
  phases: [Phase 1, Phase 2, Phase 3, Phase 4]  -- per execution-plan.md in that analysis

rulebook_task_create
  title: "TLB-001: Timeline & Branching"
  proposal_source: "docs/analysis/timeline-branching/"
  phases: [Phase 0 ADRs, Phase 1 schema/migration, Phase 2 classifier+branch, Phase 3 surfaces,
           Phase 4 cross-project, Phase 5 pre-thinking, Phase 6 observability]
```

Both task trees should declare a dependency edge: TLB-001 Phase 2 ▶ depends on ▶ CDC-001 Phase 1.
