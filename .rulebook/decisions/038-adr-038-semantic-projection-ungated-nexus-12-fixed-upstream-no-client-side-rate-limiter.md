# 38. ADR-038: semantic projection ungated — nexus#12 fixed upstream, no client-side rate limiter

**Status**: proposed
**Date**: 2026-08-02
**Related Tasks**: phase29_graph-projection-unblock

## Context

Supersedes ADR-027's gating note. nexus#12 (sustained-write busy-loop) and nexus#11 (index persistence) closed upstream 2026-06-08, before the Nexus 2.5.0 this repo pins. Validated empirically at phase15c reproduction scale: a 5000-envelope backfill burst (4678 edges in one unthrottled pass) kept Nexus 2.5.0 answering queries throughout (probes 91ms-2.1s), CPU settled to 3.7% after, no restart. Decision: enable CORTEX_GRAPH_PROJECTION_ENABLED=true in production WITHOUT the client-side token-bucket scheduler the phase29 task originally scoped - a rate limiter would be an unrequested workaround for a fixed upstream bug (simplicity-first). If a future Nexus regression re-trips the stall, the phase29 task file carries the design notes for the scheduler. Community detection over the projected architecture subgraph runs nightly (cortex-ops graph communities-detect, cron graph.community_detect, 02:30). Known residue recorded in phase29 tasks section 5: Decision anchor nodes lack id/title props (template matching gap), CALLS/IMPORTS await the phase15c classifier replay.

## Decision

_No decision recorded._

## Consequences

_No consequences documented._
