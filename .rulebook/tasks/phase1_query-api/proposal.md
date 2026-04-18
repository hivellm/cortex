# Proposal: phase1_query-api

## Why

Everything Cortex ingests is wasted unless something can retrieve it fast. The query API is the single read surface consumed by the adapter (pre-thinking), the dashboard, and analysis workflows. It has to fan out across vector + keyword + graph in parallel, fuse results with RRF, stay under 150 ms P95 on the cached hot path, and never fail-closed when one lane is unhealthy.

## What Changes

- `cortex-api` crate: Axum HTTP service + MCP tool binding for `cortex.query`.
- Orchestrator with intent → strategy table (`pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`, `free_search`).
- Three lane clients: Vectorizer KNN, Meilisearch, Nexus Cypher; parallel fan-out with sub-budgets.
- Reciprocal Rank Fusion + tie-breaks (recency then severity).
- Decision / law / graph-neighbor / similar-turn overlays.
- Synap-backed whole-response cache keyed on `hash(intent || scope || query_embedding || schema_version)`.
- Per-caller ACL + rate limiter (30 rps sustained / 60 burst).
- Final redaction pass + query-audit stream (`cortex.events.query_audit`).

## Impact

- **Affected specs:** [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md); unblocks 12, 15, 16.
- **Affected code:** new `cortex-api/` crate with `orchestrator/`, `strategies/`, `lanes/`, `cache/`, `acl/`, `audit/`.
- **Breaking change:** NO — greenfield.
- **User benefit:** first meaningful retrieval lands; closes the capture → processing → retrieval loop.

## Source

`docs/specs/11-query-api.md` · depends on specs 06 + 07 + 08 · PRD FR-11.
