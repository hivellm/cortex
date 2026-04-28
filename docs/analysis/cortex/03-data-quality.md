# 03 — Data quality (what is actually stored)

This file reports the *observed* state of the four backends. All numbers come from the audit captured 2026-04-27 22:36 UTC and recorded in the [phase4a proposal](../../../.rulebook/tasks/phase4a_fulltext_fanout_parity_and_stale_meili_cleanup/proposal.md), the [phase4c proposal](../../../.rulebook/tasks/phase4c_graph_richer_edges_defines/proposal.md), and the [end-to-end learning 2026-04-27](../../../.rulebook/learnings/2026-04-27T00-32-26-end-to-end-cortex-bootstrap-on-the-cortex-repo-pipeline-gaps-surfaced.md).

## Bootstrap walker output

From [.cortex-bootstrap.state.json](../../../.cortex-bootstrap.state.json):

| Repo     | files_walked | commits_walked | events_emitted | status |
|----------|--------------|----------------|----------------|--------|
| Cortex   | 529          | 88             | 617            | done   |
| Nexus    | 1634         | 1008           | 2642           | done   |
| Rulebook | 1136         | 518            | 1654           | done   |
| Synap    | 919          | 385            | 1304           | done   |

**Reading:** the walker successfully traversed 4 of 17 planned repos and emitted ~6.2k bootstrap envelopes. There is no record of Vectorizer being walked in the current state file even though Vectorizer data exists in three backends — almost certainly because earlier walker runs persisted into the backends but the `.cortex-bootstrap.state.json` was overwritten by later invocations (single-repo design, see [phase4b proposal](../../../.rulebook/tasks/phase4b_bootstrap_resume_remaining_repos/proposal.md)).

## Vectorizer — semantic lane

Audit snapshot (vectors stored, by repo):

| Repo       | vectors  |
|------------|----------|
| Cortex     | 17,629   |
| Rulebook   | 9,264    |
| Vectorizer | 101,293  |
| **Total**  | **128,186** |

Per the same audit, **every batch reports `total_failed=4-5`** (out of typical 64-chunk batches) and the surviving entries' `vector_count` is reported as 0 by the server response. This is the SDK 3.0.3 `/upsert` drift recorded under [knowledge anti-patterns](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side.md). Empirically, the vectors *do* end up queryable, so either:

- the server response is misreporting (the count is wrong, persistence is fine), or
- some chunks survive insertion and the failure rate masks a partial-success path.

Either way, the result is **opaque to the operator**: we don't know whether ~128k means "everything that should be there" or "everything minus 8% silently dropped". The `cortex doctor consistency` subcommand (phase4d) is the planned objective measure.

## Meilisearch — keyword lane

Audit snapshot:

| Repo       | docs |
|------------|------|
| Cortex     | 589  |
| Rulebook   | 0    |
| Vectorizer | 0    |

Plus six **stale legacy indexes** from the pre-slug naming scheme: `cortex-code`, `cortex-decisions`, `cortex-docs`, `cortex-governance`, `cortex-misc`, `cortex-turns` — all empty, none cleaned up.

**Reading:** the keyword lane today is, in practice, a single-repo lane. Pre-thinking bundles for any prompt about Rulebook or Vectorizer fall back to the vector lane only, where BM25-as-embedding scores are weak (top-1 score 0.136 on the audit's "classifier worker" probe). The routing code in [crates/cortex-fulltext/src/routing.rs:125-137](../../../crates/cortex-fulltext/src/routing.rs#L125-L137) reads `event.context_repo` correctly; the failure is in the *consumer* — the worker either never received the non-Cortex events or stopped before catching up. Phase4a is the planned diagnose-and-replay fix.

## Nexus — graph lane

Node counts (audit 2026-04-27):

| Label         | count |
|---------------|-------|
| Artifact      | 3634  |
| Repo          | 3     |
| Session       | 9     |
| Turn          | 28    |
| LawViolation  | 72    |
| Decision      | 12    |
| Memory        | 24    |

Edge counts:

| Type       | count |
|------------|-------|
| IN_REPO    | 10245 |
| REMEMBERS  | 30    |

That is only **two edge types** out of the dozen the architecture spec contemplates ([architecture.md §4.2](../../architecture.md)). The model can answer:

- "Which artifacts belong to which repo?" (`IN_REPO`)
- "Which sessions remember which memories?" (`REMEMBERS`)

It **cannot** answer:

- "Where is `PreThinkingTool` defined?"
- "What artifacts in repo X define a public symbol?"
- "What does this turn touch / produce?"
- Any cross-artifact question (imports, calls, defines, supersedes).

The data needed to populate `Symbol` nodes is **already produced upstream**. The Vectorizer payload sample captured during the audit contains:

```json
{
  "chunk_content_hash": "70dabebd...",
  "language": "rust",
  "parent_event_id": "01KQ84GPYDD3B2XCXPAQCMP70W",
  "path": "crates/cortex-mcp-server/src/tools.rs",
  "repo": "Cortex",
  "source": "code",
  "symbol": "PreThinkingTool",
  "topics": "code,rust"
}
```

So `cortex-bootstrap` / `cortex-classifier` is emitting `symbol` per chunk, but [crates/cortex-graph/src/mapper.rs](../../../crates/cortex-graph/src/mapper.rs) drops it. Phase4c is the planned fix.

The graph lane also lit up for `LawViolation` (72) and `Decision` (12), suggesting governance ingestion *partially* works — likely from the bootstrap promoting `.rulebook/decisions/*.md` and `.claude/rules/*.md` into events. There is no live engine producing new `LawViolation` nodes (see [06-governance-gap.md](06-governance-gap.md)).

## Cross-backend symmetry

| Repo       | Vectors    | Meili docs | Nexus presence (Repo node)  |
|------------|------------|------------|-----------------------------|
| Cortex     | 17,629     | 589        | ✅                          |
| Rulebook   | 9,264      | 0          | ✅                          |
| Vectorizer | 101,293    | 0          | ✅                          |

The asymmetry is a single-axis problem (Meili fan-out), not a three-way coordination problem. Once phase4a lands, all three lanes index the same three repos; phase4b extends that to all 17.

## Quality of the captured stream

The bootstrap-emitted events are dense (516 files → 589 envelopes is ~1.14 envelopes per file, accounting for commit walking and decision/memory promotion). Per-event Haiku classification was deliberately disabled in favor of `StaticClassifier` (zero-cost, deterministic) — see [analyzer.rs:9-15](../../../crates/cortex-api/src/analyzer.rs) note: "Per-event Haiku-grade classification was producing tags with no lift; what was missing was the wider lens." The Sonnet analyzer (commit `a62fcbd`) is the response.

This means topic/severity tags on individual events are mostly the static heuristic output today — they are not "wrong" but they are not adding much retrieval signal either. Production value will come from cross-event Sonnet analyses, which are computed on demand per session, not at ingestion time.

## What would unblock objective measurement

The "doctor consistency" subcommand proposed in [phase4d](../../../.rulebook/tasks/phase4d_indexing_consistency_doctor/proposal.md) gives the operator a single command that asserts:

1. Every repo present in *any* backend is present in *all* backends.
2. Per-`(repo, family)` counts are within an expected ratio across Vectorizer chunks / Meili docs / Nexus artifacts.
3. The event-archive `(repo, family)` partition list is fully reflected in each backend.
4. Same-query probe across the three lanes returns at least one overlapping `path` for queries that should have indexed coverage.

That subcommand does not exist yet. Until it does, every audit happens by hand-curling four backends and decompressing zstd archives in Python, which is exactly how the current drifts were detected — slow, manual, and accidentally-discovered weeks after they appear.
