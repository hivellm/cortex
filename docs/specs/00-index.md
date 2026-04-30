# Cortex — Specifications Index

> Each spec in this folder is a **discrete, implementable unit of work**. Specs are numbered in **build order**: a spec can assume earlier-numbered specs are decided and stable, but should not depend on later ones.
>
> Status legend: 🟢 *implemented* · 🟡 *spec drafted* · 🔴 *not started*

| #  | Spec                                                            | Status | Depends on        | Maps to architecture §       |
|----|-----------------------------------------------------------------|:------:|-------------------|------------------------------|
| 01 | [Event schema (wire format)](01-event-schema.md)                |   🟢   | —                 | §4.3                         |
| 02 | [Storage layout](02-storage-layout.md)                          |   🟢   | 01                | §3, §6, §7 (storage)         |
| 03 | [Local stack (docker-compose)](03-local-stack.md)               |   🟢   | 02                | §11 Phase 0                  |
| 04 | [Cortex Core — types, redactor, ingestion router](04-cortex-core.md) | 🟢 | 01, 02         | §5.1, §5.2                   |
| 05 | [Classifier — Haiku via CLI/SDK](05-classifier.md)              |   🟢   | 01, 04            | §5.2.1                       |
| 06 | [Embedder — chunking + Vectorizer client](06-embedder.md)       |   🟢   | 01, 02            | §5.2                         |
| 07 | [Graph writer — Nexus client](07-graph-writer.md)               |   🟢   | 01, 02            | §4.2, §5.2                   |
| 08 | [Full-text indexer — Meilisearch client](08-fulltext-indexer.md)|   🟢   | 01, 02            | §5.2                         |
| 09 | [Bootstrap CLI — index existing HiveLLM repos](09-bootstrap-cli.md) | 🟢 | 04, 05, 06, 07, 08 | §6                       |
| 10 | [Claude Code adapter — hooks + daemon](10-claude-code-adapter.md)|  🟢   | 04                | §5.1                         |
| 11 | [Query API — hybrid retrieval + RRF](11-query-api.md)           |   🟢   | 06, 07, 08        | §5.3                         |
| 12 | [Pre-thinking injection](12-pre-thinking-injection.md)          |   🟢   | 10, 11            | §5.3, §8                     |
| 13 | [Laws DSL + detector contract](13-laws-dsl.md)                  |   🟡   | 01, 04            | §5.4                         |
| 14 | [Governance engine — enforcement, punishment, trust score](14-governance-engine.md) | 🟡 | 13, 10 | §5.4                  |
| 15 | [Deep Analysis workflow](15-deep-analysis.md)                   |   🟡   | 11, 13            | §5.5                         |
| 16 | [Dashboard — views and SSE wiring](16-dashboard.md)             |   🟡   | 11, 14            | §5.6                         |
| 17 | [Additional adapters — Cursor, Codex, Gemini](17-additional-adapters.md) | 🟡 | 10           | §5.1                         |
| 18 | [Claude Code plugin — MCP server + commands + skills](18-claude-code-plugin.md) | 🟢 | 10, 11, 12 | §5.1, §8         |
| 20 | [MCP tool surface registry](20-mcp-tool-surface.md)             |   🟢   | 11, 12, 16, 18    | §5.1, §8                     |

## Spec format

Every spec file follows this skeleton:

```
# NN — Title

## Goal
What problem this spec solves, in one paragraph.

## Scope
- In: ...
- Out: ...

## Inputs / Outputs
Concrete contracts (JSON schemas, function signatures, CLI flags).

## Design
The "how" — components, data flow, key algorithms, error handling.

## Acceptance criteria
A bulleted, testable checklist. Each item is verifiable in CI or by manual demo.

## Open questions
Decisions deferred — must be resolved before this spec is "done".

## References
Links to architecture sections, related specs, external docs.
```

Specs are **living documents until status flips to 🟢**. Once a spec is implemented, it freezes — further changes go through new specs that supersede it (noted in the new spec's `## References`).
