# Pre-Thinking Analysis — Findings

> **Analysis ID:** PRE-001 · **Date:** 2026-05-05

Each finding includes: title, evidence (file:line), impact, and confidence level.

---

## Index of Files

| # | File | Module | Lines |
|---|---|---|---|
| 01 | `crates/cortex-pre-thinking/src/lib.rs` | Entry point, public API re-exports | 32 |
| 02 | `crates/cortex-pre-thinking/src/pipeline.rs` | Orchestrator: scope→intent→query→format→clip | 224 |
| 03 | `crates/cortex-pre-thinking/src/scope.rs` | Scope derivation: repo, files, topics from cwd+git+prompt | 376 |
| 04 | `crates/cortex-pre-thinking/src/intent_select.rs` | Intent selector: 55 keyword rules → 6 intents | 461 |
| 05 | `crates/cortex-pre-thinking/src/formatter.rs` | Deterministic Markdown bundle assembly (10 context bands) | 1397+ |
| 06 | `crates/cortex-pre-thinking/src/budget.rs` | Budget clipper: 6-step trim ladder | 334 |
| 07 | `crates/cortex-pre-thinking/src/metrics.rs` | Pre-think metrics registry (AtomicU64 + Mutex) | 71 |
| 08 | `crates/cortex-api/src/orchestrator.rs` | Hybrid query orchestrator: 3 lanes + RRF fusion | — |
| 09 | `crates/cortex-api/src/lanes.rs` | Lane traits: VectorLane, KeywordLane, GraphLane | — |
| 10 | `crates/cortex-api/src/types.rs` | QueryRequest/QueryResponse, Intent, Scope, ResultsBag | — |
| 11 | `crates/cortex-core/src/events.rs` | Event schema: Envelope, Kind (12 variants), payloads | — |
| 12 | `crates/cortex-core/src/ids.rs` | ID types: EventId(Ulid), SessionId(Ulid) | — |
| 13 | `crates/cortex-workers/src/classifier/` | Classifier worker: Haiku batch classification | — |
| 14 | `crates/cortex-workers/src/consolidator/` | Consolidation producer: Session/Topic/DecisionTrace | — |
| 15 | `crates/cortex-workers/src/topic_cards/` | Topic cards: living synthesis + contradiction detection | — |
| 16 | `crates/cortex-adapter-claude-code/src/` | Claude Code adapter: hooks + daemon + pre-thinking wiring | — |
| 17 | `crates/cortex-mcp-server/src/` | MCP stdio JSON-RPC bridge to api/pre-thinking | — |
| 18 | `crates/cortex-cli/src/bootstrap/walker.rs` | Bootstrap: recurse .rulebook/knowledge/** and learnings/** | — |
| 19 | `crates/cortex-storage/` | Storage layout, namespace constants, metadata + CAS | — |
| 20 | `crates/cortex-health/src/lib.rs` | Health-report types, aggregator, /healthz endpoint | — |
| 21 | `crates/cortex-build/` | Build-time version metadata (git_sha, build_ts, dirty) | — |
| 22 | `docs/architecture.md` | Full architecture (951 lines) | 951 |
| 23 | `docs/prd.md` | Product requirements (262 lines) | 262 |
| 24 | `docs/specs/12-pre-thinking-injection.md` | Pre-thinking spec (423 lines) | 423 |
| 25 | `docs/specs/11-query-api.md` | Query API spec | — |
| 26 | `docs/specs/10-claude-code-adapter.md` | Adapter spec | — |
| 27 | `docs/specs/05-classifier.md` | Classifier spec | — |
| 28 | `docs/dag.md` | Dependency graph of 22 specs | — |
| 29 | `docs/roadmap.md` | Phased delivery plan | — |
| 30 | `docs/cortex/topic-cards.md` | Topic cards operator runbook | — |
| 31 | `docs/cortex/consolidation-tuning.md` | Consolidation tuning handbook | — |

---

## F-001: Cortex Pre-Thinking Pipeline Is Fully Implemented as a 5-Stage Fail-Open Pipeline

**Evidence:**
- **[#02]** `pipeline.rs:111-213` — `run()` function orchestrates scope → intent → query → format → clip
- **[#03]** `scope.rs:57-110` — scope derivation from cwd + git status + prompt files
- **[#04]** `intent_select.rs:224-258` — 55-keyword rule table mapping prompts to 6 intents
- **[#05]** `formatter.rs:136-466` — deterministic Markdown assembly with 10 context bands
- **[#06]** `budget.rs:52-150` — 6-step trim ladder with laws as load-bearing

**Impact:** High. This is the core mechanism enabling LLMs to receive grounded context. Every error path returns empty string rather than breaking the session.

**Confidence:** High (source code verified, tests exist)

---

## F-002: Intent Routing Uses Rule-Based Keyword Matching, Not ML

**Evidence:**
- **[#04]** `intent_select.rs:47-219` — `DEFAULT_RULES` table with 55 keyword rules
- **[#04]** `intent_select.rs:245-258` — `select_matched_with()` iterates rules, first match wins
- **[#24]** Spec 12 Decision 1: "We graduate to a model only if offline eval shows >5% precision gap"

**Impact:** Medium. Provides predictable, zero-latency routing. The 6 intents (`explain`, `decision_lookup`, `similar_problems`, `law_check`, `pre_change_context`, `free_search`) each trigger different retrieval strategies and overlay sets. The `explain` intent fires before `decision_lookup` so navigational queries ("explain why we picked X") don't burn the decisions overlay budget.

**Confidence:** High

---

## F-003: Hybrid Retrieval Fuses 3 Independent Lanes via Reciprocal Rank Fusion

**Evidence:**
- **[#22]** `architecture.md:250-258` — vector (Vectorizer KNN) + keyword (Meilisearch BM25) + graph (Nexus Cypher)
- **[#08]** `orchestrator.rs` — `Orchestrator.execute()` fans out to all lanes in parallel
- **[#09]** `lanes.rs` — `VectorLane`, `KeywordLane`, `GraphLane` async traits
- [#22] Architecture §5.3: RRF alpha=0.7, k=60, results fused and returned as structured bundle

**Impact:** High. Multi-modal retrieval means the LLM gets context from three independent signal sources simultaneously — semantic meaning (vector), exact text (keyword), and structural relationships (graph). This is vastly richer than single-mode retrieval.

**Confidence:** High

---

## F-004: Topic Cards Are the Highest-Cognitive Layer — Living Synthesis With Contradiction Detection

**Evidence:**
- **[#22]** `architecture.md:342-370` — topic cards rewrite in place as evidence accumulates; one card per (topic_slug, repo_scope)
- **[#22]** `architecture.md:360-366` — 3 contradiction detectors: `DecisionSupersession`, `LawViolationMismatch`, `OutcomeDivergence`
- **[#22]** `architecture.md:352-358` — 3 trigger heuristics for rewrite: burst (≥8 events), high-impact proximity, stale+new evidence
- **[#05]** `formatter.rs:154-168` — staleness contract: confidence<0.6 OR (age>30d AND events_since_last_rev>0)
- **[#05]** `formatter.rs:486-559` — topic card section renderer with evidence + contradictions blocks

**Impact:** High. This is the most sophisticated cognitive mechanism — it doesn't just retrieve past data, it synthesizes new understanding and explicitly flags contradictions rather than smoothing them over. The pre-thinking renderer gives topic cards top priority ahead of consolidations when fresh.

**Confidence:** High (implementation exists, tests verify fresh/stale ordering)

---

## F-005: Consolidations Replace Raw Past Sessions as Higher-Fidelity Context

**Evidence:**
- **[#22]** `architecture.md:330-338` — 3-tier storage: raw events → consolidations → Parquet archive
- **[#22]** `architecture.md:345-350` — consolidation modes: Session, Topic, DecisionTrace
- **[#05]** `formatter.rs:174-182` — when consolidations≥1, past-sessions is suppressed; fallback when zero
- **[#14]** `consolidator/` — producer with Haiku (Shallow) and Opus (Deep) summarisers

**Impact:** Medium-High. Consolidations distill many raw events into a single line the agent can read instantly. They replace the "Past sessions" section entirely when available, giving the LLM curated instead of raw context.

**Confidence:** High

---

## F-006: The 6-Step Trim Ladder Is an Elegant Budget Enforcement Mechanism With Correct Priorities

**Evidence:**
- **[#06]** `budget.rs:52-150` — clip_to_budget() with 6 ordered steps
- **[#06]** `budget.rs:138-143` — step 6 drops snippets entirely as last resort
- **[#06]** `budget.rs:307-314` — test: "laws_are_never_dropped" verifies laws survive even 600-byte budget
- **[#24]** Spec 12 §Budget-aware section caps trim order: DropGraph → SlimSnippets → HalveSnippets → HalveTurns → TruncateDecisions → DropSnippets

**Impact:** Medium. The trim ladder ensures the LLM always gets the most critical context (laws, decisions) while less critical sections (graph, snippets) are sacrificed first. The explicit step tracking (`TrimStep` enum) gives operators full visibility into truncation behavior.

**Confidence:** High

---

## F-007: Scope Derivation Is Deliberately Cheap — No API Calls, Pure Functions

**Evidence:**
- **[#03]** `scope.rs:57-110` — derive() merges recent_files + prompt-extracted paths, capped at 16
- **[#03]** `scope.rs:134-181` — repo_from_cwd() walks ancestors for .git/, reads cortex.toml override
- **[#03]** `scope.rs:216-238` — extract_prompt_files() uses regex; filters version-number-shaped tokens
- **[#03]** `scope.rs:119-132` — topic_for_path() maps file extensions to `code`/`docs`/`config` topics

**Impact:** Medium. Zero-latency scope inference means the pre-thinking pipeline adds negligible overhead before the query. Topics are derived from file extensions (mirroring classifier vocabulary) so the orchestrator's lane filter targets relevant corpora without the user specifying it.

**Confidence:** High

---

## F-008: Fail-Open Semantics Are Applied at Every Pipeline Stage

**Evidence:**
- **[#02]** `pipeline.rs:142-148` — tokio::time::timeout wraps the query call; timeout → empty bundle
- **[#02]** `pipeline.rs:150-162` — None response → empty bundle with fail_open=true
- **[#05]** `formatter.rs:186-196` — all-zero sections → empty string (not "No context found")
- **[#24]** Spec 12 §Error handling table: 5 failure modes, all return empty/no-op

**Impact:** High. This is architecturally critical — the pre-thinking injection sits on the synchronous critical path of LLM sessions. A failure must never block, delay, or corrupt the session. The design is correct: an empty bundle is infinitely better than a broken session.

**Confidence:** High

---

## F-009: Context Band Formatting Is Deterministic and Designed for LLM Consumption

**Evidence:**
- **[#05]** `formatter.rs:136-199` — pure Rust string concatenation; no template engine
- **[#05]** `formatter.rs:667-687` — outcome_glyph() and decision_glyph() map to ✓/✗/⚠ for scanability
- **[#05]** `formatter.rs:596-633` — render_snippet_header() produces "repo/path:symbol — why" format
- **[#24]** Spec 12 Decision 4: "Fixed section order, no prose — the model relies on structural cues more than stylistic ones"
- **[#05]** test: byte-identical output across 1000 runs (`formatter.rs:941-947`)

**Impact:** Medium. The stable, scannable Markdown layout with glyph markers (✓/✗/⚠) optimizes for how LLMs actually consume context — they extract more signal from structured, consistent formats than from prose summaries.

**Confidence:** High

---

## F-010: The QueryFn Trait Decouples Pre-Thinking From Transport

**Evidence:**
- **[#02]** `pipeline.rs:61-82` — `QueryFn` async trait + `ClosureQueryFn` adapter wrapper
- **[#02]** `pipeline.rs:111` — `run()` takes `Arc<Q: QueryFn>`, not a network client
- **[#01]** `lib.rs:27-28` — re-exports allow adapter to wire its `SyncClient` through the closure

**Impact:** Medium. This design allows testing with canned responses (no server needed), swapping transports (HTTP vs MCP), and ensures the pipeline never depends on network availability. The adapter wires `SyncClient::pre_thinking` through the closure; tests inject static fixtures.

**Confidence:** High

---

## F-011: Knowledge + Learnings Sources Feed Pre-Thinking Bundles

**Evidence:**
- **[#24]** Spec 12 §Sources (phase10e): Knowledge (patterns/anti-patterns) and Learnings (implementation insights)
- **[#18]** `walker.rs` — populates from `.rulebook/knowledge/**` and `.rulebook/learnings/**`
- Spec 02 §Knowledge + Learnings corpus — cross-store contract for Vectorizer + Meilisearch

**Impact:** Medium-High. The LLM re-reads canonical patterns and anti-patterns before acting on a related change. These were written because someone made a mistake worth not repeating — they are the highest-signal context for pre-thinking.

**Confidence:** Medium (spec exists; implementation partially verified)

---

## F-012: In-Session Capture Bridge Closes the Mid-Session Memory Gap

**Evidence:**
- **[#24]** Spec 12 §In-session capture (phase10j): `cortex_capture_memory` MCP tool POSTs envelopes
- Spec 18 (Claude Code plugin): tool validates body ≤ 8 KiB, stamps event_id, forwards to `/v1/ingest`
- Without this tool: mid-session facts written via `rulebook_memory_save` go to disk store, NOT the pre-thinking lane

**Impact:** Medium. Closes the gap where an agent discovers something mid-session that the next pre-thinking call should surface.

**Confidence:** Medium (spec exists, implementation referenced)

---

## F-013: Graph Layer Surfaces 3 Named Edge Classes for Blast-Radius Awareness

**Evidence:**
- **[#24]** Spec 12 §Graph traversal sub-blocks (phase11k §6.2): IMPORTS_FILE, DOCUMENTED_BY, CITES
- **[#05]** `formatter.rs:367-462` — formatter splits graph_neighbors into 3 named sub-blocks + catch-all
- **[#05]** `formatter.rs:854-877` — test: "imports_file_neighbours_render_under_connected_files_block"

**Impact:** Medium. Before editing, the LLM sees: what other files import this file, what docs document this symbol, what decision chain participates.

---

## F-014: Observability Infrastructure Is Extensive (11 Phases)

**Evidence:**
- **[#22]** `architecture.md:604-793` — health endpoints (8a), freshness/divergence metrics (8b), version coherence (8c)
- **[#22]** `architecture.md:725-801` — config coherence audit (8d), silent-drop detector (8e), synthetic E2E canary (8f)
- **[#22]** `architecture.md:844-936` — dashboard health view (8g), CI smoke gate (8h)
- **[#07]** `metrics.rs:1-71` — 7 metric counters/histograms with thread-safe atomic/mutex registry

**Impact:** Medium-High. Enables operators to detect and diagnose pre-thinking failures before they affect users.

---

## F-015: Cortex Complements, Doesn't Replace, Existing LLM Memory Mechanisms

**Evidence:**
- **[#22]** `architecture.md:39-42` — NG2: "Cortex is not a coding agent", NG3: "does not replace per-tool memory"
- **[#23]** `prd.md:39-43` — NG2, NG3, NG4 restated
- [#22] Architecture §3: Cortex sits above data services, below AI tools; adapters normalize into unified format

**Impact:** Medium. Correct stance: federates Claude Code memory/, Cursor rules, Rulebook, adds cross-tool retrieval.

---

## F-016: The Dependency Architecture Is Clean — 10 Crates in a Strict DAG

**Evidence:**
- `crates/` directory — 10 crates: core, storage, build, health, workers, api, pre-thinking, adapter-claude-code, mcp-server, cli
- **[#28]** `dag.md` — dependency graph of 22 specs with blast radius analysis
- Dependency chain: `core` (zero deps) → `storage` → `workers` → `api` → `pre-thinking` → `adapter` / `mcp-server`
- [#21] `cortex-build` has zero runtime dependencies; [#20] `cortex-health` is optional via feature flags

**Impact:** Low. Clean architecture; no circular dependencies.

---

## Summary Matrix

| Finding | Area | Impact | Confidence | Status |
|---|---|---|---|---|
| F-001 | Pipeline implementation | High | High | Complete |
| F-002 | Intent routing | Medium | High | Complete |
| F-003 | Hybrid retrieval | High | High | Complete |
| F-004 | Topic cards (living synthesis) | High | High | Complete |
| F-005 | Consolidations | Medium-High | High | Complete |
| F-006 | Budget enforcement | Medium | High | Complete |
| F-007 | Scope derivation | Medium | High | Complete |
| F-008 | Fail-open semantics | High | High | Complete |
| F-009 | Deterministic formatting | Medium | High | Complete |
| F-010 | QueryFn decoupling | Medium | High | Complete |
| F-011 | Knowledge + Learnings | Medium-High | Medium | Partial |
| F-012 | In-session capture bridge | Medium | Medium | Partial |
| F-013 | Graph edge classes | Medium | High | Complete |
| F-014 | Observability (11 phases) | Medium-High | High | Complete |
| F-015 | Ecosystem stance | Medium | High | Complete |
| F-016 | Dependency DAG | Low | High | Complete |
