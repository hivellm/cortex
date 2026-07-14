# Spec 20 — Cortex MCP Tool Surface

> Status: phase10j accepted; re-verified against ToolRegistry::default_set() 2026-07 (37 tools). Source of truth for every Cortex MCP
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

### Core Query, Audit & Ingest Operations

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_query`              | `intent`, `query`             | `POST /v1/query`                                 | Hybrid retrieval via spec-11 fusion (snippet + decision + vector lanes) | read  |
| `cortex_pre_thinking`       | `user_prompt`, `cwd`          | `POST /v1/query` (orchestrated)                  | Spec-12 pre-thinking pipeline against cortex-api | read  |
| `cortex_status`             | —                             | `GET /v1/status`                                 | Cortex daemon health snapshot (pid, queue, errors, WAL bytes) | read  |
| `cortex_audit`              | `query_id`                    | `GET /v1/audit/{query_id}`                       | Audit envelope for prior cortex_query/cortex_pre_thinking call | read  |
| `cortex_capture_memory`     | `kind`, `body`, `repo`        | `POST /v1/ingest`                                | Capture in-session fact (memory/knowledge/learning) to live retrieval lane | **write** |
| `cortex_session_replay`     | `session_id`                  | `GET /v1/dashboard/conversations/{session_id}`   | Ordered turns for prior Cortex session | read  |
| `cortex_forget`             | `event_id`, `confirmation_token` | `POST /v1/admin/forget`                          | Hard-purge event across Vectorizer + Meili + Nexus + Parquet (irreversible) | **write** |
| `cortex_query_explain`      | `query`                       | `POST /v1/query/explain`                         | Diagnostic view of cortex_query run (intent, per-lane hits, fusion math) | read  |

### Fine-Grained Search Interfaces

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_keyword_search`     | `index`                       | `POST /v1/search/keyword`                        | Raw Meilisearch keyword search against named index | read  |
| `cortex_vector_search`      | `collection`, `query_vector`  | `POST /v1/search/vector`                         | Raw Vectorizer cosine search against named collection | read  |
| `cortex_graph_query`        | `mode`                        | `POST /v1/search/graph`                          | Direct Nexus graph query (neighbors walk or Cypher) | read  |
| `cortex_graph_communities`  | —                              | `GET /v1/dashboard/graph/communities`            | Phase27b §3.1 — lists detected graph communities (member count, god nodes, cross-community edges); empty until the §2.5 writeback worker is live (gated on ADR-027) | read  |
| `cortex_path`               | `from`, `to`                   | `GET /v1/dashboard/graph/path`                   | Phase27e §2.1 — BFS shortest path (by hop count, undirected) between two nodes resolved by exact `_id` or `name`; `found: false` (never an error) when an endpoint misses or no path exists within `max_hops` | read  |
| `cortex_compare`            | `a`, `b`                       | `GET /v1/dashboard/graph/compare`                | Phase27e §2.2 — shared vs divergent neighbourhoods of two nodes (`shared`/`only_a`/`only_b`, capped at 100 with true totals); `depth` 1-3 | read  |

### Session & Dashboard Surface

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_active_work`        | —                             | `GET /v1/dashboard/active-work`                  | Operator-state snapshot (rulebook tasks + recent archives) | read  |
| `cortex_similar_sessions`   | `query`, `repo`               | `POST /v1/search/similar-sessions`               | Vector search over consolidation collection for past sessions | read  |
| `cortex_decision_chain`     | `event_id`                    | `GET /v1/search/decision-chain`                  | Walk `:SUPERSEDES` edges from Decision node in both directions | read  |
| `cortex_events_by_kind`     | `kind`                        | `POST /v1/search/events`                         | List envelopes filtered by kind (turn/tool_call/consolidation/decision/etc) | read  |
| `cortex_session_timeline`   | `session_id`                  | `GET /v1/sessions/{session_id}/timeline`         | Full chronological timeline for one session | read  |
| `cortex_tool_calls`         | —                             | `POST /v1/search/tool-calls`                     | Granular ToolCall envelope search (by tool_name/outcome/repo/window) | read  |
| `cortex_files_touched`      | —                             | `GET /v1/sessions/{session_id}/files-touched` or `POST /v1/search/files-touched` | Aggregate files touched by ToolCall envelopes (per-session or window) | read  |
| `cortex_topic_search`       | `topic_prefix`                | `POST /v1/topic-cards/search`                    | Search TopicCards by topic tag (Meili direct) | read  |

### Consolidations Suite

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_consolidation_get`  | `id`                          | `GET /v1/consolidations/{id}`                    | Fetch one consolidation by event_id or consolidation_id | read  |
| `cortex_consolidations_recent` | —                            | `GET /v1/consolidations/recent`                  | Chronological feed of consolidations (newest-first, filterable) | read  |
| `cortex_consolidations_by_entity` | `entity`                   | `POST /v1/consolidations/by-entity`              | List consolidations referencing entity (file/function/decision_id/repo/model) | read  |
| `cortex_consolidations_search` | `query`                     | `POST /v1/consolidations/search`                 | Text-driven search (BM25-only, no fusion) over consolidations | read  |
| `cortex_consolidation_lineage` | `id`                        | `GET /v1/consolidations/{id}/lineage`            | Structured citation view for one consolidation | read  |
| `cortex_consolidations_diff` | `since_ts`                    | `GET /v1/consolidations/diff`                    | Return consolidations at/after epoch-ms cursor (poll pattern) | read  |
| `cortex_consolidation_costs` | `since`, `until`, `group_by`  | `POST /v1/consolidations/costs`                  | Aggregate consolidation counts by grain/model/day | read  |

### Governance, Decisions & Feedback

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_law_violations`     | `repo`                        | `POST /v1/laws/violations`                       | List LawViolation envelopes with optional filters | read  |
| `cortex_feedback_signals`   | —                             | `POST /v1/feedback/list`                         | List pre-thinking feedback rows (helpful/intent/window) | read  |
| `cortex_feedback_record`    | `query_id`, `helpful`         | `POST /v1/pre-thinking/feedback`                 | Record post-thinking feedback on Cortex bundle | **write** |
| `cortex_decision_search`    | —                             | `POST /v1/decisions/search`                      | List Decision envelopes (status/tag/repo/window, no chain walks) | read  |

### Phase18 Bitemporal Timeline

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_timeline`           | `project`                     | `GET /v1/timeline/{project}`                     | Phase18 timeline view for one project | read  |
| `cortex_branch_list`        | `project`                     | `GET /v1/branch/{project}`                       | Phase18 — list every Branch node for a project | read  |
| `cortex_branch_show`        | `project`, `branch`           | `GET /v1/branch/{project}/{branch}`              | Phase18 — full Branch payload by project:branch id | read  |
| `cortex_history`            | `entity_id`                   | `GET /v1/entity/{entity_id}/history`             | Phase18 — return every TimelineEvent tagged with entity | read  |
| `cortex_supersession`       | `entity_id`                   | `GET /v1/entity/{entity_id}/supersession`        | Phase18 — walk `:SUPERSEDES` chain in both directions | read  |

### Phase21 Access Control

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_acl_whoami`         | —                             | `GET /v1/acl/whoami`                             | Phase21 §6.4 — resolve effective principal (clearance, compartments, roles) | read  |
| `cortex_acl_grant`          | `principal_id`                | `POST /v1/acl/grants`                            | Phase21 §6.4 — assign clearance, compartments, or RBAC role to principal | **write** |

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

Captured envelopes land in the per-repo `cortex-{slug}-misc` family.
For the round-trip below to hold, `free_search` MUST fan out to the
`misc` family (alongside `code` / `docs`) on both lanes — see
spec 11 §Intent → retrieval-strategy and
`phase0_captured-memory-not-retrievable-via-query`.

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

### Requirement: registry drift is caught before it reaches 30 undocumented tools

The Registry table (this document, ## Registry section) MUST enumerate
exactly as many tools as `ToolRegistry::default_set().len()` returns at
runtime. The cardinality check MUST be automated in CI (doc-coherence
scan) and in the `cortex-ops doctor` diagnostic; a drift ≥2 tools
MUST block pull requests, preventing silent drift where new tools ship
without corresponding Registry entries.

#### Scenario: registry-runtime cardinality mismatch is caught in CI
- Given the Registry table documents 7 tools (2024-06 baseline)
- When a developer ships 30 additional MCP tools without updating the Registry
- Then CI's doc-coherence check MUST parse the Registry table and count rows
- And MUST compare the count (7) against `ToolRegistry::default_set().len()` (37)
- And MUST fail the pull request with a cardinality error
- So this incident (silent 6-month drift, 30 undocumented tools, operators
  unaware of 81% of the MCP surface) does not recur.

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
