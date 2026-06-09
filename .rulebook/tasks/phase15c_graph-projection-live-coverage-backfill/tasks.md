## 1. Bounded live-write backfill
- [x] 1.1 Add `--apply` + `--limit <N>` to `cortex-ops graph backfill`: project archived envelopes through the real `GraphWriter` (not count-only) for at most `N` events newer than `--since`. — Done (commit 83bdedc). Surfaced + fixed two blocking Nexus 2.3.2 graph-writer bugs the projection edge-write path had never exercised: (a) `r` is unusable in any SET clause → props inlined into the MERGE pattern (`render_edge_props_inline`, created_at_ms excluded for idempotency); (b) endpoint-missing MERGE returns "no rows" → soft-drop guard widened so it no longer aborts the batch (would have wedged the live worker).
- [ ] 1.2 Run the payload-driven backfill (`--apply`) against the live graph; confirm SUPERSEDES / CONTRADICTS / EMITTED_BY / ANSWERED_BY / CITES edges land via `doctor-graph-coverage`. — Finding: under StaticFallback the archive yields almost only EMITTED_BY (29,626/29,631), but its endpoint nodes `Tool[tool_name]` / `Model[model]` are NOT created by the mapper (only Tool nodes from classifier "tool" entities exist), so every EMITTED_BY soft-drops → 0 persisted. Blocks on §1.3.
- [ ] 1.3 Endpoint-node creation: have the projection (or mapper) upsert the `Tool` / `Model` stub nodes that `EMITTED_BY` references, so the edge has endpoints to MATCH. Mirror for any other kind whose endpoint label the mapper does not already create.

## 2. Classifier-replay for classifier-driven kinds
- [ ] 2.1 Re-enrich a bounded window of archived envelopes through the live classifier so CALLS / IMPORTS / DEFINES / RETURNS / ABOUT / MENTIONS_FILE / RELATES_TO carry real relations/entities/topics.
- [ ] 2.2 Project the enriched window live; gate the window size so sustained edge-writes stay under the nexus#12 stall threshold.

## 3. Acceptance
- [ ] 3.1 `cortex-ops doctor-graph-coverage` reports all 12 kinds present and above the §4.2 floor; capture the JSON output as the acceptance artifact.

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
