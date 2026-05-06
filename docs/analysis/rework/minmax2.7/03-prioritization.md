# 03 — Prioritization Matrix

> **Analysis ID:** REWORK-MINMAX-001 · **Date:** 2026-05-05

Full P0-P4 prioritization with finding IDs, recommendation references, effort, impact, and confidence.

---

## Priority P0 — Ship Now

| # | Finding | Recommendation | Effort | Impact | Confidence | Verification |
|---|---|---|---|---|---|---|
| F-003 | No circuit breaker on fail-open | R-001 | Low | High | High | `scripts/doctor/health.bat` shows degraded when breaker trips |
| F-004 | Empty bundle hides outage | R-002 | Low | High | High | Bundle contains `<!-- cortex: timeout -->` on fail-open; unit test passes |

**Gate:** Both must be green before any feature work lands. They prevent silent data quality disasters.

---

## Priority P1 — Next Sprint

| # | Finding | Recommendation | Effort | Impact | Confidence | Verification |
|---|---|---|---|---|---|---|
| F-002 | Intent routing fragility | R-003 | Low | Medium | Medium | `cortex-ops intent-stats` shows mismatch rate per intent |
| F-005 | Fixed 32KB budget for all intents | R-004 | Low | Medium | High | Metrics histogram segmented by intent; per-intent budget distribution visible |
| F-008 | Laws DSL never shipped | R-007 | Medium | High | High | `cortex laws lint laws/*.md` → 0. PreToolUse blocks LAW-007. |
| F-014 | Canary opt-in, off in prod | R-010 | Low | High | High | Canary runs in prod every 60s; 2 failures → alert |
| F-009 | Deep Analysis never shipped | R-008 | Medium | High | High | `cortex analysis start "X"` → Decision record indexed in Nexus |

**Gate:** All P0 green + these P1 items complete before Sprint 3 begins.

---

## Priority P2 — Next Quarter

| # | Finding | Recommendation | Effort | Impact | Confidence | Verification |
|---|---|---|---|---|---|---|
| F-001 | No feedback loop | R-009 | Low | High | High | `POST /api/feedback` records bundle quality; dashboard shows quality score |
| F-006 | Query rewriting loses intent | R-005 | Medium | Medium | High | Sonnet-cached calls skip API; cascade fallback never errors |
| F-007 | Contradiction detection heuristic | R-006 | Medium | Medium | High | Unit test: known contradictory pairs return true; consistent pairs return false |
| F-010 | Multi-adapter stagnant | R-012 | High | High | High | New Cursor adapter built in 1 week using `cortex-adapter-core` |
| F-015 | No bundle quality tracking | R-009 | Low | High | High | Dashboard shows per-intent helpful_rate and files_cited_rate |
| F-016 | Cross-repo identity unresolved | R-011 | High | Medium | High | `cortex graph query --symbol sha256:abc` returns all repos |

---

## Priority P3 — Backlog

| # | Finding | Recommendation | Effort | Impact | Confidence | Verification |
|---|---|---|---|---|---|---|
| F-011 | Classifier no proactive circuit breaker | R-013 | Medium | Medium | High | 90% budget warning fires; circuit_pre_open triggers tier-3 fallback |
| F-012 | Hot tier blocked on SDK | R-014 | Medium | Medium | High | Query filter `age > 90d` excludes hot-tier vectors; PQ compression works |
| F-013 | Bootstrap 4-8h, no parallelization | R-015 | Medium | High | High | Bootstrap of 17 repos completes in <60 min with 16 cores |
| F-001 | Implicit feedback via tool overlap | R-016 | Medium | Medium | Medium | Jaccard overlap metric visible in bundle feedback dashboard |

---

## Summary: By Priority Level

| Priority | Count | High-Effort Items |
|---|---|---|
| P0 | 2 | None |
| P1 | 5 | None |
| P2 | 6 | R-012 (shared adapter core, High) |
| P3 | 4 | R-011 (symbol registry, High), R-015 (parallel bootstrap, Medium) |

---

## What NOT to Do

| Don't do this | Reason |
|---|---|
| Start R-007 + R-008 + R-010 in parallel with Phase A trait work | Same org risk: patches crowd out foundations. Prior rework analysis (REWORK-001 §Phase A) must land first. |
| Enable Sonnet query rewriter (R-005) without cache + cascade fallback | Sonnet downtime would break pre-thinking entirely. Cascade is required. |
| Ship new MCP tools / new lanes during Sprint 1-2 | phase11v-style feature work on the lane monolith re-entrenches the defect. Feature freeze during P0-P1. |
| Create new `HashMap<String, String>` lane extras | The `ProjectedHit` typed replacement (ADR-011 from prior rework) must land first. |

---

## Confidence Calibration

| Claim | Confidence | What would change my mind |
|---|---|---|
| P0 items are correct must-fixes | High | A second agent disagreeing that circuit breaker is necessary |
| P1 items are correct near-term priorities | High | Evidence that Laws DSL v1 or Deep Analysis MVP are blocked on upstream |
| No parallel features during P0-P1 | Medium | If external dependency requires shipping a specific MCP tool |
| Cascade rewriter is the right query rewrite approach | Medium-High | If Haiku-only rewrite shows >90% quality score on eval set |
| Symbol registry is the right cross-repo approach | Medium | If ADR on shared artifact edges (SHARED_ARTIFACT) shows it is insufficient |