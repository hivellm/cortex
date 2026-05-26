# Vectorizer SDK 3.2 has no per-vector move/delete — pruning daemon is blocked-external
**Source**: manual
**Date**: 2026-05-03
**Related Task**: phase11j_consolidation_tier
**Tags**: phase11j, vectorizer-sdk, external-blocker, pruning, audit-before-scope
When implementing the phase11j §5 pruning daemon, audit of `vectorizer-sdk = "3.2"` showed the surface exposes only `insert_texts`, `get_vector`, `search_vectors`, `create_collection`, `delete_collection`, `get_collection_info`. There is no `move_to_collection`, no `delete_vectors`, and `delete_collection` is too coarse for per-event demotion.

This blocks every demotion strategy the proposal calls out:
- "read-then-reinsert + delete" — `delete_vector` does not exist
- "move_to_collection" — does not exist
- batch transfer surface — does not exist

The cortex-side pruner can be written but has no real way to move source events between hot/warm/cold tiers. Without the SDK extension, the pruner can only no-op on the vector side, which defeats the cost model.

Implication: the entire phase11j §5 (pruner module + demotion sinks + cron + 2 ITs + health surface) is blocked-external. Tracked by follow-up task `phase11o_vectorizer_demotion_api`. The Parquet archive remains the only complete cold-tier representation; Vectorizer + Meili tiers will grow monotonically until the SDK ships.

Lesson: before scoping a phase that depends on an upstream SDK feature, audit the SDK's actual surface — not its stated/documented intent. The proposal assumed `move_to_collection` existed because it was natural to assume so. Real audit took 2 minutes via `cargo info`. That 2-minute spend would have surfaced the gap during proposal-writing instead of mid-implementation.