# 08 — Task backlog (active and pending)

State pulled from [.rulebook/tasks/](../../../.rulebook/tasks/) and [.rulebook/STATE.md](../../../.rulebook/STATE.md).

## Active

### `phase1_classifier_worker` — in-progress (per metadata)

[tasks.md](../../../.rulebook/tasks/phase1_classifier_worker/tasks.md) shows **all 6 sections checked off**. The metadata `status: in-progress` and STATE.md "22/273 items" appear to be **stale** — the human-readable checklist is fully closed and the [related learning](../../../.rulebook/learnings/2026-04-27T00-32-26-end-to-end-cortex-bootstrap-on-the-cortex-repo-pipeline-gaps-surfaced.md) confirms end-to-end completion. The numerator/denominator in STATE.md likely counts something other than the visible checkboxes (sub-spec items?).

**Action:** archive via `rulebook_task_archive` after confirming the mandatory tail (docs + tests + verify) is satisfied.

## Pending (12 tasks, all `status: pending`)

In rough creation-time order, with the recommended priority I would assign each:

| Rec | Task                                                                                                            | Why                                                                                              | Phase |
|-----|-----------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------|-------|
| **P1** | [phase4a — fulltext fan-out parity + stale Meili cleanup](../../../.rulebook/tasks/phase4a_fulltext_fanout_parity_and_stale_meili_cleanup/) | Today the keyword lane is single-repo; closes the most visible asymmetry. | 4 |
| **P1** | [phase4d — indexing consistency doctor](../../../.rulebook/tasks/phase4d_indexing_consistency_doctor/)            | Test harness for phase4a/b/c. Without it, regressions are caught by accident. | 4 |
| **P2** | [phase4b — bootstrap orchestrator for remaining repos](../../../.rulebook/tasks/phase4b_bootstrap_resume_remaining_repos/) | Extends coverage from 3 to 17 repos. Depends on phase4a so the keyword lane catches up too. | 4 |
| **P2** | [phase4c — graph richer edges (DEFINES, etc.)](../../../.rulebook/tasks/phase4c_graph_richer_edges_defines/)     | The `symbol` field is already produced upstream; only the mapper needs to emit Symbol+DEFINES. High leverage / low cost. | 4 |
| **P3** | [phase2_dashboard](../../../.rulebook/tasks/phase2_dashboard/)                                                  | Likely the umbrella task that the phase2a-h sub-tasks closed. Confirm or archive.                | 2 |
| **P3** | [phase2_rulebook_artifact_indexer](../../../.rulebook/tasks/phase2_rulebook_artifact_indexer/)                  | Indexes Rulebook artifacts as first-class Cortex events.                                          | 2 |
| **P3** | [phase2g_dashboard_enriched_metrics](../../../.rulebook/tasks/phase2g_dashboard_enriched_metrics/)              | Tool-call analytics depth.                                                                         | 2 |
| **P3** | [phase2h_dashboard_decision_chain_and_graph_richness](../../../.rulebook/tasks/phase2h_dashboard_decision_chain_and_graph_richness/) | Dashboard-side surface for richer graph (depends on phase4c).                                  | 2 |
| **P4** | [phase3_gui_multi_connection](../../../.rulebook/tasks/phase3_gui_multi_connection/)                            | Unblocks remote/multi-environment use of the dashboard.                                           | 3 |
| **P4** | [phase2f_dashboard_auth](../../../.rulebook/tasks/phase2f_dashboard_auth/)                                      | Required before exposing the dashboard outside `127.0.0.1`.                                       | 2 |
| **P5** | [phase3_tool_call_hash_preview](../../../.rulebook/tasks/phase3_tool_call_hash_preview/)                        | Operational ergonomics — proposal not yet fleshed out (template only).                            | 3 |
| **P5** | (governance MVP — not yet a task)                                                                                | See [06-governance-gap.md](06-governance-gap.md). Should be created and prioritized P3 once phase4a/d land. | 2 |

### Priority rationale

**P1 (do first)**: phase4a + phase4d together close *and prove* the most visible drift. Without phase4d, phase4a's fix can silently regress the next time we touch the worker.

**P2**: phase4b + phase4c extend coverage and topology depth respectively. phase4c is *especially* high-leverage because the data is already produced — only one crate (`cortex-graph`) needs to change.

**P3**: closes Phase-2 dashboard scope and the Rulebook artifact indexer. None are blockers but each closes a visible gap in observable state.

**P4**: GUI multi-connection + auth become important once Cortex is deployed beyond a single laptop. Today they are not blockers; tomorrow they become deployment blockers in tandem.

**P5**: low priority either because the spec is a template (phase3_tool_call_hash_preview) or because it is gated on prerequisites (governance MVP needs phase4 done first).

## What's missing from the backlog

Reading the analysis, several gaps have **no task** yet:

1. **Retrieval-quality evaluation harness.** Recall@k / MRR / Jaccard against a curated query set. Phase-4 hardening line item per the roadmap; should be a concrete task.
2. **Vectorizer post-upsert verification.** The SDK reports `total_failed=4-5` per batch; we don't have a "did it actually land?" check. Could be folded into phase4d or its own task.
3. **MVP governance (laws + violations + trust).** See [06](06-governance-gap.md). Concrete task should be drafted.
4. **ADR backfill for load-bearing implicit decisions.** See [07](07-quality-and-tests.md).
5. **Coverage report wiring** (`cargo tarpaulin` / `llvm-cov`). The 95% bar is asserted in AGENTS.md but no tooling enforces it.
6. **Embedder JWT auto-login.** Listed as "follow-up" in the 2026-04-27 learning but no task. Small, deserves to be a task.

## Task hygiene observations

- 12 pending tasks is a lot of WIP. The current Active task (phase1_classifier_worker) appears already closed but not archived — this distorts STATE.md.
- Several phase4 tasks were created within minutes of each other (2026-04-27 22:45) — looks like the audit drove a backlog-flush. Healthy reflex.
- One task ([phase3_tool_call_hash_preview](../../../.rulebook/tasks/phase3_tool_call_hash_preview/proposal.md)) has a template-only proposal (`[Explain why this change is needed]`). Either flesh out or close.

**Action:** archive `phase1_classifier_worker`, complete the `phase3_tool_call_hash_preview` proposal or close it, and create the four "missing from backlog" tasks above. That gets the backlog to a clean 14 tracked items, all actionable.
