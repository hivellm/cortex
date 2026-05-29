# 20. phase18 §1.3 — Cross-project retrieval default: opt-in until eval evidence justifies opt-out

**Status**: proposed
**Date**: 2026-05-29
**Related Tasks**: phase18_tlb-timeline-branching

## Context

Phase18 P4 lands `CROSS_PROJECT_REF` edges and the cross-project propagation step in the fusion pipeline. The open question is whether the default for `cortex_query` should walk those edges automatically or stay scoped to the calling project. Cross-project walks expand the candidate set 5–10x in practice (Cortex depends on Vectorizer + Nexus + Synap + Lexum + Rulebook), which increases recall but also raises noise — a question about Cortex's retention sweep should not pull in Nexus's retention spec unless the operator asked.</context>
<parameter name="decision">Cross-project retrieval defaults to OFF. `query.cross_project.enabled = false` in `cortex-config`; callers opt in via the CLI `--projects p1,p2` flag, the HTTP body `projects: [...]` field, or the MCP `projects: [...]` arg. When opt-in is supplied, `query.cross_project.max_hops` defaults to 1 (one CROSS_PROJECT_REF edge walked per top-K candidate; deeper walks require an explicit `max_hops` override). The opt-in flag also drops the default `temporal_window_days` from 30 to 90 to accept slightly older cross-project facts (a cross-project dependency rarely updates as fast as in-project ones). The default-OFF rule is reassessed when the CDC harness time-sensitive subset shows cross-project MRR@10 ≥ +10% over the in-project baseline AND the time-insensitive subset shows no negative delta on noise queries (random-pick top-3 inspection).

## Decision

_No decision recorded._

## Alternatives Considered

- Default ON for every query — rejected because the operator-facing surface today expects scoped retrieval; flipping the default would silently regress every existing automation that consumes `cortex_query`
- Default ON for `decision_lookup` intent only — considered but rejected because the intent is too coarse: a `decision_lookup` for Cortex internals should NOT surface Nexus ADRs unless asked; the per-intent decision is better expressed as the operator passing `--projects cortex,nexus` explicitly
- Default ON with a noise filter (cross-project hits only if their score after temporal classification beats the in-project median) — rejected as premature; we have no production evidence the median-floor heuristic works, and shipping it without an eval gate would be the same bug shape phase11i already warned about (heuristic without an MRR floor)
- Default OFF but auto-enable when the query mentions another project name verbatim — considered but rejected as a hidden behavioural switch; explicit opt-in is auditable
- Per-repo config (default per project) — rejected as configuration sprawl; a single workspace-level default that operators override per-call keeps the rule one place to inspect

## Consequences

Wins: existing automation (Claude Code adapter, dashboard, MCP tools) keeps current behaviour without code changes; the noise budget on `cortex_query` stays bounded; the eval gate forces evidence before the default flips. Costs: operators who want cross-project context must remember the flag; downstream agents (Claude Code skills) need to plumb the flag through when an intent benefits from it (e.g. `decision_lookup` against shared deps). Reassessment trigger: re-evaluate after the CDC harness shows cross-project hit ratio AND the time-insensitive subset shows no regression. The §5.3 config knob (`query.cross_project.enabled`, `query.cross_project.max_hops`) is the flip surface; defaults live in `cortex-config::QueryConfig::Cross::default()`.
