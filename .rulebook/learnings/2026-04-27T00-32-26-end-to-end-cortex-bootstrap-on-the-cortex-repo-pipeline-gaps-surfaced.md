# End-to-end Cortex bootstrap on the Cortex repo: pipeline gaps surfaced
**Source**: manual
**Date**: 2026-04-27
**Related Task**: phase1_classifier_worker
**Tags**: bootstrap, classifier-worker, synap, meilisearch, nexus, vectorizer, end-to-end
# Outcome

Closed the spec-05-deferred classifier-worker bridge. With the new
`cortex-classifier-worker` running, `cortex-bootstrap .` against the
Cortex repo published 519 events end-to-end in 0.6 s. Two of the three
indexes lit up; the third surfaced a pre-existing bug.

## What landed

- **Meilisearch (full-text):** 519 docs across `cortex-docs` (462),
  `cortex-turns` (42), `cortex-governance` (12), `cortex-misc` (3).
  Search "classifier" returns 53 hits, including the very task files
  created during this session (recursive self-indexing works).
- **Nexus (graph):** writer logs claim `nodes_upserted=271` /
  `edges_upserted=135` but `MATCH (n) RETURN labels(n), count(n)`
  returns one unlabeled node. The graph-writer's batch transaction is
  not actually persisting through to Nexus; the SDK call swallows
  failures silently. **Pre-existing.**
- **Vectorizer (embeddings):** 4 collections created (`cortex-docs`,
  `cortex-governance`, `cortex-misc`, `cortex-turns`) but every batch
  reports `total_failed=4-5` and 0 vectors persist. **Pre-existing
  vectorizer-sdk 3.0.3 drift in the upsert path.**

## Side fixes applied

- `cortex-bootstrap` and `cortex-classifier-worker` publishers now
  auto-create the Synap room on first "Room not found".
- `cortex-fulltext` was hard-failing at boot because
  `settings.v1.json` had a tooling-only `"version": "v1"` field that
  Meili rejects on `PATCH /indexes/{uid}/settings`. Stripped at the
  client boundary.
- `cortex-classifier-worker` treats Synap "Room not found" on consume
  as an empty batch so the worker can come up before bootstrap or
  live capture.
- `cortex-embedder` now `tracing::warn!`s the underlying chunker /
  vectorizer error detail before swallowing it into the
  publish-invalid path; this is what surfaced the pre-existing
  vectorizer auth + upsert drift.

## Operational notes

- The embedder needs a JWT in `CORTEX_EMBEDDER_VECTORIZER_PASSWORD`,
  not the plain admin password — the SDK 3.0.3 transport sniffs the
  three-segment shape and only sends `Authorization: Bearer ...` when
  the value looks like a JWT. Setting the plain password yields HTTP
  401 on every request. Bootstrap script should call `/auth/login`
  once and inject the access_token.

## Follow-ups (separate tasks)

1. **Nexus graph persistence drift.** The `nodes_upserted` count
   from the writer does not match what `MATCH (n)` finds. Likely a
   transaction-commit / Cypher-driver mismatch with Nexus 1.15.0.
2. **Vectorizer SDK 3.0.3 partial-upsert failure.** Every batch loses
   4-5 chunks and the surviving ones don't actually persist
   (`vector_count=0`). The earlier ADR #1 (now superseded) about
   bypassing the SDK for `/insert` and `/get_vector` is the right
   precedent — the same drift may have re-emerged on `/upsert`.
3. **Embedder JWT acquisition.** Embedder should call `/auth/login`
   itself when `CORTEX_EMBEDDER_VECTORIZER_PASSWORD` looks like a
   plain password (no dots), instead of letting it 401.