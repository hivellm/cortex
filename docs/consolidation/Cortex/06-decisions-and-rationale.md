# Cortex — Decisions & Architectural Rationale

## Architecture Decision Records (ADRs)

All formal decisions are logged in `.rulebook/decisions/` (lifecycle: proposed → accepted → superseded/deprecated). This file links to those and explains key architectural bets.

### ADR 001 — Bypass Vectorizer SDK for `/insert` and `/get_vector`

**Status:** 🟡 Superseded (partially) | **Date:** Early phase11  
**Issue:** Vectorizer SDK 3.0.1/3.0.3 had drifts in upsert reporting and vector fetching.  
**Decision:** Use direct `reqwest` calls for `/upsert` and `/get_vector`; keep SDK for auth/collection ops.  
**Current state:** SDK 3.0.3 closed the login and upsert-reporting issues (commit c41dab0). The `/get_vector` path still uses reqwest (no SDK list operation exists). [Record](../../.rulebook/decisions/001-bypass-vectorizer-sdk-for-insert-and-get-vector-direct-reqwest-until-sdk-server-drift-is-resolved.md).

### ADR 002 — Classifier worker in separate crate

**Status:** 🟢 Accepted | **Date:** phase11a  
**Issue:** `cortex-classifier` crate needs `cortex-embedder` to call it (for redaction); `cortex-embedder` needs `cortex-classifier` to be a dep (for enriched-event type consistency). Circular dependency.  
**Decision:** Move worker logic to a new `cortex-workers` crate. Libraries never depend on workers. [Record](../../.rulebook/decisions/002-classifier-worker-lives-in-a-separate-crate-to-avoid-the-classifier-embedder-classifier-cycle.md).  
**Impact:** All workers cohabitate in one crate (cortex-workers). Reduces Docker layer complexity. See ADR 007.

### ADR 003 — Per-intent recency decay defaults

**Status:** 🟢 Accepted | **Date:** phase11i §3.1  
**Issue:** Query fusion (RRF) needs tuning parameters for different intent types (query vs. pre-thinking vs. governance).  
**Decision:** Soft-code recency decay coefficients per intent; defaults favor recent turns (π/8 weeks) over old sessions. [Record](../../.rulebook/decisions/003-per-intent-recency-decay-defaults-for-cortex-fusion-phase11i-3-1.md).

### ADR 004 — Graph node identity in Nexus's reserved slot

**Status:** 🟢 Accepted | **Date:** phase11h  
**Issue:** Cortex nodes need a stable, semantic identity (not synthetic UUID) for deduplication and cross-session linking.  
**Decision:** Store Cortex identity (e.g., `session:abc123`, `turn:xyz`) in Nexus's reserved `id` property (node label, not a relationship property). [Record](../../.rulebook/decisions/004-cortex-graph-nodes-carry-their-identity-in-nexus-s-reserved-id-slot.md).  
**Impact:** Every Nexus write includes explicit `id: <semantic-identity>`; post-write assertions verify the `id` landed.

### ADR 005 — Consolidation grain: session + topic + decision trace

**Status:** 🟢 Accepted | **Date:** phase11o  
**Issue:** Cross-event analysis must operate at a granularity that balances cost, coherence, and usefulness.  
**Decision:** Consolidate at three tiers:  
  - **Session-level:** Full conversation with turn/tool-call envelope.  
  - **Topic-level:** All turns with a given topic across all sessions (supports "law on auth" searches).  
  - **Decision-trace-level:** Decision → all references (supports "why was this chosen?").  
[Record](../../.rulebook/decisions/005-consolidation-grain-choice-session-topic-decisiontrace.md).

### ADR 006 — Governance indexing: per-repo + global

**Status:** 🟢 Accepted | **Date:** phase11q  
**Issue:** Law violations need both per-repo filtering (dashboards for repo maintainers) and global audit (compliance).  
**Decision:** Dual-write law/violation events to per-repo indexes + `cortex-global-governance` index. [Record](../../.rulebook/decisions/006-governance-kinds-dual-write-to-per-repo-global-meili-indexes.md).

### ADR 007 — Cortex workers as monolithic deployment unit

**Status:** 🟢 Accepted | **Date:** phase11l  
**Issue:** Having one Dockerfile per worker (classifier, embedder, graph, fulltext) means 5 build layers × N changes = N×5 rebuilds.  
**Decision:** Consolidate all workers in `cortex-workers` crate; Dockerfile builds once, targets per-worker stages. ADR 002 enabled this. [Record](../../.rulebook/decisions/007-cortex-workers-as-the-default-host-for-worker-style-daemons.md).

### ADR 008 — Durable consumer offset via SQLite

**Status:** 🟢 Accepted | **Date:** phase11s  
**Issue:** Workers need to resume from exact offset after restart, but Synap pub/sub does not guarantee offset persistence.  
**Decision:** Co-locate a SQLite table with consumer state in each worker's config dir (or shared archive root). Commit offset only after successful down-stream write. [Record](../../.rulebook/decisions/008-durable-consumer-offset-via-sqlite.md).  
**Impact:** Every worker has a `*.consumer-state.sqlite` file; offset table is schema-pinned and versioned.

## Key Design Rationales

### Why Meilisearch instead of Lexum (for now)?

**Cortex README:**  
> "Meilisearch for full-text; use as a stand-in until **Lexum** reaches production parity, at which point we'll migrate."

Rationale (from architecture.md §5.2.1):
- Meilisearch is stable, battle-tested open-source.
- Lexum (HiveLLM proprietary) is still in development. When it reaches parity (full-text + facets + typo-tolerance), Cortex will migrate without changing the Cortex API — only the cortex-fulltext-worker backend.

### Why Haiku (local small model) + Sonnet (cross-event)?

**Cortex architecture §5.2.1:**
- **Haiku per-event:** Fast, cheap classification for individual events. StaticClassifier (offline rules) is the default; Haiku CLI is opt-in.
- **Sonnet cross-event:** "Per-event Haiku classification was producing tags with no lift; what was missing was the wider lens." (analyzer.rs:9-12). Cross-event analysis requires larger model to find patterns across sessions.
- **Future:** Shift to Expert (HiveLLM local model) if latency/cost justify it. This is a future option, not a current blocker.

### Why reuse Rulebook decisions/laws/learnings?

**Cortex bootstrap default-discovery (commit fc87b4d):**  
cortex-bootstrap walks `.rulebook/*/` across all repos and auto-indexes decisions, laws, learnings, tasks as Cortex entities. Rationale: **avoid re-implementing Rulebook.** The source of truth is in `.rulebook/`; Cortex indexes and surfaces it.

This is consistent with the project principle: "never reimplementing HiveLLM services."

### Why per-repo collection/index naming?

**Cortex embedder (learning 2026-04-27), Cortex fulltext (routing.rs:105-107):**  
Collections and indexes are named `cortex-{repo}-{family}` (e.g., `cortex-cortex-code`, `cortex-vectorizer-code`).

Rationale:
- **Isolation:** Multi-repo deployment does not require schema/ACL changes; new repos automatically get new collections/indexes.
- **Scoped queries:** Users can filter by repo; dashboard views can highlight per-repo health.
- **Cleanup:** Retiring a repo means dropping one collection/index, not merging globals.

## Specs and Executable Definitions

Full implementation specs live in `docs/specs/` (00-index.md points to all 18). The critical ones:

| Spec | Phase | Title | Role | Status |
|------|-------|-------|------|--------|
| 01-04 | 0 | Storage, archival, ingestion, PII redaction | Foundation | 🟢 Done |
| 05-12, 18 | 1 | Classify, embed, graph, fulltext, query, pre-thinking, bootstrap, MCP | Capture + Retrieval | 🟢 Done |
| 13-14 | 2 | Laws DSL, governance engine, sandbox | Governance | 🟡 Draft |
| 15, 17 | 3 | Deep analysis, multi-adapter | Cross-event + Scope | ❌ Not started |
| 16 | 2-3 | Dashboard | UX | 🟡 Partial (GUI built, engine not) |

Each spec includes SHALL/MUST requirements and Given/When/Then scenarios.

## Rules (AGENTS.md + CLAUDE.md)

**Project-specific rules (AGENTS.override.md):**
- **LAW-CORTEX-001:** Strict task-sequence execution (no cherry-picking phases).
- **LAW-CORTEX-002:** Reserved for future extensions.

These sit above standard AGENTS.md tier-1 prohibitions (no shortcuts, no destructive git ops, research before implementing, etc.).

## How to Propose a New Decision

1. File issue / discussion in `.rulebook/proposals/`.
2. Draft ADR in `.rulebook/decisions/` with `proposed` status.
3. Record rationale (cost/benefit, alternatives considered, impact on crates/specs).
4. Link from architecture.md or this consolidation file.
5. Accept/supersede/deprecate as implementation reveals trade-offs.

See `.rulebook/decisions/*.md` for templates and examples.
