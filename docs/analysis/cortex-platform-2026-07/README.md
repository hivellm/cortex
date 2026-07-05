# Cortex platform re-scope — 2026-07-05 live audit + roadmap

> **Trigger:** 2026-07-05 platform review. **Scope**: Static code/spec audit + live Docker-stack verification (`cargo check`, `cargo test`, `cargo audit`, 12-service smoke test, doctor scripts). **Finding**: Cortex's June 2026 roadmap had drifted toward a general company knowledge platform (Person/Project/Case entities, Slack/Linear/Notion connectors, multi-tenant infrastructure). This analysis **re-narrows the scope** to one explicit goal: making Cortex **indispensable and load-bearing for Claude's own AI-assisted implementation work on the HiveLLM ecosystem** — better retrieval, better agent workflows, better observability, better memory/continuity — rather than a broad horizontal platform.

---

## Re-scope statement

This analysis **supersedes** [`docs/analysis/cortex/11-platform-vision-analysis.md`](../cortex/11-platform-vision-analysis.md) and [`docs/analysis/cortex/12-live-audit-2026-06-09.md`](../cortex/12-live-audit-2026-06-09.md). Both are marked superseded separately (do not edit them directly); this document is the new canonical narrative.

The 12 Rulebook tasks created from this analysis's roadmap (`phase28_live-testing-bugfixes` through `phase33_adapter-opencode-cursor`) are the **canonical tracked backlog** — this document provides narrative rationale and discovery context behind them.

---

## Files in this analysis

1. **[`findings.md`](./findings.md)** — 23 numbered findings (F-001 through F-023) from both static audit and live Docker-stack testing, organized by axis (Foundation / Bugs / Retrieval / Agent workflows / Observability / Memory / Strategic re-scope). Each finding includes a confidence/verification note.

2. **[`execution-plan.md`](./execution-plan.md)** — Roadmap organized by improvement axis (Fix breakage, Better retrieval, Better agent workflows, Better observability, Better memory/continuity), mapping each item to its Rulebook phase28–33 task. Includes sequencing principle: fix verified breakage first, then measure, then unblock the biggest lever (graph projection), then deepen capability, then extend coverage.

---

## Key takeaways

**What's working:** Type-checker passes, 37 MCP tools registered, 43 specs written, Docker stack running 12 services continuously 8–13 days. **What's broken:** One test failure (chunker fallback), 2 high-severity RUSTSEC advisories (quinn-proto, rmcp), cortex-graph-worker stalled undetected for 8 days, doctor scripts broken on Windows. **What's missing:** Real golden-set retrieval eval gate (currently workflow_dispatch-only), semantic graph projection disabled (biggest single retrieval lever, estimated 28%→75% on 2-hop hit-rate), runtime MCP capability discovery, live governance enforcement loop, feedback→ranking circuit wired. **What's half-built:** Consolidation→next-session pre-thinking injection (exists in pieces, unproven end-to-end), adapter coverage (Claude-Code-only in practice).

---

## Reading guide

Start at [`findings.md`](./findings.md) for the detailed audit results, organized by subsystem. Jump to [`execution-plan.md`](./execution-plan.md) to see the corresponding Rulebook tasks and rollout sequencing. The analysis is **factual and ground-truth grounded**: every claim traces to a real command run, file read, or Docker container state captured today (2026-07-05, ~10:00 UTC).
