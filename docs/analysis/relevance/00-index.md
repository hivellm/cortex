# Cortex — Relevance Analysis (2026-04-28)

> **Author:** automated audit (analysis skill, parent: phase5a tasks-backend session) · **Source data:** repo snapshot at commit `6e8c984` + on-disk lane state + the prior `docs/analysis/cortex/` audit dated 2026-04-28.
> **Scope:** end-to-end *relevance* of the bundles served by `POST /v1/query` and `cortex_pre_thinking` (MCP). Why results feel "thin" / "off-topic" today, and the smallest set of changes that turns them useful.
> **Audience:** anyone working on retrieval, fusion, intent routing, or pre-thinking bundle quality.
> **Relationship to `docs/analysis/cortex/`:** that audit catalogs the system top-to-bottom; this analysis layers a *relevance lens* on the same evidence and isolates eight gaps that, addressed in order, flip bundles from weak to useful for pre-thinking and the MCP `cortex_query` consultative path.

## Files

| #  | File                                          | Question it answers                                                                 |
|----|-----------------------------------------------|-------------------------------------------------------------------------------------|
| 01 | [findings](01-findings.md)                    | F-001..F-008 — the eight structural relevance gaps, each with evidence + impact.    |
| 02 | [execution-plan](02-execution-plan.md)        | R1..R4 — phased ordering: stop the bleed, fix lanes, measure, polish.               |
| 03 | [knowledge-and-memory](03-knowledge-and-memory.md) | Patterns / anti-patterns to add to `.rulebook/knowledge/`; memory entry to save.   |
| 04 | [files-and-references](04-files-and-references.md) | Read-paths the implementer needs (relevance pipeline + tracked tasks).             |

## Headline findings

1. **The retrieval surface is structurally complete.** Fan-out, RRF, four intents, three live lanes, deterministic Markdown formatter, audit envelope, MCP wrapper — all present. Bundles still feel weak because of eight specific gaps, not because the architecture is wrong.
2. **Three of eight gaps are already tracked.** `phase4a_fulltext_fanout_parity_and_stale_meili_cleanup` (F-001), `phase4c_graph_richer_edges_defines` (F-002), and the R10 scope-resolution risk in `09-risks-and-debt.md` (F-003) cover the structural-coverage half of the problem.
3. **Five gaps are new.** Scope-default enforcement (F-003), live-lane overlay extras (F-007), score-aware RRF (F-005), intent-table coverage (F-006), query rewriting (F-004), recall@k harness (F-008) — each is small but together they account for most of the "Cortex returned nothing useful" complaints.
4. **Two 1-day fixes have outsized leverage.** Defaulting `Scope.repo` server-side (F-003) and stamping `decision_id`/`turn_id`/`law_id` on live `LaneHit.extras` (F-007) flip overlays + scope behaviour from broken to working in production. Sequence them **before** `phase4a` so the coverage delta from `phase4a` is observable.
5. **Relevance is unmeasured.** No labeled query set, no recall@k / MRR harness. Without F-008 every "the bundle feels weak" conversation stays qualitative, fixes can't be ranked, and regressions land silently. Block further "make Cortex smarter" investment until the harness lands.

## How to use this analysis

- Implementers: read [02-execution-plan.md](02-execution-plan.md) — it sequences the work and links each step to the finding it closes.
- Maintainers: read [01-findings.md](01-findings.md) for the evidence trail (file:line citations) before challenging or re-prioritising.
- Anyone capturing decisions: [03-knowledge-and-memory.md](03-knowledge-and-memory.md) lists the patterns/anti-patterns/ADR candidates this analysis surfaces.

## Where this should land in the rulebook

This directory is consumed automatically by `phase4e_bootstrap_analysis_promotion` — `cortex-bootstrap` emits an `analysis.imported` event per file, routed to `cortex-Cortex-analyses` in Meili + Vectorizer and to `(:Analysis)-[:ANALYZES]->(:Repo)` in Nexus. No manual indexing needed; the next bootstrap run picks these files up.

If the rulebook MCP is reachable, follow up with `rulebook_analysis_create` to mint a stable Analysis id; otherwise the file slug `relevance` is the canonical reference and the IDs F-001..F-008 are stable across rewrites.
