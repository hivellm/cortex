> **SUPERSEDED 2026-10-16** by [05-joint-decision.md](05-joint-decision.md) and task `phase34_daily-agent-memory-recovery`. Astra's review of this plan is in [07-astra-review-of-superseded-claude-plan.md](07-astra-review-of-superseded-claude-plan.md). F-030 (root cause of the capture outage) remains valid evidence.

# Execution plan: make Cortex useful day to day

Date: 2026-10-16. Owners: Claude session `cortex-73` (coordination, implementation)
and Codex `gpt-6-astra` (review gate per section, Jev owner). The operator's
direction, given directly to Claude in this session: "make Cortex actually
useful together with Astra". This plan consolidates `01-*`–`04-*`.

## Root cause of the two-month silence (new, verified 2026-10-16)

**F-030 — Capture died with an unsupervised host process and nothing noticed.**

- Every hook writes to the named pipe `\\.\pipe\cortex-adapter-claude`.
  That pipe exists only while the host daemon `cortex-adapter-claude.exe` runs.
- The daemon's autostart script,
  `scripts/setup/install-adapter-autostart.ps1`, registers a scheduled task
  through `schtasks`. On this host that call needs admin and was denied. The
  consolidator received a no-admin Startup-folder `.vbs` instead. The adapter
  never did.
- The adapter therefore ran only from a manually started process. Its log ends
  2026-07-06T14:27Z. Since then `cortex-hook` gets no pipe, prints `{}` and
  exits 0. That is correct fail-open behavior, and it made the outage silent.
- With no adapter, nothing reaches ingestion, Synap or the classifier. The
  archive tailer only writes parquet (F-001), so classification stopped too.
- Installed binaries were also stale: `cortex-adapter-claude.exe` and
  `cortex-hook.exe` date from May 25–26, while the adapter crate has 15 later
  commits. `cortex-mcp-server.exe` dates from Jun 6, with 8 later commits. The
  phase30 SessionStart continuity work never ran live.

Lesson: **a fail-open hook needs a loud health channel**. Otherwise every
outage looks like "Cortex is useless" rather than "Cortex is down".

## Success definition

One seeded end-to-end test (`04-*` §6) passes on the real host install:

1. A correction is recorded in repo R by agent A.
2. In a fresh session, agent B's ordinary prompt in repo R receives that exact
   lesson ID with origin. Lessons from repo S and distractors are absent.
3. A superseding decision replaces it on the very next identical prompt.
4. The result survives daemon restarts.
5. It runs within the production budgets of 600 ms and the byte cap.

The operator should also *see* Cortex working: captured counts rising, and an
alarm when capture stops.

## Sections, in dependency order

Rulebook task: `phase34_daily-memory-recovery`. Checklist order is binding
(LAW-CORTEX-001). Astra reviews each section's diff before the next starts.

1. **Capture alive and visible.**
   - Reinstall current binaries.
   - No-admin adapter autostart, using the same `.vbs` pattern as the
     consolidator.
   - A silent-death alarm: the hook tells the session once when the daemon is
     unreachable, and `/v1/health` degrades when no event has been ingested
     for a window.
   - A live trace from prompt through ingestion, Synap, classifier, Meili and
     vector.
2. **Stop harm.**
   - Disable `retention.memory_consolidate`.
   - Restore the ten merged `consolidated_*.md` auto-memory files into
     one-fact files, with backups.
   - Make SessionStart synchronous in the live global hook and in the packaged
     plugin (F-007).
3. **Lessons delivered.**
   - Durable feedback store (F-004).
   - Capture provenance from client context (F-005).
   - An active-lessons bundle section sourced from structured state, with
     similarity used for ordering only (F-003).
   - Corpus-revision cache key (F-006).
   - Persisted delivery evidence per prompt.
4. **Jev at capture time.** Bounded typed judgments on lesson and decision
   candidates only: durable or not, scope, and contradiction with active
   lessons. F-016 egress rules apply. The all-events overlay from `02-*` is
   deferred.
5. **Footprint.**
   - Stop the 30 s full archive reload into the keyword lane (F-002).
   - Slow the claude-archive 1 s full-tree walk.
   - Make the GPU reranker opt-in.
   - Remeasure idle CPU and memory.
6. **Acceptance and tail.** The seeded end-to-end test above, docs, and the
   full quality gate.

## Out of scope for this task

Governance and law blocking (phase31 is another agent's work in progress),
graph projection, lane-weight learning, new adapters, and dashboard features.
