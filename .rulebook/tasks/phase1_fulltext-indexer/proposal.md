# Proposal: phase1_fulltext-indexer

## Why

The third retrieval lane is typo-tolerant keyword search with faceted filters (repo, severity, topic, `ts` range). Without it, a user searching "refator hnsw" gets nothing, and filters like "only critical violations in Vectorizer" require a full graph query. Meilisearch is the production-ready answer in v1 (Lexum migration is a client swap).

## What Changes

- `MeiliClient` with HTTP batched upsert, retry, task-await mode for bootstrap.
- Per-kind index set (`cortex-code`, `cortex-docs`, `cortex-decisions`, `cortex-turns`, `cortex-governance`, `cortex-misc`).
- Versioned settings file `cortex-fulltext/settings.v1.json` (searchable/filterable/sortable attrs, ranking rules, synonyms, stop-words, typo tolerance).
- Doc builders per event family (one pure function per kind) + deterministic `doc_id` (`event_id` live, `bootstrap:<repo>:<path>:<hash>` bootstrap).
- Body selection rule (summary if raw >4 KB; redacted raw otherwise).
- Worker binary consuming `cortex.events.enriched` + `cortex.events.embedded`, publishing `cortex.events.fulltext_indexed`.

## Impact

- **Affected specs:** [`docs/specs/08-fulltext-indexer.md`](../../../docs/specs/08-fulltext-indexer.md); unblocks 09 + 11.
- **Affected code:** new `cortex-fulltext/` crate, worker binary `cortex-fulltext-worker`, settings file + synonyms/stop-words tables.
- **Breaking change:** NO — greenfield.
- **User benefit:** keyword lane with typo tolerance; filterable dashboards; precise `law_check` retrievals.

## Source

`docs/specs/08-fulltext-indexer.md` · depends on specs 01 + 02 · PRD FR-8.
