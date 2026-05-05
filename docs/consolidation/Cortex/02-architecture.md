# Cortex — Main Architecture & Crate Roles

## Layering Overview

```
┌──────────────────────────────────────────────────────────────┐
│ AI Tools (Claude Code, Cursor, Gemini, Codex, Copilot)      │
└─────────────────────┬──────────────────────────────────────┘
                      │ capture (hooks) + retrieve (MCP/HTTP)
┌─────────────────────▼──────────────────────────────────────┐
│          C O R T E X   (Ingestion → Processing →           │
│          Retrieval + Governance + Dashboard)               │
└─────────────────────┬──────────────────────────────────────┘
                      │ composed services
┌─────────────────────▼──────────────────────────────────────┐
│ HiveLLM Services: Vectorizer (vectors), Nexus (graph),     │
│ Synap (bus/pub-sub), Meilisearch (keyword), Rulebook (defs)│
└──────────────────────────────────────────────────────────┘
```

## Core Crates (10-crate workspace)

### Ingestion Tier

**`cortex-core`** — Event schemas, redactor, routing logic
- Defines wire format for all events (Session, Turn, ToolCall, AgentCall, Memory, Decision, Analysis, LawViolation, Artifact, Topic).
- PII redaction rules (secrets, emails, API keys).
- Routing logic that multiplexes enriched events to downstream workers (embedder, graph, fulltext).

**`cortex-storage`** — Durability layer
- Parquet archive + zstd compression (spec 02). Raw + bootstrap event history persisted immutably.
- SQLite consumer offset table (decision 008) for durable, resumable reads.

### Processing Tier (all in `cortex-workers`, containerized separately)

**`cortex-workers`** — Five worker binaries:
- **`cortex-classifier-worker`** — consumes raw/bootstrap stream, calls Haiku (or static rules) per event, emits enriched. Landed 2026-04-27 as the critical blocker fix.
- **`cortex-embedder-worker`** — Tree-sitter chunker → Vectorizer SDK batch upsert. Per-project collection isolation. Watches Synap enriched stream.
- **`cortex-graph-worker`** — Per-row Cypher emitter → Nexus. Templated at startup from `crates/cortex-workers/cypher/`. Post-write assert validates landing.
- **`cortex-fulltext-worker`** — Meili indexer. Per-project index routing (cortex-{repo}-{family}). Watches enriched stream.
- **`cortex-claude-archive`** (phase11i §5) — Tails Claude Code JSONL archive at `~/.claude/projects/` (bind-mounted), emits one envelope per turn/tool_call/agent_call, publishes to Synap.

### Query & API Tier

**`cortex-api`** — HTTP API + dashboard aggregator
- `/v1/query` (POST) — hybrid retrieval with RRF fusion (vector lane + keyword lane + graph lane).
- `/v1/status` — health + dependency probes. Aggregates worker healthchecks + service pings.
- 16 dashboard endpoints (`/v1/dashboard/*`) — overview, timeline (SSE), memory, decisions, laws, violations, analyses, tool stats, graph, sessions, conversations, trust, handoffs.
- Sonnet analyzer (`analyzer.rs`) — consumes conversation sessions, produces cross-event summaries via Claude CLI (when on PATH) or direct Anthropic API.

**`cortex-pre-thinking`** — Pre-thinking bundle assembly
- Compiles context payload (decisions, laws, similar turns, snippets) for injection into every model prompt.
- Reused by both adapter and MCP server to avoid duplication. ADR 002 documents why this is separate from classifier.

**`cortex-mcp-server`** — MCP server
- Tools: `cortexQuery` (hybrid search), `cortexPreThinking` (bundle), `cortexStatus` (health). Spec-compliant identifier names + camelCase schemas (commit 9f14ef6).

### Adapters Tier

**`cortex-adapter-claude-code`** — Claude Code hook daemon
- Local HTTP server (default :17011) listening for hook POST callbacks (UserPromptSubmit, PreToolUse, PostToolUse, SubagentStop, Stop).
- Assembles Turn envelopes with both user prompt + assistant reply (fixed 2026-04-27 by commit 15b8931 to close the asymmetry).
- Publishes to Synap `cortex.events.raw` stream.
- Injects pre-thinking via cortex_pre_thinking pipeline (not bespoke HTTP — migration completed commit e312cd2).

### CLI & Utilities

**`cortex-cli`** — Command-line interface (not yet detailed in analysis; planned operations: doctor, ops, bootstrap subcommands).

**`cortex-health`** — Health aggregator / probe orchestrator (supports `cortex-ops` doctor workflows).

**`cortex-build`** — Build-time context capture (git SHA, dirty flag stamped into `/healthz` version block per Dockerfile phase11e hotfix).

## Ingestion Flow (How Data Flows Through the System)

```
Claude Code adapter (hooks)          Cortex bootstrap CLI
         │                                    │
         ▼                                    ▼
Synap cortex.events.raw        Synap cortex.events.bootstrap
         │                                    │
         └──────────────────┬─────────────────┘
                            ▼
           cortex-classifier-worker
            (Haiku / StaticClassifier + cache)
                            ▼
           Synap cortex.events.enriched
           ▲                          ▼
           │          ┌──────────────┴──────────────────┐
           │          ▼              ▼                  ▼
    (for pre-thinking)      cortex-embedder-worker  cortex-graph-worker  cortex-fulltext-worker
                                  │                      │                      │
                                  ▼                      ▼                      ▼
                            Vectorizer              Nexus                 Meilisearch
                          (collections)          (Cypher)              (indexes per-project)
                                  │                      │                      │
                                  └──────────────┬───────┴──────────────────────┘
                                                 ▼
                                   cortex-api /v1/query (RRF fusion)
                                   │
                                   ├─ → cortex-mcp-server
                                   ├─ → cortex_pre_thinking
                                   └─ → dashboard UI
```

## Service Integration Boundary

| Service | Cortex Crate(s) | Role | Health (2026-04-28) |
|---------|-----------------|------|-----|
| **Vectorizer 3.3.0** | cortex-embedder-worker | Dense embedding storage + semantic search | 🟡 SDK drifts (upsert reporting, vector listing) but vectors queryable |
| **Nexus 2.2.0** | cortex-graph-worker | Relationship storage, Cypher traversal | 🟡 UNWIND-writes silently drop; per-row workaround in place |
| **Synap (latest)** | cortex-workers (all), cortex-api | Event bus, stream pub/sub, live dashboard SSE | 🟢 Most reliable; auto-create-on-404 pattern prevents bootstrap races |
| **Meilisearch v1.10** | cortex-fulltext-worker, cortex-api | Full-text inverted index (keyword search, facets) | 🟡 Single-repo coverage; worker consumer offset suspected issue |
| **Claude Haiku/Sonnet** | cortex-workers (classifier), cortex-api (analyzer) | Classification + cross-event summarization | 🟢 CLI path works; API fallback added for server scenarios |
| **Rulebook** | cortex-bootstrap (default discovery) | Laws/decisions/learnings federation, task orchestration | 🟢 MCP + SDK integration live; `.rulebook/*` auto-promoted to Cortex entities |

## Key Architectural Decisions

- **ADR 001** (now superseded): Bypass SDK drifts with direct reqwest when needed. Applies to older Vectorizer paths; newer SDK versions closed some gaps.
- **ADR 002**: Classifier worker in separate crate to break `classifier → embedder → classifier` dependency cycle.
- **ADR 004**: Graph node identity stored in Nexus's reserved `id` slot; no synthetic UUIDs.
- **ADR 007**: Workers hosted in `cortex-workers` monolith (not individual crates) to reduce Docker build complexity.
- **ADR 008**: Durable consumer offsets via SQLite (not Synap offset tracking) for resumability across restarts.

See `.rulebook/decisions/` for full details.
