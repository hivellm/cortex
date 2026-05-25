# Proposal: phase18_tlb-timeline-branching

## Why

Cortex stores knowledge across 7+ HiveLLM projects (Cortex, Nexus, Vectorizer, Synap, Lexum, Rulebook, …) but treats facts as flat and current. A fix shipped last week and an ADR from a year ago occupy the same semantic space; a query for "how does X work?" can surface an obsolete decision because no temporal axis demotes it. There is also no concept of a **branch** — an exploration that forked off, was tried, and was either merged or abandoned. The literature calls this failure mode "RAG is blind to time" (T-GRAG arXiv 2508.01680; Towards Data Science 2025) — and it is exactly what the maintainer (André) has been reporting: "the data does not produce anything actually relevant." Bitemporal modeling (XTDB, Datomic, Zep) and temporal-classifier reranking are the textbook fixes; they typically lift time-sensitive query MRR by 10–25% and cut temporal hallucination by 30–60% on production deployments.

## What Changes

This umbrella task lands a temporal + branching dimension across Cortex's data model and retrieval surfaces, in 7 phases:

- **P0 — Design ADRs.** Six ADRs lock the open design questions (time precision, branch ID scheme, cross-project default, branch-merge representation, retention, consolidation vs supersession semantics).
- **P1 — Schema, migration, backfill.** Bitemporal columns (`valid_from`, `valid_to`, `recorded_at`, `superseded_at`) + `project_id` + `branch_id` + `lifecycle` on every entity. New `branch` node, new `timeline_event` entity. Seven new edge kinds (`SUPERSEDES`, `OBSOLETES`, `CONFLICTS_WITH`, `FORKED_FROM`, `MERGED_INTO`, `EVOLVES_FROM`, `CROSS_PROJECT_REF`). Idempotent backfill imputing `valid_from = recorded_at`, `valid_to = NULL`, `branch_id = "main"` for existing data.
- **P2 — Temporal classifier + branch filters.** State-machine classifier (`EXPIRED | SUPERSEDED | VALID | TEMPORAL | NOT_YET_VALID | ABANDONED`) inserted after fusion, before rerank. Branch filter honored at every retrieval lane (Meili, Vectorizer, Nexus). **Supersedes CDC-001 P4.**
- **P3 — Public surfaces.** New CLI (`cortex timeline`, `cortex branch …`, `cortex history`, `cortex supersession`), HTTP routes (`/v1/timeline/{project}`, `/v1/branch/{project}`, `/v1/entity/{id}/history`), MCP tools (`cortex_timeline`, `cortex_branch_*`, `cortex_history`, extended `cortex_query`).
- **P4 — Cross-project axis.** Activate `CROSS_PROJECT_REF` traversal in retrieval (opt-in via `query.cross_project.enabled`). Backfill edges from `Cargo.toml` / `package.json` / lockfiles / explicit ADR mentions.
- **P5 — Pre-thinking bundle additions.** New bundle sections: `timeline_window`, `supersession_overlay`, `branch_context`. Section caps prevent unbounded growth.
- **P6 — Observability + dashboard.** Audit events for `temporal_classification`, `branch_resolution`, `cross_project_propagation`. Dashboard panels for adoption + effectiveness signals.

## Impact

- **Affected specs:** New `docs/specs/30-bitemporal-schema.md`, `31-temporal-classifier.md`, `32-branches.md`, `33-timeline-api.md`, `34-cross-project-axis.md`, `35-temporal-pre-thinking.md`.
- **Affected code:** `crates/cortex-workers/src/{temporal,branches,scoring}/`, `crates/cortex-cli/src/bin/cortex-ops/{timeline,branch_cmd,migrate_bitemporal}.rs`, `crates/cortex-api/src/http.rs` (new routes), `crates/cortex-mcp-server/src/tools.rs` (new tools), `crates/cortex-pre-thinking/src/bundle.rs` (new sections), `crates/cortex-config/src/config.rs` (new sections), `crates/cortex-workers/src/fulltext/meili_loader.rs` (filterable attrs).
- **Breaking change:** NO. Schema migrations are additive; existing endpoints default to `as_of = now`, `branch = "main"`; reindexes use versioned aliases for rollback.
- **User benefit:** Point-in-time queries; cross-project correlation; superseded ADRs stop polluting top-K; abandoned approaches stop being re-tried by agents; audit answers "what did we know on date X?" exactly.

## Source

`docs/analysis/timeline-branching/` (README, findings, design, execution-plan, references). Cross-references TLB-001 throughout.

## Dependencies

- Hard: `phase14c_golden-set-eval-harness` must be green before P2 lands — every temporal-classifier delta must be measurable.
- Soft: `phase17_cdc-code-doc-correlation` P2 (cross-encoder rerank) and TLB P2 (temporal classifier) share the fusion-pipeline refactor; coordinate so both land cleanly. CDC P4 is superseded by TLB P2 once shipped.
