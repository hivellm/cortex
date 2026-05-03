# Redeploy `cortex-fulltext-worker` after phase11k

> **Status:** required after the phase11k commit lands. Without this redeploy the dual-write contract (`cortex_decisions` / `cortex_laws`) and the v5 settings projection do **not** activate on the running stack.

## Why

Phase11k §1 + §2 ship two write-side changes that take effect only inside the fulltext worker process:

1. **Top-level governance projection** (`cortex-workers/src/fulltext/document.rs`) — `decision_id`, `decision_title`, `decision_status`, `decision_supersedes`, `law_id`, `law_severity`, `law_tier`, `turn_id` stamped at the top level of every Meili document. Settings v4 → v5 marks them filterable + searchable.
2. **Global dual-write** (`cortex-workers/src/fulltext/routing.rs::index_for_event_global`) — every `Kind::Decision` envelope now writes to both `cortex-{slug}-decisions` AND the global `cortex_decisions` index; same shape for `Kind::LawViolation` → `cortex_laws`.

The pre-phase11k binary in production knows nothing about either change. Its existing per-repo writes still work; new envelopes just don't reach the global lanes and don't carry the projection fields. The dashboard's `decision_lookup` / `law_check` overlays stay empty until the redeploy lands.

## Pre-flight

1. Confirm the local commit includes phase11k. From the repo root:
   ```sh
   git log --oneline -5 | grep phase11k
   ```
   Expect to see `feat(governance): phase11k §1-§6 — governance lane projection + dual-write + watcher`.

2. Confirm the build is clean:
   ```sh
   cargo check -p cortex-workers
   cargo test -p cortex-workers --lib fulltext::
   ```

3. Snapshot the current Meili index count so the post-redeploy verification has a baseline:
   ```sh
   curl -s -H "Authorization: Bearer $MEILI_MASTER_KEY" http://127.0.0.1:17004/stats \
     | python -c "import sys,json; d=json.load(sys.stdin); print('indexes:',len(d['indexes']),'total docs:',sum(v.get('numberOfDocuments',0) for v in d['indexes'].values()))"
   ```

## Redeploy sequence

The container build picks up the host's HEAD via the standard Dockerfile build args.

```sh
# Stamp the git sha into the binary so /healthz reports real values.
export CORTEX_GIT_SHA=$(git rev-parse HEAD)
export CORTEX_GIT_DIRTY=$(git status --porcelain | head -c1 | grep -q . && echo true || echo false)

# Rebuild only the fulltext-worker image (depends_on services keep running).
docker compose build cortex-fulltext-worker

# Roll the worker. `--no-deps` avoids restarting Synap / Meili.
docker compose up -d --no-deps cortex-fulltext-worker

# Confirm the new binary booted.
curl -s http://127.0.0.1:17023/healthz | python -m json.tool
```

The new binary applies settings v5 lazily — the first batch destined for any per-repo `cortex-{slug}-{family}` index triggers a settings PATCH carrying the new `filterableAttributes` (`decision_id`, `decision_title`, `decision_status`, `decision_supersedes`, `law_id`, `law_severity`, `law_tier`, `turn_id`) plus the `decision_title` / `law_id` searchable additions. The PATCH is additive — Meili keeps every prior attribute. Older documents already on disk still serve correctly; only newly-upserted ones carry the top-level projection.

The global indexes (`cortex_decisions`, `cortex_laws`) materialise on the first governance event after the redeploy. They do not exist yet on the live cluster (verified via `/stats` on 2026-05-03).

## Post-redeploy verification

1. **Fire one governance envelope through the live pipeline.** The simplest path: re-run `cortex-bootstrap` against any one repo with at least one ADR or `LAW-*` declaration. Watch the worker log for the `index = "cortex_decisions"` / `index = "cortex_laws"` line confirming dual-write fired.

2. **Inspect the global indexes:**
   ```sh
   curl -s -H "Authorization: Bearer $MEILI_MASTER_KEY" \
     "http://127.0.0.1:17004/indexes/cortex_decisions/stats" | python -m json.tool
   curl -s -H "Authorization: Bearer $MEILI_MASTER_KEY" \
     "http://127.0.0.1:17004/indexes/cortex_laws/stats" | python -m json.tool
   ```
   Expect `numberOfDocuments` to be > 0.

3. **Inspect a per-repo decisions document:**
   ```sh
   curl -s -H "Authorization: Bearer $MEILI_MASTER_KEY" \
     -X POST http://127.0.0.1:17004/indexes/cortex-cortex-decisions/search \
     -H 'Content-Type: application/json' \
     -d '{"q":"","limit":1}' | python -m json.tool
   ```
   Expect the hit's body to carry `decision_id` / `decision_title` / `decision_status` at the top level (not just nested under `ext.decision.*`).

4. **Hit the orchestrator:**
   ```sh
   curl -s -X POST http://127.0.0.1:17000/v1/query \
     -H 'Content-Type: application/json' \
     -H 'X-Cortex-Caller: redeploy-smoke' \
     -d '{"intent":"decision_lookup","query":"adopt","scope":{"repo":"Cortex"},"include":["snippets","decisions"],"budget_ms":500}' \
     | python -c "import sys,json; d=json.load(sys.stdin); print('decisions count:',len(d['results'].get('decisions',[])))"
   ```
   Expect `decisions count` > 0 (was always 0 pre-phase11k against a populated corpus).

## Rollback

The redeploy adds top-level fields and dual-writes; nothing in the new binary breaks the legacy contract. To roll back, redeploy any pre-phase11k image — settings v5 fields stay on the indexes (Meili does not auto-prune unused filterable attributes), but no new top-level projections are written. The global `cortex_decisions` / `cortex_laws` indexes stay populated; they are honest snapshots of what the upgraded worker indexed during its uptime window. Drop them via `cortex-ops sweep-empty --apply` (phase11p §1) if a clean baseline is needed.

## Companion tasks

- `phase11p_corpus_cleanup_sweep` — mechanical cleanup (this runbook is §2.2).
- `phase11q_corpus_consolidation_run` — LLM consolidation pass; runs after this redeploy and after §1 sweep.
