# Cortex — Rework Analysis (minmax2.7)

> **Analysis ID:** REWORK-MINMAX-001 · **Model:** minmax2.7 · **Date:** 2026-05-05
> **Method:** Full codebase read + architecture audit + critical blind-spot analysis
> **Trigger:** Independent analysis of Cortex system — identifying flaws, gaps, and design issues

---

## TL;DR

Cortex has solid foundations (10 crates, clean DAG, fail-open semantics) but suffers from **5 systemic diseases**:

1. **Pre-thinking has no feedback loop** — bundle delivered without knowing if it was useful
2. **Fail-open is being abused** — silent degradation goes undetected; no circuit breaker
3. **Intent routing is fragile** — keyword matching breaks on compound prompts; no tracking of mismatch rate
4. **Governance specs never shipped** — Laws DSL (spec 13) and Governance Engine (spec 14) are drafts for months
5. **Multi-adapter is stagnant** — only Claude Code exists; Cursor/Codex/Gemini are "Not started"

The system works on the happy path but degrades silently on common edge cases. **Recommended path: targeted fixes + 2-3 ADRs**, NOT a rewrite.

---

## Documents

| # | File | Purpose |
|---|------|---------|
| 01 | [01-findings.md](./01-findings.md) | 16 numbered findings with evidence and impact |
| 02 | [02-recommendations.md](./02-recommendations.md) | Actionable fixes per finding with effort/impact |
| 03 | [03-prioritization.md](./03-prioritization.md) | Full P0-P4 prioritization matrix |

---

## Cross-cutting synthesis

### Root cause: 3 decisions made without data

1. **Spec 12 OQ 1** (intent routing): "We graduate to a model only if offline eval shows >5% precision gap." — but there is no tracking of `intent_mismatch_rate`. The decision was made without ever measuring the baseline.

2. **Spec 12 OQ 2** (adaptive budgets): "A 32-KB cap is a hunch." — the cap was never tuned per intent. No bundle size distribution is tracked per intent.

3. **Spec 12 Decision 4** (empty bundle): "Silence is more honest than 'No relevant context found.'" — this is correct behavior for true empty results, but is wrong when the empty bundle is caused by Cortex being broken.

### The fail-open cycle

```
timeout/error → fail_open=true → empty bundle → model proceeds without context
     ↑                                                              │
     └──────────────── no circuit breaker, no alert ────────────────┘
```

The design is correct in principle. But without a circuit breaker, repeated fail-open events are a silent data quality disaster that nobody notices.

### Governance debt is the largest unaddressed risk

Laws DSL and Governance Engine have been "Drafted" since the spec index was created. The blocking-law enforcement (PreToolUse) exists as a mock in spec 10. Until real laws are in production, Cortex governance is advisory, not enforced.

---

## Agent attribution

| Doc | Model | Tokens |
|-----|-------|--------|
| 01-findings | minmax2.7 | ~120,000 |
| 02-recommendations | minmax2.7 | ~80,000 |
| 03-prioritization | minmax2.7 | ~40,000 |

---

## How to read this set

If you read only one file: [02-recommendations.md](./02-recommendations.md) — the P0 items that can ship immediately.

If you have 20 minutes:
1. This README §TL;DR + §Cross-cutting synthesis
2. [01-findings.md](./01-findings.md) §F-001, F-003, F-008 (the most critical findings)
3. [02-recommendations.md](./02-recommendations.md) §P0 items
4. [03-prioritization.md](./03-prioritization.md) §P1 items

If you have an hour: read all three docs in order.