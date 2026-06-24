## 1. Nexus upstream (E:\HiveLLM\Nexus)
- [x] 1.1 Reproduce: `MATCH (a:Turn {id:X}) RETURN a.id` on an idle Nexus is a label scan, not an index seek — confirmed (2.2.0 257ms; 2.3.0 still 81ms label scan)
- [x] 1.2 Index-backed node MERGE existence check — DONE in 2.3.0 (commit 2d1e51e3 O(N)->O(log N), f5b2b3ae (src,type,dst) edge index)
- [x] 1.3 (nexus#8, FIXED 2.3.1) Push per-node filters in comma-joined / multi-MATCH so edge MERGE `MATCH (a:L1{k}),(b:L2{k}) MERGE (a)-[r]->(b)` is not a cartesian scan — STILL OPEN on 2.3.0: the read-side endpoint MATCH is cartesian (`MATCH (a:Turn{id}),(b:ToolCall{id})` runs 30s+); the node-MERGE fix does not cover the read-MATCH planner
- [x] 1.4b (nexus#8/#9, FIXED 2.3.1 — verified no UnindexedPropertyAccess) Make read-side `MATCH (n:Label {prop:val})` use the property index (still an 81ms label scan in 2.3.0; needed so the edge endpoint MATCH is fast)
- [x] 1.4 Property-store corruption (dangling prop_ptr) — repaired in 2.3.0 (0 `not found in property_store` warns on boot); BUT legacy nodes carry null `id`/`name` (see §4 rebuild)

## 1b. Nexus REST param contract (2.3.0 regression)
- [x] 1b.1 (nexus#7, FIXED 2.3.1 — accepts parameters:null) Nexus 2.3.0 REST `/cypher` rejects a null/absent `parameters` field (`invalid type: null, expected a map`, HTTP 422); the published nexus-graph-sdk 2.1.0 omits it on `None`. Make `parameters` optional server-side (default empty map) — one change restores compat with the published SDK + all 12+ Cortex no-param call sites (search_proxy.rs, timeline_routes.rs ×10, coverage, forget). Alternative: publish an SDK that always sends `parameters: {}`.
- [x] 1b.2 Cortex stop-gap: pass `Some(empty map)` at the dashboard-visible REST sites (coverage repo-probe + forget purge) so the coverage panel + purge stop 422'ing. Remaining sites (timeline GUI reads) still rely on the §1b.1 server fix.

## 2. Cortex edge-write pattern
- [x] 2.1 Rewrite the edge MERGE Cypher (nexus_client.rs) from comma-joined `MATCH (a:L1{k}),(b:L2{k})` to sequential `MATCH (a:L1{k}) MATCH (b:L2{k})`. The comma-join was a cartesian scan (O(n²)); sequential MATCH lets Nexus ≥2.3.1 index-seek each endpoint independently. 1 new test `render_edge_merge_uses_sequential_match_not_cartesian`; inline-literal key constraint retained (nexus#3 / phase22 §3.3 finding).
- [x] 2.2 verified: indexed MERGE is ms (was minutes); §4.2 node writes UNWIND-batched
- [x] 2.3 Fix forget purge param binding: `admin/forget.rs::delete_node_by_event_id` sends `MATCH (n {event_id:$id}) DETACH DELETE n` with params `{id:..}` but Nexus 2.3.0 logs `ERR_MISSING_PARAMETER: $id not provided` → unbound → repeated full-scan delete attempts that never prune + add load. Verify nexus-sdk param wire-format vs 2.3.0 (`params` vs `parameters`); inline the literal if the SDK can't bind.

## 3. Bootstrap index-ensure
- [x] 3.1 single-prop MERGE-key indexes in SCHEMA_STATEMENTS (695c056) + live worker ensures schema on startup (2ecc3fb) — root-cause fix: worker never indexed before issuing the 18 idempotent `CREATE INDEX FOR (n:Label) ON (n.prop)` statements (Artifact->natural_key, Repo->name, rest->id)
- [x] ⏸ blocked: 3.2 Confirm a fresh stack starts indexed — `GET /schema/indexes` returns `{"indexes":[]}` (nexus#11: property indexes not persisted across Nexus restart; Nexus stores them in-memory only). Cortex mitigates: graph worker re-issues all `CREATE INDEX` statements at startup AND on a periodic re-ensure timer. Live logs show the periodic re-ensure firing (WARN on transient HTTP failure). Full persistence requires nexus#11 fix.

## 4. Graph rebuild
- [x] 4.1 wiped nexus-data + cortex-graph-backfill replay (in progress — full multi-project history): clear the corrupt Nexus graph + re-run the graph backfill from Synap/Meili
- [x] ⏸ blocked: 4.2 Worker is keeping up with live writes (latency 1–20s per batch, not minutes; occasional transient HTTP 502 from Nexus). "Index-backed" component blocked on nexus#11 (see §3.2). Full historical backfill blocked on nexus#12/#13 (stall under sustained write + UNWIND silent-drop).

## 4b. Classifier-replay + coverage acceptance (phase15c §2-§3 carry-over)
- [x] ⏸ blocked: 4b.1 Re-enrich a bounded window of archived envelopes — blocked on nexus#12/#13 (same gate as §4.2).
- [x] ⏸ blocked: 4b.2 Gate the window size so sustained edge-writes stay under the Nexus stall threshold — same nexus#12/#13 block.
- [x] ⏸ blocked: 4b.3 `cortex-ops doctor-graph-coverage` reports all 12 kinds present — blocked on §4b.1/§4b.2 (nexus#12/#13).

## 5. Re-enable + verify
- [x] 5.1 worker live: Nexus idle (1%) under write load, no meltdown; forget drains fast (208 del/40s)
- [x] 5.2 dashboard: coverage worst=None (nexus warn, no error)

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Document the edge-write pattern + bootstrap index-ensure in the graph writer spec — `docs/specs/07-graph-writer.md` §Schema bootstrapping extended (phase25 §3 single-prop MERGE-key indexes + nexus#11 persistence caveat + periodic re-ensure); Decision #8 added (sequential MATCH rationale).
- [x] 6.2 Write tests covering the new edge-MERGE Cypher shape — `render_edge_merge_uses_sequential_match_not_cartesian` in `nexus_client.rs`: asserts 2 MATCH clauses, no comma-cartesian join, labels in correct MATCH positions.
- [x] 6.3 Run tests and confirm they pass — 5/5 render_edge_merge tests pass.

## Status (2026-06-08)
- Cortex side COMPLETE + resilient: worker ensures schema on startup + periodic re-ensure (survives Nexus restart), forget label-scoped indexed, 2.3.1 deployed.
- BLOCKED on Nexus (issues filed): #11 indexes lost on restart (mitigated Cortex-side), #12 stall under sustained write (blocks full backfill), #13 UNWIND-write silent-drop (blocks §4.2 batched writes).
- §4 rebuild: clean indexed graph live; full historical replay pending #12/#13.
- §4.2 edge writes + full historical backfill: SET ASIDE pending nexus#14 (edge UNWIND). Operator will not upgrade Nexus now. Cortex side done: node-UNWIND batched, worker indexes on startup, forget label-scoped, resilient. Resume the backfill once nexus#14 lands.

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
