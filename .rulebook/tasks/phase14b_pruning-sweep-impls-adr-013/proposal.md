# Proposal: phase14b_pruning-sweep-impls-adr-013

Source: `docs/analysis/rework/02-memory-cleanup.md` Phases 4-5; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase B.2.

## Why

Tier-pruning today is incomplete: hot-tier prune logic exists but cold-tier cascade across Vectorizer / Meili / Nexus / archive is broken. Vectorizer SDK 3.1 lacks a per-vector move primitive — collection-level re-encode-and-replace is the only path until SDK 3.2 ships. The 4-doc + opus5.7 analyses both flag this as the structural blocker for full retention closure.

Lands atop the `Sweep` trait (phase13a) and `EventIdentity` index (phase13d) so the cascade is one indexed lookup per event.

## What Changes

- New ADR-013 — "Vectorizer pruning is collection-level until SDK 3.2".
- New `impl Sweep for HotTierPrune` (was tier_sweep). Reads `event_identity` to dispatch deletes per backend.
- New `impl Sweep for ColdTierPrune` — runs nightly, finds events with `age > 365` and cascades delete to all 4 backends via `EventIdentity`.
- Vectorizer prune: collection-level re-encode (drop expired vectors, rebuild PQ index) gated by `CORTEX_VECTORIZER_PRUNE_MODE=collection`. Documented as ADR-013 trade-off until SDK 3.2.
- Post-prune assertion: `event_identity` row absent ↔ event absent in Synap, Meili, Nexus, Vectorizer, archive.

## Impact

- Affected specs: `docs/specs/02-quantization.md` § Tier pruning + ADR-013.
- Affected code: `crates/cortex-workers/src/retention/{hot_tier_prune.rs,cold_tier_prune.rs}` (rewrites of existing modules), `crates/cortex-workers/src/embedder/vectorizer_prune.rs` (new collection-level re-encode).
- Breaking change: NO.
- User benefit: events older than retention horizon disappear from all backends (no orphaned vectors / Meili docs / Nexus nodes).
