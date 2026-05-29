# 18. phase18 §1.1 — Bitemporal time precision: UTC RFC3339 with second precision; day-precision for ADR-facing tooling

**Status**: proposed
**Date**: 2026-05-29
**Related Tasks**: phase18_tlb-timeline-branching

## Context

phase18_tlb-timeline-branching introduces bitemporal columns (`valid_from`, `valid_to`, `recorded_at`, `superseded_at`) on every retrievable entity. The precision the storage layer commits to is load-bearing: a fact that says "valid through 2026-03-12" should not be filtered out by a `as_of = 2026-03-12T08:00:00Z` query that resolves to second precision against a date-rounded boundary. The retrieval surface also accepts human-typed dates, so the CLI must clamp to a precision the storage layer can answer authoritatively. Two design questions: storage precision and ADR/decision-facing CLI default precision.</context>
<parameter name="decision">Store every bitemporal timestamp as UTC RFC3339 with second precision (`YYYY-MM-DDTHH:MM:SSZ`). Reject sub-second precision at the wire boundary (writers truncate). For ADR / decision tooling (cortex history, cortex supersession, cortex timeline with the ADR kind), the CLI accepts day-precision input (`--as-of 2026-04-01`) and expands it to two range probes: `valid_from <= 2026-04-01T23:59:59Z AND (valid_to IS NULL OR valid_to >= 2026-04-01T00:00:00Z)`. The second-precision storage column is the single source of truth; the day-precision input is a UX shortcut. ULID timestamps remain millisecond-precision internally; the bitemporal columns clamp to seconds for indexability + human readability.

## Decision

_No decision recorded._

## Alternatives Considered

- Millisecond precision storage — rejected because Meili sortable integers and Vectorizer payload fields stay cheaper as seconds; no observed query pattern benefits from millisecond granularity on facts that change at the ADR / commit cadence
- Microsecond precision — same rejection with a higher cost on Meili and Vectorizer side
- Day-precision storage with second-precision overlay — rejected because the inverse keeps the source of truth canonical and the UX flexible; day-precision storage forces every commit / live-event probe to round-trip through day boundaries
- Allow sub-second precision on the wire — rejected because deduplication and supersession-edge math become race-prone when two events land in the same second
- Two columns per field (seconds + original string) — rejected as duplication; the single canonical Zulu-suffix RFC3339 covers storage and audit

## Consequences

Wins: Meili sortable axis stays an int (epoch seconds); Vectorizer payload stays compact; supersession-edge math is race-free (two events in the same second still get one canonical winner via the event_id tiebreak); audit logs stay human-grepable. Day-precision CLI input keeps the operator surface friendly. Costs: client code that generates timestamps must explicitly truncate sub-second precision (a one-line guard at the producer side); ULID-encoded timestamps need a `to_second()` helper at the bitemporal projection layer (already exists for the `ts` field in fulltext/builders.rs). Reassessment trigger: if any future fact type needs sub-second granularity, add a separate `*_at_ms` companion column rather than relaxing the rule. Phase18 P1 §2.6 (Meili filterable attrs) and §2.7 (Vectorizer payload) carry the second-precision constraint into the index settings.
