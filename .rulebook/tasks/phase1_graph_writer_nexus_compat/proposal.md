# Proposal: phase1_graph_writer_nexus_compat

## Why

`cortex-graph` writes graph patches against Nexus 1.15.0 using
parametrised UNWIND-MERGE templates. The writer logs
`nodes_upserted=271 edges_upserted=135 outcome="ok"` after every
batch — but Nexus actually has **1 unlabeled node, 0 relationships**
after a full bootstrap. Probing the live server with `curl` shows:

- `UNWIND $rows AS row MERGE (n:Label {key: row.k}) SET n += row.props`
  → HTTP 200, **0 rows persisted, no error**.
- `MERGE (n:Label {key: $k}) SET n += $props`
  → HTTP 200, **0 rows persisted, no error**.
- `MERGE (n:Label {key: "literal"}) SET n.x = "lit", n.y = "lit"`
  → persists ✅.
- `MATCH (a:L1{key:"l1"}), (b:L2{key:"l2"}) MERGE (a)-[r:T]->(b) RETURN r`
  → persists ✅ (separate statement, no UNWIND, no params for write data).
- `FOREACH (k IN [...] | MERGE …)` → parse error.
- POST `/data/nodes` typed SDK API → persists ✅.

So Nexus 1.15.0 silently drops every write that touches an UNWIND row
or that uses `$param` substitution **inside** a write clause. Reads with
`$param` work fine. The cortex-graph templates use exactly the shapes
Nexus drops, and `execute_with_retry` only checks that the HTTP call
succeeds — it never verifies that anything was actually written.
Result: writer reports success, no data lands.

## What Changes

Replace the UNWIND-MERGE template path in
`crates/cortex-graph/src/nexus_client.rs::run_write_tx` with a
per-row Cypher execution that interpolates values directly into the
statement (with proper escaping) instead of relying on `$param`
substitution for write data.

- **Nodes:** one `MERGE (n:<Label> { <key_field>: "<escaped_natural_key>" }) SET n.<k1>=<lit>, n.<k2>=<lit>, ...` per node. Optional fast-path: SDK `create_node` for greenfield writes when no SET-on-existing is needed.
- **Edges:** one `MATCH (a:<FromLabel> { <key>: "<from>" }), (b:<ToLabel> { <key>: "<to>" }) MERGE (a)-[r:<TYPE>]->(b) SET r.<k>=<lit>, ...` per edge.
- **Verification:** execute every write with a `RETURN count(*) AS n` (or `RETURN r`) so we can assert `result.rows.len() > 0` and stop reporting silent successes.
- **Templates:** the `.cypher` template registry stays for governance, but the templates are rewritten to per-row shapes. The writer interpolates the literal values into a placeholder (`{key}`, `{props_set}`, etc.) and substitutes through a small in-process renderer that escapes Cypher string literals.
- **Escape function:** lift a `cypher_str_escape(&str)` helper that handles `\`, `'`, `"`, control chars, and refuses any input that would close out of the string literal.

The fix is intentionally scoped to "make the writer actually persist what it claims it persisted." Throughput will drop (one round-trip per row vs. one per batch), but a writer that lies is worse than one that's slow. A future task can revisit batching once we know which Cypher shape Nexus 1.15 accepts for writes (or once Nexus ships a fix for UNWIND-write).

## Impact

- Affected specs: 07 (graph writer) — replace §Cypher generation
  notes about UNWIND-MERGE with the per-row interpolated shape, plus
  a §Compat note about Nexus 1.15.0 silently dropping UNWIND writes.
- Affected code:
  - `crates/cortex-graph/src/nexus_client.rs::run_write_tx` (rewrite).
  - `crates/cortex-graph/src/cypher.rs` (renderer + escape helper).
  - `crates/cortex-graph/cypher/*.cypher` (rewrite all node + edge
    templates to single-row interpolated shape).
  - `crates/cortex-graph/tests/worker.rs` and
    `tests/mapper.rs` — re-baseline expected Cypher strings.
- Breaking change: NO for callers (GraphPatch / GraphWriter API
  unchanged); YES for the wire shape (no more UNWIND-batch).
- User benefit: cortex-graph actually writes to Nexus, the dashboard's
  Graph view stops showing an empty canvas, and `MATCH (n)` in the
  cortex-api graph endpoint returns real data.

Source: docs/specs/07-graph-writer.md and direct Nexus 1.15.0 probes
captured in this task's `.notes/` (added during implementation).
