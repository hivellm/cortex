# Cortex — Specifications Index

> Each spec in this folder is a **discrete, implementable unit of work**. Specs are numbered in **build order**: a spec can assume earlier-numbered specs are decided and stable, but should not depend on later ones.
>
> Status legend: 🟢 *implemented* · 🟡 *spec drafted* · 🔴 *not started*

## Known numbering collisions

This index documents **43 distinct spec files**. Four spec numbers are currently used by two files each: **20** (mcp-tool-surface.md + opencode-adapter.md), **26** (eval.md + cortex-config.md), **27** (consolidation.md + retrieval-rerank.md), and **28** (gui-contract.md + phantom-link-verifier.md). Additionally, numbers 23, 29, and 37–39 have no corresponding file on disk. These numbering conflicts will be resolved by a follow-up renumbering task (`phase28_docs-truth-reconciliation`) rather than silently fixed here.

| #  | Spec                                                            | Status | Depends on        | Maps to / Notes              |
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
| 19 | [Retention — decay & purge policy](19-retention.md)             |   🟢   | —                 | phase9a+11o+13a              |
| 20 | [MCP tool surface registry](20-mcp-tool-surface.md)             |   🟢   | 11, 12, 16, 18    | §5.1, §8                     |
| 20 | [OpenCode adapter — TypeScript plugin + daemon](20-opencode-adapter.md) | 🟢 | 10, 11, 12, 18 | §5.1                    |
| 21 | [Dashboard push — SSE deltas via Synap](21-dashboard-push.md)   |   🟡   | 16, 20            | §5.6                         |
| 22 | [Fine-grained search — direct read-only proxies](22-fine-grained-search.md) | 🟢 | 11, 18 | phase11v                     |
| 24 | [Producer trait — checkpoint accumulation](24-producer-trait.md) |   🟢   | —                 | ADR-010 (phase13b)           |
| 25 | [Event identity — cross-backend join key](25-event-identity.md) |   🟡   | 02, 06, 07, 08    | ADR-012                      |
| 26 | [Eval — end-to-end quality gates](26-eval.md)                   |   🟢   | —                 | phase14c                     |
| 26 | [Typed Config crate](26-cortex-config.md)                       |   🟢   | all crates        | ADR-016                      |
| 27 | [Consolidation — daemon, triggers, grain dispatch](27-consolidation.md) | 🟢 | — | phase14a                     |
| 27 | [Retrieval reranker — BGE-reranker-v2-m3 via TEI](27-retrieval-rerank.md) | 🟢 | 11 | phase17 (P2)                 |
| 28 | [GUI contract — codegen pipeline for type safety](28-gui-contract.md) | 🟢 | — | phase14d                     |
| 28 | [Phantom link verifier — (path, symbol) validation](28-phantom-link-verifier.md) | 🟢 | 11, 27 | phase17 (P3)        |
| 30 | [Bitemporal schema — temporal + branching retrieval](30-bitemporal-schema.md) | 🟢 | 07, 08, 11, 16 | phase18 (P0+P1)       |
| 31 | [Temporal classifier — bitemporal axis filtering](31-temporal-classifier.md) | 🟡 | 11, 30 | phase18 (P2, partial)         |
| 32 | [Branches — retrieval scope for exploration paths](32-branches.md) | 🟡 | 30, 31 | phase18 (P2, partial)         |
| 33 | [Timeline API — HTTP/CLI/MCP bitemporal queries](33-timeline-api.md) | 🟢 | 30, 31, 32 | phase18 (P3)         |
| 34 | [Cross-project axis — ecosystem navigation](34-cross-project-axis.md) | 🟢 | 30, 31, 33 | phase18 (P4)         |
| 35 | [Temporal pre-thinking — bitemporal anchors](35-temporal-pre-thinking.md) | 🟡 | 12, 30, 31, 32 | phase18 (P5, partial)  |
| 36 | [Temporal observability — audit envelopes](36-temporal-observability.md) | 🟡 | 31, 34 | phase18 (P6, partial)      |
| 40 | [Classification model — Bell-LaPadula lattice](40-classification-model.md) | 🟡 | 07, 08, 30 | phase21 (P1, schema)     |
| 41 | [Principal + RBAC — identity + clearance](41-principal-and-rbac.md) | 🟢 | — | phase21 §4                   |
| 42 | [Access enforcement — ACL filtering stack](42-access-enforcement.md) | 🟢 | 40, 41 | phase21 §5                   |
| 43 | [ACL admin API — CLI/HTTP/MCP surfaces](43-acl-admin-api.md)    |   🟢   | 41                | phase21 §6                   |
| 44 | [Access audit & eval — audit envelopes, CI gates](44-access-audit-and-eval.md) | 🟢 | 42, 41 | phase21 §8-§9      |

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
