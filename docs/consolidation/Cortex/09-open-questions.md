# Cortex — Open Questions & Deferred Work

## Unresolved Issues (Blocking or High-Risk)

### 1. Vectorizer SDK Upsert Reporting Drift

**Issue:** Every embedding batch reports `total_failed=4-5` with `vector_count=0`, but vectors are queryable downstream.  
**Root cause:** Unknown — either misreporting or partial-success path undocumented by Vectorizer.  
**Current workaround:** Trust the queryable-downstream signal; ignore SDK response metrics.  
**Deferred to:** Phase4d (consistency check via `cortex doctor`).  
**Tracked in:** [Knowledge anti-pattern](../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side.md).

### 2. Meilisearch Single-Repo Coverage

**Issue:** Bootstrap walked 3+ repos (Cortex, Nexus, Rulebook, Synap) but only Cortex appears in Meili.  
**Root cause:** Worker consumer offset not advancing or stream topology not wired for multi-repo.  
**Current workaround:** None; full-text search is single-repo only.  
**Deferred to:** Phase4a (diagnose worker offset, multi-repo orchestrator).  
**Impact:** Dashboard views (memory, decisions, violations, analyses) have reduced coverage.

### 3. Graph Topology is Flat (Missing Symbol Relations)

**Issue:** Chunker emits `symbol` field per Vectorizer payload, but `cortex-graph` mapper drops it. No `(:Symbol)-[:DEFINES]->(:Artifact)` edges exist.  
**Impact:** Graph cannot answer "what functions are defined in this file?" or "what calls this function?"  
**Current workaround:** None; must ask embedder/fulltext lanes instead (slower, less precise).  
**Deferred to:** Phase4c (emit symbol relations).  
**Tracked in:** ADR 004 follow-up (graph richness).

### 4. Governance Engine Unbuilt (Phase 2)

**Issue:** Laws DSL drafted (spec 13); enforcement engine, sandbox detector, trust scoring not implemented.  
**Current state:** Dashboard renders law/violation tables from fixtures, not live state.  
**Deferred to:** Phase 2 (specs 13–14).  
**Milestone:** Depends on Phase 1 completion + governance team availability.

### 5. Nexus UNWIND-Writes Silently Drop

**Issue:** Batch writes with `UNWIND` or parameter substitution don't land; SDK returns 200 OK regardless.  
**Current workaround:** Per-row Cypher with explicit parameters + post-write assert (commit 5bd0185).  
**Deferred to:** Track Nexus 2.2+ releases for fix; if fixed, per-row becomes wasteful but safe.  
**Tracked in:** [Knowledge anti-pattern](../../.rulebook/knowledge/anti-patterns/cypher-unwind-write-and-param-write-substitution-silently-drop-in-nexus-1-15.md).

## Incomplete Features (Partially Implemented, Spec Flagged 🟡)

### 6. Dashboard (Spec 16 — partial)

**Status:** 🟡 Draft  
**Implemented views:**
- Timeline (live SSE) ✅
- Memory (Meili-backed) ✅
- Decisions (register) ✅
- Conversations (turns + Sonnet summary) ✅
- Analyses (report browser) ✅
- Tools (stat aggregator) ✅
- Handoffs (envelope timeline) ✅

**Incomplete views:**
- Laws (reads fixtures, not live) 🟡
- Violations (audit trail; no live detector) 🟡
- Trust (governance stub) 🟡
- Graph (Cytoscape backend exists; richness blocked by phase4c) 🟡

**Deferred to:** Phase 2 (specs 13–14 governance implementation).

### 7. Pre-Thinking Injection (Spec 12 — partially)

**Status:** 🟢 Core implemented; gaps in quality/tuning  
**Implemented:**
- Bundle assembly pipeline (cortex-pre-thinking) ✅
- Context fusing (decisions, laws, similar turns, snippets) ✅
- MCP tool exposure (cortexPreThinking) ✅

**Known gaps:**
- Tuning per intent type (ADR 003 drafted; weights TBD).
- Quality evaluation (no benchmark; Phase 4 hardening).
- Freshness decay (soft-coded; could be user-tunable).

**Deferred to:** Phase 4 (quality / perf tuning).

### 8. Bootstrap Multi-Repo Orchestrator (Phase 4b)

**Status:** 🟡 Partial  
**What works:**
- Single-repo bootstrap via cortex-bootstrap CLI.
- 4 repos walked in current state (Cortex, Nexus, Rulebook, Synap).
- Default-discovery of `.rulebook/*`.

**What's missing:**
- Multi-repo orchestration (no parent task manager).
- State persistence across invocations (`.cortex-bootstrap.state.json` overwritten per run).
- Progress tracking (no checkpoints for partial retries).
- Targeted re-bootstrap (no way to reindex just Rulebook without re-walking Cortex).

**Deferred to:** Phase4b.  
**Target:** 17 Hive repos indexed in one pass.

### 9. Cortex CLI (cortex-ops)

**Status:** ❌ Not implemented  
**Planned subcommands:**
- `cortex ops doctor` — consistency checks across backends.
- `cortex ops prune` — cleanup stale indexes.
- `cortex ops reindex` — rebuild specific backend.

**Deferred to:** Phase 4 (ops hardening).

### 10. Non-Claude Adapters (Spec 17)

**Status:** ❌ Not started  
**What's needed:**
- Cursor adapter (hook contract differs from Claude Code).
- Codex / Gemini / Copilot adapters (async capture, no hooks available?).
- Generic LLM adapter (CLI-over-HTTP for any LLM).

**Deferred to:** Phase 3.  
**Complexity:** Cursor is straightforward (same hook API); others TBD.

## Unresolved Design Questions

### Q1. Local vs. Cloud Governance Execution

**Question:** Should law detectors run locally (Deno sandbox in container) or offload to cloud?  
**Trade-offs:**
- **Local:** Lower latency, better privacy, no credential leakage.
- **Cloud:** Easier horizontal scaling, shared detector repository.

**Current assumption:** Local (sandbox in phase 13 spec). No decision made yet.  
**Depends on:** Phase 2 governance implementation.

### Q2. Graph Consolidation Grain

**Question:** At what granularity should cross-event analysis consolidate (ADR 005)?  
**Options:**
- **Session** — full conversation (implemented ✅).
- **Topic** — all turns with topic X (spec 15 analysis only).
- **Decision trace** — decision + all references (spec 15 analysis only).
- **Time window** — e.g., "last 3 hours of interactions" (not yet considered).

**Current state:** ADR 005 proposes session + topic + trace. Analysis/decision lanes to validate.  
**Depends on:** Phase 3 (deep analysis). Field-testing needed.

### Q3. Meili vs. Lexum Migration Path

**Question:** When Lexum reaches parity, how to migrate Meili indexes?  
**Options:**
- **Zero-downtime:** Dual-write for N weeks, then cutover.
- **Reindex:** Stop Cortex, dump Meili, re-index into Lexum.
- **Hybrid:** Lexum for new data, Meili for archive.

**Current state:** Meilisearch is a stand-in (README.md §5.2). No migration plan drafted.  
**Depends on:** Lexum production-readiness (HiveLLM/Lexum team).

### Q4. Trust Scoring Model

**Question:** How to quantify "trust score" for a turn/session/agent?  
**Factors to consider:**
- Law violations (high weight).
- Tool-call patterns (are they anomalous?).
- Confidence of classification/analysis.
- Temporal decay (older = less trusted?).

**Current state:** Stub in dashboard (/v1/dashboard/trust returns empty).  
**Deferred to:** Phase 2 governance spec detail.

## Tracked Backlog Items (Linked to .rulebook/tasks/)

| Phase | Task | Status | Description |
|-------|------|--------|-------------|
| Phase4a | consolidation-gang | 🟡 In progress | Diagnose Meili coverage gap; multi-repo bootstrap state |
| Phase4b | multi-repo-orchestrator | ⏸ Blocked | Orchestrate bootstrap across 17 Hive repos |
| Phase4c | symbol-graph-relations | ⏸ Blocked | Emit `(:Symbol)-[:DEFINES]->(:Artifact)` edges |
| Phase4d | cortex-doctor-consistency | ⏸ Blocked | Consistency checks across backends (Vectorizer upsert, Nexus UNWIND) |
| Phase 11v | mcp-fine-grained-backend-search | 🟡 Active | Add backend-specific search filters to MCP cortexQuery tool (5 items) |
| Phase 2 (future) | governance-engine | ❌ Not started | Specs 13–14 implementation (law detectors, sandbox, trust scores) |
| Phase 3 (future) | deep-analysis + adapters | ❌ Not started | Spec 15 (debate workflow) + Spec 17 (Cursor/Codex/Gemini) |

## Maintenance Debt & Anti-Patterns

**Recorded anti-patterns** (in `.rulebook/knowledge/anti-patterns/`):

1. **Don't bake tooling-only fields into payloads** — Learned when Meili rejected `"version": "v1"` in settings. Strip at client boundary.
2. **Vectorizer SDK 3.0.3 upsert reporting drift** — 6 drifts tracked; 2 resolved (login, upsert); 4 still open (server-side).
3. **Cypher UNWIND-write silently drops** — Use per-row Cypher + post-assert instead.
4. **Don't ship a bespoke HTTP client when an in-tree pipeline crate already drives that endpoint** — Migration from adapter's HTTP → cortex-pre-thinking crate (commit e312cd2).

See `.rulebook/knowledge/` for full list and mitigation strategies.

## Performance Benchmarks (None Yet)

**Missing (Phase 4 hardening):**

- **Query latency** — target P50 < 50ms, P99 < 100ms (from spec 11).
- **Indexing throughput** — events per second (per worker).
- **Embedding quality** — recall@k, MRR (for Vectorizer lane).
- **Graph traversal latency** — Cypher query on 10k+ nodes.
- **Storage footprint** — Parquet archive vs. Meili vs. Vectorizer vs. Nexus.

**Baseline context:** Phase 1 is functionally complete; Phase 4 will instrument and tune.

## Future: Cortex 2.0 Vision

Not in scope yet, but documented for continuity:

- **Multi-org federation** — secure sharing of decisions/learnings across independent HiveLLM instances.
- **LLM pluggability** — swap Claude → open-source model (e.g., Llama) with single config change.
- **Mobile/browser-only UI** — detach GUI from Electron (web-based dashboard).
- **Audit trail immutability** — archive signed / sealed per Rulebook law escrow.
