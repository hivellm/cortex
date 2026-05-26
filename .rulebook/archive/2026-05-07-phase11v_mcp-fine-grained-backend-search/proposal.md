# Proposal: phase11v_mcp-fine-grained-backend-search

## Why

`cortex_query` (spec 11) is a fused retrieval primitive: it runs the keyword + vector + graph lanes in parallel, fuses scores, and returns one ranked list. That is the right default surface for "give me context about X" but the wrong surface for three classes of operator + agent question:

1. **"Show me only the relationships"** — the graph lane is hidden behind fusion. There is no way to ask "every node connected to event X within 2 hops, edge labels included" without the keyword/vector noise. Every Nexus introspection today requires shelling into the container or hand-writing Cypher against the SDK.
2. **"Show me the closest vectors in collection Y"** — the vector lane is per-repo by default, hot-tier (fp32) only, and embedded inside the fusion score. There is no way to ask "top-50 nearest neighbours in `cortex.consolidation.fp32` to this query string, raw cosines, no filtering" — useful for recall debugging, embedding-drift checks, and consolidation cluster inspection.
3. **"Show me only the keyword hits"** — the keyword lane fuses against the others; raw Meili results (with all returned fields, exact `processingTimeMs`, full facets) never reach the agent. Retrieval-quality work needs the unfiltered view.

Result: agents and operators reach for shell + curl + the SDKs whenever they need to inspect a single backend. That defeats the MCP surface's whole point and locks fine-grained corpus introspection out of the agent loop.

## What Changes

Three new MCP tools — one per backend — each backed by a thin cortex-api endpoint that proxies to the existing client handle. Nothing new on the storage side; this is a read-only surface that exposes data the daemon already pulls.

1. **`cortex_vector_search`** -> `POST /v1/search/vector`
   - Inputs: `{ collection, query_text | query_vector, k, repo?, kind?, score_threshold? }`
   - Output: `{ collection, hits: [{ event_id, score, payload_excerpt, repo, kind, occurred_at }] }`
   - Backed by `LiveVectorizerClient::search_vectors` (no fusion, no keyword/graph cross-lane mixing).
   - Defaults to hot tier (fp32). Operator can request `cortex.consolidation.pq` / `cortex.cold.binary` explicitly.
2. **`cortex_keyword_search`** -> `POST /v1/search/keyword`
   - Inputs: `{ index, q, limit, filter?, sort?, attributes_to_retrieve? }`
   - Output: `{ index, hits: [...full Meili documents...], processing_time_ms, estimated_total_hits }`
   - Backed by `LiveMeiliClient::search` raw — operator chooses the index (`cortex_decisions` / `cortex_consolidations` / `cortex-{repo}-turns` / etc.).
3. **`cortex_graph_query`** -> `POST /v1/search/graph`
   - Two query modes:
     - `{ mode: "neighbors", node_id, depth?, edge_kinds? }` -> BFS/DFS walk with edge labels
     - `{ mode: "cypher", statement, parameters? }` -> raw Cypher (operator-gated)
   - Output: `{ mode, nodes: [{ node_id, labels, properties }], edges: [{ from, to, kind, properties }] }`
   - Backed by `nexus_sdk::NexusClient`. Cypher mode is opt-in via env var `CORTEX_GRAPH_CYPHER_ENABLED=1` so unsigned operator input cannot land arbitrary Cypher in production.

Each tool returns the same hard-cap envelope shape `cortex_query` uses (the MCP transport's 30 KB cap is shared) — soft-error `budget_exceeded` with `suggested_limit` when payload overflows.

## Impact

- **Affected specs:** new `docs/specs/22-fine-grained-search.md` documenting the three endpoints + tool descriptors.
- **Affected code:** `crates/cortex-api/src/search_proxy.rs` (new), `crates/cortex-api/src/http.rs` (3 new routes), `crates/cortex-mcp-server/src/tools.rs` (3 new tool impls + registry bump 7 -> 10), `crates/cortex-mcp-server/src/lib.rs` (re-exports).
- **Breaking change:** NO. Additive tools; existing `cortex_query` unchanged.
- **User benefit:** agents can ask "what does the graph think about X?" / "what's in this Vectorizer collection?" / "what does Meili literally have for query Y?" without leaving the MCP surface or piping through `docker exec`.

## Source

Conversation 2026-05-04: user requested fine-grained per-backend search to inspect relationships, specific vectors, etc. — beyond the current fused `cortex_query`.
