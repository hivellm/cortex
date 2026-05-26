# Diagnosis (2026-04-27 18:40 live probes)

## Pre-fix Nexus state (after Cortex bootstrap with phase1 writer)

| Label / Edge | Count |
|---|---|
| Session | 2 |
| Turn | 17 |
| Repo | 1 |
| Artifact | 1 827 |
| Decision / Memory / LawViolation / Law / ToolCall / AgentCall / Analysis | 0 |
| `IN_REPO` | 3 075 |
| `HAS_TURN` | **2** (expected ≥ 17) |
| Every other edge type | **0** |

The bootstrap corpus only emits `Artifact` + a synthesised `Session` + per-commit `Turn` envelopes, so the zero counts on `HAS_TOOL_CALL`, `TOUCHED`, `LINKED_TO`, `OF`, `OBSERVED_IN`, `SUPERSEDES` are partly explained by the absence of the source events, **not** by writer drops. The acid test is `HAS_TURN`: 17 source Turns / 2 surviving edges → 15 edges silently lost.

Both endpoints are well-formed:

- `MATCH (s:Session) RETURN s.id` → 2 distinct ids.
- `MATCH (t:Turn) RETURN DISTINCT t.session_id` → same 2 ids. So every Turn has a Session that exists in Nexus.
- The mapper (`mapper.rs::map_event_to_patch`) emits the Session node into the *same* patch as the Turn, so within-batch ordering can't be the cause either.

## Hypothesis A — template / key-field mismatch

| File | Key field |
|---|---|
| `cypher/node_session.cypher` | `MERGE (n:Session { id: row.key })` |
| `cypher/node_turn.cypher` | `MERGE (n:Turn { id: row.key })` |
| `cypher/edge_session__has_turn__turn.cypher` | `MERGE (a:Session { id: row.from }) … (b:Turn { id: row.to })` |

No mismatch. (And those templates are no longer the runtime path post-`phase1_graph_writer_nexus_compat` — the writer renders per-row Cypher inline. Same `id` field is used.)

**Hypothesis A: rejected.**

## Hypothesis B — coalescer drops cross-batch edges

`coalescer.rs` deduplicates **nodes** by `(label, natural_key)` but forwards **every edge unchanged** (per spec-07 §Acceptance criteria). `seen_edges` exists on the struct but is never written to from the live path. So coalescing cannot drop edges.

**Hypothesis B: rejected.**

## Hypothesis C — silent MATCH-MERGE failure (the actual cause)

Direct probes against Nexus 1.15:

```
# both endpoints exist
MATCH (s:Session { id: "01KQ81954QP2GB89CB4MKAPE2T" }),
      (t:Turn    { id: "01KQ819Y4JSTQAB6PM8WCKRAY0" })
MERGE (s)-[r:HAS_TURN]->(t) RETURN count(r) AS written
→ rows = [[null]]              ← success, edge persists (count went 2 → 3)

# from-endpoint missing
MATCH (s:Session { id: "DOES-NOT-EXIST" }),
      (t:Turn    { id: "01KQ81A4WBQJ2TE6554PQQM6Q9" })
MERGE (s)-[r:HAS_TURN]->(t) RETURN count(r) AS written
→ rows = [[0]]                 ← write did NOT land
```

The writer's `assert_write_landed` only checks `result.rows.is_empty()`. Both responses above have `rows.len() == 1`, so the writer treats *both* as success. The `[[0]]` case is the silent-drop signal Nexus actually emits, and it's invisible to the current contract.

A second Nexus 1.15 quirk surfaced during probing: `MERGE (s)-[r:HAS_TURN]->(t)` is **not** idempotent at the relationship level. Repeating the same MERGE for an already-connected `(s, t)` pair creates a duplicate `HAS_TURN` row instead of returning the existing one. This is orthogonal to the silent-drop bug — it inflates counts, doesn't lose them — and is left for a future task once the writer reliability is restored.

**Hypothesis C: confirmed.**

## Verdict

The writer reliably calls `MATCH … MERGE (s)-[…]->(t) RETURN count(r) AS written` for every edge in the patch, but the post-write check ignores the `count` value. When the MATCH finds nothing (most often: the cross-batch case where the from-endpoint exists in Nexus but is not yet visible to the running query — Nexus' SDK uses an HTTP transport, so each `execute_cypher` is a fresh session), Nexus returns `[[0]]` and the writer happily increments `edges_upserted`.

## Targeted fix (matches proposal §What Changes path c)

1. Tighten `assert_write_landed`: read the first cell of `rows[0]`; treat an integer `0` as a failed write. `null` (Nexus 1.15's success placeholder for write queries) and any positive integer keep counting as success.
2. Surface every shortfall via `cortex_graph_edges_dropped{edge_type}` so the operator can spot recurrences after deploy.
3. Demote the writer's `edges_upserted` field from "things we sent" to "things Nexus confirmed it persisted" so the published `cortex.events.graphed` envelope stops lying to downstream lanes.

The fix is the same root cause as `phase1_graph_writer_nexus_compat`'s `[[null]]` quirk, just observed at the `count(*) == 0` boundary instead of the `rows.is_empty()` boundary.
