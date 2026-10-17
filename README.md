# Cortex

> **Status:** Active development · 43 specs (13+ core specs shipped) in [docs/specs/](docs/specs/) · architecture in [docs/architecture.md](docs/architecture.md) · current state & roadmap in [docs/analysis/cortex-platform-2026-07/](docs/analysis/cortex-platform-2026-07/)

Cortex stores explicit decisions and verified lessons so coding agents can use them in later sessions. The daily profile keeps capture, cited recall and feedback in local SQLite, without mandatory Docker, GPU or bulk indexers. An optional full stack indexes code, documents and archived interactions.

## Daily Memory

For routine agent work, start with the [daily runtime guide](docs/operations/daily-memory-runtime.md). Capture an accepted decision or verified lesson with `cortex_capture_memory`; recover applicable context with `cortex_memory_read` or `cortex_pre_thinking`. Corrections explicitly supersede an earlier decision using the observed revision.

- Claude Code supports verified lifecycle delivery through its local adapter. Codex uses verified MCP reads; other harnesses have [documented capability limits](docs/analysis/daily-agent-memory-2026-10/09-client-contracts.md).
- Context carries source IDs and includes at most five memories within 1,536 bytes. Superseded, proposed and out-of-scope records stay out of ordinary advice.
- Capture remains local by default. Optional `classify=true` requests bounded Jev advice for an authorized candidate; classification never changes its status, scope or authority.
- Native agent memory files are preserved. Daily mode disables bulk publishing, archive replay and scheduled maintenance.
- [Behavioral acceptance](docs/analysis/daily-agent-memory-2026-10/11-cross-model-acceptance.md) verifies a Claude correction recalled and used by Codex after restart. [Resource measurements](docs/analysis/daily-agent-memory-2026-10/10-resource-acceptance.md) state the host, samples and limits.


## What Cortex is (and isn't)

**Is:** an orchestrator that composes existing HiveLLM services (**Vectorizer**, **Nexus**, **Synap**, **Rulebook**) plus external pieces (**Meilisearch**, **Claude Haiku** via Claude Code CLI) into a capture → classify → retrieve → govern loop.

**Isn't:** a new vector DB, graph DB, search engine, or coding agent. Cortex informs agents; it does not generate code.

## Architecture at a glance

```
┌───────────────────────────────────────────────────────────────────────┐
│  AI Tools / IDEs                                                      │
│  Claude Code · OpenCode · Cursor · Gemini · Codex · Copilot · Windsurf │
└───────────────────────────────────────────────────────────────────────┘
                       ▲                       ▲
                       │ capture (hooks)       │ retrieve (MCP / HTTP)
                       ▼                       │
┌───────────────────────────────────────────────────────────────────────┐
│                          C O R T E X                                  │
│  Ingestion → Processing → Retrieval + Governance + Dashboard          │
└───────────────────────────────────────────────────────────────────────┘
                              ▼
┌───────────────────────────────────────────────────────────────────────┐
│  HiveLLM Data Services                                                │
│  Vectorizer · Nexus · Synap · Meilisearch · Haiku (CLI)               │
└───────────────────────────────────────────────────────────────────────┘
```

Full architecture doc: [`docs/architecture.md`](docs/architecture.md).

## Key capabilities

| Layer            | What it does                                                                                     | Spec                                                               |
|------------------|--------------------------------------------------------------------------------------------------|--------------------------------------------------------------------|
| **Capture**      | Hooks in Claude Code / OpenCode / Cursor / Codex / Gemini publish events (prompts, tool calls, decisions). | [10](docs/specs/10-claude-code-adapter.md), [17](docs/specs/17-additional-adapters.md), [20](docs/specs/23-opencode-adapter.md) |
| **Classify**     | Claude Haiku (via CLI or SDK) enriches events with topics, severity, PII risk.                    | [05](docs/specs/05-classifier.md)                                  |
| **Embed**        | Symbol-level code chunks (Tree-sitter) + doc sections go to Vectorizer (BM25 + dense).           | [06](docs/specs/06-embedder.md)                                    |
| **Link**         | Nexus stores the graph of sessions, turns, tool calls, artifacts, decisions, laws, violations.    | [07](docs/specs/07-graph-writer.md)                                |
| **Full-text**    | Meilisearch indexes every enriched event with typo-tolerance and faceted filters.                 | [08](docs/specs/08-fulltext-indexer.md)                            |
| **Retrieve**     | Hybrid query API (vector + keyword + graph), fused with Reciprocal Rank Fusion. P50 < 50 ms.     | [11](docs/specs/11-query-api.md)                                   |
| **Pre-thinking** | Adapter injects a compact context bundle (decisions, laws, similar turns, snippets) into every prompt. | [12](docs/specs/12-pre-thinking-injection.md)                 |
| **Govern**       | Laws with sandboxed detectors (Deno); blocking + observational; punishment ladder; trust scores.  | [13](docs/specs/13-laws-dsl.md), [14](docs/specs/14-governance-engine.md) |
| **Analyze**      | Deep Analysis: structured multi-agent debate → audited Decision.                                  | [15](docs/specs/15-deep-analysis.md)                               |
| **Observe**      | React + TS dashboard: live timeline, memory, decisions, laws, analyses, tool analytics, graph.    | [16](docs/specs/16-dashboard.md)                                   |
| **Bootstrap**    | One-shot + incremental CLI that backfills the ~17 existing HiveLLM repos before live capture.     | [09](docs/specs/09-bootstrap-cli.md)                               |

## Repository layout

```
Cortex/
├─ docs/
│  ├─ architecture.md          # the big picture (read first)
│  └─ specs/                   # 17 discrete, implementable unit specs
│     ├─ 00-index.md
│     └─ NN-*.md
├─ AGENTS.md                   # team-shared rules (generated by @hivellm/rulebook)
├─ AGENTS.override.md          # project-specific overrides (stable)
├─ CLAUDE.md                   # imports AGENTS.md + critical rules
├─ .claude/                    # agents, skills, hooks, rules, settings
├─ .rulebook/                  # specs, decisions, knowledge, tasks, memory
└─ README.md                   # you are here
```

Code (`cortex-core`, `cortex-workers`, `cortex-api`, `cortex-classifier`, `cortex-laws`, `cortex-adapters`, `cortex-dashboard`, `cortex-cli`) arrives as each spec is implemented — see [Roadmap](#roadmap).

## Getting started

For routine agent memory, use the [daily runtime guide](docs/operations/daily-memory-runtime.md). The prerequisites and indexing workflow below apply to the optional full stack.

### Prerequisites (local dev)

- **Rust** (stable toolchain) — for the core services.
- **Node.js** 20+ — for the dashboard and detector sandbox tooling.
- **Docker + Compose** — for the local data-services stack (Vectorizer, Nexus, Synap, Meilisearch). See [`docs/specs/03-local-stack.md`](docs/specs/03-local-stack.md).
- **Claude Code CLI** — classifier default backend (SDK path is an optional alternative).

### Read-first

1. [`docs/architecture.md`](docs/architecture.md) — vision, goals, ecosystem context.
2. [`docs/specs/00-index.md`](docs/specs/00-index.md) — spec index + dependency order.
3. [`AGENTS.md`](AGENTS.md) — rules for contributors and AI agents working in this repo.

The optional Compose stack includes Cortex API, ingestion, classifiers, embedders, graph/fulltext/reranker workers and data services. Daily memory uses the smaller native runtime described above; a configured Compose stack does not imply those services are running. Most core specs (01–18) are implemented and shipped. See [`docs/specs/00-index.md`](docs/specs/00-index.md) for the per-spec status and [`docs/analysis/cortex-platform-2026-07/`](docs/analysis/cortex-platform-2026-07/) for known current issues (e.g., graph-worker degradation since 2026-06-27, pending active fix).

### First-time indexing (per repo)

The daemon answers `/v1/query`, `cortex_pre_thinking`, and the
dashboard against repos that have been bootstrapped at least once. A
brand-new repo returns empty hits with
`notice = { code: "repo_not_indexed", … }` and `cortex_status` lists
the indexed slugs under `daemon.indexed_repos`, so callers can detect
the gap before issuing the next query.

To bootstrap a repo:

```sh
# canonical entry point: spec-09 bootstrap CLI
cargo run -p cortex-bootstrap -- --repo /path/to/repo
```

Subsequent calls scoped to that repo (`scope.repo = "<slug>"`) drop
the notice and start returning real results. The slug is the value
`cortex-storage`'s `slug_for_repo` produces — lowercase ASCII; the
same form `daemon.indexed_repos` reports.

### Build hygiene (keep `target/` small)

Cargo never garbage-collects build artifacts, so `target/` grows without
bound (it once passed **500 GB** here). The repo already limits this:
`[profile.dev].debug = "line-tables-only"` and `[profile.release].strip`
keep debuginfo and symbols in check. For the rest, sweep periodically —
this removes only artifacts you are not actively building, so the next
build stays incremental:

```sh
cargo install cargo-sweep      # once
scripts/sweep-target.sh        # remove artifacts unused for 14 days
scripts/sweep-target.sh --dry-run
cargo clean                    # full reclaim (forces a fresh rebuild)
```

Automate it (daily Task Scheduler job on Windows, cron elsewhere) and set
`CARGO_INCREMENTAL=0` in CI. Full guide:
[`docs/rust-target-hygiene.md`](docs/rust-target-hygiene.md).

## Roadmap

Five phases, mapped to the spec list:

- **Phase 0 — Foundations** (2 weeks)
  Event schema, storage layout, docker-compose local stack, `cortex-core` skeleton, single-repo bootstrap proof of concept (Vectorizer).
  _Specs: [01](docs/specs/01-event-schema.md), [02](docs/specs/02-storage-layout.md), [03](docs/specs/03-local-stack.md), [04](docs/specs/04-cortex-core.md)._

- **Phase 1 — Bootstrap + Claude Code adapter + basic retrieval** (4 weeks)
  Haiku classifier, embedder, graph writer, full-text indexer, bootstrap CLI for all 17 repos, Claude Code hooks, hybrid query API, pre-thinking injection.
  _Specs: [05](docs/specs/05-classifier.md), [06](docs/specs/06-embedder.md), [07](docs/specs/07-graph-writer.md), [08](docs/specs/08-fulltext-indexer.md), [09](docs/specs/09-bootstrap-cli.md), [10](docs/specs/10-claude-code-adapter.md), [11](docs/specs/11-query-api.md), [12](docs/specs/12-pre-thinking-injection.md)._

- **Phase 2 — Governance** (4 weeks)
  Laws DSL + detector sandbox, enforcement engine, trust score, dashboard v1.
  _Specs: [13](docs/specs/13-laws-dsl.md), [14](docs/specs/14-governance-engine.md), [16](docs/specs/16-dashboard.md)._

- **Phase 3 — Deep Analysis + multi-adapter** (open)
  Analysis workflow, Cursor/Codex/Gemini adapters, retrieval-quality evaluation.
  _Specs: [15](docs/specs/15-deep-analysis.md), [17](docs/specs/17-additional-adapters.md)._

- **Phase 4+ — Hardening + HivehubCloud integration** (open)
  Multi-tenant, distributed deployment, trust-score feed into cloud routing.

Exact timelines are aspirational; each spec has its own acceptance criteria and dependency list.

**Current delivery status:** The five-phase sketch above is the original plan from April 2026. Actual delivery has now progressed through Phase 0 and Phases 1–27+ (spanning bootstrap, ACL/governance groundwork, bitemporal/branching support, GraphRAG foundations, and live-ops hardening). For the current, accurate roadmap and the full list of phases in progress, see [`docs/analysis/cortex-platform-2026-07/`](docs/analysis/cortex-platform-2026-07/). Per-spec implementation status is tracked in [`docs/specs/00-index.md`](docs/specs/00-index.md).

## Contributing

This project is governed by [@hivellm/rulebook](https://github.com/hivellm/rulebook). Before making changes:

1. Read [`AGENTS.md`](AGENTS.md) and [`AGENTS.override.md`](AGENTS.override.md). These contain project-specific conventions that override generic guidance.
2. Use the Rulebook MCP tools (`mcp__rulebook__*`) for task management — never `mkdir` tasks manually.
3. Pick the **first unchecked item from the lowest-numbered phase/spec**. No cherry-picking.
4. Run diagnostics (type-check, lint) **before** tests. Zero warnings; tests must pass 100%; coverage ≥95%.
5. Commits use conventional commits (`feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`) and stay in English.
6. Capture patterns, anti-patterns, and decisions through `rulebook_knowledge_add`, `rulebook_learn_capture`, `rulebook_decision_create` at the end of each task.

## References

- **Vectorizer** — https://github.com/hivellm/vectorizer — vector DB, MCP, embeddings.
- **Nexus** — https://github.com/hivellm/nexus — graph DB, Cypher, KNN.
- **Synap** — https://github.com/hivellm/synap — KV, streams, pub/sub.
- **Rulebook** — https://github.com/hivellm/rulebook — rules, memory, MCP for task management.
- **Meilisearch** — https://www.meilisearch.com — full-text search.
- **Claude Haiku** — https://www.anthropic.com/claude/haiku — classification backend via Claude Code CLI.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
