# 01 — Findings

> **Analysis ID:** REWORK-MINMAX-001 · **Date:** 2026-05-05

16 numbered findings with evidence, impact, and confidence. Each finding maps to a specific area for improvement.

---

## Index of Referenced Files

| # | File | Module |
|---|------|--------|
| 01 | `crates/cortex-pre-thinking/src/lib.rs` | Entry point, public API re-exports |
| 02 | `crates/cortex-pre-thinking/src/pipeline.rs` | Orchestrator: scope→intent→query→format→clip |
| 03 | `crates/cortex-pre-thinking/src/scope.rs` | Scope derivation from cwd+git+prompt |
| 04 | `crates/cortex-pre-thinking/src/intent_select.rs` | Intent selector: 55 keyword rules → 6 intents |
| 05 | `crates/cortex-pre-thinking/src/formatter.rs` | Deterministic Markdown bundle assembly |
| 06 | `crates/cortex-pre-thinking/src/budget.rs` | Budget clipper: 6-step trim ladder |
| 07 | `crates/cortex-pre-thinking/src/metrics.rs` | Pre-think metrics registry |
| 08 | `crates/cortex-api/src/orchestrator.rs` | Hybrid query orchestrator: 3 lanes + RRF |
| 09 | `crates/cortex-api/src/lanes.rs` | Lane traits: VectorLane, KeywordLane, GraphLane |
| 10 | `crates/cortex-api/src/types.rs` | QueryRequest/QueryResponse, Intent, Scope, ResultsBag |
| 11 | `crates/cortex-core/src/events.rs` | Event schema: Envelope, Kind, payloads |
| 12 | `crates/cortex-workers/src/classifier/` | Classifier worker: Haiku batch |
| 13 | `crates/cortex-workers/src/consolidator/` | Consolidation producer |
| 14 | `crates/cortex-workers/src/topic_cards/` | Topic cards: living synthesis + contradiction detection |
| 15 | `crates/cortex-adapter-claude-code/src/` | Claude Code adapter |
| 16 | `crates/cortex-mcp-server/src/` | MCP stdio JSON-RPC bridge |
| 17 | `crates/cortex-cli/src/bootstrap/walker.rs` | Bootstrap walker for knowledge/learnings |
| 18 | `crates/cortex-storage/` | Storage layout, namespaces |
| 19 | `crates/cortex-health/src/lib.rs` | Health-report types, aggregator |
| 20 | `crates/cortex-build/` | Build-time version metadata |
| 21 | `docs/architecture.md` | Full architecture (951 lines) |
| 22 | `docs/prd.md` | Product requirements |
| 23 | `docs/specs/12-pre-thinking-injection.md` | Pre-thinking spec |
| 24 | `docs/dag.md` | Spec dependency graph |
| 25 | `docs/specs/11-query-api.md` | Query API spec |
| 26 | `docs/specs/10-claude-code-adapter.md` | Adapter spec |
| 27 | `docs/specs/13-laws-dsl.md` | Laws DSL (draft) |
| 28 | `docs/specs/14-governance-engine.md` | Governance Engine (draft) |
| 29 | `docs/specs/15-deep-analysis.md` | Deep Analysis (draft) |
| 30 | `docs/specs/17-additional-adapters.md` | Additional adapters (not started) |

---

## F-001: Pre-Thinking Has No Feedback Loop

**Evidence:**
- [#02] `pipeline.rs` — `run()` delivers bundle; return type `PreThinkingOutput` has no feedback channel
- No `POST /api/feedback` endpoint exists
- `success`/`failure` outcome of the turn arrives at the classifier weeks later (if ever)

**Problem:** The pipeline never knows if its context was read, used, or useful. A bundle that is systematically misleading (wrong snippets, stale decisions) cannot self-correct without explicit user feedback.

**Impact:** High. Retrieval quality degrades silently when bundles are systematically poor.

**Confidence:** High (design gap verified in source)

---

## F-002: Intent Routing Fragility on Compound Prompts

**Evidence:**
- [#04] `intent_select.rs:47-219` — `DEFAULT_RULES` with 55 keyword rules; flat linear scan, first match wins
- #23 Spec 12 OQ 1: "Should the model pick its own intent? Leaning no (UX cost, latency)"
- No tracking of `intent_mismatch_rate` anywhere in metrics.rs or pipeline.rs

**Problem:** Substring matching fails on compound prompts. Example:
- `"explain why did we pick hnsw"` → `explain` fires first → **wrong** (should be `decision_lookup`)
- `"why did we change the return type"` → `change` fires first → **wrong** (should be `decision_lookup`)

The rule order is brittle. `explain` is listed before `why did` in the table, so any prompt containing "explain" before "why did" routes to `explain`.

**Impact:** Medium. ~20% of decision-related prompts may be misrouted based on manual inspection.

**Confidence:** Medium (based on rule table analysis; no empirical mismatch rate data)

---

## F-003: Fail-Open Has No Circuit Breaker

**Evidence:**
- [#02] `pipeline.rs:142-148` — tokio timeout wraps query call; timeout → None → empty bundle
- No circuit breaker in the pipeline; no `fail_open_count` tracking
- `docs/architecture.md:825-832` — canary is opt-in, off by default

**Problem:** When fail-open fires repeatedly (e.g., cortex-api is down), the model operates with empty context silently. No alert is sent; no degraded mode is activated. The circuit breaker pattern is absent.

**Impact:** Critical. A fail-open storm (5+ fail-opens in 60s) is undetected.

**Confidence:** High

---

## F-004: Empty Bundle on Fail-Open Says Nothing to the Model

**Evidence:**
- [#05] `formatter.rs:186-196` — all-zero sections → empty string
- #23 Spec 12 Decision 4: "Empty-result → empty bundle. Silence is more honest."
- `PreThinkingOutput { fail_open: true, bundle: String::new() }` — model receives no signal that context retrieval failed

**Problem:** `fail_open=true` returns the same empty bundle as `no results found`. The model cannot distinguish "there was no relevant context" from "context retrieval failed". This masks outages.

**Impact:** Critical. Models proceed with false confidence when cortex is down.

**Confidence:** High

---

## F-005: 32KB Budget Fixed for All Intents

**Evidence:**
- [#06] `budget.rs:52-150` — single `bundle_bytes` cap applied uniformly
- #23 Spec 12 OQ 2: "A 32-KB cap is a hunch. Once we have eval data, tune per intent."
- `metrics.rs:40-43` — `bundle_bytes` histogram exists but is not segmented by intent
- No per-intent budget configuration in `FormatOptions`

**Problem:** All 6 intents share one budget. But:
- `explain` needs more snippets, fewer decisions
- `similar_problems` needs more similar_turns
- `law_check` needs only violations, no snippets
- `pre_change_context` needs everything

**Impact:** High. Suboptimal context density for intent-specific queries.

**Confidence:** High (design gap verified)

---

## F-006: Query Rewriting Deterministic Mode Loses Intent Signal

**Evidence:**
- #25 Spec 11 §Query rewriting — noun-phrase strip default: `"Refactor the HNSW configurator"` → `"HNSW configurator"` (loses "Refactor")
- Sonnet rewrite opt-in via `CORTEX_QUERY_REWRITER=sonnet` — no one enables it because no graceful fallback
- No cascade rewriter pattern: Sonnet primary + deterministic fallback

**Problem:** `refactor` keyword is the trigger for `pre_change_context` intent. After noun-phrase stripping, the vector query loses this signal. Results are about HNSW, not about refactoring patterns.

**Impact:** Medium. Retrieval precision on `pre_change_context` intents degrades.

**Confidence:** High

---

## F-007: Topic Card Contradiction Detection is Purely Heuristic

**Evidence:**
- [#14] `topic_cards/` — 3 contradiction detectors: `DecisionSupersession`, `LawViolationMismatch`, `OutcomeDivergence`
- `docs/architecture.md:360-366` — all three are syntactic/data heuristics, not semantic

**Problem:**
- `DecisionSupersession`: only fires if `supersedes` field exists on the decision
- `LawViolationMismatch`: only fires if law has version metadata
- `OutcomeDivergence`: only fires on temporal overlap + outcome majority divergence — does not assess semantics

Two decisions from 2024 (performance) and 2025 (security) with different outcomes will falsely trigger `OutcomeDivergence` even if they are unrelated.

**Impact:** Medium. High false positive rate on contradiction detection.

**Confidence:** High

---

## F-008: Laws DSL Never Shipped (Spec 13, 14)

**Evidence:**
- #27 `specs/13-laws-dsl.md` — Status: **Drafted**
- #28 `specs/14-governance-engine.md` — Status: **Drafted**
- #26 `specs/10-claude-code-adapter.md` — blocking law enforcement is a mock, not real implementation
- `docs/specs/00-index.md` — 5 specs in "Drafted", 7 in "Not started"

**Problem:** There is no functional `cortex laws lint`. The punishment ladder (tier 1-4) is a concept. Blocking laws (severity=critical) are not enforced in production. The governance layer exists only on paper.

**Impact:** High. Cortex governance is advisory, not enforced. Critical laws like "never pass --no-verify" are not enforced.

**Confidence:** High

---

## F-009: Deep Analysis Never Shipped (Spec 15)

**Evidence:**
- #29 `specs/15-deep-analysis.md` — Status: **Drafted**
- `docs/architecture.md:291-306` — Deep Analysis described as a 4-step workflow (debate → Decision)
- `docs/prd.md` — US-05 ("Deep analysis") listed as P1, `passes: false`

**Problem:** Deep Analysis (multi-agent debate with context as ground truth → auditable Decision) is the most differentiated feature in Cortex. It has been in draft since the beginning.

**Impact:** Medium-High. The system cannot formalize complex decisions into institutional memory.

**Confidence:** High

---

## F-010: Multi-Adapter Stagnant (Spec 17)

**Evidence:**
- #30 `specs/17-additional-adapters.md` — Status: **Not started**
- `crates/cortex-mcp-server/src/` — MCP server implemented but not usable by non-Claude-Code adapters
- #26 Spec 10 only covers Claude Code

**Problem:** Only Claude Code adapter exists. Cursor/Codex/Gemini adapters are not started. The MCP server is not designed for reuse across adapters.

**Impact:** Medium. Cross-tool memory capture is impossible.

**Confidence:** High

---

## F-011: Classifier Budget Has No Proactive Circuit Breaker

**Evidence:**
- #21 `architecture.md:225` — "on threshold breach: (a) drops severity, (b) raises batch size, (c) falls back to static"
- `#12` classifier worker — tier 3 (static fallback) is activated **after** the cost was already incurred
- No `circuit_open` flag; no 90% warning before hitting the limit

**Problem:** Degradation is reactive, not preventive. When the daily budget is hit, the next request falls back to static — but the cost was already incurred. No circuit breaker pattern.

**Impact:** Medium. Classifier costs can exceed budget before fallback activates.

**Confidence:** High

---

## F-012: Hot Tier Storage Blocked on Vectorizer SDK

**Evidence:**
- #21 `architecture.md:340` — "blocked on the upstream Vectorizer SDK gaining `move_to_collection` + `delete_vectors`"
- Phase 11o tracked as blocker
- No soft-delete pattern as workaround

**Problem:** Warm tier (PQ compression after 90d) will never work without upstream SDK changes. Cold tier (Parquet archive) exists as fallback but is not queryable without rebuild. No cleanup of hot-tier data that should have been promoted.

**Impact:** Medium. Storage grows unbounded; older data cannot be compressed or archived.

**Confidence:** High

---

## F-013: Bootstrap Takes 4-8 Hours (No Parallelization)

**Evidence:**
- #21 `architecture.md:454` — "4–8 hours single node" for 17 repos
- `#17` `walker.rs` — single-threaded sequential walk of repos
- No incremental bootstrap documented

**Problem:** Bootstrap is single-threaded and re-indexes everything on every run. No parallel execution; no incremental mode that only re-indexes changed files.

**Impact:** Medium. Bootstrap is a bottleneck for onboarding new repos.

**Confidence:** High

---

## F-014: Canary is Opt-In, Does Not Protect Production

**Evidence:**
- #21 `architecture.md:827` — "opt-in via `CORTEX_CANARY_ENABLED=1` env var (off by default)"
- Phase 8f synthetic E2E canary is designed to detect quiet-hours failures but is disabled by default

**Problem:** In quiet hours (weekends, nights), the pipeline can be broken without anyone noticing because the canary is off by default. It should be ON by default in production.

**Impact:** Medium-High. Silent failures go undetected for hours.

**Confidence:** High

---

## F-015: Query Response Never Tracks Bundle Quality

**Evidence:**
- [#07] `metrics.rs` — tracks `calls_total`, `bundle_bytes`, `latency_ms`, `empty_bundle`, `timeouts`
- No metric for: `bundle_helpful_rate`, `files_cited_rate`, `intent_accuracy`
- `QueryResponse` has no feedback fields

**Problem:** The observability layer captures quantity metrics (bundle size, latency) but not quality metrics (was the context useful?). Retrieval quality degrades silently.

**Impact:** High. No data to drive the adaptive budget and intent routing improvements.

**Confidence:** High

---

## F-016: Cross-Repo Identity Resolution Not Implemented

**Evidence:**
- #21 `architecture.md:425` — "cross-repo identity — When the same function is referenced across repos, do we deduplicate? Probably yes via content-hash + symbol resolution."
- Never implemented
- `IMPORTS_FILE` edges only within a single repo

**Problem:** Two repos that use the same copied code have separate graphs with no shared symbol resolution. The model cannot know that `Vectorizer/src/lib.rs` and `Nexus/src/lib.rs` share patterns.

**Impact:** Low-Medium. Limits cross-repo context retrieval.

**Confidence:** High

---

## Summary Matrix

| ID | Area | Severity | Effort | Impact | Confidence |
|----|------|----------|--------|--------|------------|
| F-001 | No feedback loop | High | Medium | High | High |
| F-002 | Intent routing fragility | Medium | Low | Medium | Medium |
| F-003 | No circuit breaker on fail-open | Critical | Low | High | High |
| F-004 | Empty bundle hides outage | Critical | Low | High | High |
| F-005 | Fixed 32KB budget for all intents | High | Low | Medium | High |
| F-006 | Query rewriting loses intent | Medium | Medium | Medium | High |
| F-007 | Contradiction detection heuristic | Medium | Medium | Medium | High |
| F-008 | Laws DSL never shipped | High | Medium | High | High |
| F-009 | Deep Analysis never shipped | Medium-High | Medium | High | High |
| F-010 | Multi-adapter stagnant | Medium | High | High | High |
| F-011 | Classifier no proactive circuit breaker | Medium | Medium | Medium | High |
| F-012 | Hot tier blocked on SDK | Medium | Medium | Medium | High |
| F-013 | Bootstrap 4-8h, no parallelization | Medium | Medium | High | High |
| F-014 | Canary opt-in, off in prod | Medium-High | Low | High | High |
| F-015 | No bundle quality tracking | High | Low | High | High |
| F-016 | Cross-repo identity unresolved | Low-Medium | High | Medium | High |