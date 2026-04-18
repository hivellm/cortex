# Cortex — Product Requirements Document

> **Version:** 0.1 · **Status:** Draft · **Owner:** HiveLLM core team · **Last updated:** 2026-04-17
> **Source of truth for "what" and "why".** For "how", see [`architecture.md`](architecture.md) and [`specs/`](specs/).

---

## 1. Summary

Cortex is the **cognitive substrate of the HiveLLM ecosystem**: a captured, classified, embedded, and governed memory of every meaningful AI interaction across our codebases. Before any AI model proposes a change, Cortex supplies it with the relevant prior decisions, past similar problems, and active laws. After the session, Cortex records what happened so the next session starts informed instead of blind.

## 2. Problem

Every `/clear` destroys context that was expensive to build. Across Claude Code, Cursor, Codex, Gemini, and Copilot sessions we lose:

- **What happened** — the conversation, tool calls, rationale.
- **What was decided** — and why one path was chosen over others.
- **What was learned** — patterns that worked, dead ends, recurring bugs.

Consequences: rework, contradicted decisions, accidental regressions of issues we already solved, and no institutional audit trail for AI-assisted changes.

## 3. Vision

An AI workflow that is **analytical rather than purely generative** — grounded in HiveLLM's own history, not just the model's training data. Every agent consults Cortex before acting; every decision is captured; every law violation is audited; every repo benefits from the institutional memory of every other repo.

## 4. Goals (v1)

| #  | Goal                                                                                                     |
|----|----------------------------------------------------------------------------------------------------------|
| G1 | Capture 100% of AI interactions (turns, tool calls, agent calls, memory ops, decisions) across every supported tool. |
| G2 | Classify, embed, and relate captured artifacts; redact secrets at the edge before anything leaves the host. |
| G3 | Hybrid retrieval API (semantic + keyword + graph) with P50 < 50 ms / P95 < 150 ms on cached hot paths.    |
| G4 | Force consultation of Cortex before non-trivial code changes via adapter hooks (PreToolUse / pre-commit). |
| G5 | Codify, enforce, and audit development laws; graduated punishment ladder; per-(model, repo) trust score.  |
| G6 | Operator dashboard: live timeline, decision trails, tool-usage analytics, law-violation reports.          |
| G7 | Reuse Vectorizer / Nexus / Synap / Rulebook; Cortex orchestrates, not reimplements.                        |

## 5. Non-Goals (v1)

- **NG1.** Not a new vector DB, graph DB, or search engine.
- **NG2.** Not a coding agent — Cortex informs the agents that code.
- **NG3.** Not a replacement for per-tool memory (`CLAUDE.md`, Cursor rules) — it federates them.
- **NG4.** Not a multi-tenant SaaS — single-org first; HivehubCloud later.

## 6. Target users

| Persona                   | Needs                                                                                          | Primary surfaces                                 |
|---------------------------|------------------------------------------------------------------------------------------------|--------------------------------------------------|
| **HiveLLM maintainer** (André & team) | Fewer repeated mistakes; auditable AI decisions; governance teeth.                   | Dashboard, `cortex` CLI, pre-thinking bundle     |
| **AI agent** (Claude Code, Cursor, Codex, Gemini) | Relevant prior context at prompt time; blocking guardrails for critical laws. | Pre-thinking injection, `PreToolUse` law check   |
| **Future HivehubCloud user** | Trust scores that drive model routing; portable governance across projects.                 | Trust feed API (Phase 5)                         |

## 7. User stories

Each story cites the spec(s) that define acceptance. The machine-readable Ralph-compatible form lives at the end (§12).

### US-01 — "Before-change context"
> **As** an AI agent about to edit a file, **I want** Cortex to inject the most relevant prior decisions, similar past turns, and active laws into my system prompt **so that** I don't repeat mistakes or contradict accepted decisions.
> **Specs:** [11](specs/11-query-api.md), [12](specs/12-pre-thinking-injection.md), [10](specs/10-claude-code-adapter.md).

### US-02 — "Blocking law enforcement"
> **As** a maintainer, **I want** critical laws (e.g. "never `--no-verify`") to block offending tool calls at the adapter layer **so that** governance is proactive, not post-hoc.
> **Specs:** [13](specs/13-laws-dsl.md), [14](specs/14-governance-engine.md).

### US-03 — "Observational law capture"
> **As** a maintainer, **I want** non-critical violations captured asynchronously with evidence **so that** I can review them without interrupting active sessions.
> **Specs:** [13](specs/13-laws-dsl.md), [14](specs/14-governance-engine.md).

### US-04 — "Decision register"
> **As** a maintainer, **I want** every formal Decision (ADR, OpenSpec proposal, analysis outcome) to be indexed, supersedable, and citable by future queries **so that** the "why" of past choices is never lost.
> **Specs:** [11](specs/11-query-api.md), [15](specs/15-deep-analysis.md), [16](specs/16-dashboard.md).

### US-05 — "Deep analysis"
> **As** a maintainer facing a hard question, **I want** to orchestrate a structured debate between 2–5 AI agents with prior context as ground truth, ending in a judged Decision **so that** stuck topics become institutional memory.
> **Specs:** [15](specs/15-deep-analysis.md), [11](specs/11-query-api.md).

### US-06 — "Backfill the existing corpus"
> **As** a maintainer, **I want** to index the ~17 existing HiveLLM repos (code, docs, commits, ADRs, memories, rules) before live capture starts **so that** day-1 retrieval is already useful.
> **Specs:** [09](specs/09-bootstrap-cli.md).

### US-07 — "Live capture across tools"
> **As** an agent using Claude Code / Cursor / Codex / Gemini, **I want** my prompts and tool calls captured uniformly **so that** my work is retrievable regardless of which tool I used.
> **Specs:** [10](specs/10-claude-code-adapter.md), [17](specs/17-additional-adapters.md).

### US-08 — "Operator visibility"
> **As** a maintainer, **I want** a dashboard with a live timeline, memory browser, decision register, law dashboard, analysis library, tool analytics, and graph explorer **so that** the whole substrate is observable.
> **Specs:** [16](specs/16-dashboard.md).

### US-09 — "Trust-based routing"
> **As** HivehubCloud, **I want** per-(model, repo) trust scores based on violations and decision fidelity **so that** I can prefer more trustworthy models for riskier scopes.
> **Specs:** [14](specs/14-governance-engine.md).

### US-10 — "Right to forget"
> **As** a maintainer, **I want** a session or repo's data to cascade-delete from every backend on request **so that** we meet privacy and compliance obligations.
> **Specs:** [02](specs/02-storage-layout.md), [14](specs/14-governance-engine.md) (future).

## 8. Functional requirements

Top-level FRs, each mapped to specs. The specs hold the testable Acceptance Criteria.

| FR   | Requirement                                                                                  | Specs                               |
|------|----------------------------------------------------------------------------------------------|-------------------------------------|
| FR-1 | Define a stable, versioned event envelope + per-kind schemas (JSON Schema).                   | [01](specs/01-event-schema.md)      |
| FR-2 | Persist events durably (Parquet archive) and route them through Synap streams.                | [02](specs/02-storage-layout.md), [04](specs/04-cortex-core.md) |
| FR-3 | Provision the local data-services stack (Vectorizer, Nexus, Synap, Meilisearch) via compose.   | [03](specs/03-local-stack.md)       |
| FR-4 | Ingestion router with typed validation + static redaction before publication.                  | [04](specs/04-cortex-core.md)       |
| FR-5 | Classify events via Claude Haiku (CLI default, SDK opt-in) with content-hash caching + budget. | [05](specs/05-classifier.md)        |
| FR-6 | Embed chunks via Vectorizer; Tree-sitter symbol-level for code, section-level for docs.         | [06](specs/06-embedder.md)          |
| FR-7 | Write graph nodes/edges to Nexus idempotently; session/turn/tool/artifact/decision/law model.  | [07](specs/07-graph-writer.md)      |
| FR-8 | Index events in Meilisearch with typo-tolerance, facets, ranking rules.                        | [08](specs/08-fulltext-indexer.md)  |
| FR-9 | Backfill the 17 existing HiveLLM repos end-to-end.                                             | [09](specs/09-bootstrap-cli.md)     |
| FR-10| Claude Code adapter: hooks + local daemon + sync law check + pre-thinking injection.           | [10](specs/10-claude-code-adapter.md) |
| FR-11| Hybrid query API (vector + keyword + graph) fused with RRF, MCP + HTTP bindings.               | [11](specs/11-query-api.md)         |
| FR-12| Pre-thinking bundle formatter with byte budget + fail-open semantics.                          | [12](specs/12-pre-thinking-injection.md) |
| FR-13| Laws DSL (Markdown + YAML frontmatter) + sandboxed Deno detectors + linter.                     | [13](specs/13-laws-dsl.md)          |
| FR-14| Governance engine: enforcement ladder, reminders, materialized-view detectors, trust score.    | [14](specs/14-governance-engine.md) |
| FR-15| Deep Analysis workflow with panel orchestration, judge modes, cost guardrails.                 | [15](specs/15-deep-analysis.md)     |
| FR-16| Dashboard SPA with 7 views, live SSE, minimal authoring.                                       | [16](specs/16-dashboard.md)         |
| FR-17| Adapters for Cursor, Codex, Gemini sharing `cortex-adapters/common/`.                           | [17](specs/17-additional-adapters.md) |

## 9. Non-functional requirements

### Performance

| Surface                              | P50     | P95      | Notes                              |
|--------------------------------------|---------|----------|------------------------------------|
| Pre-thinking query (cached)          | < 50 ms | < 150 ms | Hot path; spec 11                  |
| Pre-thinking query (cold)            | < 250 ms| < 500 ms |                                    |
| Classifier (Haiku, batch of 32)      | < 1.5 s | < 3.0 s  | Spec 05                            |
| Classifier (cached hit)              | < 5 ms  | < 15 ms  |                                    |
| Embed + persist (per event)          | < 200 ms| < 500 ms | Spec 06                            |
| Dashboard SSE end-to-end             | < 200 ms| < 500 ms | Spec 16                            |
| Blocking law check                    | —       | < 100 ms | Spec 13; under adapter hook budget |

Throughput target: **500 events/sec sustained, 2 000/sec burst**, single node.

### Privacy / security

- Static redactor runs **before** any event leaves the host (spec 04), and again on the read path (spec 11).
- Classifier payloads traverse the network to Anthropic only after redaction; a deployment flag routes to Expert / self-hosted when available.
- Single API key in v1; OIDC placeholder.
- Detector sandbox: no network, no filesystem, no env (spec 13).

### Availability

- Single-node v1. All data services (Vectorizer, Nexus, Synap, Meilisearch) are mission-critical; a degraded subset still returns partial results with `debug.errors.<lane>` (spec 11).
- Overflow WAL on the adapter (spec 10) makes capture durable through brief core outages.

### Observability

- Unified metric namespace `cortex.*` with counters + histograms for every worker and lane.
- Query audit stream records every retrieval.
- Structured JSON logs everywhere; Prometheus scrape endpoints on each service.

### Cost guardrails

- Haiku classifier: daily spend cap with 3-tier degradation (warn → degrade prompt → static fallback) (spec 05).
- Deep Analysis: per-run `--budget-usd` hard cap; truncates rounds before overshoot (spec 15).

## 10. Success metrics

### North-star

- **Pre-thinking consultation rate:** ≥ 95% of Claude Code prompts in opted-in repos result in a non-empty bundle within 600 ms.
- **Decision-adherence rate:** 30-day rolling share of turns that cite or respect active Decisions (target: ≥ 0.75 after Phase 2).

### Leading

| Metric                                                          | v1 target |
|-----------------------------------------------------------------|-----------|
| Event capture success rate (per adapter)                          | ≥ 99.9%   |
| Hot-path retrieval P95 (cached)                                  | < 150 ms  |
| Cold-path retrieval P95                                          | < 500 ms  |
| Blocking-law false-block rate                                    | < 1%      |
| Classifier cache-hit rate (post-bootstrap soak)                   | ≥ 60%    |
| Bootstrap of 17 repos end-to-end                                  | ≤ 8 h     |
| Dashboard Lighthouse a11y score                                  | ≥ 90      |

### Lagging

- Number of **Decisions** surfaced in pre-thinking bundles per week.
- Number of **Analyses** concluded per month.
- Trust-score variance per `(model, repo)` pair (should trend down as laws stabilize).

## 11. Assumptions, dependencies, risks

### Assumptions

- A Claude Code subscription with sufficient Haiku quota is available.
- Vectorizer, Nexus, Synap are production-ready for the v1 throughput targets.
- All 17 HiveLLM repos live under `e:/HiveLLM/` and are reachable by file path during bootstrap.

### Dependencies

- **Hive services:** Vectorizer, Nexus, Synap, Rulebook.
- **External:** Meilisearch (self-hosted), Claude Haiku (Anthropic), Deno (detector sandbox), Tree-sitter (chunker).
- **Tools:** Docker + Compose, Rust stable, Node.js 20+, optional `gh` CLI.

### Risks & mitigations

| Risk                                                                | Mitigation                                                                  |
|---------------------------------------------------------------------|-----------------------------------------------------------------------------|
| Classifier cost grows beyond budget                                  | Budget tracker + degradation ladder + static fallback (spec 05).            |
| Lexum not ready for full-text                                        | Meilisearch today; migration is a client swap (spec 08 Decision 2).          |
| Retrieval quality disappoints on first pass                          | Golden-set eval, per-intent RRF tuning, Phase-2 quality pass.                |
| Adapter's sync hook budget is too tight                              | Fail-open everywhere on the critical path; async follow-up (specs 10, 14).   |
| Schema drift breaks old data                                         | `schema_version` in cache keys + human-in-the-loop migrations (specs 06/08). |
| Users reject workspace-side artifacts (e.g. Cursor `_cortex_context.md`) | Opt-out flag (`--no-workspace-write`, spec 17 OQ 3).                   |
| Blocking laws produce false positives                                | Require sandboxed pure detectors + lint-time shape checks (spec 13).         |

## 12. User stories — machine-readable (Ralph-compatible)

The following snippet maps 1:1 to the narrative stories above; status tracked via `passes: boolean` per the Ralph PRD convention documented in [`AGENTS.md`](../AGENTS.md).

```json
{
  "userStories": [
    { "id": "US-01", "title": "Before-change context", "priority": "P0",
      "description": "Pre-thinking bundle injected into agent prompts.",
      "acceptanceCriteria": ["Specs 10, 11, 12 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-02", "title": "Blocking law enforcement", "priority": "P0",
      "description": "Critical violations block offending tool calls at the adapter.",
      "acceptanceCriteria": ["Specs 13, 14 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-03", "title": "Observational law capture", "priority": "P1",
      "description": "Non-critical violations captured asynchronously with evidence.",
      "acceptanceCriteria": ["Specs 13, 14 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-04", "title": "Decision register", "priority": "P0",
      "description": "Every formal Decision is indexed, supersedable, citable.",
      "acceptanceCriteria": ["Specs 11, 15, 16 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-05", "title": "Deep analysis", "priority": "P1",
      "description": "Structured multi-agent debate ending in a judged Decision.",
      "acceptanceCriteria": ["Spec 15 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-06", "title": "Backfill existing corpus", "priority": "P0",
      "description": "All 17 HiveLLM repos indexed before live capture opens.",
      "acceptanceCriteria": ["Spec 09 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-07", "title": "Live capture across tools", "priority": "P0",
      "description": "Uniform capture across Claude Code / Cursor / Codex / Gemini.",
      "acceptanceCriteria": ["Specs 10, 17 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-08", "title": "Operator visibility", "priority": "P1",
      "description": "Dashboard with 7 views + live SSE.",
      "acceptanceCriteria": ["Spec 16 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-09", "title": "Trust-based routing", "priority": "P2",
      "description": "Per-(model, repo) trust score published to Rulebook.",
      "acceptanceCriteria": ["Spec 14 acceptance pass"], "passes": false, "notes": "" },
    { "id": "US-10", "title": "Right to forget", "priority": "P2",
      "description": "Cascade delete a session or repo across all backends.",
      "acceptanceCriteria": ["Specs 02, 14 acceptance pass"], "passes": false, "notes": "" }
  ]
}
```

## 13. Open questions

Tracked in [`architecture.md` §12](architecture.md) — classifier migration trigger, event-bus durability, adapter granularity, law authoring UX, cross-repo identity, user-vs-model punishment, schema evolution. These must be decided before Phase 0 ends.

## 14. References

- [`architecture.md`](architecture.md) — the "how".
- [`specs/00-index.md`](specs/00-index.md) — spec list + dependency order.
- [`dag.md`](dag.md) — dependency graph of the 17 specs.
- [`roadmap.md`](roadmap.md) — phased delivery plan.
- [`AGENTS.md`](../AGENTS.md) — contributor + AI-agent rules.
