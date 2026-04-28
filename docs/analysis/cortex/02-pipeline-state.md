# 02 — Pipeline state (capture → enriched → indexes)

The conceptual pipeline: **adapter / bootstrap → `cortex.events.raw` (or `cortex.events.bootstrap`) → classifier-worker → `cortex.events.enriched` → embedder + graph + fulltext → query API**. This file walks each leg and reports its state.

```
┌──────────────────┐                ┌────────────────────────┐
│ Claude Code      │  hooks (10)    │ cortex.events.raw      │
│ adapter (spec10) │ ─────────────▶ │  Synap stream          │
└──────────────────┘                └────────────────────────┘
                                                │
┌──────────────────┐  one-shot                  │
│ cortex-bootstrap │ ─────────────▶ ┌────────────────────────┐
│ (spec 09)        │                │ cortex.events.bootstrap│
└──────────────────┘                └────────────────────────┘
                                                ▼
                              ┌──────────────────────────────┐
                              │ cortex-classifier-worker     │   spec-05 follow-up
                              │ (Haiku-CLI / Static / Cached)│   landed 2026-04-27
                              └──────────────────────────────┘
                                                ▼
                              ┌──────────────────────────────┐
                              │ cortex.events.enriched       │
                              └──────────────────────────────┘
                                                ▼
              ┌─────────────────────┬───────────────────────┬─────────────────────┐
              ▼                     ▼                       ▼                     ▼
       cortex-embedder       cortex-graph             cortex-fulltext     [archive]
       → Vectorizer          → Nexus                  → Meilisearch       Parquet zstd
        (spec 06)             (spec 07)                (spec 08)           (spec 02)
              │                     │                       │                     │
              └─────────────────────┴───────────────────────┴─────────────────────┘
                                              ▼
                              ┌──────────────────────────────┐
                              │ cortex-api  /v1/query        │   spec 11 (RRF)
                              │ + dashboard backend          │   spec 16 / 18 / 12
                              └──────────────────────────────┘
```

## Leg-by-leg

### A. Capture — `cortex-adapter-claude-code`

- ✅ **Hook contract complete.** `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `SubagentStop`, `Stop` wired to the local daemon. Stop hook now produces a `Turn` envelope with `assistant_message` (commit `15b8931`), closing the asymmetry where user prompts were captured but model replies were not.
- ✅ **Pre-thinking pipeline reused, not re-implemented.** Earlier the adapter shipped a bespoke HTTP client; the [recorded anti-pattern](../../../.rulebook/knowledge/anti-patterns/don-t-ship-a-bespoke-http-client-when-an-in-tree-pipeline-crate-already-drives-that-endpoint.md) drove the migration to `cortex_pre_thinking`. Commit `e312cd2`.
- ✅ **MCP descriptors fixed** to identifier-safe names with camelCase fields (commit `9f14ef6`) — earlier they used dot-separated names and snake_case fields that the MCP spec rejects.
- 🟡 **Other adapters (Cursor, Codex, Gemini) — not started.** Spec 17 still 🟡.

### B. Backfill — `cortex-bootstrap`

- ✅ Single-repo design works: walks one tree from a `cortex.toml`, emits envelopes per file/commit/decision/memory.
- ✅ Default-discovery for `.rulebook/*` (commit `fc87b4d`) — picks up tasks/learnings/decisions across repos automatically.
- ✅ **Per-event publish failure tolerance** (commit `845b5eb`) with a 5% / 20-floor — accommodates Synap "Room not found" first-time misses.
- ⚠️ **No multi-repo orchestrator.** [.cortex-bootstrap.state.json](../../../.cortex-bootstrap.state.json) tracks 4 repos walked (Cortex 617 events, Nexus 2642, Rulebook 1654, Synap 1304) but the `repos` map is overwritten per invocation — phase4b is the planned fix.
- 🟡 Plan target: 17 Hive repos. Today: 4 walked, of which only **3** have surviving data in any backend.

### C. Classification — `cortex-classifier` + `cortex-classifier-worker`

- ✅ **Worker bridge landed 2026-04-27** ([learning](../../../.rulebook/learnings/2026-04-27T00-32-26-end-to-end-cortex-bootstrap-on-the-cortex-repo-pipeline-gaps-surfaced.md)). Before that, both the raw stream and the bootstrap stream had no consumer — Vectorizer/Nexus/Meili would stay empty regardless of how many events were emitted. This was the single biggest blocker on the Phase-1 critical path.
- ✅ Default mode is `StaticClassifier` behind cache + budget tracker (offline, deterministic, zero LLM cost). Opt-in to `HaikuCliClassifier` via `CORTEX_CLASSIFIER_MODE=cli`.
- ✅ Worker lives in its own crate to avoid the `classifier → embedder → classifier` dependency cycle. ADR [002](../../../.rulebook/decisions/002-classifier-worker-lives-in-a-separate-crate-to-avoid-the-classifier-embedder-classifier-cycle.md).
- ✅ Recent fix (commit `c41dab0`) — drop `--max-tokens` flag for Claude Code CLI 2.x compatibility.
- ⚠️ **Per-event Haiku classification was producing low-lift tags.** Note in [analyzer.rs:9-12](../../../crates/cortex-api/src/analyzer.rs) — "Per-event Haiku-grade classification was producing tags with no lift; what was missing was the wider lens." This is why Sonnet-backed cross-event analysis was added.

### D. Embedding — `cortex-embedder`

- ✅ Tree-sitter chunker working (cc/0.23 conflict resolved per [learning 2026-04-22](../../../.rulebook/learnings/2026-04-22T11-49-48-tree-sitter-0-22-grammars-cc-version-conflict-resolved-by-bumping-all-to-0-23.md)).
- ✅ Per-project collection isolation: collection name = `cortex-{repo}-{family}` ([learning 2026-04-27](../../../.rulebook/learnings/2026-04-27T17-28-02-per-project-collection-isolation-slug-repo-into-every-collection-index-name.md)).
- ✅ JWT auth working — [learning](../../../.rulebook/learnings/) documents that the SDK transport sniffs three-segment shape.
- ❌ **Vectorizer 3.0.3 `/upsert` drift.** Every batch reports `total_failed=4-5` and the surviving chunks don't actually persist (`vector_count=0`). Recorded as [knowledge anti-pattern](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side.md). The same drift was previously worked around by direct `reqwest` (ADR 001, now superseded).
- ⚠️ Embedder should call `/auth/login` itself when the password env var doesn't look like a JWT, rather than 401-ing. Listed as a follow-up in the 2026-04-27 learning.

### E. Graph — `cortex-graph`

- ✅ Per-row Cypher renderers + escape helper (commit `1e34417`).
- ✅ Stamps human-readable `name` on every Nexus node (commit `450d147`).
- ✅ One Session per bootstrap run + Turn label fallback (commit `0fdbd38`).
- ✅ `assert_write_landed` surfaces silently-dropped edges (commit `5bd0185`) — this was the visible fix for the Nexus UNWIND/parameter-substitution drop bug.
- ❌ **Topology is still flat.** Audit on 2026-04-27 found only `IN_REPO` (10245) and `REMEMBERS` (30) edges — no `DEFINES`, no `CALLS`, no `IMPORTS`. The chunker emits a `symbol` field per Vectorizer payload, but [crates/cortex-graph/src/mapper.rs](../../../crates/cortex-graph/src/mapper.rs) drops it. **Phase4c** is the planned fix.
- ❌ **Recorded knowledge:** [Cypher UNWIND-write and param-write substitution silently drop in Nexus 1.15](../../../.rulebook/knowledge/anti-patterns/cypher-unwind-write-and-param-write-substitution-silently-drop-in-nexus-1-15.md). The drift was previously masked because the SDK call returned 200 OK regardless.

### F. Full-text — `cortex-fulltext`

- ✅ Per-project index isolation: `cortex-{repo}-{family}` ([crates/cortex-fulltext/src/routing.rs:105-107](../../../crates/cortex-fulltext/src/routing.rs#L105-L107)).
- ✅ Settings stripped of tooling-only fields (commit per [knowledge](../../../.rulebook/knowledge/anti-patterns/don-t-bake-tooling-only-fields-into-json-payloads-sent-verbatim-to-a-strict-downstream.md)) — earlier the worker hard-failed at boot because `settings.v1.json` had a `"version": "v1"` field Meili rejects on `PATCH /indexes/{uid}/settings`.
- ✅ `agent_call` events now route to turns + a `routed_total` metric exists (commit `66c4450`).
- ✅ Artifacts route by path + topics instead of dumping in `docs` (commit `86417b5`).
- ❌ **Fan-out gap.** Meilisearch has only the `Cortex` repo indexed despite Vectorizer + Nexus showing Rulebook and Vectorizer. The routing code is correct; the *worker offset / consumer state* is the suspected root cause. **Phase4a** is the planned fix.

### G. Query API — `cortex-api`

- ✅ `/v1/query` (RRF fusion) + 16 dashboard endpoints. See [crates/cortex-api/src/dashboard.rs](../../../crates/cortex-api/src/dashboard.rs).
- ✅ Three live lanes wired:
  - **VectorLane** — Vectorizer SDK 3.0.3 (commit `dbd60e8`)
  - **MeiliKeywordLane** — live + source-attribution invariant (commit `99e8ef3`)
  - **GraphLane** — Nexus-backed (commit `f8966c4`)
- ✅ Canonical scope echo + slug-aware cache invalidation (commit `350e30a`).
- ✅ **Sonnet analyzer** ([crates/cortex-api/src/analyzer.rs](../../../crates/cortex-api/src/analyzer.rs), commit `a62fcbd`) — produces cross-event session summaries via Claude CLI or direct API. Falls back gracefully when the CLI is not on PATH (CI / server scenarios).
- ✅ SSE timeline stream + reconnect ladder (commit `ac10b5e`).
- 🟡 **Result quality unmeasured.** No retrieval-quality benchmark (recall@k, MRR). Phase 4 hardening line item.

### H. Pre-thinking + MCP — `cortex-pre-thinking`, `cortex-mcp-server`, `cortex-plugin`

- ✅ MCP server with identifier-safe tool names + camelCase schemas (commit `9f14ef6`).
- ✅ Spec-18 (Claude Code plugin) marked 🟢 — `cortex_query`, `cortex_pre_thinking`, `cortex_status` exposed.
- ✅ Adapter sync paths use `cortex_pre_thinking` instead of bespoke HTTP (commit `e312cd2`, recorded as anti-pattern).

## Summary

| Leg                  | State            | Notes                                                            |
|----------------------|------------------|------------------------------------------------------------------|
| Capture (Claude)     | 🟢 OK            | Stop→Turn closes asymmetry; non-Claude adapters not started.     |
| Bootstrap            | 🟡 Single-repo   | 4 repos walked, 3 partially indexed, no orchestrator (phase4b).  |
| Classifier worker    | 🟢 OK            | Bridge landed 2026-04-27. Static default; CLI opt-in.            |
| Embedder             | ❌ Drift          | SDK 3.0.3 `/upsert` losing chunks. Auth needs JWT auto-login.    |
| Graph                | 🟡 Shallow       | Two edge types; symbol info dropped at mapper (phase4c).         |
| Full-text            | ❌ Fan-out gap    | Only 1 repo of 3 indexed (phase4a).                              |
| Query API            | 🟢 OK             | Three live lanes; quality unmeasured.                            |
| Pre-thinking + MCP   | 🟢 OK             | Adapter uses pipeline crate; MCP descriptors compliant.          |
