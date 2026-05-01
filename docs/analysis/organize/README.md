# Cortex as persistent agent memory — corpus + relevance plan

> **Date:** 2026-05-01
> **Owners:** Andre Ferreira + claude-opus-4-7
> **Trigger:** "indexar memórias do Claude Code (`C:\Users\Bolado\.claude\projects`) e
> organizar tudo para que os resultados sejam cada vez mais relevantes"
> **Status:** 🟢 Analysis complete; implementation tracked under
> [`.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/`](../../../.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/)

This analysis answers two questions:

1. **What can we ingest?** — Inventory of every conversation, todo, plan,
   and config artifact under the user's `~/.claude/` tree (and the
   parallel `~/.codex/` corpus), keyed against Cortex's canonical
   `Envelope` schema so we know exactly which kinds to emit.
2. **How do we make it relevant?** — Five orthogonal retrieval axes
   (temporal recency, project scope, author/model, session cohesion,
   outcome signal), mapped to the data we already have, the data we
   need to start capturing, and the lane / RRF / pre-thinking changes
   needed to surface them at query time.

The output is a six-phase implementation plan (see
[05-implementation-plan.md](./05-implementation-plan.md)) that turns
~4.5 GB of conversation transcripts into a queryable institutional
memory the agent reads on every turn.

## Files

| File | Purpose |
|---|---|
| [01-corpus-inventory.md](./01-corpus-inventory.md) | Full layout of `~/.claude/projects/`, `~/.claude/*` sidecars, `~/.codex/`, JSONL record schema, fixture paths |
| [02-current-pipeline.md](./02-current-pipeline.md) | End-to-end ingestion flow (Envelope → Synap → workers → Vectorizer / Meili / Nexus → query API → pre-thinking), with file:line citations |
| [03-coverage-gaps.md](./03-coverage-gaps.md) | What Cortex captures today vs. what it was designed to capture; per-family doc counts; backend health |
| [04-relevance-axes.md](./04-relevance-axes.md) | Five-axis framework (recency, scope, author, session, outcome) with the schema fields, lane changes, and RRF tweaks needed for each |
| [05-implementation-plan.md](./05-implementation-plan.md) | Six-phase build plan: `cortex-claude-archive` crate, family additions, watcher, query-API filters, RRF tuning, pre-thinking wiring |
| [findings.json](./findings.json) | Machine-readable summary (counts, paths, axes) for the task generator and CI fixtures |

## TL;DR

- **Corpus is huge and well-structured.** 9 835 JSONL session
  transcripts, 4.5 GB total, across 31 project directories. JSONL
  records are typed (`user`, `assistant`, `attachment`, `system`,
  `file-history-snapshot`, `last-prompt`, `queue-operation`) with
  stable fields (`sessionId`, `parentUuid`, `cwd`, `gitBranch`,
  `model`, `version`, `entrypoint`, ISO-8601 `timestamp`). Sibling
  data: `history.jsonl` (660 KB global command history), `todos/`
  (1.6 MB per-agent task lists), `plans/` (5 markdown plans),
  `shell-snapshots/` (7 MB bash envs). Parallel `~/.codex/` corpus
  exists but dormant.

- **Schema fits.** Every JSONL record collapses cleanly into Cortex's
  existing `Envelope` kinds: user+assistant pair → `Kind::Turn`,
  `tool_use`/`tool_result` → `Kind::ToolCall`, sub-agent invocations
  → `Kind::AgentCall`. Sidecar artifacts (todos, plans, settings)
  ingest as `Kind::Memory` or `Kind::Artifact`. **No schema changes
  needed for the minimum-viable path.**

- **Bootstrap doesn't fit.** `cortex-bootstrap` walks git repos, not
  flat directories. We need a sibling crate `cortex-claude-archive`
  that walks `~/.claude/projects/` and emits canonical `Envelope`
  JSON onto `cortex.events.bootstrap` — reusing every downstream
  worker without modification.

- **Relevance gap is the real blocker.** We can dump 4.5 GB into
  Meili and call it a day, but recall ≠ relevance. The axes that
  matter — recency decay, same-session boost, model attribution,
  outcome signal — are not exposed in the query API or applied in
  RRF fusion today. Closing them is roughly the same engineering
  cost as the ingestor itself.

- **Coverage warn is independent.** Vectorizer 4/144, Meili 29/144 —
  driven by missing bootstrap runs across the 16 indexed repos, not
  by the Claude archive. Phase 11h (already filed) closes that gap.
  Phase 11i (this analysis) builds on top.

## Why this matters for the agent loop

Today the agent reads `MEMORY.md` (8 lines), the Cortex pre-thinking
bundle (5 generic snippets), and whatever the user types. Every
session starts from near-zero context. The 798 sessions of Cortex
work and 9 037 sessions across 30 other projects are sitting on
disk, unread.

After this work, the pre-thinking bundle for a question like
*"how did we decide to use Meili instead of Lexum?"* will return:

- the original conversation where the call was made (turn similarity)
- every subsequent turn that referenced the decision (graph traversal)
- any law or ADR that codified it (governance lane)
- the commits that landed the choice (git history adapter)

…ranked by recency × project-scope × outcome, all under the 32 KiB
pre-thinking budget enforced by `phase11c`'s clipper.

That's the bar. The plan in
[05-implementation-plan.md](./05-implementation-plan.md) is what gets
us there.
