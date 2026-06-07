## 1. Nexus upstream (E:\HiveLLM\Nexus)
- [x] 1.1 Reproduce: `MATCH (a:Turn {id:X}) RETURN a.id` on an idle Nexus is a label scan, not an index seek — confirmed (2.2.0 257ms; 2.3.0 still 81ms label scan)
- [x] 1.2 Index-backed node MERGE existence check — DONE in 2.3.0 (commit 2d1e51e3 O(N)->O(log N), f5b2b3ae (src,type,dst) edge index)
- [ ] 1.3 (nexus#8) Push per-node filters in comma-joined / multi-MATCH so edge MERGE `MATCH (a:L1{k}),(b:L2{k}) MERGE (a)-[r]->(b)` is not a cartesian scan — STILL OPEN on 2.3.0: the read-side endpoint MATCH is cartesian (`MATCH (a:Turn{id}),(b:ToolCall{id})` runs 30s+); the node-MERGE fix does not cover the read-MATCH planner
- [ ] 1.4b (nexus#8) Make read-side `MATCH (n:Label {prop:val})` use the property index (still an 81ms label scan in 2.3.0; needed so the edge endpoint MATCH is fast)
- [x] 1.4 Property-store corruption (dangling prop_ptr) — repaired in 2.3.0 (0 `not found in property_store` warns on boot); BUT legacy nodes carry null `id`/`name` (see §4 rebuild)

## 1b. Nexus REST param contract (2.3.0 regression)
- [ ] 1b.1 (nexus#7) Nexus 2.3.0 REST `/cypher` rejects a null/absent `parameters` field (`invalid type: null, expected a map`, HTTP 422); the published nexus-graph-sdk 2.1.0 omits it on `None`. Make `parameters` optional server-side (default empty map) — one change restores compat with the published SDK + all 12+ Cortex no-param call sites (search_proxy.rs, timeline_routes.rs ×10, coverage, forget). Alternative: publish an SDK that always sends `parameters: {}`.
- [x] 1b.2 Cortex stop-gap: pass `Some(empty map)` at the dashboard-visible REST sites (coverage repo-probe + forget purge) so the coverage panel + purge stop 422'ing. Remaining sites (timeline GUI reads) still rely on the §1b.1 server fix.

## 2. Cortex edge-write pattern
- [ ] 2.1 Rewrite the edge MERGE Cypher (cypher.rs / nexus_client.rs) into a planner-indexable form (separate MATCH clauses or USING INDEX) — only helps once §1.4b lands
- [ ] 2.2 Verify via the Nexus slow-query log that edge MERGE latency drops from minutes to ms
- [x] 2.3 Fix forget purge param binding: `admin/forget.rs::delete_node_by_event_id` sends `MATCH (n {event_id:$id}) DETACH DELETE n` with params `{id:..}` but Nexus 2.3.0 logs `ERR_MISSING_PARAMETER: $id not provided` → unbound → repeated full-scan delete attempts that never prune + add load. Verify nexus-sdk param wire-format vs 2.3.0 (`params` vs `parameters`); inline the literal if the SDK can't bind.

## 3. Bootstrap index-ensure
- [ ] 3.1 Add a startup step issuing the 18 idempotent `CREATE INDEX FOR (n:Label) ON (n.prop)` statements (Artifact->natural_key, Repo->name, rest->id)
- [ ] 3.2 Confirm a fresh stack starts indexed (SHOW INDEXES lists all 18)

## 4. Graph rebuild
- [ ] 4.1 Provide a clean rebuild: clear the corrupt Nexus graph + re-run the graph backfill from Synap/Meili
- [ ] 4.2 Verify the rebuilt graph is deduped + index-backed and the worker keeps up (latency_ms small, no backpressure)

## 5. Re-enable + verify
- [ ] 5.1 Restart cortex-graph-worker and confirm Nexus CPU stays sane under live write load
- [ ] 5.2 Confirm dashboard clears: coverage.nexus present>0 (no timeout) + freshness graph.last_job OK

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Document the edge-write pattern + bootstrap index-ensure in the graph writer spec
- [ ] 6.2 Write tests covering the new edge-MERGE Cypher shape
- [ ] 6.3 Run tests and confirm they pass
