# Cortex

> **Status:** Draft v0.1 · specs in [`docs/specs/`](docs/specs/) · architecture in [`docs/architecture.md`](docs/architecture.md)

**Cortex is the cognitive substrate of the HiveLLM ecosystem** — a single, queryable, governed memory of every meaningful interaction an AI agent has with our codebases.

Today, when an LLM (Claude, Cursor, Gemini, Codex, Copilot) works on one of our projects, three things are lost the moment the session ends:

1. **What happened** — the conversation, tool calls, agent invocations, rationale.
2. **What was decided** — and why one path was chosen over another.
3. **What was learned** — patterns that worked, dead ends, recurring bugs, conventions that emerged.

Each new session starts blind. The model falls back to its weights and a few `CLAUDE.md` files. We've been losing institutional knowledge at every `/clear`.

**Cortex changes the contract.** Every interaction is captured, classified, embedded, related, and made retrievable. Before any model proposes a change, it consults Cortex. Decisions are formalized and audited. Laws govern behavior, violations are tracked. A dashboard makes the whole substrate observable.

The end state: an AI workflow that is **analytical rather than purely generative** — grounded in our own history, not just the model's training data.

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
| **Capture**      | Hooks in Claude Code / OpenCode / Cursor / Codex / Gemini publish events (prompts, tool calls, decisions). | [10](docs/specs/10-claude-code-adapter.md), [17](docs/specs/17-additional-adapters.md), [20](docs/specs/20-opencode-adapter.md) |
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

### Prerequisites (local dev)

- **Rust** (stable toolchain) — for the core services.
- **Node.js** 20+ — for the dashboard and detector sandbox tooling.
- **Docker + Compose** — for the local data-services stack (Vectorizer, Nexus, Synap, Meilisearch). See [`docs/specs/03-local-stack.md`](docs/specs/03-local-stack.md).
- **Claude Code CLI** — classifier default backend (SDK path is an optional alternative).

### Read-first

1. [`docs/architecture.md`](docs/architecture.md) — vision, goals, ecosystem context.
2. [`docs/specs/00-index.md`](docs/specs/00-index.md) — spec index + dependency order.
3. [`AGENTS.md`](AGENTS.md) — rules for contributors and AI agents working in this repo.

Because no spec has flipped to 🟢 *implemented* yet, there is no runnable binary in `master` at the time of writing. The work unfolds in spec order (see below).

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
