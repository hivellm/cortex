## 1. Nexus upstream (E:\HiveLLM\Nexus)
- [ ] 1.1 Reproduce: `MATCH (a:Turn {id:X}) RETURN a.id` on an idle Nexus is a label scan (~257ms empty), not an index seek
- [ ] 1.2 Make the planner use property indexes for `MATCH (n:Label {prop:$v})` (index seek <5ms)
- [ ] 1.3 Push per-node filters in comma-joined / multi-MATCH so edge MERGE `MATCH (a:L1{k}),(b:L2{k}) MERGE (a)-[r]->(b)` is not a cartesian scan
- [ ] 1.4 Investigate + repair property-store corruption (dangling prop_ptr, null Repo.name)

## 2. Cortex edge-write pattern
- [ ] 2.1 Rewrite the edge MERGE Cypher (cypher.rs / nexus_client.rs) into a planner-indexable form (separate MATCH clauses or USING INDEX)
- [ ] 2.2 Verify via the Nexus slow-query log that edge MERGE latency drops from minutes to ms

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
