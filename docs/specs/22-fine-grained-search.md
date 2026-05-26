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

## Phase19 — Granular tool surface (16 new tools)

> **Status**: 🟢 implemented (phase 19). Builds on the three
> `/v1/search/{keyword,vector,graph}` proxies above by exposing
> envelope-shape and consolidation-first verbs the fused
> `cortex_query` cannot answer without post-filtering through a
> pre-thinking budget. Registry size lands at 29 (13 baseline +
> 16 phase19 verbs).

### Group A — envelope-shape granularity (5)

| Tool | Wire shape | Reads |
|------|------------|-------|
| `cortex_events_by_kind` | `POST /v1/search/events {kind, repo?, session_id?, since?, until?, q, limit≤50}` | Kind-routed Meili index per `kind_to_index()`; `occurred_at_ms` filter |
| `cortex_session_timeline` | `GET /v1/sessions/{session_id}/timeline?limit≤200&kind?` | `cortex_storage::archive::scan_envelopes_by_session` (Parquet archive walk on a blocking task) |
| `cortex_tool_calls` | `POST /v1/search/tool-calls {tool_name?, outcome?, repo?, since?, until?, q, limit≤50}` | `cortex_tool_calls` Meili index; `outcome` discriminator pinned to writer vocab (`ok`/`transient`/`rejected`/`task_failed`/`error`) |
| `cortex_files_touched` | `GET /v1/sessions/{session_id}/files-touched?limit≤100` OR `POST /v1/search/files-touched {repo, since, until, limit≤100}` | Per-session: `scan_envelopes_by_session`; window: `walk_envelopes` filtered to `kind=ToolCall`. `touched: Vec<TouchedArtifact>` preferred; fallback to `input.{path,file_path,filename,file}` with `classify_op` |
| `cortex_topic_search` | `POST /v1/topic-cards/search {topic_prefix, repo?, q, limit≤30}` | `cortex_topic_cards` Meili `topics` filter (trailing `:` stripped to bare-tag literal) |

### Group B — consolidation-first (6)

| Tool | Wire shape | Reads |
|------|------------|-------|
| `cortex_consolidation_get` | `GET /v1/consolidations/{id}` | ONE Meili call with OR filter `event_id="X" OR ext.consolidation.consolidation_id="X"` so re-emitted envelopes resolve via stable producer id |
| `cortex_consolidations_recent` | `GET /v1/consolidations/recent?repo&grain&since&until&limit≤30` | `cortex_consolidations` Meili sorted `occurred_at:desc`; grain vocab pinned to producer enum |
| `cortex_consolidations_by_entity` | `POST /v1/consolidations/by-entity {entity:{kind, value}, limit≤30}` | `repo`/`model` → authoritative Meili filter; `decision_id`/`file`/`function` → Meili keyword `q` fallback. `match_strategy` echoed |
| `cortex_consolidations_search` | `POST /v1/consolidations/search {query, k≤20, intent_hint?, repo?, grain?}` | BM25 over `cortex_consolidations` (vector+RRF deferred to a future commit that wires `kinds` scope into `cortex_query`). `match_strategy = "bm25"` |
| `cortex_consolidation_lineage` | `GET /v1/consolidations/{id}/lineage` | Doc-only projection: `topics` (`session:`/`file:`/`decision:`) + `DEC-\d{3,}` regex over title/summary/body + `ext.consolidation.model`. `match_strategy = "doc_only"` |
| `cortex_consolidations_diff` | `GET /v1/consolidations/diff?since_ts=<ms>&repo&limit≤200` | `cortex_consolidations` Meili sorted `occurred_at:asc` (schema lacks `accumulated_at`); poll-cursor pattern |

### Group C — governance + telemetry (5)

| Tool | Wire shape | Reads |
|------|------------|-------|
| `cortex_law_violations` | `POST /v1/laws/violations {repo, session_id?, law_id?, since?, until?, limit≤50}` | Per-repo `cortex-<slug>-governance` Meili index with `kind="law_violation"` pinned. `repo` REQUIRED (global `cortex_laws` schema is filter-poor) |
| `cortex_feedback_signals` | `POST /v1/feedback/list {helpful?, intent?, since?, until?, limit≤50}` | `pre_thinking_feedback` SQLite table via `MetadataStore::list_pre_thinking_feedback`. `repo` filter rejected with `bad_input` (table has no repo column) |
| `cortex_decision_search` | `POST /v1/decisions/search {q?, status?, tag?, repo?, since?, until?, limit≤50}` | Global `cortex_decisions` Meili index; `tag` mapped to `topics` filterable. `supersedes`/`superseded_by` rejected — pivot via `cortex_decision_chain` |
| `cortex_consolidation_costs` | `POST /v1/consolidations/costs {since, until, group_by:["grain"\|"model"\|"day"], repo?}` | `cortex_consolidations` Meili (cap 500 hits, `truncated` flag); local fold per axis. Cost cents/tokens `null` until writer projection lands |
| `cortex_query_explain` | `POST /v1/query/explain {query, intent?, scope?}` | Dispatches `state.service.handle_with_headers(...)` and projects into `{per_lane_hits[], fusion_math{rrf_k, alpha, recency_decay_lambda, drops[]}, final_envelope}`. `match_strategy = "envelope_only"` |

### Error taxonomy

All 16 verbs use the canonical reason set
(`crates/cortex-mcp-server/src/tools.rs`):

| Reason | HTTP | Meaning |
|--------|------|---------|
| `bad_input` | 400 | Caller-side validation failure (unknown enum, malformed RFC3339, missing required field, schema-unsupported filter) |
| `invalid_input` | — | Same as `bad_input` but surfaced through the MCP JSON-RPC `error` channel BEFORE the round-trip (MCP-side validation) |
| `not_found` | 404 | The id resolved nowhere (e.g. `cortex_consolidation_get` for an unknown ULID) |
| `api_unreachable` | 502 | The upstream backend (Meili / Vectorizer / Nexus) refused the connection |
| `api_http_error` | 502 | The upstream backend returned a non-2xx status the proxy could not classify |
| `tool_timeout` | 504 | Phase14i-shaped timeout fired while waiting for the backend response |
| `budget_exceeded` | 413 | Phase11c byte-budget clipper trimmed the response below the caller's `budget_bytes` floor |
| `rate_limited` | 429 | Caller's quota exhausted; `retry-after` header carries the back-off |
| `scope_forbidden` | 403 | ACL denied the resolved scope (only `cortex_query_explain` surfaces this; the other verbs do not run through the ACL gate) |

`match_strategy` (on the four tools that surface it —
`cortex_consolidations_by_entity` / `cortex_consolidations_search`
/ `cortex_consolidation_lineage` / `cortex_consolidation_costs` /
`cortex_query_explain`) signals which projection produced the
response so callers can detect partial / placeholder shapes
without re-running the request. Values land in
`{filter, q, bm25, doc_only, envelope_only}` today; the reserved
`hybrid_rrf` / `with_lane_hits` / `joined` values track the
follow-up wiring documented inline in each handler module.

### Tests (phase19 §5)

| Suite | Count | Covers |
|-------|-------|--------|
| `cortex-api::search::{events_by_kind,session_timeline,tool_calls,files_touched,topic_search}::tests` | 32 | filter assemblers, kind→index mapping, clamp_limit, RFC3339 parsing, classify_op |
| `cortex-api::search::{consolidation_get,consolidations_recent,by_entity,search,lineage,diff}::tests` | 40 | id validators, grain/status/intent vocabs, build_filter shapes, plan_search routing, lineage extractors |
| `cortex-api::search::{law_violations,decision_search,consolidation_costs,query_explain}::tests` | 25 | per-repo index uid, ADR lifecycle vocab, axis vocab + fold, drops/per-lane explain |
| `cortex-api::feedback::tests` | 5 | UPSERT round-trip; SQL list lands in `cortex-storage` |
| `cortex-storage::metadata::tests::list_pre_thinking_feedback*` | 1 | dynamic SQL: filter coverage + newest-first ordering |
| `cortex-mcp-server::tests::*_it` (wiremock) | 78 | one IT file per tool: happy path, bad_input, api_unreachable, descriptor pin |
| `cortex-mcp-server::server::tests::tools_list_returns_twentynine_descriptors` | 1 | `tools/list` round-trip post-phase19 |
| `cortex-mcp-server::transport_stdio::tests::round_trips_initialize_and_tools_list_over_pipe` | 1 | stdio transport sees 29 tools |

Live smoke verifying the consolidation-first surface
end-to-end against the running stack:

```bash
curl -sS -X GET 'http://127.0.0.1:17000/v1/consolidations/recent?repo=cortex&grain=session&limit=3' \
  -H 'content-type: application/json'
```
