# 6. Governance kinds dual-write to per-repo + global Meili indexes

**Status**: proposed
**Date**: 2026-05-03
**Related Tasks**: phase11k_governance_lane_projection

## Context

`decision_lookup` and `law_check` strategies in `crates/cortex-api/src/strategies.rs` fan out to BOTH the global `cortex_decisions` / `cortex_laws` indexes AND the per-repo `cortex-{slug}-decisions` / `cortex-{slug}-governance` indexes. Pre-phase11k, the workers only wrote per-repo; the global indexes were queried but empty, so cross-repo lookups silently failed.

The proposal called out two options:
1. Dual-write: workers write each `Kind::Decision` / `Kind::LawViolation` envelope to BOTH per-repo and global lanes.
2. Drop globals: update strategies to fan out to per-repo only, force callers to enumerate every repo when asking "have we ever decided X?".

## Decision

Adopt the dual-write strategy. `cortex-workers/src/fulltext/routing.rs::index_for_event_global(kind)` returns `Some("cortex_decisions")` for `Kind::Decision` and `Some("cortex_laws")` for `Kind::LawViolation`; the indexer fans out per-repo + global on every governance envelope. Per-doc id stays the same across both writes so the worker-side `content_hash` dedupe + Meili's primary-key replacement keep both lanes idempotent under re-runs.

## Alternatives Considered

- Drop the global lane from the strategy and force every caller to enumerate scope.repo. Rejected because the orchestrator does not have access to the workspace repo enumeration at query time, and forcing every MCP / dashboard caller to pass the full repo list breaks the 'tell me what we decided about X' UX.
- Use a Meili 'multi-index search' query (Meili 1.10+) that fans out the keyword lane across every per-repo decisions index. Rejected because per-repo index naming is not enumerable from a single Meili API call without operator-side index listing, which adds an extra HTTP round-trip and breaks the lane's fail-open contract.

## Consequences

**Positive**: cross-repo `decision_lookup` / `law_check` answers without scope enumeration. Per-repo lane stays authoritative for scoped reads. Global lane is the cross-repo overlay. The dual-write doubles the write traffic for governance kinds only (a small fraction of total envelope volume — the cortex bootstrap shows 14 + 389 governance docs vs ~8k code/doc artifacts).

**Negative**: storage doubles for governance kinds. Mitigation: governance is rare — under 1% of total document count in observed corpora. Two Meili upserts per governance event vs one (~2 ms latency per event, batched).

**Migration**: existing per-repo content stays. The global lane lazily materialises on first governance write; per-repo writes continue unchanged. No backfill needed to read existing per-repo content (the strategies already query both lanes).
