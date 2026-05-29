# 21. phase18 §1.4 — Branch merge representation: keep-and-link with classifier-driven fold-in

**Status**: proposed
**Date**: 2026-05-29
**Related Tasks**: phase18_tlb-timeline-branching

## Context

Phase18 introduces branches as first-class retrieval scopes. When a branch is merged back into `main`, the open design question is whether the merge rewrites the branch's facts (changing their `branch_id`) or whether the branch facts stay where they are and the classifier handles the fold-in during retrieval. Rewriting feels intuitive but breaks bitemporal audit (the `recorded_at` column would no longer match the writer event) and forces a full reindex of every branch row on merge.</context>
<parameter name="decision">Keep-and-link. Branch facts are NEVER rewritten on merge. Instead the merge writes a `MERGED_INTO(branch:cortex:feat/x, branch:cortex:main, strategy=accept|partial|discard, merge_point_event_id)` edge to Nexus and stamps the branch node's `status = merged`, `merge_strategy`, `merge_point_event_id`. Retrieval-time semantics live in the temporal classifier (`crates/cortex-workers/src/temporal/classifier.rs`): a `main` retrieval at `as_of >= merge_point.valid_time` walks the `MERGED_INTO` edges and folds in branch facts whose `valid_from <= as_of`, subject to the merge strategy: `accept` includes all branch facts unchanged; `partial` includes only branch facts whose `branch_facts.merge_kept = true` flag is set (operator-curated on merge); `discard` includes none (the branch fact is retained for audit but never surfaces on `main` retrievals). Branch retrieval (`branch_id = feat/x`) is unchanged by the merge — the fact still lives on the original branch for `cortex history` walks.

## Decision

_No decision recorded._

## Alternatives Considered

- Rewrite branch facts to `branch_id = main` on merge (strategy=accept) — rejected because it breaks the `recorded_at` invariant (the original writer event happened on the branch; rewriting falsifies the audit), forces a full reindex per merge, and loses the ability to retrieve the same fact via `--branch feat/x` post-merge
- Duplicate branch facts onto `main` (strategy=accept) — rejected because it doubles storage and creates two `(project, branch_id, ref_entity_id)` rows that the temporal classifier has to reconcile; the dedup logic would be re-implemented at every read site
- Materialised view per merge (precompute the fold-in result and store it as a separate index slice) — considered but rejected as premature optimisation; the classifier-time walk against the `MERGED_INTO` edge is O(merge_count) per query which is bounded (≤ a few hundred merges per project) and adds <5ms on the live stack
- Per-fact merge_strategy override (every branch fact carries its own keep/discard flag) — considered but rejected as operator burden; the branch-level strategy covers the 95% case and the per-fact override is recoverable via the `partial + merge_kept` flag without a schema change
- Drop branch facts on merge (strategy=discard) — rejected outright because audit answers like `cortex history --branch feat/x` must survive the branch lifecycle; archival is the answer when the data ages out (phase18 §1.5), not deletion

## Consequences

Wins: bitemporal audit stays intact (every `recorded_at` matches the writer event); merges are an O(1) write (the edge + status stamp); the classifier already runs on every retrieval so the fold-in adds no new pipeline stage; the same branch fact answers two questions (`--branch main as-of post-merge` returns it via the edge walk; `--branch feat/x as-of any` returns it directly). Costs: classifier complexity grows (one extra branch-edge walk per `main` retrieval); merge strategies must be designed before the §3 classifier ships (no late binding). Reassessment trigger: if the live `MERGED_INTO` count per project exceeds 5,000 (degrading the per-query walk), promote the merge-time materialisation path (precompute the `main` post-merge slice into a versioned Meili / Vectorizer alias). The §3 classifier carries the walk logic; the §4 CLI shape (`cortex branch merge --strategy`) carries the operator surface.
