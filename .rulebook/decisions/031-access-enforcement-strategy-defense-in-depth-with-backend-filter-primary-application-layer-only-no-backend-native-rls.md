# 31. Access enforcement strategy: defense-in-depth with backend-filter-primary; application-layer only, no backend-native RLS

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

We need to decide at which layer(s) the lattice check runs: inside each backend (Meili, Vectorizer, Nexus), at the application layer, or both. Backend-native row-level security varies widely across these three stores and would require store-specific feature work. Missing a single surface in a single-layer architecture leaks data.

## Decision

Defense-in-depth with the per-lane backend filter as the primary load-bearing control. The lattice check runs at four redundant points: (1) per-lane backend filter — the daemon injects a where-clause / payload-filter / Cypher predicate into every query so unauthorized rows never leave the backend (primary); (2) post-fusion ACL drop-wedge in the orchestrator — mirrors `apply_temporal_classifier`, drops any `LaneHit` whose `class_*` extras fail `can_read`; (3) pre-thinking bundle filter — before any section (laws/decisions/snippets) enters the assembled prompt; (4) raw `/v1/search/*` proxy filter — these bypass the orchestrator and must independently enforce. All enforcement is application-layer: the daemon constructs the per-backend filter clauses; no dependency on Meili RLS, Vectorizer column-ACL, or Nexus role grants. This keeps the implementation self-contained and avoids per-backend feature gates. A single leak at any one of the four points is a CI hard failure (see ADR-028 eval gate).

## Alternatives Considered

- Backend-native RLS only — rejected: Meili has no native RLS; Vectorizer payload filters are a Cortex construct; Nexus role-based node visibility would require upstream feature work. Depending on unshipped features blocks the entire phase.
- Application-layer only (no per-lane backend filter) — rejected: the backend would still execute the full query and return all rows to the daemon, which then drops classified hits; this leaks classification metadata via query latency (timing side-channel) and wastes compute on rows that will be dropped.
- Single enforcement point (post-fusion wedge only) — rejected: a bug in one of the four surfaces (e.g. a new raw proxy that misses the wedge) would silently leak; defense-in-depth means any single point failing is not a leak.

## Consequences

Pros: defense-in-depth means no single point of failure leaks classified data; backend filter reduces data transferred to the daemon (performance + timing side-channel mitigation); application-layer implementation is self-contained without upstream backend feature work; the four enforcement points map cleanly onto existing code seams (lane trait, orchestrator wedge, bundle, proxy). Cons: four enforcement points must be kept in sync (mitigated by a shared `AclFilter::can_read` function all four call); adding a new retrieval surface requires adding enforcement (tracked by the spec and the CI leak-detection gate).
