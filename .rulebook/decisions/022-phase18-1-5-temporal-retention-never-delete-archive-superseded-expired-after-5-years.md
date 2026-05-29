# 22. phase18 §1.5 — Temporal retention: never delete; archive superseded/expired after 5 years

**Status**: proposed
**Date**: 2026-05-29
**Related Tasks**: phase18_tlb-timeline-branching

## Context

Phase18 P1 §2 introduces bitemporal columns that turn the storage layer into an append-only audit log. The retention question is when and how facts move out of the hot retrieval slice without breaking the "what did we know on date X" promise. Cortex already runs retention sweeps (phase9a..9k, phase11v, phase19) for envelopes but those operate on raw event kinds; bitemporal facts are a different shape — a superseded ADR from 2024 is still load-bearing for an `as_of=2024-06-01` query and must stay reachable.</context>
<parameter name="decision">Never delete temporal facts. Move facts to an archive slice only when (a) `lifecycle in (superseded, expired)` AND (b) the most recent `superseded_at` or `valid_to` is older than 5 years from now. Archive slice = Vectorizer cold-binary collection (`cortex.archive.binary`) + Meili archival index (`cortex-archive-bitemporal-v1`) + the Nexus node stays in place (graph never archives; nodes are cheap, edges to/from archived facts stay live so `cortex history` keeps working). The hot retrieval slice (Meili `cortex-<slug>-<family>`, Vectorizer fp32/pq collections) drops the archived rows on the next scheduled sweep. Archive purge from the cold slice requires an explicit `cortex-ops temporal-archive-purge --before <date> --confirm` invocation — never automated. `cortex history <entity>` and `cortex supersession <entity>` walk the archive slice transparently (lazy lookup; one extra hop on cold hits). The pruner (phase11o) does NOT touch bitemporal columns — its existing tier-demotion rules continue to operate on `occurred_at`, not on the bitemporal `valid_*` axes.

## Decision

_No decision recorded._

## Alternatives Considered

- Delete facts older than 5 years — rejected outright; breaks the bitemporal contract (any `as_of` older than 5 years would return empty even when the operator knows the fact existed), invalidates the 5-year audit window promised to compliance reviewers
- Single-tier retention (everything stays in the hot slice forever) — rejected because the hot slice grows unboundedly; the §1.5 hot/cold split keeps the Meili sortable indexes under a million rows per repo even after a decade of operation
- Tie archival to `lifecycle = expired` regardless of age — rejected because a fact that expired last week is still high-recall; the temporal classifier's `EXPIRED` action is a heavy demote, not a relocation
- Per-project archival policy — rejected as configuration sprawl; the 5-year window matches the rulebook's compliance commitment and applies uniformly; operators can override per-archive call with `--before` for one-off compliance sweeps
- Hard-delete on `lifecycle = abandoned` — rejected because abandoned branches must remain audit-reachable (the design.md §2.4 promise: "abandoned approaches stop being re-tried by agents" relies on the classifier dropping them from retrieval, NOT on the storage layer forgetting them)

## Consequences

Wins: bitemporal audit window holds for 5+ years without compromise; hot retrieval slice stays bounded; archive purge stays operator-controlled (no scheduled deletion of compliance-relevant data); the `cortex history` walk continues working post-archival via lazy cold-slice lookup. Costs: the archive slice adds one Vectorizer collection per project + one Meili index alias; the lazy cold-slice fan-out adds 50-150ms on `history` walks that touch archived rows (acceptable — `history` is operator-driven, not hot-path); the migration in P1 §2 has to provision the archive slice up-front. Reassessment trigger: if a compliance review extends the audit window past 5 years, raise the threshold via `cortex-config::TemporalConfig::archive_after_years` (default 5); if the cold-slice fan-out exceeds 500ms, promote the archive index to a dedicated Vectorizer collection per project + family. The §3 classifier is responsible for routing reads through the cold slice transparently; the operator-facing `cortex-ops temporal-archive-purge` lives in the §4 CLI surface.
