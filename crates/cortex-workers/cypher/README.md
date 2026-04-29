# cortex-graph Cypher templates

> **DEPRECATED RUNTIME PATH (2026-04-27).**
> The live writer (`crates/cortex-graph/src/nexus_client.rs::run_write_tx`)
> no longer reads from these templates. Every node and edge MERGE is
> rendered per-row at runtime with values inline-escaped into the
> Cypher string (see [`crates/cortex-graph/src/cypher.rs`](../src/cypher.rs)
> §`render_node_merge` / `render_edge_merge`). The on-disk files in
> this directory are kept only so the
> `shipped_cypher_dir_satisfies_required_set` regression test still
> exercises the file-based loader API for future read-side use.

## Why the runtime stopped reading them

Nexus 1.15.0 silently drops every Cypher write that touches an
`UNWIND` row or that uses `$param` substitution inside a write
clause. The HTTP call returns 200 with zero rows but **nothing
persists**. Reads with `$param` work fine; only the write
substitution path is broken.

The previous template path used exactly that shape:

```cypher
UNWIND $rows AS row
MERGE (n:Session { id: row.key })
SET n += row.props
```

…which Nexus 1.15 acknowledges and discards. The fix —
`phase1_graph_writer_nexus_compat` — moves the writer to
per-row literal-interpolated Cypher (`MERGE (n:Session { id:
'<escaped>'} ) SET n.x = '<v>' RETURN count(*) AS written`)
that the server actually persists, with a `RETURN count`
guard that surfaces silent drops as real errors instead of
`outcome="ok"` lies.

## When this directory becomes load-bearing again

Re-enable the template-driven write path once one of:

1. Nexus ships a release that supports `UNWIND $rows … MERGE …`
   with row-bound literals, **or**
2. We move read queries (graph traversals, `MATCH`-only) onto
   this registry so the file format earns its keep.

Either path leaves the file naming convention (`node_<label_snake>`,
`edge_<from>__<rel>__<to>`) intact — this README is the dependency
the rewrite needs to find before re-wiring the runtime.

## Source of truth right now

Reading order for anyone debugging a graph-write bug:

1. `docs/specs/07-graph-writer.md` — top-level spec.
2. `crates/cortex-graph/src/cypher.rs` — the per-row renderers.
3. `crates/cortex-graph/src/nexus_client.rs::run_write_tx` — the
   live writer.
4. `crates/cortex-graph/tests/worker.rs` — the integration probe.
