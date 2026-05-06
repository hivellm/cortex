# Proposal: phase15b_graph-mapper-edge-expansion

Source: `docs/analysis/rework/03-relevance.md` Achado 3 (HIGH).

## Why

The graph mapper today emits only `IN_REPO` and `REMEMBERS` edges. The graph spec defines ~12 edge kinds (`CALLS`, `IMPORTS`, `DEFINES`, `RETURNS`, `SUPERSEDES`, `CONTRADICTS`, `EMITTED_BY`, `ABOUT`, `ANSWERED_BY`, `CITES`, `MENTIONS_FILE`, `RELATES_TO`). The graph lane returns "nothing useful" because there are no edges to walk. The 4-doc relevance audit names this the third largest source of "nada relevante".

## What Changes

- Implement the 10 missing edge kinds in `crates/cortex-workers/src/graph/projection.rs`.
- Each edge kind ships with an extractor function (`extract_calls`, `extract_imports`, ...) that runs on every projected envelope and writes the matching edges to Nexus.
- New `graph_mapper_test_corpus` ships 50 fixture envelopes covering all 12 edge kinds.
- Doctor `cortex-ops doctor graph-coverage` reports edge count per kind across the live graph.

## Impact

- Affected specs: `docs/specs/07-graph.md` § Edge taxonomy.
- Affected code: `crates/cortex-workers/src/graph/{projection.rs,extractors/{calls,imports,defines,returns,supersedes,contradicts,emitted_by,about,answered_by,cites,mentions_file,relates_to}.rs}` (new modules).
- Breaking change: NO at the wire; new node + edge data lands additively.
- User benefit: graph queries return non-empty result sets; multi-hop reasoning becomes possible.
