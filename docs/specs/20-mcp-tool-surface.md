# Spec 20 — Cortex MCP Tool Surface

> Status: phase10j accepted. Source of truth for every Cortex MCP
> tool. Cross-reference for [spec 18](./18-claude-code-plugin.md)
> (transport / wire protocol) and [spec 11](./11-query-api.md) (the
> HTTP endpoints the tools wrap).

The `cortex-mcp-server` binary advertises a fixed set of tools over
JSON-RPC `tools/list`. Every tool is a thin wrapper over a stable HTTP
endpoint on `cortex-api`; the wire-shape contract for that endpoint
lives with its owning spec, not here. This document is the
`(name, schema, endpoint, principle, write_or_read)` registry the
operator audits when asking "what can an agent do through Cortex
today?".

## Principle

Two MCP surfaces, deliberately split:

- **Rulebook MCP** — source of truth for curated, on-disk artifacts
  (decisions, knowledge, tasks, learnings). Survives restarts; lives
  in `.rulebook/`.
- **Cortex MCP** — live-lane window. Reads what `cortex-api` and the
  ingestion pipeline have indexed; writes through to that same lane
  via `/v1/ingest`. Memory is bounded; the canonical record lives in
  the synap stream + on-disk archive, not in the daemon's RAM.

Each tool here belongs to one surface. The capture tool
(`cortex_capture_memory`) is the only WRITE in the Cortex MCP surface;
the rest are READ.

## Registry

| Tool                        | Schema (required)             | HTTP endpoint                                    | Spec      | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|-----------|-------|
| `cortex_query`              | `intent`, `query`             | `POST /v1/query`                                 | spec 11   | read  |
| `cortex_pre_thinking`       | `user_prompt`, `cwd`          | `POST /v1/query` (orchestrated by spec-12)       | spec 12   | read  |
| `cortex_status`             | —                             | `GET /v1/status`                                 | spec 18   | read  |
| `cortex_audit`              | `query_id`                    | `GET /v1/audit/{query_id}`                       | spec 11 §audit / spec 20 | read  |
| `cortex_capture_memory`     | `kind`, `body`, `repo`        | `POST /v1/ingest`                                | spec 12 §capture / spec 20 | **write** |
| `cortex_session_replay`     | `session_id`                  | `GET /v1/dashboard/conversations/{session_id}`   | spec 16 / spec 20 | read  |

The MCP server's runtime registry is the binding contract: every name
listed above MUST be returned by `tools/list`, and every tool MUST
implement [`Tool`](../../crates/cortex-mcp-server/src/tools.rs) with
the descriptor JSON-Schema shown in the source.

## ADDED Requirements

### Requirement: cortex_audit returns the full envelope

The Cortex MCP server MUST expose a `cortex_audit` tool that takes
`query_id` and returns the audit envelope for that retrieval. The
envelope MUST carry `caller`, `intent`, `scope`, per-lane
`(name, hits, latency_ms)` (rendered as the published envelope's
`counts` block + `debug.lanes` when present), `cache_hit` (the
envelope's `cache` field), `fail_open`, and `generated_at` (the
envelope's `query_id` ULID is the timestamp source — ULIDs encode the
generation time).

#### Scenario: agent debugs a missed retrieval
- Given a `cortex_pre_thinking` call returned `query_id=01KQDX...` with zero relevant snippets
- When the agent calls `cortex_audit { query_id: "01KQDX..." }`
- Then the response MUST list every lane the orchestrator consulted
- And MUST include each lane's hit count + latency
- So the agent can see that, e.g., the meili lane returned 0 hits while the vector lane returned 10.

### Requirement: cortex_capture_memory writes through /v1/ingest

The MCP server MUST expose `cortex_capture_memory` that POSTs a
canonical envelope to `/v1/ingest`. The tool MUST accept
`kind ∈ {memory, knowledge, learning}`, `body` (≤ 8 KiB), `repo`
(lowercase per phase10d), optional `topic`, and optional `severity`
(`info` / `notable`). It MUST return `{event_id, content_hash,
indexed_at}` synchronously.

#### Scenario: in-session capture is queryable next turn
- Given the agent calls `cortex_capture_memory { kind: "memory", body: "Phase9k uses per-name semaphore for concurrency", repo: "cortex" }`
- When the response returns `event_id` and the embedder lane has drained
- And the agent then calls `cortex_query { intent: "free_search", query: "phase9k semaphore concurrency" }`
- Then the captured memory MUST appear in `results.snippets`.

#### Scenario: oversized body rejects with structured error
- Given the agent passes a 16 KiB `body`
- When `cortex_capture_memory` runs
- Then it MUST return an error of the form `{ "reason": "body_too_large", "max_bytes": 8192, "received": 16384 }`
- And no envelope MUST be ingested.

### Requirement: cortex_session_replay returns ordered turns

The MCP server MUST expose `cortex_session_replay` that takes
`session_id`, optional `max_turns` (default 20, max 200), and
optional `include_tool_calls` (default false). It MUST return turns
in chronological order with `{role, occurred_at_ms, summary,
tool_calls?}`.

#### Scenario: agent re-reads an earlier session
- Given session `01QDKXMY4TG7` has 9 turns indexed
- When the agent calls `cortex_session_replay { session_id: "01QDKXMY4TG7", max_turns: 5 }`
- Then the response MUST contain at most 10 rows (each dashboard turn flattens into one user + one assistant row)
- And rows MUST be sorted by `occurred_at_ms` ascending.

### Requirement: tool surface registry stays in sync

This document MUST enumerate every Cortex MCP tool with `(name,
schema_summary, http_endpoint, purpose, write_or_read)`. New tools
MUST be added to this registry as part of the merge that introduces
them.

#### Scenario: registry stays in sync
- Given a developer adds a new MCP tool
- When the merge lands
- Then the registry MUST list the tool
- And `cortex-ops doctor` SHOULD surface a yellow warning when the
  registry diverges from the live tool list (future work — phase10k
  doctor entry).

## Bounded-memory contract

`cortex_audit` reads from an in-process ring buffer
(`crates/cortex-api/src/audit_store.rs`, default 1024 entries). A
404 from `/v1/audit/{query_id}` is therefore not authoritative "this
query never ran"; it is "this daemon does not currently have the
envelope retained". The canonical history lives on the synap stream
`cortex.events.query_audit` and the dashboard's timeline panel.

`cortex_capture_memory` writes are durable: cortex-api forwards to
`cortex-ingestion`, which archives the envelope before returning
202 ACCEPTED. A 503 from the proxy means the upstream ingestion
service was unreachable — the agent SHOULD fall back to
`rulebook_memory_save` for on-disk persistence and retry the
capture later when the live stack is back.

## Naming + protocol contract

- All tool names use snake_case and `cortex_` prefix.
- No tool name contains `.` (per the MCP 2024-11-05 contract).
- Every descriptor uses camelCase `inputSchema` (NOT snake_case
  `input_schema`); Claude Code silently drops descriptors that
  emit the snake_case spelling.
- Every tool wires through the same `ToolContext` (HTTP client,
  pre-thinking metrics, started_at instant) so auth + rate-limit
  middleware applies uniformly. Per-tool middleware overrides are
  forbidden — they create per-tool drift the registry can't audit.
