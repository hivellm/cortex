# phase0 — Nexus 2.3.2 MERGE silently drops inline relationship properties

## Why

Live manual verification of phase27a (graph edge confidence tiers) on the
running stack (`hivehub/nexus:2.3.2`, `cortex/api:dev` @ 22b997b) found
that **no edge in the graph carries any relationship property** — not
`confidence`, not the provenance triple, not `TOUCHED.operation`.

Root cause, isolated against the live Nexus `/cypher` endpoint:

- `MATCH (a),(b) MERGE (a)-[r:T { k: v }]->(b)` **creates the edge but
  drops the inline relationship properties** — read-back of `r.k` is
  `null`. Confirmed on fresh node pairs + fresh edge types, so it is not
  a stale-edge / merge-key artefact.
- `MATCH (a),(b) CREATE (a)-[r:T { k: v }]->(b)` **does persist** the
  props — read-back of `r.k` returns the value.
- `SET r.x = ...` / `SET r += {...}` / `ON CREATE SET` all raise
  `Unknown variable 'r' in SET clause` (already documented in
  `crates/cortex-workers/src/graph/nexus_client.rs::render_edge_merge`).

The production writer `render_edge_merge` (nexus_client.rs:509) emits the
MERGE-with-inline-props form on purpose (idempotency; the phase15c
comment claims it is "the only accepted way" and was validated). That
claim does not hold for relationship properties on Nexus 2.3.2 — the
write is accepted, the edge lands, the props vanish.

## What Changes

Pick one (decide in design):

1. **Writer fix — persist rel props via a mechanism Nexus 2.3.2 honours.**
   Keep MERGE for relationship identity (idempotent), then attach props
   without `SET r.*` (rejected). Candidates: MERGE the propless edge then
   a guarded delete + `CREATE` when props are present; or an UNWIND batch
   form if Nexus persists rel props under UNWIND+MERGE the way it does
   for nodes (nexus#13). Must stay idempotent under replay.
2. **Upstream Nexus fix.** File a Nexus issue: MERGE must persist inline
   relationship properties (or support `SET` on a relationship variable).
   Gate phase27a + provenance persistence on the fixed release, mirroring
   the nexus#12 pattern.

Either path ends with a **live** re-verification: write a real edge
through the worker, read `r.confidence` back from Nexus, confirm the
graph lane + dashboard surface the tier.

## Impact
- Affected specs: `docs/specs/07-graph-writer.md` (edge-prop persistence
  contract; Edge-confidence tiers section).
- Affected code: `crates/cortex-workers/src/graph/nexus_client.rs`
  (`render_edge_merge` / `render_edge_props_inline`); knock-on for the
  stale-edge sweeper (provenance props) and phase27a confidence consumers.
- Breaking change: NO (writer-internal; read paths already null-safe).
- User benefit: phase27a confidence weighting actually takes effect in
  production; provenance-based stale-edge retirement becomes reliable
  when the projection is re-enabled.

Source: live manual test 2026-06-20 (this session).
