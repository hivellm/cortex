# 36. Community summaries as a fourth consolidation grain with offline-map / budgeted-reduce GraphRAG global route

**Status**: proposed
**Date**: 2026-07-07
**Related Tasks**: phase27c_graphrag-community-summaries, phase27b_graph-community-detection, phase29_graph-projection-unblock

## Context

Phase27c needed (a) GraphRAG-style community summaries so Cortex can answer architecture-level ("global") questions no single chunk answers, and (b) a query route that serves them. Constraints: the sync query path carries an 800ms default budget (a Haiku call costs ~1.5s, so no LLM call can run at query time); the existing consolidation infrastructure (envelope storage, summariser stack, cost budget, daemon dispatch) already handles three grains (Session/Topic/DecisionTrace — DEC-005); and the graph carries no community properties until the phase27b §2.5 writeback unblocks (ADR-027, semantic projection gated on nexus#12).

## Decision

1) Community summaries are a FOURTH ConsolidationGrain (alongside DEC-005's three), not a new storage tier: structured scope {community_id, level} (first non-string scope variant — JSON schema gained a oneOf arm), stable cons-com-<hash> ids so re-runs upsert, one summary per (community_id, Leiden level) giving multi-resolution for free. The grain is OPTIONAL on the consolidator daemon (with_community() builder) because it needs a live graph client; an unwired CommunityDetected trigger fails-and-acks rather than wedging the queue. 2) The global query route follows GraphRAG's own map-reduce split: MAP runs offline in the community grain (Haiku per community per level), query-time REDUCE is a budgeted top-N plan over the consolidation corpus (architecture_plan: cortex.consolidation.fp32 + cortex_consolidations, no graph fan-out, no overlays), assembled by the pre-thinking renderer's byte budget. Detection (is_architecture_query, conservative marker heuristic) applies to free_search ONLY — pre_change_context always keeps its per-chunk plan so a false positive can never degrade the agent hot path. 3) Community-aware dedup emits merge PLANS only (union-find, smallest-id survivor); Nexus edge-rewiring application rides the same §2.5 unblock. The community penalty is sized (0.15) so an exact homonym across communities drops into the ambiguous band instead of merging; an optional LLM tiebreaker (default OFF) arbitrates that band through the existing summariser + cost-budget stack.

## Alternatives Considered

- Query-time LLM map-reduce (true GraphRAG global search): rejected — violates the 800ms sync budget by 2x per call, and cost scales with query volume instead of corpus-change volume
- New wire Intent::ArchitectureOverview variant: rejected — breaks the MCP tool schema surface for a routing concern the server can decide internally from the query text
- Mutating the Topic grain to cover communities: rejected — DEC-005/ADR-006 established grains as distinct axes; community partition is graph-topological, embedding-topic is semantic, and both axes must coexist
- MinHash/LSH blocking for dedup as the proposal specified: premise was stale — no MinHash existed in the workspace; lifted the actual hashed-4-gram+cosine util into cortex_core::textsim instead (same blocking role; LSH banding can replace it when bucket sizes hurt)

## Consequences

Positive: architecture/onboarding questions get a corpus purpose-built to answer them; summaries refresh on partition change (offline) rather than per query; all four §1-§3 surfaces are fully offline-tested despite the live graph being empty. Negative/tradeoffs: the global route degrades to ordinary consolidations until phase27b §2.5 + the grain actually run (empty community corpus today); the marker-based detector will miss paraphrased architecture questions (conservative by design — false negatives fall through to normal free_search, false positives are the expensive direction); O(n²) cosine blocking within label buckets caps dedup scalability until LSH banding lands; the community penalty/boost constants (0.15/0.05) are tuned against unit fixtures, not live data — revisit once the projection unblocks and real distributions exist.
