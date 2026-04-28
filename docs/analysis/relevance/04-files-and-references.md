# 04 — Files and references

Authoritative read-paths for anyone implementing R1..R4. Each file is annotated with the finding(s) it bears on so an implementer reading top-to-bottom understands *why* this file matters before opening it.

## Relevance pipeline (cortex-api)

| File | Why it matters | Findings |
|------|---------------|----------|
| [`crates/cortex-api/src/strategies.rs`](../../../crates/cortex-api/src/strategies.rs) | Intent → plan; defines `repo_scoped` fallback to `UNKNOWN_REPO_SLUG`; defines per-intent fan-out. | F-003, F-006 |
| [`crates/cortex-api/src/orchestrator.rs`](../../../crates/cortex-api/src/orchestrator.rs) | Fan-out, fusion call, overlay derivation (`derive_decisions`, `derive_similar_turns`). | F-007 |
| [`crates/cortex-api/src/fusion.rs`](../../../crates/cortex-api/src/fusion.rs) | RRF (`1/(K+rank)`) — score-blind today. | F-005 |
| [`crates/cortex-api/src/meili_lane.rs`](../../../crates/cortex-api/src/meili_lane.rs) | Meili lane projection — captures `_rankingScore` but loses overlay extras. | F-001, F-007 |
| [`crates/cortex-api/src/vectorizer_lane.rs`](../../../crates/cortex-api/src/vectorizer_lane.rs) | Vectorizer lane projection — same overlay-extras gap. | F-007 |
| [`crates/cortex-api/src/service.rs`](../../../crates/cortex-api/src/service.rs) | `/v1/query` entry — where to enforce scope resolution. | F-003 |
| [`crates/cortex-api/src/types.rs`](../../../crates/cortex-api/src/types.rs) | `Scope`, `QueryRequest`, `LaneHit` shapes. | F-003, F-005, F-007 |
| [`crates/cortex-api/src/audit.rs`](../../../crates/cortex-api/src/audit.rs) | Carries `query_id` across fan-out — input to the recall harness. | F-008 |
| [`crates/cortex-api/src/analyzer.rs`](../../../crates/cortex-api/src/analyzer.rs) | Sonnet wrapper — reusable for query rewriting. | F-004 |

## Pre-thinking pipeline

| File | Why it matters | Findings |
|------|---------------|----------|
| [`crates/cortex-pre-thinking/src/pipeline.rs`](../../../crates/cortex-pre-thinking/src/pipeline.rs) | Pre-thinking orchestration — forwards `user_prompt` verbatim. | F-004 |
| [`crates/cortex-pre-thinking/src/intent_select.rs`](../../../crates/cortex-pre-thinking/src/intent_select.rs) | Intent table — narrow keyword coverage. | F-006 |
| [`crates/cortex-pre-thinking/src/scope.rs`](../../../crates/cortex-pre-thinking/src/scope.rs) | Scope derivation from cwd — only pre-thinking benefits today. | F-003 |
| [`crates/cortex-pre-thinking/src/formatter.rs`](../../../crates/cortex-pre-thinking/src/formatter.rs) | Bundle Markdown assembly — empty-overlay sections render as blanks. | F-007 (downstream effect) |
| [`crates/cortex-pre-thinking/src/budget.rs`](../../../crates/cortex-pre-thinking/src/budget.rs) | Trim ladder — wastes budget on irrelevant overlays today. | F-006 (downstream effect) |

## MCP surface

| File | Why it matters | Findings |
|------|---------------|----------|
| [`crates/cortex-mcp-server/src/tools.rs`](../../../crates/cortex-mcp-server/src/tools.rs) | `cortex_query` / `cortex_pre_thinking` — direct callers without a scope. | F-003 |

## Indexing legs (root cause sources)

| File | Why it matters | Findings |
|------|---------------|----------|
| [`crates/cortex-graph/src/mapper.rs`](../../../crates/cortex-graph/src/mapper.rs) | Drops `symbol` field; reason graph topology is shallow. | F-002 |
| [`crates/cortex-fulltext/src/routing.rs`](../../../crates/cortex-fulltext/src/routing.rs) | Routing is correct — root cause for F-001 is consumer state, not routing. | F-001 |

## Companion analyses (read alongside)

| File | What it adds |
|------|--------------|
| [`docs/analysis/cortex/02-pipeline-state.md`](../cortex/02-pipeline-state.md) | Leg-by-leg state of capture → classify → embed → graph → fulltext. |
| [`docs/analysis/cortex/03-data-quality.md`](../cortex/03-data-quality.md) | Backend counts (Meili 589 / Vectorizer 128k / Nexus 3.6k) — the numeric foundation for F-001 + F-002. |
| [`docs/analysis/cortex/09-risks-and-debt.md`](../cortex/09-risks-and-debt.md) | R3 (coverage opacity), R9 (no relevance harness), R10 (scope resolution). |
| [`docs/analysis/cortex/10-improvement-roadmap.md`](../cortex/10-improvement-roadmap.md) | Phase4a/4c/4d sequencing + ADR backfill list. |

## Already-tracked tasks that close subsets of these findings

| Task | Closes |
|------|--------|
| [`.rulebook/tasks/phase4a_fulltext_fanout_parity_and_stale_meili_cleanup`](../../../.rulebook/tasks/phase4a_fulltext_fanout_parity_and_stale_meili_cleanup/proposal.md) | F-001 |
| [`.rulebook/tasks/phase4c_graph_richer_edges_defines`](../../../.rulebook/tasks/phase4c_graph_richer_edges_defines/proposal.md) | F-002 |
| [`.rulebook/tasks/phase4d_indexing_consistency_doctor`](../../../.rulebook/tasks/phase4d_indexing_consistency_doctor/proposal.md) | Verification surface for F-001 / F-002. |

## New tasks to open (in execution order)

| Slug suggestion | Phase | Closes |
|-----------------|-------|--------|
| `phase5c_relevance_scope_default_enforce` | R1.1 | F-003 |
| `phase5d_relevance_lane_extras_contract` | R1.2 | F-007 |
| `phase5e_relevance_score_aware_rrf` | R2.5 | F-005 |
| `phase5f_relevance_intent_table_expansion` | R2.6 | F-006 |
| `phase5g_relevance_recall_mrr_harness` | R3.7 | F-008 |
| `phase5h_relevance_query_rewriting_pre_pass` | R3.8 | F-004 |

`phase5a_dashboard_tasks_backend` (just shipped) and `phase5b_gui_tasks_view` (next) are unrelated to this analysis but live in the same `phase5` family — the sequence above continues from `phase5c`.
