# Cortex — Consolidated Overview

## Purpose and Role

Cortex is the **cognitive substrate of HiveLLM**: a capture → classify → embed → relate → govern → retrieve system that preserves and indexes every meaningful AI interaction across the organization's codebases. It is the "glue" that captures institutional knowledge at the moment it is lost (session end), classifies it, makes it semantically searchable, and enforces development laws.

**What Cortex is:** An orchestrator composing existing HiveLLM services (Vectorizer, Nexus, Synap, Rulebook) with external pieces (Meilisearch, Claude Haiku/Sonnet via CLI).

**What Cortex is not:** A new vector DB, graph DB, search engine, or code-generation agent. It informs agents; it does not generate.

## Language & Tech Stack

- **Rust workspace:** 10 core crates (`cortex-core`, `cortex-workers`, `cortex-api`, `cortex-cli`, `cortex-adapter-claude-code`, `cortex-pre-thinking`, `cortex-mcp-server`, `cortex-health`, `cortex-build`).
- **Node.js GUI:** React + TypeScript + Electron dashboard (interactive timeline, memory browser, decision register, law/violation audit).
- **External services:** Vectorizer 3.3.0 (vectors), Nexus 2.2.0 (graph), Synap (events/pub-sub), Meilisearch v1.10 (keyword search), Claude Haiku/Sonnet (via CLI or Anthropic API).
- **Edition:** Rust 2021, MSRV 1.80; Apache-2.0 licensed.

## Maturity & Status (2026-05-04)

- **Phase 1 (Capture + Retrieval):** 13 of 18 specs flagged 🟢 implemented. Pipeline lights up end-to-end on the Cortex repo: bootstrap → classifier-worker → enriched stream → embedder/graph/fulltext → query API.
- **Phase 2 (Governance):** Specs 13–14 still 🟡 (draft). Laws DSL outlined; enforcement engine not yet built. Dashboard GUI scaffolded but reads fixtures, not live state.
- **Phases 3+ (Analysis, adapters, hardening):** Not started. Sonnet analyzer precursor landed.

## Top-Level Directory Layout

```
Cortex/
├── crates/                           # Rust workspace (10 crates)
│   ├── cortex-core/                  # Event types, redactor, router
│   ├── cortex-workers/               # 5 worker binaries (classifier, embedder, fulltext, graph, archive-watcher)
│   ├── cortex-api/                   # Query API + dashboard backend
│   ├── cortex-cli/                   # CLI surface (bootstrap, ops)
│   ├── cortex-adapter-claude-code/   # Claude Code hook daemon
│   ├── cortex-pre-thinking/          # Bundle assembly pipeline (reused by adapter + MCP)
│   ├── cortex-mcp-server/            # MCP endpoint (cortexQuery, cortexPreThinking, cortexStatus)
│   ├── cortex-health/                # Health aggregator
│   └── cortex-build/                 # Build context capture
├── gui/                              # Electron + React dashboard
├── docs/                             # Specs (00-18) + architecture + analysis
├── .rulebook/                        # Decisions, knowledge, learnings, tasks, memory
├── .claude/                          # Agents, skills, rules, hooks
├── Dockerfile                        # Multi-stage builder → per-binary targets
├── docker-compose.yml                # Full stack (Vectorizer + Nexus + Synap + Meilisearch + all Cortex workers)
├── Cargo.toml                        # Workspace manifest
└── README.md                         # Ecosystem overview

Codebase: ~44k LOC Rust, 291 unit tests in 58 files.
```

## Headline Health Snapshot

| Leg | State | Notes |
|-----|-------|-------|
| Capture | 🟢 OK | Stop→Turn captures async model replies; non-Claude adapters not started |
| Bootstrap | 🟡 Partial | 4 repos walked, 3 partially indexed, no multi-repo orchestrator |
| Classification | 🟢 OK | Worker landed 2026-04-27; defaults to `StaticClassifier` (offline); CLI opt-in |
| Embedding | ❌ Drifted | Vectorizer SDK 3.0.3 reports `total_failed=4-5/batch` but vectors queryable downstream |
| Graph | 🟡 Shallow | Only `IN_REPO` + `REMEMBERS` edges; symbol info dropped at mapper (phase4c TBD) |
| Full-text | ❌ Fan-out gap | Only 1 of 3 bootstrap repos indexed; worker offset/consumer state suspected |
| Query API | 🟢 OK | Three live lanes (vector, keyword, graph); RRF fusion working; quality unmeasured |
| Pre-thinking + MCP | 🟢 OK | Adapter uses unified pipeline; MCP descriptors spec-compliant |
| Governance | 🔴 Unbuilt | Laws DSL drafted; no detector sandbox, enforcement engine, or trust scoring |

Key blockers for Phase 2: worker consumer-state drift, Vectorizer SDK upsert reporting, symbol-level graph topology.
