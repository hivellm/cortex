# 01 — Overview

## What Cortex is

Cortex is the cognitive substrate of HiveLLM: a captured, classified, embedded, related, and governed memory of every meaningful AI interaction across the org's repos. It is not a vector DB or a coding agent — it is the orchestrator that composes Vectorizer, Nexus, Synap, Meilisearch, Rulebook, and Claude (Haiku for per-event labeling, Sonnet for cross-event analysis) into a capture → classify → retrieve → govern loop.

Reference: [README.md](../../../README.md), [docs/architecture.md](../../architecture.md), [docs/prd.md](../../prd.md).

## Workspace layout

14 Rust crates in [Cargo.toml](../../../Cargo.toml):

| Crate                              | Role                                                                |
|------------------------------------|---------------------------------------------------------------------|
| `cortex-core`                      | Event types, redactor, ingestion router (spec 04)                   |
| `cortex-storage`                   | Parquet archive + zstd encoding (spec 02)                           |
| `cortex-ops`                       | Operator CLI (spec 13 doctor extension planned)                     |
| `cortex-ingestion`                 | Ingestion bus surface                                               |
| `cortex-classifier`                | Haiku classifier library (spec 05)                                  |
| `cortex-classifier-worker`         | Bridges raw/bootstrap → enriched (spec 05 follow-up)                |
| `cortex-embedder`                  | Tree-sitter chunker + Vectorizer client (spec 06)                   |
| `cortex-graph`                     | Nexus client + Cypher mappers (spec 07)                             |
| `cortex-fulltext`                  | Meilisearch indexer (spec 08)                                       |
| `cortex-bootstrap`                 | One-shot + incremental backfill CLI (spec 09)                       |
| `cortex-adapter-claude-code`       | Hooks daemon, PreThinking pipeline (spec 10, 12)                    |
| `cortex-api`                       | Hybrid query API + dashboard backend (spec 11, 16)                  |
| `cortex-pre-thinking`              | Bundle assembly used by adapter and MCP server (spec 12)            |
| `cortex-mcp-server`                | MCP exposure of pre-thinking + query (spec 18)                      |

Plus the `gui/` Electron + React dashboard and the `cortex-plugin/` Claude Code plugin.

Approximate codebase size: **~44k LOC** of Rust (`find crates -name "*.rs" | xargs wc -l`), **291 unit-test functions** in **58 test files** (`grep -r '^#\[test\]\|^#\[tokio::test\]'`).

## Spec implementation status (from [docs/specs/00-index.md](../../specs/00-index.md))

```
🟢 Implemented  →  01 02 03 04 05 06 07 08 09 10 11 12 18      (13 specs)
🟡 Draft        →  13 14 15 16 17                              (5 specs — Phase 2-3)
```

Note: spec 16 (Dashboard) is flagged 🟡 in the index but the GUI ships 9 working views ([gui/src/views/](../../../gui/src/views/)). The 🟡 reflects that some sub-tasks (`phase2f` auth, `phase2g` enriched metrics, `phase2h` decision chain + graph richness) are still pending — see [08-task-backlog.md](08-task-backlog.md).

## Recent activity (last 50 commits)

- **a62fcbd** — feat: Sonnet-backed session analyzer + Conversations summary pane (latest substantive feature)
- **2dc4832** — fix(gui): drawer covers full width on narrow viewports
- **6908ed2** — feat(gui): conversations + handoffs views; per-project decision filter
- **fc87b4d** — feat(bootstrap): default-discovery for `.rulebook/*` across all repos
- **15b8931** — feat(adapter): capture `assistant_message` via Stop → Turn envelope
- **f8966c4 / dbd60e8 / 99e8ef3** — three lane backends (Nexus / Vectorizer / Meili) wired live behind the dashboard query lanes

The trajectory of the last week is consistent with the stated priorities: close the live-data lanes, then layer cross-event analysis (Sonnet) on top, then start polishing the GUI for multi-environment use.

## Phase reading

| Phase                                   | Status            | Evidence                                                            |
|-----------------------------------------|-------------------|---------------------------------------------------------------------|
| Phase 0 — Foundations                   | ✅ Done           | Specs 01–04 implemented, docker-compose stack runs.                 |
| Phase 1 — Capture + retrieval           | ✅ Mostly done    | Specs 05–12 + 18 implemented; bootstrap covers 3/17 repos with data; per-backend drifts remain (see [03](03-data-quality.md), [04](04-integrations.md)). |
| Phase 2 — Governance + dashboard        | ⚠️ Partial         | GUI scaffolded; laws/governance engine **not implemented** (specs 13–14 still 🟡). |
| Phase 3 — Deep analysis + adapters      | ❌ Not started    | Sonnet analyzer is a precursor, but the orchestrated debate workflow (spec 15) and Cursor/Codex/Gemini adapters (spec 17) are open. |
| Phase 4+ — Hardening + cloud            | ❌ Open           | Multi-tenant + trust feed not in scope yet.                          |

## What "Cortex maps and indexes the project" really means today

Plain reading: an operator can run `cortex-bootstrap .` against the Cortex repo and watch 600+ events land across Synap → classifier-worker → enriched stream → embedder/graph/fulltext, with the dashboard showing the timeline live. That works. It does **not yet** mean the data is symmetrically distributed across all 17 Hive repos and three backends, nor that retrieval has been quality-evaluated against a recall benchmark. Both are tracked as Phase 4 / Phase 4d work.
