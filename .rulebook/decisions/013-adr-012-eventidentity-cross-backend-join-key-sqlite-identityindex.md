# 13. ADR-012 — EventIdentity cross-backend join key + SQLite IdentityIndex

**Status**: proposed
**Date**: 2026-05-24

## Context

forget, dedup, doctor, retention re-derive cross-backend mapping per call; doctor_consistency over 100k events takes minutes; admin_forget silently misses a backend when one is added.

## Decision

Typed EventIdentity struct keyed by event_id, persisted in SQLite event_identity table with PK on event_id plus 3 partial UNIQUE indexes on nexus_id/vec_id/meili_id. Every projection writes back via IdentityIndex::upsert_identity. doctor + forget collapse to one indexed scan.

## Consequences

Positive: doctor <10s for 100k events; forget structurally complete; missed-backend bugs become NULL columns the doctor flags. Negative: ~3-day refactor touching every projection path + backfill of pre-phase13d rows. Neutral: wire format unchanged.
