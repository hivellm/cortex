# Cortex — Public Surface (API, MCP, CLI)

## HTTP API (`cortex-api` on port 17000)

### Query Endpoint

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/v1/query` | Hybrid retrieval (vector + keyword + graph) with RRF fusion. Spec 11. |
| GET | `/v1/status` | Health probe + dependency status. |

### Dashboard Backend

| GET | `/v1/dashboard/overview` | System overview (recent events, health, stats snapshot). |
| GET | `/v1/dashboard/timeline/recent` | Recent events from archive. Paginated. |
| GET | `/v1/dashboard/timeline/stream` | SSE stream of live events from Synap pub/sub. |
| GET | `/v1/dashboard/memory` | Browsable memory entries (Meili-backed). Scope filters available. |
| GET | `/v1/dashboard/decisions` | Decision register (Meili-backed). Per-project filter. |
| GET | `/v1/dashboard/decisions/{id}` | Single decision detail. |
| GET | `/v1/dashboard/laws` | Law catalogue (derived from law_violation envelopes; fixtures, not live). |
| GET | `/v1/dashboard/violations` | Law violation audit trail. |
| GET | `/v1/dashboard/analyses` | Deep-analysis reports. |
| GET | `/v1/dashboard/tools/stats` | Tool invocation analytics. |
| GET | `/v1/dashboard/graph` | Graph traversal results (Nexus via GraphLane). Cytoscape-compatible. |
| GET | `/v1/dashboard/sessions` | Session index. Paginated. |
| GET | `/v1/dashboard/conversations` | Sessions as conversations (turns + tool calls grouped). |
| GET | `/v1/dashboard/conversations/{session_id}` | Conversation detail + Sonnet-generated summary. |
| GET | `/v1/dashboard/handoffs` | Handoff envelope history. |
| GET | `/v1/dashboard/trust` | Trust score / governance status (stub; governance engine not yet built). |

## MCP Server (`cortex-mcp-server` exposed via Claude Code plugin)

Three tools available (spec 18, commit 9f14ef6 — identifier-safe names, camelCase schemas):

| Tool | Input | Purpose |
|------|-------|---------|
| `cortexQuery` | `scope`, `query`, `laneWeights` (optional) | Execute hybrid query against Cortex. Returns ranked results from vector + keyword + graph lanes, fused via RRF. |
| `cortexPreThinking` | `scope`, `sessionId` (optional) | Assemble a context bundle (laws, decisions, similar turns, code snippets) for injection into the next prompt. |
| `cortexStatus` | (none) | Health check. Reports all backend service status (Vectorizer, Nexus, Synap, Meilisearch, analyzer availability). |

## CLI (`cortex-cli`, `cortex-ops`)

### `cortex bootstrap` (planned subcommand, spec 09)
- One-shot + incremental backfill of ~17 HiveLLM repos.
- Walks repo tree, emits events to Synap `cortex.events.bootstrap` stream.
- Default-discovery for `.rulebook/*` (commit fc87b4d).
- Per-event publish-failure tolerance (5% or 20 events, whichever larger).
- Current state: 4 repos walked (Cortex 617 events, Nexus 2642, Rulebook 1654, Synap 1304).

### `cortex ops` (planned, spec 13 follow-up)
- Operator commands: `doctor` (consistency checks), `prune` (cleanup), `reindex` (rebuild indexes).
- Health aggregator integration.

### `cortex-adapter-claude` (binary, not CLI; acts as daemon)
- Listens on `:17011` for Claude Code hook callbacks.
- Assembles Turn envelopes, publishes to Synap.

## GUI (`gui/`, Electron + React)

**Views** (source in `gui/src/views/`):

| View | File | Status |
|------|------|--------|
| Timeline | Timeline.tsx | 🟢 Live SSE; stream controls; sparkline grid |
| Memory | Memory.tsx | 🟢 Browser; Meili-backed |
| Decisions | Decisions.tsx | 🟢 Register; per-project filter |
| Laws | Laws.tsx | 🟡 Reads catalogue from law_violation envelopes; no live engine yet |
| Violations | (part of Laws view) | 🟡 Audit trail; no live detector |
| Analyses | Analyses.tsx | 🟢 Deep-analysis report browser |
| Tools | (part of overview) | 🟢 Tool invocation stats |
| Graph | (implied, Cytoscape backend) | ✅ Exists; Nexus-backed via dashboard API |
| Conversations | Conversations.tsx | 🟢 Sessions + turns; Sonnet analyzer summaries |
| Handoffs | Handoffs.tsx | 🟢 Handoff envelope timeline |

**Design:** Electron single-window, per-view drawers (sidebar). Responsive; design system in `gui/src/atoms/` (sparklines, density controls, markdown renderer).

## Deployment & Health Endpoints

Each Cortex service exposes `/healthz` (or `/health` for upstream services):

- `cortex-api:17000/healthz` — main aggregator
- `cortex-ingestion:17010/healthz` — ingest bus
- `cortex-adapter-claude:17011/healthz` — hook daemon
- `cortex-classifier-worker:17021/healthz` — classification
- `cortex-embedder-worker:17022/healthz` — embedding
- `cortex-fulltext-worker:17023/healthz` — indexing
- `cortex-graph-worker:17024/healthz` — graph writes
- `cortex-claude-archive:17030/healthz` — archive watcher (phase11i §5.2)

Upstream services:
- Vectorizer `:17001` (mapped from 15002)
- Nexus `:17002` (mapped from 15474)
- Synap `:17003` (HTTP) + `:17013` (WS)
- Meilisearch `:17004` (mapped from 7700)

See `docker-compose.yml` for full port and environment setup.
