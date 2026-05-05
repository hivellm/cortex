# Cortex Consolidation Knowledge Base (2026-05-04)

This directory contains a synthesis of Cortex's state, architecture, and operational knowledge, designed for ingestion into Cortex itself and for continuity across sessions.

**Source:** Consolidated from 11 deep-analysis files in `docs/analysis/cortex/` + specs + codebase audit.  
**Audience:** HiveLLM maintainers, Cortex operators, future sessions.  
**Scope:** End-to-end Cortex overview — vision, architecture, APIs, deployment, known issues.

## How to Use This KB

Start with **01-overview.md** for the big picture (1 screen). Then read by role:

| Role | Start with | Then read |
|------|-----------|-----------|
| **Maintainer** | 01 | 02, 06, 09 |
| **API Consumer** (Claude Code user) | 03 | 05, 04 |
| **Operator / DevOps** | 07 | 04, 05 |
| **Architect / Design Review** | 02, 06 | 09, 08 |
| **New Contributor** | 01, 02, 03 | 04, 05, 07 |
| **Cortex Developer** | All | Start-to-finish reading |

## File-by-File Breakdown

| File | Question it answers | Length | Audiences |
|------|--------------------|---------| ----------|
| **01-overview.md** | What is Cortex? What's its health right now? | ~150 lines | Everyone (start here) |
| **02-architecture.md** | How are the 10 crates layered? What does each do? | ~180 lines | Architects, maintainers, new contributors |
| **03-public-surface.md** | What HTTP endpoints, MCP tools, CLI commands exist? | ~140 lines | API consumers, integrators, operators |
| **04-data-and-storage.md** | What data model? How does it flow? Where is it stored? | ~180 lines | Developers, data engineers, queries writers |
| **05-integrations.md** | Which services? What drifts? What workarounds? | ~150 lines | Operators, architects, Hive service liaisons |
| **06-decisions-and-rationale.md** | Why was Cortex designed this way? ADRs? | ~150 lines | Architects, design reviewers, historians |
| **07-operational.md** | How to deploy, monitor, and troubleshoot Cortex? | ~200 lines | Operators, DevOps, CI/CD engineers |
| **08-cortex-relevance.md** | What should Cortex index about itself? Redaction? | ~120 lines | Cortex maintainers, governance teams |
| **09-open-questions.md** | What's incomplete? What's deferred? Design unknowns? | ~200 lines | Planners, phase leads, architects |

**Total:** ~1250 lines (all files), digestible in ~30 min for maintainers, ~1 hour deep-dive.

## Key Findings at a Glance

### Health Status (2026-04-28)

| Dimension | Status | Notes |
|-----------|--------|-------|
| **Phase 1 (Capture + Retrieval)** | 🟢 Live | 13 of 18 specs implemented; pipeline works end-to-end on Cortex repo |
| **Phase 2 (Governance)** | 🟡 Draft | Specs 13–14 outlined; engine not built |
| **Phase 3+ (Analysis, adapters)** | ❌ Not started | Sonnet analyzer precursor landed |
| **Codebase** | 🟢 Healthy | ~44k LOC Rust, 291 tests, zero warnings (clippy), no TODO/FIXME |
| **Docker Stack** | 🟢 Containerized | Full `docker compose up -d` works; all services health-checked |
| **Integrations** | 🟡 Partial | Synap ✅, Nexus 🟡 (UNWIND drops), Vectorizer 🟡 (upsert reporting), Meili 🟡 (single-repo coverage) |

### Top 3 Blockers for Phase 2

1. **Meili single-repo coverage** (phase4a) — only Cortex indexed of 3+ repos walked. Worker consumer offset suspected.
2. **Governance engine unbuilt** (phase 2) — laws DSL drafted; enforcement, sandbox, trust scoring not implemented.
3. **Vectorizer upsert reporting drift** (phase4d) — "total_failed=4-5" reported but vectors queryable. Truth unknown.

### Top 3 Ingestion Priorities

1. **`.rulebook/decisions/`** — ADRs + architectural choices; core to "why is Cortex designed this way?"
2. **`docs/specs/00-18.md`** — Executable definitions of every Cortex subsystem; source of truth for "what is supposed to happen?"
3. **`docs/analysis/cortex/*.md`** — Health snapshots + roadmap; captured institutional knowledge from phase leads.

## Architectural Decisions Summary

**8 ADRs + 1 reserved:**

| ADR | Title | Status | Impact |
|-----|-------|--------|--------|
| 001 | Bypass Vectorizer SDK for drifted paths | 🟡 Superseded (partial) | Per-row Cypher workaround for Nexus applies same principle |
| 002 | Classifier worker in separate crate | 🟢 Accepted | Enables cortex-workers monolith (ADR 007) |
| 003 | Per-intent recency decay defaults | 🟢 Accepted | Query fusion tuning (soft-coded; not yet exposed as config) |
| 004 | Graph node identity in Nexus `id` slot | 🟢 Accepted | Enables deduplication + cross-session linking |
| 005 | Consolidation grain: session + topic + trace | 🟢 Accepted | Cross-event analysis operates at 3 tiers |
| 006 | Governance: per-repo + global dual-write | 🟢 Accepted | Balances per-maintainer views + org compliance |
| 007 | Workers as monolithic deployment unit | 🟢 Accepted | Single Dockerfile build; per-target staging |
| 008 | Durable offsets via SQLite | 🟢 Accepted | Resumability across restarts without relying on Synap state |
| 009+ | Reserved | — | Future extensions |

See **06-decisions-and-rationale.md** for full details.

## Quick Reference: Common Operations

### Start the stack
```bash
docker compose up -d
curl http://localhost:17000/healthz  # cortex-api ready
```

### Bootstrap the Cortex repo itself
```bash
docker compose exec cortex-api cortex-bootstrap /workspaces/Cortex --scope cortex
```

### Query via MCP (from Claude Code)
```
cortexQuery(scope: "cortex", query: "ADR graph nodes identity")
```

### Check health aggregator
```bash
curl http://localhost:17000/v1/status | jq .
```

### View dashboard
Open GUI at `http://localhost:17000` (Electron app) or navigate to tab in Claude Code.

## Known Gaps & Deferred Work

**Incomplete features (phase 2+):**
- Governance engine (laws DSL → sandbox detection → trust scoring)
- Symbol-level graph relations ((:Symbol)-[:DEFINES]->(:Artifact))
- Multi-repo bootstrap orchestrator (17 Hive repos)
- Non-Claude adapters (Cursor, Codex, Gemini, Copilot)
- Cortex CLI (cortex-ops: doctor, prune, reindex)

**Open questions (design unknowns):**
- Local vs. cloud governance execution?
- When to migrate from Meili to Lexum?
- How to quantify trust scores?
- Graph consolidation grain (session, topic, trace, time-window)?

See **09-open-questions.md** for full backlog + trackers.

## Redaction & Privacy (for Self-Indexing)

When Cortex indexes itself (bootstrap):
- **REDACT:** `.env` files, `VECTORIZER_PASSWORD`, `MEILI_MASTER_KEY`, API keys, internal emails.
- **PRESERVE:** Public project contacts, ADR rationale, code patterns, test fixtures (unless they contain real secrets).
- **Sensitive paths:** `.rulebook/secrets/` (fully redacted), `docker-compose.yml` env overrides (partial redaction).

See **08-cortex-relevance.md** for detailed rules.

## How to Contribute to This KB

1. **Find a gap:** Notice something missing? Check **09-open-questions.md** first.
2. **Update the KB:** Modify the corresponding file (01–09) in place.
3. **Sync with analysis:** If you update deep analysis in `docs/analysis/cortex/`, reflect it here.
4. **Commit:** Follow conventional commits (`docs(consolidation): <description>`).
5. **Ingest into Cortex:** On next bootstrap run, new KB updates will be indexed via `cortex-bootstrap`.

## External References

- **Full specs:** `docs/specs/00-index.md` (18 discrete, implementable specs).
- **Deep analysis:** `docs/analysis/cortex/00-index.md` (11 focused analysis files).
- **Architecture:** `docs/architecture.md` (vision, goals, ecosystem context).
- **Decisions:** `.rulebook/decisions/` (ADRs with full rationale).
- **Knowledge base:** `.rulebook/knowledge/` (patterns, anti-patterns, learnings).
- **Project rules:** `AGENTS.md` (team-shared), `AGENTS.override.md` (project-specific), `CLAUDE.md` (critical rules).
- **Task backlog:** `.rulebook/tasks/` (current phase + pending phases).

## Maintenance

**Update frequency:**
- **01-overview.md** — every release (health snapshot, status).
- **02-architecture.md** — on crate refactor or new crate.
- **03-public-surface.md** — on API endpoint or MCP tool addition.
- **04-data-and-storage.md** — on schema or storage layout change.
- **05-integrations.md** — on SDK version bump or drift discovery.
- **06-decisions-and-rationale.md** — on ADR approval (link only; content frozen).
- **07-operational.md** — on deploy/port/env change.
- **08-cortex-relevance.md** — on redaction rule or priority shift.
- **09-open-questions.md** — when items move from "open" to "blocked" to "archived".

**Last updated:** 2026-05-04 by automated consolidation.
