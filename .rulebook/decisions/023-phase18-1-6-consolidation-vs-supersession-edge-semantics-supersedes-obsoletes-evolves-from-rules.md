# 23. phase18 §1.6 — Consolidation vs supersession edge semantics: SUPERSEDES / OBSOLETES / EVOLVES_FROM rules

**Status**: proposed
**Date**: 2026-05-29
**Related Tasks**: phase18_tlb-timeline-branching

## Context

Phase18 P1 §2.4 adds three temporal edges that look similar but mean different things — `SUPERSEDES`, `OBSOLETES`, `EVOLVES_FROM`. Without a sharp rule for when each fires, the writer side becomes ambiguous and the retrieval classifier loses its discrimination signal. The Cortex consolidator already produces "topic cards" + "consolidation envelopes" that look like evolution but sometimes also replace; we need to lock which edge each writer emits so the classifier's drop / demote / keep decisions stay deterministic.</context>
<parameter name="decision">Three edges with disjoint semantics: (1) `SUPERSEDES(from, to)` — `from` REPLACES `to`. Use when the new fact directly overrides the old one (ADR-016 replaces ADR-014; a revised learning replaces the prior version; a decision is rescinded by a new one). Writer-side side effects: `to.superseded_at = supersession_event.valid_time`; `to.lifecycle = superseded`; the temporal classifier's `SUPERSEDED` state filters `to` from default retrievals. (2) `EVOLVES_FROM(from, to)` — non-replacing precursor link. Use when both facts are independently valid and one informed the other (a consolidation envelope evolved from a set of underlying topic cards; an ADR cites a prior learning that informed it). `to.superseded_at` is NOT stamped; `to.lifecycle` is NOT changed; both facts remain VALID at the same `as_of`. (3) `OBSOLETES(from, to)` — `from` makes `to` inapplicable WITHOUT replacing it. Use when a feature is deprecated or a project pivots away from an approach. `to.lifecycle = deprecated`; `to.valid_to` is NOT stamped (the fact remains historically true; it just no longer applies going forward). The classifier treats `deprecated` like `superseded` for default retrievals but with a separate audit channel. The consolidator-side rule: a consolidation envelope emits `EVOLVES_FROM` to its source topic cards (the topic cards are still valid; the consolidation summarises them). The ADR promotion path (phase20 §8) emits `SUPERSEDES` to the prior accepted ADR. Decision matrix carries 1:1 cardinality for SUPERSEDES and OBSOLETES (one replacement per supersession event); many-to-many cardinality for EVOLVES_FROM (one consolidation can EVOLVES_FROM multiple cards).

## Decision

_No decision recorded._

## Alternatives Considered

- Single `REPLACES` edge with a `kind` discriminator (supersedes / obsoletes / evolves) — rejected because the classifier needs to switch behaviour at edge-walk time and a single edge type forces every walk to inspect the kind property; three distinct edges make the query plan simpler in Cypher (`MATCH (n)<-[r:SUPERSEDES]-(s) RETURN s` vs `MATCH (n)<-[r:REPLACES {kind:'supersedes'}]-(s) RETURN s`)
- Use only `SUPERSEDES` and treat consolidation as a special case in the writer — rejected because the consolidator legitimately produces non-replacing summaries; conflating them would either drop the source topic cards prematurely (bad recall) or never demote a truly replaced ADR (bad ranking)
- Add `CONSOLIDATES` as a fourth edge for the consolidator-specific path — considered but rejected because the consolidator's behavior fits cleanly under `EVOLVES_FROM`; introducing a fourth edge would create a writer-only label without a distinct classifier rule
- Per-project edge semantics (each project picks which rule applies) — rejected as configuration sprawl; the temporal classifier MUST behave identically across projects for cross-project queries to compose
- Time-bound `OBSOLETES` (auto-stamp `valid_to = now`) — rejected because deprecation is about applicability going forward, not about historical truth; the bitemporal audit must keep the fact's original `valid_to = NULL` so an `as_of` query before the deprecation still surfaces it as VALID

## Consequences

Wins: classifier rules become a single switch on edge type (SUPERSEDED if SUPERSEDES edge; ABANDONED if part of a discarded merge; deprecated demote if OBSOLETES edge; VALID otherwise); writer side becomes deterministic (the consolidator emits EVOLVES_FROM only; the ADR promoter emits SUPERSEDES only; deprecation tooling emits OBSOLETES only); the Cypher query plan in the classifier stays cheap (label-keyed seek). Costs: writers must pick the right edge at emit time (no late binding); the migration in P1 §2.11 has to backfill SUPERSEDES edges for existing ADRs with `status = superseded` (deterministic from the existing `supersedes` frontmatter field). Reassessment trigger: if a new writer surface emerges where neither rule fits (e.g. a "branch fork" event that needs both EVOLVES_FROM and CROSS_PROJECT_REF semantics), add a fourth edge with explicit classifier rules rather than overloading any existing edge. The §3.2 classifier state machine encodes these rules in code; the §2.4 schema definition + §2.11 backfill encode them at the data layer.
