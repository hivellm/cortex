# Cortex — System Analysis Index

> **Author:** automated audit · **Source data:** repo snapshot at commit `c41dab0` + bootstrap state + rulebook learnings (most recent dated 2026-04-27).
> **Scope:** end-to-end review of the Cortex platform — what is implemented, what is producing results, what is degraded, and what to fix next.
> **Audience:** project maintainers and anyone proposing changes to the indexing pipeline, GUI, or governance layer.
> **Status:** draft. Indexed as first-class `Analysis` entities by [phase4e_bootstrap_analysis_promotion](../../../.rulebook/tasks/phase4e_bootstrap_analysis_promotion/proposal.md) — `cortex-bootstrap` emits an `analysis.imported` event per file under this directory, routed to `cortex-Cortex-analyses` in Meili + Vectorizer and to `(:Analysis)-[:ANALYZES]->(:Repo)` in Nexus.

This index points to ten focused files. Each file is self-contained: read in order for the full picture, or jump straight to the area you own.

| #  | File                                                            | Question it answers                                                  |
|----|-----------------------------------------------------------------|----------------------------------------------------------------------|
| 01 | [overview](01-overview.md)                                      | What does Cortex look like today, in one screen?                     |
| 02 | [pipeline-state](02-pipeline-state.md)                          | Capture → classify → embed → graph → fulltext: which legs are live?  |
| 03 | [data-quality](03-data-quality.md)                              | What is actually in each backend right now?                          |
| 04 | [integrations](04-integrations.md)                              | Vectorizer / Nexus / Meili / Synap / Claude — what is healthy?       |
| 05 | [gui-and-api](05-gui-and-api.md)                                | Dashboard surface area, API routes, observed gaps.                   |
| 06 | [governance-gap](06-governance-gap.md)                          | Laws, violations, trust score — the Phase-2 hole.                    |
| 07 | [quality-and-tests](07-quality-and-tests.md)                    | Test coverage, captured patterns, captured anti-patterns.            |
| 08 | [task-backlog](08-task-backlog.md)                              | Active and pending Rulebook tasks, prioritized.                      |
| 09 | [risks-and-debt](09-risks-and-debt.md)                          | Recurring drifts, tech debt, structural traps.                       |
| 10 | [improvement-roadmap](10-improvement-roadmap.md)                | Prioritized recommendations with rationale and effort estimates.     |
| 11 | [platform-vision-analysis](11-platform-vision-analysis.md)      | **June 2026** — full inventory + tool validation + roadmap to general assistance platform. |
| 12 | [live-audit-2026-06-09](12-live-audit-2026-06-09.md)            | **Live audit** — 10 bugs ativos encontrados via queries diretas ao stack real. |

## Headline findings (April 2026)

1. **Specs 01–12 + 18 are flagged 🟢; specs 13–17 are still 🟡.** The indexing/retrieval/pre-thinking loop is structurally complete; governance, deep analysis, multi-adapter, and dashboard polish are not.
2. **Pipeline lights up end-to-end on the `Cortex` repo.** Bootstrap → classifier-worker → embedder/graph/fulltext → query API works. The classifier-worker bridge that gated this for weeks landed on 2026-04-27.
3. **Data fan-out is asymmetric across backends.** Vectorizer has 3 repos (~128k vectors), Nexus has 3 repos (~3.6k Artifact nodes), Meilisearch has only **1 repo** (589 docs). The keyword lane is effectively single-repo despite the worker being live.
4. **Two upstream-SDK drifts still bite.** Vectorizer SDK 3.0.3 `upsert` reports `total_failed=4-5` per batch with `vector_count=0`; Nexus 1.15.0 silently drops `UNWIND` writes when transactions don't commit through the SDK driver path.
5. **Graph topology is shallow.** Only `IN_REPO` and `REMEMBERS` edges exist. The chunker emits a `symbol` field but `cortex-graph` drops it, so `(:Symbol)-[:DEFINES]->(:Artifact)` does not exist — the graph lane cannot answer symbol-level questions.
6. **No automated consistency check.** All structural drifts above were caught by hand-curling the four backends and decompressing the event archive in Python. Phase4d (`cortex doctor consistency`) is the planned automation.
7. **Phase 2 governance is unbuilt.** No law detector sandbox, no enforcement engine, no trust score. The dashboard already renders law/violation tables — they read from Meili-loaded fixtures, not a live engine.
8. **Sonnet-backed session analyzer just landed (commit `a62fcbd`).** This is the first cross-event analysis layer — sits above per-event Haiku classification.

If you read only one follow-up file, read [10-improvement-roadmap.md](10-improvement-roadmap.md).

## Update — June 2026

See [11-platform-vision-analysis.md](11-platform-vision-analysis.md) for a full updated inventory (14 crates, MCP tool validation, governance gaps, and the roadmap for expanding Cortex into a general company assistance platform covering both code and business knowledge).
