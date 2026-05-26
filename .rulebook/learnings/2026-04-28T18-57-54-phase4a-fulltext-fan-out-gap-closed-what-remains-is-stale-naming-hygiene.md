# phase4a: fulltext fan-out gap closed; what remains is stale-naming hygiene
**Source**: manual
**Date**: 2026-04-28
**Related Task**: phase4a_fulltext_fanout_parity_and_stale_meili_cleanup
**Tags**: phase4a, fulltext, meilisearch, audit, diagnostic
## Question
The 2026-04-27 22:36 UTC audit reported Meilisearch had only `Cortex` indexed (589 docs) while Vectorizer had `Cortex/Rulebook/Vectorizer`. Phase4a was scoped to fix that fan-out gap.

## Re-probe (2026-04-28 / phase4a diagnostic)
Live Meili at `127.0.0.1:17004` (auth `cortex-dev-master-key`) returns 35 indexes via `/stats`:

```
cortex-cortex-{code,decisions,docs,governance,misc,turns,analyses}    populated
cortex-nexus-{code,decisions,docs,misc,turns}                          populated
cortex-rulebook-{code,docs,misc,turns}                                 populated
cortex-synap-{code,decisions,docs,misc,turns}                          populated
cortex-tml-{code,docs,turns}                                           populated (TML=185k docs)
cortex-vectorizer-{code,docs,misc,turns}                               populated
cortex-{analyses,code,decisions,docs,governance,misc,turns}            ALL EMPTY (stale)
```

7 stale 2-token names exist, all with `numberOfDocuments == 0`. The audit's original "Rulebook missing" diagnosis is no longer true — phase4b's bootstrap (or a normal worker run after phase4b) populated rulebook + nexus + synap + tml + vectorizer. The 2-token names are residue from before the per-project naming migration.

## Root cause
Two things, separable:

1. The audit's "fan-out gap" was real at audit time but has been closed by intervening bootstrap runs. No code bug to fix on the fan-out path itself — `routing::index_for_event` correctly reads `event.context_repo` and slug-routes per project.
2. The 7 stale 2-token indexes were never swept. They pollute `/indexes` listings and risk being targeted by old client code.

## Implication for phase4a
- The replay-missing-repos path (proposal §2) is a useful **defense in depth** but does not address an active gap today. Worth shipping for future regressions where a worker stops mid-replay.
- The stale-sweep (proposal §3) is the active fix — those 7 indexes need to go.
- The routing invariant guard (proposal §4) is preventive.

## Probe artifacts
- [scripts/probe_partitions.py](../../../scripts/probe_partitions.py) — counts `(repo_slug, family)` partitions in `~/.cortex/archive/events/**/*.parquet` so future audits can re-baseline without manual JSON-grep.
- Re-run command: `python scripts/probe_partitions.py` — outputs each partition's projected index name.