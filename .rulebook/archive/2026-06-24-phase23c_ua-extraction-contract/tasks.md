## 1. Two-phase split
- [x] 1.1 Define the deterministic fact set type (nodes + imports/exports + call edges + line ranges)
- [x] 1.2 Phase 1 extractor produces facts with no LLM call
- [x] 1.3 Phase 2 LLM annotation consumes facts, emits summary/tags/complexity + semantic edges only

## 2. Reconciliation gate
- [x] 2.1 Reject any annotated node whose id is absent from the fact set
- [x] 2.2 Reject any edge whose `source`/`target` is absent from facts ∪ existing graph
- [x] 2.3 Assert per-file import-edge count equals the deterministic import count
- [x] 2.4 Significance filter: drop function/class nodes under 10 lines and not exported
- [x] 2.5 Normalize node ids to the strict prefix scheme; reject malformed ids

## 3. Violation handling
- [x] 3.1 On a rejected node/edge: drop it and log to the audit envelope
- [x] 3.2 On import-count mismatch: re-run annotation once, then accept deterministic import edges (extractor wins)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation
- [x] 4.2 Write tests: seeded hallucinated edge rejected; omitted import backfilled; 8-line helper filtered unless exported; malformed id rejected
- [x] 4.3 Run tests and confirm they pass
