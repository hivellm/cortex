# Proposal: phase0_storage-layout

## Why

Cortex is a composition over four external services (Vectorizer, Nexus, Synap, Meilisearch) plus a durable archive. We need to lock **what lives where** before any worker writes a byte: collection names, graph constraints, index names, Synap stream topology, Parquet partitioning, KV namespaces, and SQLite schemas. Without this, every downstream worker would ad-hoc its own naming and render `cortex forget` impossible.

## What Changes

- Parquet archive partitioning (`cortex.events/<yyyy>/<mm>/<dd>/<kind>.parquet`) and writer contract.
- Synap stream topology (`cortex.events.raw`, `cortex.events.bootstrap`, `cortex.events.enriched`, `cortex.events.embedded`, `cortex.events.graphed`, `cortex.events.fulltext_indexed`, `cortex.events.invalid`, `cortex.events.law_violation`, `cortex.events.query_audit`, `cortex.metrics`, `cortex.cache.invalidate`).
- Vectorizer collection namespace (`cortex-code`, `cortex-docs`, `cortex-decisions`, `cortex-turns`, `cortex-governance`, `cortex-misc`) with schemas.
- Nexus constraints + indexes (see spec 07's CREATE statements).
- Meilisearch index names + default settings file path.
- Synap KV namespaces (`cache:classify:*`, `cache:query:*`, `governance:reminders:*`).
- SQLite metadata DB for classifier spend, trust scores, materialized views (`.cortex/meta.sqlite`).
- Retention policy table by `pii_risk`.

## Impact

- **Affected specs:** [`docs/specs/02-storage-layout.md`](../../../docs/specs/02-storage-layout.md); unblocks 03, 04, 06, 07, 08.
- **Affected code:** `cortex-core/src/router/archive.rs`, `cortex-core/src/storage/{vectorizer,nexus,meili,synap,sqlite}.rs`, constants module.
- **Breaking change:** NO — greenfield.
- **User benefit:** predictable naming, cascading delete is possible, schema-drift detection has a clear baseline.

## Source

`docs/specs/02-storage-layout.md` · depends on spec 01 · PRD FR-2.
