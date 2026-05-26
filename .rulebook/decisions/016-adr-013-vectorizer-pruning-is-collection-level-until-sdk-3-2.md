# 16. ADR-013 — Vectorizer pruning is collection-level until SDK 3.2

**Status**: accepted
**Date**: 2026-05-25
**Related Tasks**: phase14b_pruning-sweep-impls-adr-013

## Context

Phase14b cold-tier pruning has to remove individual expired vectors from Vectorizer collections (the `event_identity`-driven cascade across Synap / Meili / Nexus / archive / Vectorizer). Vectorizer SDK 3.1 surfaces collection-scoped CRUD but NOT a per-vector remove primitive — `delete_vector(collection, vector_id)` is on the SDK 3.2 roadmap but unscheduled. Without an upstream API, the only correct way to drop expired rows is to re-encode the entire collection: stream every still-alive vector to a sibling collection, drop the original, atomically swap the sibling into place.

## Decision

Until Vectorizer SDK 3.2 ships per-vector removal, Cortex MUST prune Vectorizer collections at the COLLECTION level. The implementation lives at `crates/cortex-workers/src/embedder/vectorizer_prune.rs::reencode_collection(name, predicate)`. The function:

1. opens `<name>.tmp`,
2. streams every vector from `<name>` for which `predicate(payload)` returns true (i.e. the vector should survive),
3. upserts each survivor into `<name>.tmp`,
4. on full success renames `<name>.tmp` over `<name>` atomically,
5. on any failure leaves the original `<name>` intact and drops `<name>.tmp` so a partial pass never poisons the live collection.

The operator-tunable knob `CORTEX_VECTORIZER_PRUNE_MODE` defaults to `collection` and is the only supported value until 3.2; a future `per_vector` value will swap the implementation when the SDK primitive lands.

Hot-tier prune (90d) skips Vectorizer because hot rows still live in FP32/PQ and the tier-transition sweep (TierSweep, spec 02 §Quantization) is responsible for moving them between collections. Cold-tier prune (365d) is the sole caller of `reencode_collection`.

## Alternatives Considered

- Block phase14b on Vectorizer SDK 3.2 — rejected because retention cascade is the load-bearing fix and the SDK roadmap is unscheduled.
- Implement a per-vector `delete_vector` shim directly via the Vectorizer HTTP API (bypassing the SDK) — rejected because the wire format is not part of the SDK contract and would break on Vectorizer's next storage-engine swap.
- Mark expired vectors with a `pruned=true` payload flag and filter them out at query time — rejected because every search lane (vector / fusion / similarity) would need a pruned-flag filter clause, expanding the contract surface and leaving disk usage permanently inflated.
- Run the cascade Vectorizer leg as a best-effort warn-and-skip — rejected because partial cascade (Meili/Nexus deleted, Vectorizer left intact) violates the cross-backend integrity contract ADR-012 pins.

## Consequences

**Trade-offs accepted.**

1. Time complexity is O(collection_size) per cold-tier run rather than O(expired_count). For a 2M-vector cold collection with 5% expiry, the per-vector primitive would touch 100k vectors; the collection re-encode touches all 2M. Acceptable because cold-tier prune runs weekly (Sunday 05:00 UTC) and the daemon owns the wall-clock cost.
2. Disk usage temporarily doubles for the affected collection during the swap window (`<name>` + `<name>.tmp` co-exist). The atomic rename swap means the doubling is bounded to the duration of one re-encode pass.
3. Search availability against the pruned collection is preserved throughout — readers see the original until the rename lands.
4. The sibling-collection naming convention reserves `.tmp` as a suffix; production code MUST NOT create collections whose name ends in `.tmp` for any other purpose.
5. When SDK 3.2 ships per-vector removal, the cold-tier prune swaps `reencode_collection` for the per-vector primitive without changing the Sweep surface — `ColdTierPrune::run` keeps the same shape.
6. Doctor consistency now MUST tolerate the re-encode window: a vector lookup against `<name>.tmp` returns "not found" even mid-flight; the doctor's existing per-id probe against the canonical name `<name>` keeps reading the live collection correctly until the swap.
