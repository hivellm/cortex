# cortex-api

> Spec: [`docs/specs/11-query-api.md`](../../docs/specs/11-query-api.md), [`docs/specs/16-dashboard.md`](../../docs/specs/16-dashboard.md)

The hybrid retrieval API + dashboard backend for Cortex. Composes
three lanes — Vectorizer (semantic), Meilisearch (keyword), Nexus
(graph) — into a single `POST /v1/query` endpoint, fuses the results
with Reciprocal Rank Fusion, and serves the read side of every GUI
view (timeline, decisions, laws, memory, tools, graph, conversations,
handoffs, analyses).

```
                      ┌──────────────────────────────┐
   /v1/query  ──────▶ │ cortex-api                   │
   /v1/dashboard/*    │  ├─ VectorLane (Vectorizer)  │
                      │  ├─ MeiliKeywordLane         │
                      │  ├─ GraphLane (Nexus)        │
                      │  ├─ analyzer (Sonnet)        │
                      │  └─ SSE timeline (Synap)     │
                      └──────────────────────────────┘
```

## Endpoints

### Query

| Method | Path           | Purpose                                                                   |
|--------|----------------|---------------------------------------------------------------------------|
| POST   | `/v1/query`    | Hybrid retrieval (vector + keyword + graph) + RRF fusion (spec 11).       |
| GET    | `/v1/status`   | Health probe — surfaces Vectorizer / Nexus / Meili / Synap reachability.  |

### Dashboard

Per [`src/dashboard.rs`](src/dashboard.rs), the read side currently
exposes:

- `/v1/dashboard/overview`
- `/v1/dashboard/timeline/recent`, `/v1/dashboard/timeline/stream` (SSE)
- `/v1/dashboard/memory`
- `/v1/dashboard/decisions`, `/v1/dashboard/decisions/{id}`
- `/v1/dashboard/laws`, `/v1/dashboard/violations`
- `/v1/dashboard/analyses`
- `/v1/dashboard/tools/stats`
- `/v1/dashboard/graph`
- `/v1/dashboard/sessions`
- `/v1/dashboard/trust`
- `/v1/dashboard/conversations`, `/v1/dashboard/conversations/{session_id}`
- `/v1/dashboard/handoffs`
- `/v1/dashboard/stream` (SSE — dashboard delta events; spec 21)

Conversation detail goes through the [`analyzer`](src/analyzer.rs)
module, which calls Sonnet (CLI when on PATH, direct Anthropic API
otherwise) to produce a cross-event session summary.

## Configuration

| Variable                     | Default                            | Notes                                                  |
|------------------------------|------------------------------------|--------------------------------------------------------|
| `CORTEX_API_BIND`            | `127.0.0.1:17000`                  | HTTP listen address.                                   |
| `CORTEX_API_VECTORIZER_URL`  | `http://127.0.0.1:17001`           | Vectorizer base URL.                                   |
| `CORTEX_API_NEXUS_URL`       | `http://127.0.0.1:17002`           | Nexus base URL.                                        |
| `CORTEX_API_MEILI_URL`       | `http://127.0.0.1:17004`           | Meilisearch base URL.                                  |
| `CORTEX_API_MEILI_KEY`       | `cortex-dev-master-key`            | Meili master key.                                      |
| `CORTEX_API_SYNAP_URL`       | `http://127.0.0.1:17003`           | Synap base URL (for the SSE timeline bridge).          |
| `CORTEX_ANALYZER_MODEL`      | `claude-sonnet-4-6`                | Sonnet model id used by the cross-event analyzer.      |
| `CORTEX_ANALYZER_API_KEY`    | (none)                             | Falls back to `ANTHROPIC_API_KEY`. Skips CLI when set. |
| `CORTEX_DASHBOARD_WATCH`     | `1`                                | Spec 21 — set `0` to disable the `.rulebook/` filesystem watcher feeding `/v1/dashboard/stream`. |
| `CORTEX_DASHBOARD_PUBLISH`   | `1`                                | Spec 21 — gates the MCP-side publisher (read by `cortex-mcp-server`). |
| `CORTEX_DASHBOARD_MEMORY_TAIL` | `1`                              | phase11n §2 — set `0` to disable the SQLite tail loop polling `.rulebook/memory/memory.db` for new rows. The loop runs at the same 250 ms cadence the FS watcher debounces at; rows committed after daemon start emit one `memory.appended` event per row. |

## Run

```bash
# from a checkout of the Cortex repo
cargo run --release -p cortex-api
```

The binary is also driven by the `bin/cortex-up` helper script, which
brings up the docker-compose stack first.

## Lanes

- **VectorLane** — uses the official `vectorizer-sdk` 3.0.3. See
  [`.rulebook/decisions/001-bypass-vectorizer-sdk-for-insert-and-get-vector-direct-reqwest-until-sdk-server-drift-is-resolved.md`](../../.rulebook/decisions/001-bypass-vectorizer-sdk-for-insert-and-get-vector-direct-reqwest-until-sdk-server-drift-is-resolved.md)
  for the (now-superseded) reqwest bypass that some methods still hold.
- **MeiliKeywordLane** — talks to Meilisearch directly; carries
  source attribution per spec-11 invariant.
- **GraphLane** — Nexus client; consumes the Cypher response shape
  produced by the per-row writer in `cortex-graph`.

## Tests

```bash
cargo test -p cortex-api
```

Unit tests cover the RRF fusion math, scope echo, and slug-aware
cache invalidation. Integration tests against the live stack are
gated on `CORTEX_IT=1`.

## Stability

Pre-1.0. The `/v1/query` schema is the load-bearing contract — every
adapter and the GUI both depend on it. Breaking changes go through
the same review path as `cortex-core` envelope changes.
