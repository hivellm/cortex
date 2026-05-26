# Proposal: phase12g_meili-index-audit-rulebook-vectorizer

Source: `docs/analysis/rework/03-relevance.md` Achado 2 (HIGH); `docs/analysis/rework/opus5.7/03-recommendation.md` patch #5.

## Why

The 4-doc relevance audit found that `cortex-rulebook-*` and `cortex-vectorizer-*` Meili indexes are essentially empty. Bootstrap configures the indexes but never reindexes the existing event corpus into them. Queries scoped to those repos return zero hits even when Synap clearly has the events. This is the second largest single source of "data so far doesn't result in anything actually relevant".

## What Changes

- Audit pass: `cortex-ops meili audit --repo <slug>` reports doc-count per index against the matching event-count in Synap. Any divergence > 5% is flagged as drift.
- Reindex pass: `cortex-ops meili reindex --repo <slug> [--from <RFC3339>]` walks Synap, batches 1k events at a time, and writes them through the same projection chain the live indexer uses (`cortex-workers/src/fulltext/projection.rs`).
- Boot-time guard: if any configured-but-empty index is detected at boot, emit one WARN per index telling the operator which `meili reindex` to run.

## Impact

- Affected specs: `docs/specs/06-fulltext.md` § Reindex tooling.
- Affected code: `crates/cortex-cli/src/bin/cortex-ops.rs`, `crates/cortex-workers/src/fulltext/{audit.rs,reindex.rs}` (new modules).
- Breaking change: NO. New tooling.
- User benefit: queries against rulebook + vectorizer repos start returning relevant hits.
