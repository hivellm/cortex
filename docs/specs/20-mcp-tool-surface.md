# Spec 20 — Cortex MCP Tool Surface

> Status: phase34 daily-memory contract; aligned with ToolRegistry::default_set() 2026-10 (42 full-profile tools; 12 daily-profile tools). Source of truth for every Cortex MCP
> tool. Cross-reference for [spec 18](./18-claude-code-plugin.md)
> (transport / wire protocol) and [spec 11](./11-query-api.md) (the
> HTTP endpoints the tools wrap).

The `cortex-mcp-server` binary advertises a profile-specific set of tools over
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
- **Cortex MCP** — local daily-memory access and optional indexed live-lane
  retrieval through `cortex-api`. Explicit decisions and lessons commit to
  file-backed SQLite via `/v1/ingest` and are readable independently of bulk
  indexing. Indexed event history remains in the synap stream + on-disk archive,
  not solely in the daemon's RAM.

Each tool here belongs to one surface. Four tools WRITE
(`cortex_capture_memory`, `cortex_forget`, `cortex_feedback_record`,
`cortex_acl_grant`); the rest are READ. Since phase29
(mcp-surface-doc-and-discovery) the classification is queryable at
runtime — `Tool::read_or_write()` feeds `cortex_capabilities`, so this
table's read/write column has a machine-checkable counterpart.

## Registry

### Core Query, Audit & Ingest Operations

| Tool                        | Required Input                | HTTP endpoint                                    | Purpose | R/W   |
|-----------------------------|-------------------------------|--------------------------------------------------|---------|-------|
| `cortex_query`              | `intent`, `query`             | `POST /v1/query`                                 | Hybrid retrieval via spec-11 fusion (snippet + decision + vector lanes) | read  |
| `cortex_pre_thinking`       | `user_prompt`, `cwd`          | `POST /v1/query` (orchestrated)                  | Spec-12 pre-thinking pipeline against cortex-api | read  |
| `cortex_status`             | —                             | `GET /v1/status`                                 | Cortex daemon health snapshot (pid, queue, errors, WAL bytes) | read  |
| `cortex_audit`              | `query_id`                    | `GET /v1/audit/{query_id}`                       | Audit envelope for prior cortex_query/cortex_pre_thinking call | read  |
| `cortex_capture_memory`     | `kind`, `body`, `repo`        | `POST /v1/ingest`                                | Durably capture an explicit local decision/lesson; optional `classify=true` permits candidate-only external advice after commit | **write** |
| `cortex_memory_read`        | `repo`, `query`              | `POST /v1/memory/query`                          | Read accepted, applicable local decisions/lessons for the exact authorized repo; optional `branch`, `paths`, `include_global=false`, `limit` (0-5), `budget_bytes` (0-1536); cited prepared delivery, no lifecycle mutation | read |
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
| `cortex_capabilities`       | —                             | (in-process — built from the registry itself)    | Phase29 — runtime discovery: every tool's name, one-line purpose (derived from its own descriptor), and read/write classification | read  |

The full profile exposes all 42 tools listed above. The daily profile exposes
only `cortex_query`, `cortex_pre_thinking`, `cortex_status`, `cortex_audit`,
`cortex_capture_memory`, `cortex_memory_read`, `cortex_active_work`,
`cortex_feedback_signals`, `cortex_feedback_record`, `cortex_acl_whoami`,
`cortex_acl_grant`, and `cortex_capabilities` (12 tools). Runtime discovery and
dispatch use the same filtered registry; an omitted tool cannot be called by
name. Capabilities are built after filtering and describe only that profile.

The MCP server's runtime registry is the binding contract: every name
in the selected profile MUST be returned by `tools/list`, and every tool MUST
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

The MCP server MUST expose `cortex_capture_memory` that POSTs an
explicit capture request to `/v1/ingest`. The tool MUST accept
`kind ∈ {memory, knowledge, learning, decision}`, `body` (≤ 8 KiB),
and exact `repo` identity (nonempty, trimmed, ≤ 256 bytes, no control characters;
no case folding or slug-based authorization). Legacy kinds become local lessons;
`decision` captures an explicit decision. Optional `topic` and `severity`
(`info` / `notable`) remain compatible.

Optional fields are `title`, `memory_id`, `status` (`proposed`, `accepted`,
`deprecated`), `agent`, `session_id`, `turn_id`, `retry_key`, `expected_revision`,
`supersedes`, `branch`, `path`, `pinned`, `sources`, `global` (false), and
`classify` (false). Agent/session labels are attribution, never authority.
Repository/global authorization is enforced by the API. Explicit supersession
requires an accepted successor naming an accepted predecessor in the same scope
and a matching `expected_revision`; proposals, feedback and classifier advice
never perform that lifecycle transition.

The response MUST follow a successful local SQLite commit and include
`event_id`, `content_hash`, `memory_id`, `version`, `scope_revision`,
`durability: "local_committed"`, and separate optional-indexing state
(`indexed_at: null`, `mirror_status: "not_attempted"`, `index_complete: false`).
It MUST NOT equate local durability with completed bulk indexing. Only explicit
`classify=true` permits candidate-only external advice after commit; missing or
false classification makes no provider call. Advice never rewrites the record.

#### Scenario: in-session capture is queryable next turn
- Given the agent calls `cortex_capture_memory { kind: "memory", body: "Phase9k uses per-name semaphore for concurrency", repo: "cortex" }`
- When the response returns `durability: "local_committed"`
- And the agent then calls `cortex_memory_read { repo: "cortex", query: "phase9k semaphore concurrency" }`
- Then the accepted applicable memory MUST be locally readable without waiting for bulk indexing.

#### Scenario: oversized body rejects with structured error
- Given the agent passes a 16 KiB `body`
- When `cortex_capture_memory` runs
- Then it MUST return an error of the form `{ "reason": "body_too_large", "max_bytes": 8192, "received": 16384 }`
- And no envelope MUST be ingested.

### Requirement: cortex_memory_read returns bounded local prepared context

The tool MUST accept required exact `repo` and ordinary-text `query` (≤ 4 KiB
and at most 32 distinct terms, never caller-supplied FTS syntax). Optional inputs
are `branch`, `paths` (at most 32 repository-relative paths using `/`),
`include_global` (default false),
`limit` (default/max 5), and `budget_bytes` (default/max 1536). Rendered context,
including IDs and provenance/source citations, MUST respect the byte budget;
only records actually rendered belong to the returned `memories` list.

Normal context MUST include only accepted, applicable, nonstale records from
the exact authorized repository, plus authorized global records only when
explicitly requested. Proposed, superseded, deprecated and stale history MUST
NOT become active advice. Repository rules retain higher authority than memory;
reading or negative feedback MUST NOT promote or supersede a record.

The MCP transport forwards its configured API key as `Authorization: Bearer`.
The API resolves authentication under its configured policy, verifies any supplied
credential, and checks exact repository ACL scope; caller/agent
labels MUST NOT grant access. Global opt-in requires an authenticated ACL admin
and the configured principal store. Local read MUST operate without Vectorizer,
Meili, Nexus, optional TypeSafe classification or bulk ingestion services.

The response includes `context`, exact rendered `memories` IDs/versions/event
IDs/scopes, `revision`, `global_revision`, and durable `query_id` equal to
`delivery_id`. Receipt persistence records preparation only; it MUST NOT claim
client receipt or model consumption. `cortex_feedback_record` can join that ID
to the exact prepared versions. Missing durable storage or denied scope MUST
produce an explicit error rather than fabricated delivery evidence.

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
- And `cortex-ops doctor-registry-sync` (phase29
  mcp-surface-doc-and-discovery §2) MUST report the divergence by
  name — missing/extra on either side — exiting 1 on any drift and 2
  (critical) at ≥2 tools.

### Requirement: registry drift is caught before it reaches 30 undocumented tools

The Registry table (this document, ## Registry section) MUST enumerate
exactly as many tools as `ToolRegistry::default_set().len()` returns at
runtime. The concrete enforcement mechanisms (phase29
mcp-surface-doc-and-discovery) are `cortex-ops doctor-registry-sync`
(parses this table's rows and compares names + count against a
generated registry manifest) and `cortex_capabilities` (the runtime
half — advertises the registry as the agent sees it). The check runs
in CI on every PR touching the registry or this document
(`.github/workflows/registry-sync-gate.yml`); a drift ≥2 tools exits
critical and MUST block pull requests, preventing silent drift where
new tools ship without corresponding Registry entries.

#### Scenario: registry-runtime cardinality mismatch is caught in CI
- Given the Registry table documents 7 tools (2024-06 baseline)
- When a developer ships 30 additional MCP tools without updating the Registry
- Then CI's doc-coherence check MUST parse the Registry table and count rows
- And MUST compare the count (7) against `ToolRegistry::default_set().len()` (42)
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

`cortex_capture_memory` writes are durable in the existing file-backed metadata
SQLite before 202 ACCEPTED; `cortex_memory_read` uses the same local store and
durably records prepared deliveries. These operations do not require the bulk
ingestion/search stack. A local-storage 503 MUST NOT be presented as a committed
capture or prepared delivery. Optional indexing and its completion state remain
separate from local durability.

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
