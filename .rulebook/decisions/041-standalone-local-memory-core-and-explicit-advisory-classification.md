# 41. Standalone local memory core and explicit advisory classification

**Status**: accepted
**Date**: 2026-10-18
**Related Tasks**: phase36_local-first-core

## Context

The operator replaced the mandatory Docker/service stack with one self-hosted CLI, embedded web GUI, local files, queryable indexes and simple links, while retaining months of collected history. The old daily profile still needed API/adapter lifecycle management, and populated backend indexes had not established useful agent recall. Claude reviewed the file/index boundary and migration contracts before implementation.

## Decision

Make crates/cortex-local the default workspace member and sole default cortex executable. Persist immutable memory revisions as Markdown with JSON frontmatter and historical events as bounded NDJSON segments. Treat SQLite FTS5 and link tables as rebuildable projections under cross-process locking, with exact case-sensitive repository scopes and stable provenance. CLI, stdio MCP and lifecycle hooks call the same local engine; cortex serve adds one foreground loopback process with bundled web assets. No Docker, GPU, external index service, adapter daemon or OS schedule is required. Import legacy SQLite and zstd NDJSON read-only using explicit mappings/namespaces, resumable bounded batches and complete rejection/partial-frame accounting. Jev work requires explicit authorized queue/run actions; validated advice and usage stay separate from source content, authority and revisions. ADR-039 authority and source safeguards remain; ADR-040 describes the retained legacy runtime, not the new default.

## Alternatives Considered

- Keep the daily API plus adapter daemons and add autostart: rejected because mandatory process coordination recreates the failure mode and exceeds the requested core.
- Use one Markdown file per historical event: rejected after Claude review because high-volume history benefits from immutable bounded NDJSON segments.
- Keep Nexus, Vectorizer, Synap and Meilisearch mandatory: rejected because initial recall and typed links must work offline with local files.
- Classify every captured record automatically or rewrite authority from classifier advice: rejected because capture must remain offline and source authority must be explicit.

## Consequences

The default search is literal token AND over SQLite FTS5, not semantic search. Raw events are searchable explicitly but excluded from automatic memory advice; hook context is bounded to 6 KiB. Files are plaintext and redaction is best effort; there is no automatic pruning, so operators own disk growth and backups. Exact scope names, including Cortex versus cortex, remain distinct. The one-time historical migration recovered 106,930 unique events while preserving all source hashes; sixteen open zstd frames remain honestly incomplete, and quarantine remains excluded. Installed Codex and Claude comparisons establish use of two specific memories, not general productivity. One authorized Jev request proves wiring, not classification accuracy. Installation backs up the executable and plugin bundle but their combined publication is not crash-atomic; rerun or restore a compatible standalone backup, retaining local/. Browser extensions/security integrations can observe GUI credentials despite loopback and Cortex headers. The final gate passes 128 standalone plus 4,100 legacy tests, with nine pre-existing ignored external checks/examples. Evaluation, durable use audit and optional feedback ranking remain separate retained tasks. Operational contract: docs/operations/local-first-memory.md. Evidence: docs/analysis/local-first-core-2026-10/05-implementation-evidence.md.
