# Proposal: phase13d_event-identity-index-adr-012

Source: `docs/analysis/rework/04-architecture.md` §A.4; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.4.

## Why

`forget`, dedup, doctor, and retention all need to answer "where does event X live across the 5 backends?" — Synap, Vectorizer, Nexus, Meili, archive. Today each path re-derives the join via per-backend lookups. This is slow (`cortex doctor consistency` over 100k events takes minutes) and fragile (a missed backend silently breaks `forget`).

A single typed `EventIdentity` keyed by `event_id` and persisted in SQLite makes every cross-backend op an indexed lookup.

## What Changes

- New ADR-012 — "EventIdentity as cross-backend join key + SQLite IdentityIndex".
- New struct `EventIdentity { event_id, nexus_id: Option<String>, vec_id: Option<String>, meili_id: Option<String>, archive_partition: Option<String> }`.
- New SQLite table `event_identity` indexed by `event_id` (PK), with secondary indexes on `nexus_id`, `vec_id`, `meili_id`.
- Every ingest / projection path writes the corresponding id back to `event_identity` via `upsert_identity(event_id, backend, id)`.
- `cortex doctor consistency` rewritten to walk `event_identity` once and check existence per backend. Budget: <10s for 100k events on the running stack.
- `admin forget` becomes one transaction reading `event_identity` and dispatching deletes per backend.

## Impact

- Affected specs: `docs/specs/04-event-schema.md` § Identity; new `docs/specs/25-event-identity.md`.
- Affected code: `crates/cortex-storage/src/identity.rs` (new), `crates/cortex-storage/src/metadata.rs` (table migration), `crates/cortex-workers/src/{embedder,fulltext,graph}/projection.rs` (each writes back), `crates/cortex-cli/src/bin/cortex-doctor.rs` (rewrite).
- Breaking change: NO wire-format change.
- User benefit: doctor + forget become one indexed lookup; missed-backend bugs become impossible by construction.
