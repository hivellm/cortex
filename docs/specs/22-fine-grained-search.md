# Spec 22 — Fine-grained per-backend search

> **Status**: 🟢 implemented (phase 11v).
> **Source**: rework analysis 2026-05-04 (operator + agent could not
> inspect a single backend without leaving the MCP surface for shell
> + curl + raw SDK). Builds on spec 11 (`cortex_query` fused
> retrieval) and spec 18 (MCP tool surface).

## Goal

Expose three direct read-only proxies to the keyword (Meili), vector
(Vectorizer), and graph (Nexus) backends so MCP callers can ask
single-lane questions that the fused `cortex_query` cannot answer
without losing fidelity. The fused surface stays the right default
for "give me context about X"; this spec covers the right tools for
"show me ONLY the relationships" / "raw cosines in collection Y" /
"literal Meili hits, no projection".

## Endpoints

All three endpoints are mounted on the same `cortex-api` HTTP
service that hosts `/v1/query`. Auth posture matches `/v1/query`:
no key required by default; the dashboard sub-router stays
gated on `CORTEX_DASHBOARD_AUTH=1`.

### `POST /v1/search/keyword`

Request:

```jsonc
{
  "index": "cortex_consolidations",   // required, Meili uid
  "q": "retention daemon recovery",   // optional, default ""
  "limit": 20,                        // 1..=100, default 20
  "filter": "repo = \"cortex\"",     // optional Meili filter
  "sort": ["occurred_at:desc"],       // optional
  "attributes_to_retrieve": ["body", "occurred_at"]  // optional
}
```

Response:

```jsonc
{
  "index": "cortex_consolidations",
  "hits": [ /* raw Meili hits, pass-through */ ],
  "processing_time_ms": 4,
  "estimated_total_hits": 12345
}
```

Backed by direct `POST {meili}/indexes/{uid}/search`. URL resolution:
`CORTEX_FULLTEXT_MEILI_URL` → `MEILI_URL`. Optional bearer:
`CORTEX_FULLTEXT_MEILI_KEY` → `CORTEX_FULLTEXT_MEILI_API_KEY` →
`MEILI_MASTER_KEY`.

### `POST /v1/search/vector`

Request (v1 — raw vector only):

```jsonc
{
  "collection": "cortex.consolidation.fp32",
  "query_vector": [0.012, -0.043, /* … */],  // f32 list
  "k": 10,                          // 1..=100, default 10
  "score_threshold": 0.65           // optional cosine floor
}
```

Response:

```jsonc
{
  "collection": "cortex.consolidation.fp32",
  "hits": [
    { "id": "01ULID...", "score": 0.91, "payload": { /* */ } }
  ],
  "upstream_latency_ms": 23
}
```

Backed by direct `POST {vectorizer}/collections/{name}/search/text`
(the text-search endpoint accepts a raw embedding when the caller
phrases it as the body's vector field; mirrors the
`VectorizerLane` bypass that avoids the SDK's payload-dropping bug
documented in phase 11d). URL resolution: `CORTEX_VECTORIZER_URL` →
`CORTEX_EMBEDDER_VECTORIZER_URL` → `VECTORIZER_URL`.

`query_text` is reserved for a follow-up (server-side embedding
needs the embedder bin's model handle); the v1 handler returns
HTTP 400 + reason `not_implemented` when the field is set so callers
fail loudly rather than silently falling back to a different
embedding shape.

### `POST /v1/search/graph`

Two modes via the `mode` discriminator.

**`neighbors` mode** — canned Cypher walk:

```jsonc
{
  "mode": "neighbors",
  "node_id": "01ULID...",  // event_id property on the seed node
  "depth": 2,              // 1..=5, default 1
  "edge_kinds": ["IN_REPO", "REMEMBERS"]  // optional, alphanumeric
}
```

**`cypher` mode** — raw Cypher, gated on
`CORTEX_GRAPH_CYPHER_ENABLED=1`. Otherwise returns HTTP 403 + reason
`cypher_disabled`. The gate exists because raw Cypher exposure on
the MCP surface is operator-elevated and an unsigned descriptor
cannot land arbitrary statements in production.

```jsonc
{
  "mode": "cypher",
  "statement": "MATCH (n:Decision)-[:SUPERSEDES]->(m) RETURN n, m LIMIT 25",
  "parameters": { /* optional */ }
}
```

Response (both modes):

```jsonc
{
  "mode": "neighbors",   // echoed
  "nodes": [
    { "node_id": "01ULID...", "labels": ["Turn"], "properties": { /* */ } }
  ],
  "edges": [
    { "from": "01ULID...", "to": "01ULID...", "kind": "REMEMBERS",
      "properties": { /* */ } }
  ]
}
```

Backed by `nexus_sdk::NexusClient::execute_cypher`. The handler
projects the Nexus query result into the lean `nodes` + `edges`
shape using the rows' `id` / `labels` / `properties` (nodes) and
`start` / `end` / `type` / `properties` (edges); deduplication is
by `node_id` and by the `(from, to, kind)` tuple respectively.

## Soft-error envelope

Every endpoint shares the `cortex_query` payload contract. When the
serialised response would cross **`MCP_RESPONSE_HARD_CAP = 30 KB`**
(the MCP transport's per-tool cap), the handler returns HTTP 413 +
the soft-error envelope:

```jsonc
{
  "error": "budget_exceeded",
  "payload_bytes": 41201,
  "transport_cap_bytes": 30720,
  "suggested_limit": "reduce `k` / `limit` or scope the query more tightly"
}
```

Wire reasons used today (one per failure path):

| Reason                  | Status | When |
|-------------------------|--------|------|
| `bad_input`             | 400    | required field missing or empty |
| `not_implemented`       | 400    | `query_text` mode not yet supported |
| `index_not_found`       | 404    | Meili 404 on the named index |
| `cypher_disabled`       | 403    | `mode=cypher` without env gate |
| `depth_out_of_range`    | 400    | `depth` not in `1..=5` |
| `vectorizer_unconfigured` / `meili_unconfigured` / `nexus_unconfigured` | 503 | daemon booted without the corresponding URL |
| `vectorizer_transport` / `meili_transport` / `nexus_transport`         | 502 | upstream connect / send failed |
| `vectorizer_non_2xx` / `meili_non_2xx` / `vectorizer_decode` / `meili_decode` | 502 | upstream returned non-success or unparseable body |
| `client_build`          | 500 | `reqwest::Client::builder` failed |
| `serialise`             | 500 | response body could not be serialised |
| `budget_exceeded`       | 413 | response body crossed `MCP_RESPONSE_HARD_CAP` |

## MCP tool surface (spec 18 link)

`ToolRegistry::default_set()` registered three new tools alongside the
seven phase11 originals (size 7 → 10). Phase13g §1–§3 adds three more
grounding tools (10 → 13).

| Tool                       | Endpoint                          | MCP descriptor highlights |
|----------------------------|-----------------------------------|---------------------------|
| `cortex_keyword_search`    | `POST /v1/search/keyword`         | `index` required; `q`, `limit` (≤ 100), `filter`, `sort`, `attributes_to_retrieve`. |
| `cortex_vector_search`     | `POST /v1/search/vector`          | `collection` + `query_vector` required; `k` (≤ 200), `score_threshold`. v1 declines `query_text`. |
| `cortex_graph_query`       | `POST /v1/search/graph`           | `mode` discriminator; `neighbors.depth ≤ 5`; `cypher` gated. |
| `cortex_active_work`       | `GET /v1/dashboard/active-work`   | Optional `repo` filter; returns `{ active_tasks, in_progress_count, blocked_count, recent_archives }`. MCP caps `active_tasks` at 50. |
| `cortex_similar_sessions`  | `POST /v1/search/similar-sessions` | `query` + `repo` required; `k` (≤ 10, default 5), `confidence_floor` (default 0.6); returns `{ rows: ConsolidationHit[], total, filter }`. |
| `cortex_decision_chain`    | `GET /v1/search/decision-chain`   | `event_id` (ULID `[0-9A-Z]{26}`) required; `max_hops` (≤ 16, default 16); returns `{ chain, walked_predecessors, walked_successors }`. |

### Phase13g §1–§3 wire shapes

- **`cortex_active_work`** returns
  ```json
  {
    "active_tasks": [{"id": "phase13g_demo", "phase": "phase13g", "status": "in-progress",
                      "next_unchecked_item": "4.5 add renderer tests", "blocked_reason": null,
                      "repo": "cortex"}],
    "in_progress_count": 1,
    "blocked_count": 0,
    "recent_archives": [{"id": "2026-05-25-phase13f_demo",
                         "archived_at": "2026-05-25",
                         "title": "Dashboard handlers are pure readers"}]
  }
  ```
  Error taxonomy: HTTP 200 with empty arrays when no metadata store
  wired or the workspace tree is missing; soft-error envelope on
  filesystem permission denied (`reason: workspace_unreachable`).

- **`cortex_similar_sessions`** returns
  ```json
  {
    "rows": [{"consolidation_id": "cons-ses-001", "session_id": "01HSESS",
              "title": "rework analysis", "summary_markdown": "…",
              "source_event_count": 3, "occurred_at": "2026-05-19T12:00:00Z",
              "score": 0.91}],
    "total": 1,
    "filter": {"repo": "cortex", "k": 5, "confidence_floor": 0.6}
  }
  ```
  Error taxonomy: `400 bad_input` on missing `query`/`repo` or
  non-slug repo; `404 collection_missing` when the consolidation
  collection does not exist for the repo; `502 vectorizer_unreachable`
  on backend transport failure.

- **`cortex_decision_chain`** returns
  ```json
  {
    "chain": [{"event_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
               "slug": "adr-009-sweep-trait", "status": "superseded",
               "date": "2026-05-19", "title": "Sweep trait …",
               "supersedes": null,
               "superseded_by": "01ARZ3NDEKTSV4RRFFQ69G5FAW"}],
    "walked_predecessors": 0,
    "walked_successors": 1
  }
  ```
  Error taxonomy: `400 bad_input` on non-ULID `event_id` or
  `max_hops` outside `[1, 16]`; `502 nexus_unreachable` on backend
  transport failure.

## Decisions

- **Direct HTTP for keyword + vector** rather than threading the
  client traits through `ApiState`. The lanes use the same
  bypass-the-SDK shape (phase 11d). Reusing the env-resolved direct
  POST keeps `search_proxy` self-contained; the cost is duplicating
  ~20 lines of URL / auth resolution.
- **Cypher mode env-gated** rather than an explicit operator
  allow-list. The agent loop cannot escalate to raw Cypher unless
  the operator opted in at boot; an allow-list of canned templates
  belongs in a follow-up if Cypher mode survives to production with
  non-trivial usage.
- **`query_text` deferred**, not removed. The field is part of the
  v1 schema so the next release can land server-side embedding
  without a wire-shape break.
- **`30 KB` cap shared with `cortex_query`** rather than per-tool
  budgets. One ceiling keeps the MCP transport contract uniform
  across the four search surfaces.

## Follow-ups

- `query_text` (server-side embedding) lands once the embedder bin
  exposes a `embed_text(text) -> [f32]` accessor or an HTTP
  pass-through.
- Allow-list of canned Cypher templates as a third graph mode
  (`mode: "template"`) so common walks (decision-trace, evidence-of,
  etc.) are reachable without env-gating raw Cypher.
- Per-tool latency histograms on `/metrics` once the search surface
  sees enough traffic to size the buckets.
- ADR — if `cypher` mode survives to production with non-trivial
  usage, lift the env gate + the canned-template allow-list into a
  new ADR ("Raw Cypher exposure on the MCP surface — env-gated
  read-only by default").

## Tests

| Suite | Count | Covers |
|-------|-------|--------|
| `cortex-api::search_proxy::tests` | 9 | request serde defaults, clamps, budget cap, cypher gate env read |
| `cortex-mcp-server::tools::tests::registry_returns_ten_tools_with_unique_names` | 1 | tool registry size + name set |
| `cortex-mcp-server::server::tools_list_returns_ten_descriptors` | 1 | `tools/list` round-trip |
| `cortex-mcp-server::tests::hook_drift::plugin_hook_shims_match_adapter_canonical_sources` | 1 | (touched indirectly when phase 11x retired the `.ps1` shims) |

Live smoke verifying the surface end-to-end against the running
stack:

```bash
curl -sS -X POST http://127.0.0.1:17000/v1/search/keyword \
  -H 'content-type: application/json' \
  -d '{"index":"cortex-cortex-consolidations","q":"","limit":3}'
```
