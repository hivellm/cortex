# Proposal: phase2_graph_emit_session_and_provenance_edges

## Why

After the 2026-04-27 reindex, Nexus carries 11 854 nodes across 7 labels and only **2** edge types:

| Edge | Rows | Status |
|---|---|---|
| `IN_REPO` | 9 455 | ✅ working |
| `REMEMBERS` | 9 | ✅ working |
| `HAS_TURN` | **0** | ❌ should be 45 (Turn count) |
| `HAS_TOOL_CALL` | **0** | ❌ no tool_call envelopes flowing yet, but template registered |
| `TOUCHED` | **0** | ❌ |
| `LINKED_TO` | **0** | ❌ |
| `OF` | **0** | ❌ |
| `OBSERVED_IN` | **0** | ❌ |
| `SUPERSEDES` | **0** | ❌ |

`Turn` (45 nodes) and `Session` (19 nodes) exist but float disconnected. `mapper.rs::emit_turn` does push the `HAS_TURN` edge into the patch (verified in source), and `Artifact-IN_REPO->Repo` writes work, so the writer's UNWIND path can persist edges in general. The `HAS_TURN` template specifically is failing.

Hypotheses, ranked:

1. **Cypher template typo / case mismatch.** Template `cypher/edge_session__has_turn__turn.cypher` may use a key field name that differs from what the writer interpolates (e.g. `Session.id` vs `Session.session_id`).
2. **Coalescer drops cross-batch edges.** If the Session arrives in batch N and the Turn in batch N+1, the edge upsert in batch N+1 may be deduped against an in-memory cache that does not yet know the Session exists.
3. **Edge written before both nodes exist.** Per Nexus 1.15 Cypher, `MERGE (a:Session {id:$x})-[:HAS_TURN]->(b:Turn {id:$y})` needs both nodes to MERGE atomically; if the writer separates node MERGEs from edge MERGEs into different statements (which it does), and the Session MERGE for that key is in a still-uncommitted transaction or a later batch, the edge MATCH fails silently.

The bug blocks every downstream view that tries to traverse `Session → Turn → ToolCall → Artifact` provenance: the dashboard's Graph explorer, cortex-api's `/v1/dashboard/graph` Cypher fallback, and any future "what did this turn touch?" query.

Source: 2026-04-27 reindex audit. Depends on `phase1_graph_writer_nexus_compat` if that lands a writer-shape change first; otherwise this task hardens the existing path.

## What Changes

- Diagnose the failure mode by running the three hypotheses against the live stack (each is a 5-minute probe — verifiable, not speculative).
- Once root cause known, the fix lands in one of three places:
  - **(a)** rewrite the affected `.cypher` template to align key names;
  - **(b)** rebuild the coalescer's in-memory edge dedup to key on `(from_label, from_key, edge_type, to_label, to_key)` AND scope to the current batch only — never across batches;
  - **(c)** make the writer order each batch as `nodes-first then edges` and verify both endpoints exist (`MATCH ... RETURN count(*)>0`) before emitting the edge.
- Add a writer-side smoke test that, after every batch, runs `MATCH ()-[r]->() RETURN type(r), count(r)` and asserts the actually-written counts match the patch's intended counts. Today's writer reports `edges_upserted=N` based on what was sent, not what landed — same lying-success class as `phase1_graph_writer_nexus_compat`.

The same investigation covers `TOUCHED`, `LINKED_TO`, `OF`, `OBSERVED_IN`, `SUPERSEDES` since they all share the cross-label edge MERGE pattern. They have zero rows today partly because their source events (tool_call, decision-with-supersedes, law_violation-with-observed_event) are rare in the bootstrap corpus, but the `HAS_TURN` failure proves the writer would drop them too.

## Impact

- Affected specs: spec-07 (graph writer — codify the post-write verification step).
- Affected code:
  - `crates/cortex-graph/src/coalescer.rs` (edge dedup scope)
  - `crates/cortex-graph/src/nexus_client.rs::run_write_tx` (post-write verification)
  - `crates/cortex-graph/cypher/edge_session__has_turn__turn.cypher` (and siblings — review every cross-label edge template)
  - `crates/cortex-graph/tests/worker.rs` (regression test)
- Breaking change: NO — failure-mode fix only.
- Depends on: `phase1_graph_writer_nexus_compat` (if it changes the writer-Cypher shape); coordinate so this task does not redo the same template work.
- User benefit: Session→Turn→ToolCall provenance becomes traversable; the Graph explorer in the GUI shows edges instead of disconnected nodes; queries like "show me everything this turn touched" return real data.
