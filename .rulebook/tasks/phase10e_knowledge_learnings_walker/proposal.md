# Proposal: phase10e_knowledge_learnings_walker

## Why

The audit found 60 curated entries on disk that are invisible to
the agent because no lane indexes them:

- `.rulebook/knowledge/*.md` — 20 patterns / anti-patterns the
  Rulebook MCP captures via `rulebook_knowledge_add`.
- `.rulebook/learnings/*.md` — 40 implementation learnings the
  Rulebook MCP captures via `rulebook_learn_capture`.

These are the highest-signal corpus we have. They were written
specifically because someone made a mistake worth not repeating.
Every other surface (`memory`, `decisions`, `analyses`) flows
through Cortex; knowledge + learnings sit on disk and never reach
the embedder, the keyword lane, or the graph.

## What Changes

1. Add two new envelope kinds: `knowledge` (pattern /
   anti-pattern) and `learning` (implementation insight). Both
   inherit the canonical envelope shape; `payload.category`
   discriminates pattern vs anti-pattern; `payload.source` is
   `rulebook.knowledge` / `rulebook.learning`.
2. Bootstrap walker MUST recurse into `.rulebook/knowledge/` and
   `.rulebook/learnings/` for every repo it bootstraps and emit
   one envelope per file.
3. Embedder collections: `cortex.knowledge.fp32` /
   `cortex.learning.fp32` (single-tier — these are small, dense,
   and worth keeping at full precision).
4. Meili indexes: `cortex_knowledge` + `cortex_learnings`,
   settings inherit from the existing `cortex_memories` shape.
5. Nexus labels `:Knowledge` and `:Learning` with edges
   `(:Session)-[:LEARNED]->(:Learning)` and
   `(:Knowledge)-[:RELATES_TO]->(:Decision)` so the graph lane
   surfaces them next to the decisions they support.
6. `/v1/dashboard/memory` exposes a `?facets=knowledge,learning`
   filter so the GUI can show them next to memories.
7. Pre-thinking pulls knowledge + learnings on every
   `pre_change_context` and `decision_lookup` query — these are
   the corpora the agent benefits from re-reading before acting.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` §collections
  + indexes + labels, `docs/specs/06-embedder.md`
  §single-tier kinds, `docs/specs/12-pre-thinking-injection.md`
  §sources.
- Affected code: `crates/cortex-storage/src/collections.rs`,
  `crates/cortex-storage/src/fulltext.rs`,
  `crates/cortex-storage/src/graph.rs`,
  `crates/cortex-cli/src/bootstrap/walker.rs`,
  `crates/cortex-workers/src/embedder/`,
  `crates/cortex-pre-thinking/src/scope.rs`.
- Breaking change: NO. Pure additive corpus.
- User benefit: 60 curated entries (the most valuable corpus we
  have) become reachable via every retrieval surface.
