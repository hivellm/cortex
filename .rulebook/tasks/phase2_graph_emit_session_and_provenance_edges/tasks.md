## 1. Diagnose
- [ ] 1.1 Probe hypothesis A: dump every cross-label edge template + the writer's interpolated Cypher; compare key field names against the actual node MERGE templates (Session uses `id`; HAS_TURN edge target uses `id`)
- [ ] 1.2 Probe hypothesis B: instrument the coalescer to log every edge dedup decision for one bootstrap run; confirm whether HAS_TURN edges are being dropped pre-write
- [ ] 1.3 Probe hypothesis C: run a manual Cypher of `MATCH (s:Session {id:"<known-id>"}), (t:Turn {id:"<known-id>"}) MERGE (s)-[r:HAS_TURN]->(t) RETURN r` against the live Nexus; if it persists, the failure is upstream of the wire (coalescer or Cypher gen); if it fails, it is in the writer template
- [ ] 1.4 Capture the verdict in this task's `notes/diagnosis.md` so the fix lands with evidence

## 2. Fix
- [ ] 2.1 Apply the targeted fix from §1.4 (template rewrite OR coalescer scope OR writer ordering — pick exactly one based on diagnosis)
- [ ] 2.2 Re-run the manual Cypher probe and verify HAS_TURN persists end-to-end through the writer
- [ ] 2.3 Apply the same fix to siblings (`TOUCHED`, `LINKED_TO`, `OF`, `OBSERVED_IN`, `SUPERSEDES`) — they share the cross-label MERGE pattern

## 3. Post-write verification
- [ ] 3.1 `nexus_client.rs::run_write_tx` runs `MATCH ()-[r]->() WHERE type(r) IN $expected RETURN type(r), count(r)` after each batch
- [ ] 3.2 Compare the counts against the patch's intended counts; emit a `WARN` on every shortfall, with the affected `(label_from, edge_type, label_to)` triples
- [ ] 3.3 New `cortex_graph_edges_dropped{edge_type}` counter — operator can spot recurrences

## 4. End-to-end
- [ ] 4.1 Wipe Nexus, restart cortex-graph-worker
- [ ] 4.2 Re-run cortex-bootstrap on the 17 Hive repos
- [ ] 4.3 Assert `MATCH ()-[r:HAS_TURN]->() RETURN count(r)` returns at least one row per Session (today: 0 / 19)
- [ ] 4.4 Spot-check: pick a known Session ULID, traverse `(:Session {id:$x})-[:HAS_TURN]->(:Turn)` and confirm at least one Turn surfaces

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — extend spec-07 with the post-write verification contract
- [ ] 5.2 Write tests covering the new behavior — regression test that constructs a `GraphPatch { nodes: [Session, Turn], edges: [HAS_TURN] }` and asserts both endpoints + the edge land in Nexus
- [ ] 5.3 Run tests and confirm they pass — `cargo test -p cortex-graph`, `cargo clippy -p cortex-graph --all-targets -- -D warnings`
