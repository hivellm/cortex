# Proposal: phase4c_graph_richer_edges_defines

## Why

The Nexus graph today carries only two edge types — confirmed by
the audit on 2026-04-27 22:36 UTC:

```
nodes:  Artifact 3634, Repo 3, Session 9, Turn 28,
        LawViolation 72, Decision 12, Memory 24
edges:  IN_REPO 10245, REMEMBERS 30
```

Without `(:Symbol)-[:DEFINES]->(:Artifact)` (or any imports / calls
graph), Nexus answers exactly two relationship questions:

1. "Which artifacts belong to which repo?" (`IN_REPO`)
2. "Which sessions remember which memories?" (`REMEMBERS`)

It cannot answer "where is `PreThinkingTool` defined", "what
artifacts in repo X define a public symbol", or any cross-artifact
question — degrading the graph lane to a path enumerator. The
audit's classifier-worker probe demonstrated this: Cypher returned
five paths matching `CONTAINS "classifier"` but no symbol-level
information.

The data needed to populate `Symbol` nodes is **already produced
upstream**. The Vectorizer payload sample captured during the audit
contains the symbol field at the chunk level:

```json
{
  "chunk_content_hash": "70dabebd...",
  "language": "rust",
  "parent_event_id": "01KQ84GPYDD3B2XCXPAQCMP70W",
  "path": "crates/cortex-mcp-server/src/tools.rs",
  "repo": "Cortex",
  "source": "code",
  "symbol": "PreThinkingTool",
  "topics": "code,rust"
}
```

So the chunker (cortex-bootstrap or cortex-classifier) is emitting
`symbol` per chunk, but `cortex-graph` is dropping it. The mapper
at [crates/cortex-graph/src/mapper.rs](../../../crates/cortex-graph/src/mapper.rs)
only emits Artifact + IN_REPO from artifact events.

## What Changes

- Extend the Nexus schema with a new node label `Symbol` and a new
  edge type `DEFINES`.
- `Symbol` natural key: `(repo, language, qualified_name)` —
  qualified by file path when language doesn't carry FQN (e.g. C
  without namespaces) — keeps cross-file collisions distinct.
- `cortex-graph::mapper` reads the `symbol` field from every
  artifact-chunk event whose `source = "code"` (and `symbol` is
  non-empty), produces a Patch with:
  - `MERGE` on the `Symbol` node
  - `MERGE (:Symbol)-[:DEFINES]->(:Artifact)` keyed by
    (symbol_key, artifact_path, repo)
- `cortex-graph::cypher` gets a new template `symbol_merge` and
  `defines_merge`, registered in `CypherTemplates`.
- The writer's existing dedup / ack pipeline carries the new
  patch type without changes — `Patch` is generic over node /
  edge shape.
- Backfill: replay the existing event archive once with the new
  mapper to populate symbols for the artifacts already in Nexus.
  Idempotent via `MERGE`.
- Out-of-scope (deferred to a follow-up phase): `IMPORTS`, `CALLS`,
  `EXTENDS`, `IMPLEMENTS`. Those need parser-level analysis and
  belong in a dedicated task once the chunker emits richer
  metadata.

After this change, the audit's "find PreThinkingTool" question
becomes:

```cypher
MATCH (s:Symbol {name: "PreThinkingTool"})-[:DEFINES]->(a:Artifact)
RETURN a.repo, a.path, s.language
```

— a single hop, deterministic, with file path + repo + language.

## Impact

- Affected specs: spec-12 (graph mapper — adds `Symbol` and
  `DEFINES`).
- Affected code:
  - `crates/cortex-graph/src/schema.rs` — `Symbol` label,
    `DEFINES` edge in node/edge enums
  - `crates/cortex-graph/src/mapper.rs` — emit symbol patches
    from code-chunk events
  - `crates/cortex-graph/src/cypher.rs` —
    `render_symbol_merge`, `render_defines_merge`
  - `crates/cortex-graph/src/identity.rs` — natural key
    composition for `Symbol`
  - tests: unit on the mapper (chunk → patch), integration on
    nexus_client against a live Nexus container, end-to-end via
    a fixture archive
- Breaking change: NO. Schema is purely additive; existing IN_REPO
  / REMEMBERS edges and queries are untouched.
- User benefit: graph lane gains symbol-level resolution, lifting
  Nexus from "list files in repo" to "find definition of X" — a
  qualitative step.

## Source

- Audit data captured 2026-04-27 22:36 UTC.
- Symbol field confirmed in Vectorizer payload (e.g.
  `crates/cortex-mcp-server/src/tools.rs` → `PreThinkingTool`).
- Current mapper at
  [crates/cortex-graph/src/mapper.rs](../../../crates/cortex-graph/src/mapper.rs).
- Cypher templates at
  [crates/cortex-graph/src/cypher.rs](../../../crates/cortex-graph/src/cypher.rs).
