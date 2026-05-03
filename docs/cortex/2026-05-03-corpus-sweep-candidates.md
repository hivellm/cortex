# Corpus sweep candidates — 2026-05-03

> Phase11p §1.4 — captured via `cortex-ops sweep-empty --json`. Operator review gate before any `--apply` call.

## Summary

- **Meili host:** `http://127.0.0.1:17004`
- **Non-canonical empty indexes:** **0** (boot-time `sweep_stale_indexes` already reaps these)
- **Canonical-but-empty indexes:** **91**
- **Total drop candidates:** **91**

The 91 canonical-empty entries break down across families:

| Repo | Empty families | Notes |
|---|---|---|
| `csharp`, `go`, `java`, `python`, `rust`, `typescript`, `php`, `tests`, `x`, `project-v` | full `{analyses, code, decisions, docs, governance, misc, turns}` set | language-name "repos" — likely typos / placeholder bootstrap runs that never produced events |
| `compressionprompt`, `expert`, `hivehubcloud`, `lexum` | `{analyses, decisions, governance}` partial | repos that produce `code/docs/turns` but no governance content; the empty governance / decisions indexes survived because the bootstrap created the index ahead of the first upsert |
| `gui`, `transmutation`, `transmutationlite`, `umicp`, `vectorizer`, `synap`, `nexus`, `tml`, `tmldocs`, `tmltextmate`, `vectorizersync` | various per-family | mostly-empty optional families per real repo |

Full list lives in the sibling JSON: [2026-05-03-corpus-sweep-candidates.json](./2026-05-03-corpus-sweep-candidates.json).

## Recommendation

Run `cortex-ops sweep-empty --apply` once the operator confirms there are no in-flight bootstrap runs that would re-create these names. The `sweep_empty_canonical` predicate is the new sibling to the existing boot-time stale-name reaper (`fulltext::sweep::sweep_stale_indexes`); the canonical-but-zero bucket is operator-only because empty-canonical can be a legitimate transient state right after a settings PATCH (the per-repo lazy materialisation flow).

Post-sweep, the Meili index count drops from **190 → 99** (47 % reduction).

## Sibling action — `law_violation` dedupe

Phase11p §4 ships `cortex-ops dedupe-laws`. Live dry-run on 2026-05-03 reports:

| Metric | Value |
|---|---|
| Governance indexes scanned | 24 |
| Total `law.imported` docs | **3,804** |
| Duplicate groups by `(law_id, content_hash)` | **1,104** |
| To keep (one per group) | 1,104 |
| **To drop** | **2,696** |
| Post-dedupe doc count | ~1,108 |
| Reduction | **71 %** |

Per-index breakdown (top contributors):

| Index | Total | Drop |
|---|---|---|
| `cortex-hivegpu-governance` | 646 | 514 |
| `cortex-tml-governance` | 612 | 457 |
| `cortex-cortex-governance` | 523 | 389 |
| `cortex-rulebook-governance` | 486 | 324 |
| `cortex-tmldocs-governance` | 420 | 315 |
| `cortex-vectorizer-governance` | 441 | 292 |
| `cortex-synap-governance` | 402 | 268 |
| `cortex-nexus-governance` | 274 | 137 |

Full plan in `docs/cortex/2026-05-03-dedupe-laws-plan.json`.

### Apply

```sh
CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:17004 \
CORTEX_FULLTEXT_MEILI_API_KEY=$MEILI_MASTER_KEY \
  cortex-ops dedupe-laws --apply
```

The grouping is `(law_id, content_hash)` — every group keeps the OLDEST `ts` and drops the rest. The original phase11p §4.4 target was `< 500` docs; the actual achievable floor is `~1,108` because each per-repo governance index dedupes locally (cross-repo dedupe needs the global `cortex_laws` lane that activates only after the §2 fulltext-worker redeploy). Re-run the dedupe AFTER the redeploy to collapse cross-repo dups via the global lane.

## Re-run

```sh
CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:17004 \
CORTEX_FULLTEXT_MEILI_API_KEY=$MEILI_MASTER_KEY \
  cortex-ops sweep-empty --json   # dry-run, lists candidates
CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:17004 \
CORTEX_FULLTEXT_MEILI_API_KEY=$MEILI_MASTER_KEY \
  cortex-ops sweep-empty --apply  # destructive
```
