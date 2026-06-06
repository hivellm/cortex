# phase25 — Nexus graph write performance (planner does not use property indexes)

## Why

The dashboard went CRITICAL on 2026-06-06: `coverage.nexus` "nexus query
timeout after 5s" + `freshness graph.last_job` gap 578s. Investigation
traced it to Nexus, not Cortex:

- Nexus pinned at 100% CPU. The graph worker's batch-write latencies had
  grown to **13–34 minutes per 256-event batch** (`latency_ms=2069737`),
  then `/cypher` started failing → the worker entered backpressure → the
  graph fell behind (last_job 578s while classifier/embedder/fulltext
  stayed at 18s).
- Nexus logs showed **property-store corruption**: many
  `load_node_properties: prop_ptr ... not found in property_store` and
  `points to wrong entity ... using reverse_index instead`; several
  `Repo.name` values come back `null`.
- A single stuck edge MERGE,
  `MATCH (a:Turn {id}), (b:ToolCall {id}) MERGE (a)-[r:HAS_TOOL_CALL]->(b)`,
  ran **271s and climbing** (query-48), holding all CPU so every other
  query (incl. the trivial `MATCH (r:Repo) RETURN r.name` coverage probe)
  starved and timed out at the 5s cap.

Root cause: **Nexus does not use property indexes for label+property
MATCH/MERGE.** After creating indexes on every MERGE key
(`Artifact.natural_key`, `Repo.name`, and `id` on Turn/Session/ToolCall/
Symbol/Decision/… — 18 total, all confirmed created via `SHOW INDEXES`
returning `<Label>.<prop>.property`), an idle-Nexus point lookup
`MATCH (a:Turn {id:X}) RETURN a.id` still took **257ms returning empty**
(an index seek on a non-existent key should be <5ms — this is a full
label scan), and the two-node edge pattern still **timed out at 20s as a
pure read**. The planner ignores the indexes; the comma-joined two-node
MATCH is a cartesian scan → O(n²) on the 154,225-node graph.

The graph also carries large duplicate bloat: governance events were
re-emitted every tick (see phase24 / the fulltext dedup fix `e9b06ee`),
inflating node count and making the full scans worse. The dedup fix
reduces future bloat but the existing nodes remain and the planner
problem is independent.

## What Changes

The fix is primarily in the **Nexus project** (`E:\HiveLLM\Nexus`), with a
Cortex-side write-pattern change and bootstrap index-ensure as support:

1. **Nexus (upstream):** make the Cypher planner use property indexes for
   `MATCH (n:Label {prop: $v})` and push the per-node filter in
   comma-joined / multi-MATCH patterns so edge MERGE is not a cartesian
   scan. Verify `MATCH (a:Turn {id:X}) RETURN a` is an index seek (<5ms),
   not a label scan. Investigate + repair the property-store corruption
   (dangling `prop_ptr`, null `Repo.name`).
2. **Cortex edge-write Cypher:** rewrite the edge MERGE in
   `crates/cortex-workers/src/graph/cypher.rs` /
   `nexus_client.rs` from the comma-joined
   `MATCH (a:L1 {k}), (b:L2 {k}) MERGE (a)-[r]->(b)` to a form the planner
   can index (separate `MATCH` clauses, or `USING INDEX` hints if Nexus
   supports them) once the Nexus planner change lands. Verify with the
   slow-query log that edge MERGEs drop from minutes to ms.
3. **Cortex bootstrap index-ensure:** add a startup step that issues the
   18 `CREATE INDEX FOR (n:Label) ON (n.prop)` statements (idempotent) so
   a fresh stack starts indexed instead of accumulating until full scans
   melt. Key map from `LiveNexusClient::key_field_for`: Artifact→
   natural_key, Repo→name, all other labels→id.
4. **Graph rebuild path:** because the live graph is corrupt + bloated,
   provide a clean rebuild (clear the Nexus graph + re-run the graph
   backfill from Synap/Meili) so the 154k-node corrupt set is replaced by
   a deduped, index-backed graph.

## Interim mitigation (applied 2026-06-06, live)

- Created the 18 indexes on the running Nexus (persisted; confirmed but
  currently **ignored by the planner** — no perf gain yet).
- Restarted `cortex-nexus` to clear stuck query-48 (recovery takes ~108s
  of 100% CPU each restart due to the corrupt store).
- **Stopped `cortex-graph-worker`** so it stops melting Nexus. With the
  worker stopped Nexus is idle + responsive (coverage probe passes, graph
  read lane works), but graph ingestion is paused so `graph.last_job`
  freshness will alarm. `docker compose up` will restart the worker and
  re-trigger the meltdown until the Nexus planner fix or rebuild lands.

## Impact
- Affected specs: docs/specs (graph writer / Nexus client contract)
- Affected code: crates/cortex-workers/src/graph/cypher.rs,
  crates/cortex-workers/src/graph/nexus_client.rs, graph bootstrap;
  upstream `E:\HiveLLM\Nexus` planner + storage
- Breaking change: NO
- User benefit: restores graph ingestion + the graph retrieval lane;
  stops the recurring Nexus CPU meltdown.

## Source

Discovered while resolving a dashboard CRITICAL during phase22/phase24
work, 2026-06-06.
