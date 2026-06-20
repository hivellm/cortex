# 03 — GraphRAG global queries + community summaries — **HIGH**

## What graphify does

graphify supports the two GraphRAG query modes:
- **Local** (`serve.py:query`): seed → BFS neighborhood → answer from a subgraph (Cortex already does the equivalent via its fusion/graph lane).
- **Global**: questions that no single chunk answers — "what are the major subsystems", "how does X relate to Y at the architecture level". These are served from **community structure** (file 02) + **per-community summaries**. `global_add(repo_tag)` even merges per-project graphs into one cross-repo global graph so the same global questions span repos.
- **Community/node summaries** (`docs/node-summaries-rfc.md`): deterministic file summaries (docstrings + exported symbols) by default, optional LLM-generated summaries behind an explicit flag, **budgeted at query time** (return only N nodes' summaries to fit the context window).

## What Cortex does today

Cortex has two summary tiers, but **neither is graph-community-derived**:
- **Consolidations** (`crates/cortex-workers/src/consolidator/`): LLM syntheses at grains Session / Topic / Decision. The Topic grain groups by **HDBSCAN over embeddings** (`consolidator/source/topic.rs`), i.e. semantic similarity, not graph subsystems.
- **Topic-cards** (`crates/cortex-workers/src/topic_cards/`, spec 12): living per-slug syntheses, rewritten on event/impact/age triggers; surfaced top-of-bundle by pre-thinking.

So Cortex can answer "what do we know about topic X" (embedding cluster) and "show neighbors of node Y" (graph traversal), but **not** "what are the structural subsystems of this codebase and how do they connect" — because that requires summarizing over *graph communities*, which don't exist yet (file 02).

**Gap:** the global/architecture-level query class is unserved. Cortex's summaries are organized by semantic topic and by event grain, not by the graph's own community partition.

## Recommendation for Cortex

Once communities exist (file 02), add **community summaries as a new consolidation grain** and a **global query path**:

1. **New consolidation grain `Community`** in the consolidator: input = the nodes/edges of one graph community (+ its god nodes and cross-community edges); output = a summary envelope ("Subsystem: retrieval lane — fuses BM25/dense/graph, owns RRF + reranker; talks to Vectorizer, Meili, Nexus"). Reuse the existing consolidation envelope + storage; only the *source selector* is new (community membership instead of HDBSCAN/session).
2. **Hierarchy:** Leiden levels → multi-resolution summaries (coarse "subsystems" → fine "modules"), matching graphify's oversized-split hierarchy.
3. **Global query routing:** detect architecture-level intent in the orchestrator (`crates/cortex-api/src/search/`) and route to community summaries first (map-reduce over community summaries → synthesized answer), instead of the per-chunk fusion lane. This is the GraphRAG "global search" pattern.
4. **Budgeted return:** adopt graphify's RFC discipline — return top-N community summaries within the pre-thinking byte budget rather than all of them.
5. **Cross-project:** Cortex is already multi-repo with cross-project propagation; community summaries per repo + a global tier give the `global_add` equivalent for free.

## Distinction to preserve

Keep **both** axes — they answer different questions:
- Embedding-topic consolidations / topic-cards = "what do we *know/decide* about subject X" (semantic, session-aware, governance).
- Community summaries = "how is the *system structured*" (topological).
Don't replace one with the other; add the community axis.

## Effort / impact

- **Impact:** HIGH — unlocks the architecture/onboarding query class agents currently can't get without reading many files.
- **Effort:** MEDIUM (mostly a new source-selector + intent route; reuses consolidation + orchestrator machinery). **Hard prereq: file 02.**
- **ADR-worthy:** "Community summaries as a consolidation grain + global query route" — record alongside the existing consolidation-grain decision (DEC-005).
