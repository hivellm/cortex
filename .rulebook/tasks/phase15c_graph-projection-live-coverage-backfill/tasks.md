## 1. Bounded live-write backfill
- [x] 1.1 Add `--apply` + `--limit <N>` to `cortex-ops graph backfill`: project archived envelopes through the real `GraphWriter` (not count-only) for at most `N` events newer than `--since`. — Done (commit 83bdedc). Surfaced + fixed two blocking Nexus 2.3.2 graph-writer bugs the projection edge-write path had never exercised: (a) `r` is unusable in any SET clause → props inlined into the MERGE pattern (`render_edge_props_inline`, created_at_ms excluded for idempotency); (b) endpoint-missing MERGE returns "no rows" → soft-drop guard widened so it no longer aborts the batch (would have wedged the live worker). **Deployed**: `cortex/graph-worker:dev` rebuilt at 83bdedc and recreated; the live worker is healthy (0 errors) with both fixes + the phase15b projection wiring active, so new enriched traffic no longer risks a write-tx abort.
- [x] 1.2 Run the payload-driven backfill (`--apply`) against the live graph; confirm payload-driven edges land via `doctor-graph-coverage`. — Validated after §1.3: bounded `graph backfill --apply --limit 500` now persists **481/481** edges (was 0 dropped); live `EMITTED_BY` count 0 → 481 with real `Tool[Bash/Edit/Read/Grep/mcp__*/…]` anchor nodes present. Pre-§1.3 every EMITTED_BY soft-dropped because its `Tool[tool_name]`/`Model[model]` endpoints didn't exist.
- [x] 1.3 Endpoint-node creation — done (commit fff14c6): `project_envelope` appends one empty-props `NodeOp::with_identity` per distinct edge endpoint (both `from` and `to`, deduped, first-seen order → byte-identical re-projection). Empty props + `ConflictPolicy::Match` mean the coalescer (props union via `extend`) never clobbers a richer mapper/static node for the same `(label, key)`. Generalises beyond Tool/Model — every projection edge kind now has guaranteed endpoints, including CONTRADICTS's non-event `from`. 8 projection unit tests green (idempotency + every-endpoint-anchored + dedup).

## 2. Classifier-replay for classifier-driven kinds
- [ ] 2.1 Re-enrich a bounded window of archived envelopes through the live classifier so CALLS / IMPORTS / DEFINES / RETURNS / ABOUT / MENTIONS_FILE / RELATES_TO carry real relations/entities/topics.
- [ ] 2.2 Project the enriched window live; gate the window size so sustained edge-writes stay under the nexus#12 stall threshold.

## 3. Acceptance
- [ ] 3.1 `cortex-ops doctor-graph-coverage` reports all 12 kinds present and above the §4.2 floor; capture the JSON output as the acceptance artifact.

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
