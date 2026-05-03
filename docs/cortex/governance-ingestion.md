# Cortex governance ingestion contract

> **Status:** 🟢 closed — write-side ✅ / read-side projection ✅ / cross-repo lane ✅ / extraction ✅ / watcher ✅
> **Last updated:** 2026-05-03 (phase11k §1–§5)
> **Owner:** Core team

This is the contract for how ADRs and behavioral laws end up in
Cortex, queryable through `/v1/query` intents `decision_lookup` and
`law_check`.

## Sources of truth

| Artifact | Lives in | Promote pattern (cortex.toml) |
|---|---|---|
| ADR | `.rulebook/decisions/*.md`, `docs/decisions/**/*.md`, `ADR-*.md` | `[cortex.decisions].promote_patterns` |
| Behavioral law | `.rulebook/laws/*.yaml`, `.rulebook/laws/*.yml`, `.claude/rules/*.md` | `[cortex.laws].promote_patterns` |
| Spec / RULEBOOK | `.rulebook/specs/**/*.md` | also classified as Law (FileClass::Law) |
| Override / memory | `AGENTS.override.md`, `CLAUDE.md`, `AGENTS.md`, `.rulebook/memory/**/*.md` | `[cortex.memories].import_files` |

> **Resolved (phase11k §3):** `[cortex.laws].promote_patterns` now
> includes `AGENTS.override.md` and `AGENTS.md`, and a new
> `[cortex.laws].extract_pattern = "^LAW-[A-Z0-9-]+$"` knob splits
> AGENTS files into one `law.imported` envelope per matching `## `
> heading. `LAW-CORTEX-001`, `LAW-CORTEX-002`, … reach the law lane
> as addressable rows.

## Write path (bootstrap)

1. `cortex-bootstrap` walks the repo (or every repo in the
   workspace TOML).
2. `cortex_cli::bootstrap::classify_path` decides each file's
   `FileClass`. ADRs land as `FileClass::Decision`; laws as
   `FileClass::Law`.
3. `cortex_cli::bootstrap::emitter::emit_decision_imported` /
   `emit_law_imported` build canonical `Envelope`s with
   `kind=decision.imported` (mapped to `Kind::Decision`) or
   `kind=law.imported` (mapped to `Kind::LawViolation` per the
   workers' classifier).
4. The envelope flows onto `cortex.events.bootstrap` via Synap.
5. The classifier worker (`cortex-classifier`) stamps topics +
   severity, re-publishes to `cortex.events.enriched`.
6. The fulltext worker (`cortex-fulltext-worker`) calls
   `index_for_event(prefix, env)` to compute the target Meili
   index — for `Kind::Decision` it lands under
   `cortex-{slug}-decisions`; for `Kind::LawViolation` under
   `cortex-{slug}-governance`.
7. The embedder worker writes vectors into the matching
   Vectorizer collections.

> **Reality check (2026-05-01):** the cortex repo's own bootstrap
> produced 14 documents in `cortex-cortex-decisions` and 389 in
> `cortex-cortex-governance`. Per-repo write path is healthy.

## Read path (query API)

`/v1/query` resolves intent → strategy:

| Intent | Vector lane | Keyword lane | Notes |
|---|---|---|---|
| `decision_lookup` | `cortex.decision.fp32` (global) + `cortex-{slug}-decisions` (per-repo) | `cortex_decisions` (global) + `cortex-{slug}-decisions` | `results.decisions[]` populated via the spec-11 lane projection contract: `decision_id` / `decision_title` / `decision_status` / `decision_supersedes` are top-level fields stamped by the worker (phase11k §1); the global lane is dual-written by phase11k §2. |
| `law_check` | (none) | `cortex_laws` (global) + `cortex-{slug}-governance` | `results.laws_active` populated by the same projection contract: `law_id` / `law_severity` / `law_tier` top-level. The global lane is dual-written by phase11k §2 so `law_check` answers cross-repo without enumerating repos. |

## Phase11k closure summary

1. **Writer-side top-level projection** ✅ — `cortex-workers/src/fulltext/document.rs::Document` carries `decision_id`, `decision_title`, `decision_status`, `decision_supersedes`, `law_id`, `law_severity`, `law_tier`, `turn_id` as top-level optional fields. Settings v5 marks them filterable + searchable so the dashboard's facet view + the orchestrator's lane projection both pick them up.
2. **Global decisions/laws Meili indexes** ✅ — `routing::index_for_event_global(kind)` returns `cortex_decisions` for Decision and `cortex_laws` for LawViolation; the indexer dual-writes per-repo + global so a query without `scope.repo` resolves cross-repo.
3. **`AGENTS.override.md` law extraction** ✅ — `cortex.toml` adds `AGENTS.override.md` / `AGENTS.md` to `[cortex.laws].promote_patterns` and ships an `extract_pattern = "^LAW-[A-Z0-9-]+$"` knob. The bootstrap emitter splits the body into one `law.imported` envelope per matching `## LAW-...` heading.
4. **Auto-republish on file change** ✅ — `cortex-claude-archive::governance_watcher::GovernanceWatcher` polls `.rulebook/decisions/`, `.rulebook/laws/`, `.claude/rules/`, `AGENTS.override.md`, `AGENTS.md` and emits a change to a `GovernanceEmitter` on every content-hash drift. `MemoryGovernanceEmitter` is the test seam; a Synap-bound emitter ships in the §6 follow-up.

## Verification today

```sh
# Confirm an ADR is reachable as a snippet (works):
curl -s -X POST http://127.0.0.1:17000/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"intent":"decision_lookup","query":"<ADR title or keyword>",
       "scope":{"repo":"cortex"},
       "include":["decisions","snippets"]}' | jq '.results.snippets[0]'

# Inspect the Meili index directly (works):
curl -s -H "Authorization: Bearer $MEILI_MASTER_KEY" \
  http://127.0.0.1:17004/indexes/cortex-cortex-decisions/search \
  -H 'Content-Type: application/json' \
  -d '{"q":"<keyword>","limit":3}'

# Pull the per-repo governance index for laws:
curl -s -H "Authorization: Bearer $MEILI_MASTER_KEY" \
  http://127.0.0.1:17004/indexes/cortex-cortex-governance/search \
  -H 'Content-Type: application/json' \
  -d '{"q":"<law text>","limit":3}'
```

The first call returns the ADR as `results.snippets[0]`; it does
**not** populate `results.decisions[]` until phase11k §1 lands.
