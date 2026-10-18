# Proposal: phase28_retrieval-eval-gate-live

## Scope reset and prerequisite

This task ID is retained, but its scope is reset on 2026-10-17 to evaluate the
standalone engine approved in `.rulebook/tasks/phase36_local-first-core/design.md`.
Implementation MUST wait until `phase36_local-first-core` is complete. This
does not alter that task's order or the concurrent Codex adapter task.

The former checklist had 20/20 items checked; that did not establish acceptance:
reranker p95 delta was +2122 ms, phantom-link rate was 0.50, and nightly execution
omitted retrieval. Original evidence remains at commit
`2ba3d2f502f3f3955166ac272ae5d8b28c2aea05`; disposition is recorded in
`docs/analysis/local-first-core-2026-10/02-backlog-review.md`. These failures
are historical evidence, not a passing baseline for the new engine.

## Why

API reachability, indexed record counts and edge counts do not prove useful
recall. The standalone engine needs reproducible measurements of returned
answers, identity, provenance, repository isolation and end-to-end latency.
Previous fixtures depended on changing private corpora and backend-specific IDs.

## What changes

- One versioned public/local corpus pins canonical source hashes, stable record
  identities, provenance and relevance judgments. Include held-out queries,
  readable historical turns/tool events, corrections, superseded records,
  empty-result cases, distractors and repositories sharing names/text.
- Production import/reindex builds isolated fixture homes. Recall@5 and MRR@10
  measure expected identities and provenance with per-query diagnostics and
  documented intent coverage. Do not select only queries that already pass.
- Scope checks cover results, graph traversal, caches, statistics and ranking
  inputs. Alternating repositories exposes shared-cache contamination.
- Graph checks evaluate expected nodes, edge kinds and source evidence, including
  bounded paths where supported. Counts are not answers; unsupported tools are
  not added merely to satisfy a benchmark.
- Offline Jev fixtures test typed responses, scoring, unknown/transient outcomes
  and authority preservation. Separately opt-in live evaluation uses reviewed
  public payloads, bounded requests/time/cost and model/schema/usage reporting.
  Mock scores do not prove live provider quality. Credentials never enter output.
- CLI, direct MCP and supported hooks receive end-to-end latency measurements
  with separate cold/warm runs, bounded samples/concurrency and p50/p95 reports.
  Record build, corpus, hardware and timing boundaries. Prepared context does not
  establish observed agent use.
- Reviewed standalone baselines and justified floors precede enforcement.
  Preserve failing evidence; fixture/floor changes require recorded justification.
  Legacy TEI/cosine thresholds do not automatically apply.
- Installed read-only smoke uses an isolated fixture home to compare binary
  identity, advertised/implemented/documented tool equality and CLI/MCP answers, identities,
  provenance and scope. It never captures into the real store or classifies.

## Boundaries and impact

Run deterministic gates explicitly and in push/PR CI. No nightly external
schedule, OS scheduler, Docker, running HTTP daemon or live secret is required.
Live Jev reports remain separate and never silently gate offline validation.
Historical data and legacy fixtures stay intact.

Affected surfaces: standalone evaluation harness/fixtures, CLI/MCP/hook tests,
CI and evaluation documentation. Reuse existing evaluation primitives only where
they remain independent of the legacy stack. Record measured limitations and
preserve historical failed-gate evidence.
