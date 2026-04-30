# Spec: MCP audit + capture + replay surface

## ADDED Requirements

### Requirement: cortex_audit returns the full envelope

The Cortex MCP server MUST expose a `cortex_audit` tool that
takes `query_id` and returns the audit envelope for that
retrieval. The envelope MUST carry `caller`, `intent`, `scope`,
per-lane `(name, hits, latency_ms)`, `cache_hit`, `fail_open`,
and `generated_at`. When `include_samples=true`, each lane row
MUST also carry up to 3 sample hits.

#### Scenario: agent debugs a missed retrieval
Given a `cortex_pre_thinking` call returned `query_id=01KQDX...`
  with zero relevant snippets
When the agent calls `cortex_audit { query_id: "01KQDX..." }`
Then the response MUST list every lane the orchestrator consulted
And MUST include each lane's hit count + latency
So the agent can see that, e.g., the meili lane returned 0 hits
  while the vector lane returned 10.

### Requirement: cortex_capture_memory writes through /v1/ingest

The MCP server MUST expose `cortex_capture_memory` that POSTs
a canonical envelope to `/v1/ingest`. The tool MUST accept
`kind ∈ {memory, knowledge, learning}`, `body` (≤ 8 KiB),
`repo` (lowercase per phase10d), optional `topic` and
`severity`. It MUST return `{event_id, content_hash,
indexed_at}` synchronously.

#### Scenario: in-session capture is queryable next turn
Given the agent calls
  `cortex_capture_memory { kind: "memory", body: "Phase9k uses
  per-name semaphore for concurrency", repo: "cortex" }`
When the response returns `event_id` and the embedder lane has
  drained
And the agent then calls
  `cortex_query { intent: "free_search", query: "phase9k
  semaphore concurrency" }`
Then the captured memory MUST appear in `results.snippets`.

#### Scenario: oversized body rejects with structured error
Given the agent passes a 16 KiB `body`
When `cortex_capture_memory` runs
Then it MUST return an error of the form `{ "error":
  "body_too_large", "max_bytes": 8192, "received": 16384 }`
And no envelope MUST be ingested.

### Requirement: cortex_session_replay returns ordered turns

The MCP server MUST expose `cortex_session_replay` that takes
`session_id`, optional `max_turns` (default 20, max 200), and
optional `include_tool_calls` (default false). It MUST return
the turns in chronological order with `{role, occurred_at,
summary, tool_calls?}`.

#### Scenario: agent re-reads an earlier session
Given session `01QDKXMY4TG7` has 9 turns indexed
When the agent calls
  `cortex_session_replay { session_id: "01QDKXMY4TG7",
  max_turns: 5 }`
Then the response MUST contain exactly 5 turn rows
And the turns MUST be sorted by `occurred_at` ascending.

### Requirement: tool surface registry

`docs/specs/20-mcp-tool-surface.md` MUST enumerate every
Cortex MCP tool with `(name, schema_summary, http_endpoint,
purpose, write_or_read)`. New tools MUST be added to this
registry as part of the merge.

#### Scenario: registry stays in sync
Given a developer adds a new MCP tool
When the merge lands
Then the registry MUST list the tool
And `cortex-ops doctor` MUST surface a yellow warning when the
  registry diverges from the live tool list.
