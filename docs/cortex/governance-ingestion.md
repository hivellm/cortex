# Cortex governance ingestion contract

> **Status:** 🟡 partial — write-side ✅ / read-side projection ⚠️
> **Last updated:** 2026-05-01 (phase11h §3.6)
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

> **Caveat:** `LAW-CORTEX-*` declarations currently live in
> `AGENTS.override.md`, which is classified as `Kind::Memory` (per
> `[cortex.memories]`). It is **not** picked up by the law promote
> patterns. To make `law_check` retrieve `LAW-CORTEX-001`, either:
>
> 1. Move the LAW-CORTEX-* declarations into `.claude/rules/` (or
>    `.rulebook/laws/*.yaml`) so the law walker picks them up, **or**
> 2. Extend `[cortex.laws].promote_patterns` to include
>    `AGENTS.override.md`.
>
> Filed as part of `phase11k` (writer-side governance projection).

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
| `decision_lookup` | `cortex.decision.fp32` (global) + `cortex-{slug}-decisions` (per-repo, phase11e §5.3) | `cortex_decisions` (global, **does not yet exist server-side**) + `cortex-{slug}-decisions` | Per-repo lane returns hits as `results.snippets` today; `results.decisions[]` requires writer-side projection of `decision_id` + `decision_title` + `decision_status` to top-level fields (open follow-up). |
| `law_check` | (none) | `cortex_laws` (global, **does not yet exist server-side**) | Per-repo `cortex-{slug}-governance` is queried but `results.violations[]` requires writer-side projection (`law_id`, `severity`, `tier`) to top-level fields (open follow-up). |

## Open follow-ups (filed for phase11k)

1. **Writer-side top-level projection** — `cortex-workers/src/fulltext/builders.rs`'s `Document` struct needs `decision_id`, `decision_title`, `decision_status`, `law_id`, `severity`, `tier`, `turn_id` as top-level fields when the source kind matches. Today the entire payload is serialised into `body` as a JSON string and the spec-11 lane projection contract has nothing to read.
2. **Global decisions/laws Meili indexes** — `cortex_decisions` and `cortex_laws` (no repo prefix) are queried by the orchestrator strategies but never written. Either:
   - Update the fulltext worker to ALSO write each Decision/LawViolation envelope to the global index, or
   - Update the strategies to drop the global lane and rely on per-repo only.
3. **`AGENTS.override.md` law extraction** — `LAW-CORTEX-*` declarations live in a file currently classified as Memory. Either move them or extend `[cortex.laws].promote_patterns`.
4. **Auto-republish on file change** — today every ADR / law update needs a manual `cortex-bootstrap --force <repo>` to re-flow through the workers. A file watcher (or bootstrap-time scan triggered by `inotify` / `fs::Event`) should re-publish on change.

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
