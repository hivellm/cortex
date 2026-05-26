# 5. Consolidation grain choice — Session / Topic / DecisionTrace

**Status**: proposed
**Date**: 2026-05-03
**Related Tasks**: phase11j_consolidation_tier, phase11o_vectorizer_demotion_api

## Context

Phase11j §1 introduced `Kind::Consolidation` — an LLM-summarised distillation of raw events the agent reads instead of (or before) reading the raw turn-by-turn corpus. The grain enum drives every downstream decision: which producer runs, which Vectorizer collection / Meili index lands the row, which model summarises (Haiku 4.5 or Opus 4.7), the depth (Shallow / Deep) that drives the fidelity threshold, and the format the pre-thinking renderer prints in the `## Consolidated context` section.

The grain space could have been organised many ways: per-day, per-tool, per-error-class, per-spec, per-repo, etc. Choosing the wrong axis early forces every future producer to fight the data shape.

Three axes the proposal evaluated:

1. **Session** — one consolidation per Claude Code session. Natural anchor: every `session_id` already groups N raw envelopes; the rendering surface ("past sessions" → "consolidated context") was already in the spec; cost predictable (Haiku × ~30 envelopes per session).

2. **Topic** — one consolidation per HDBSCAN cluster of sessions sharing semantic similarity (anchored on a single repo so the cluster does not bleed cross-project). Useful for cross-session "what's the pattern across the last month of HNSW work" questions; scales to corpus growth without producing more rows than the agent can read.

3. **DecisionTrace** — one consolidation per `Kind::Decision` envelope plus its parent chain (≤ 16 hops). Auto-promotes to Opus 4.7 because the chain depth + design-intent fidelity gate (≥ 98 %) needs the bigger model. Drives the "trace the design behind decision X" use case the dashboard's decision detail view surfaces.

Other axes the proposal explicitly considered and rejected:

- **Per-day**: anchors on a calendar boundary instead of semantic boundary; produces useless consolidations on quiet days and oversized ones on busy days.
- **Per-tool**: too narrow; loses cross-tool context that the renderer's existing `Similar past turns` section already surfaces well.
- **Per-error-class**: useful for postmortems but dwarfed by `decision_lookup` intent which already covers it.
- **Per-spec**: too coarse; a single spec spans years and dozens of decisions.
- **Per-repo**: monthly cadence would land 12 × N consolidations / year; the "consolidated context" lane saturates quickly.

## Decision

Ship three grains: `Session`, `Topic`, `DecisionTrace`. Anchor each on the natural identifier the source data already carries (`session_id`, HDBSCAN cluster label, `decision_id`). Pin the producer / model / depth mapping in `ConsolidationGrain` so future grain additions inherit the same shape:

- `Session` → Haiku 4.5, Shallow depth, scope = `SessionId(_)`.
- `Topic` → Haiku 4.5, Shallow depth, scope = `Topic(label)`. Minimum cluster size 3.
- `DecisionTrace` → Opus 4.7 (auto-promote), Deep depth, scope = `DecisionId(_)`. Max chain hops 16.

Encode the (grain, scope) variant compatibility in `validate_consolidation_payload` so a producer that emits `grain = Topic` with `scope = SessionId` fails validation up-front instead of polluting the lane.

## Alternatives Considered

- Per-day cadence — rejected: calendar boundary is not a semantic boundary; quiet/busy days produce useless or oversized consolidations.
- Per-tool — rejected: too narrow; loses cross-tool context the existing `Similar past turns` section already surfaces well.
- Per-error-class — rejected: dwarfed by the `decision_lookup` intent's existing coverage.
- Per-spec — rejected: too coarse; one spec spans years and dozens of decisions.
- Per-repo monthly — rejected: 12 × N consolidations/year saturates the `Consolidated context` lane quickly.
- Single grain (Session-only) — rejected: would not capture cross-session patterns (Topic) or design-intent chains (DecisionTrace) the dashboard's decision-detail view needs.

## Consequences

**Positive:**
- The renderer's `## Consolidated context` line shape (`grain/id · date · ✓|✗|⚠ · title`) needs only the grain label as a one-token prefix; no per-axis rendering branches.
- Fidelity threshold tuning (Shallow ≥ 90 % / Deep ≥ 98 %) follows depth, which follows grain — operators tune one knob per producer instead of a matrix.
- `derive_consolidation_id(grain, scope)` produces a stable id per (grain, scope) pair so re-runs are idempotent. Cross-grain collisions are impossible because the prefix differs (`cons-ses-`, `cons-top-`, `cons-dec-`).
- Cost predictable per grain: Haiku-on-session ~80 cents, Haiku-on-topic ~80 cents, Opus-on-decision-trace ~4 000 cents. The orchestrator's per-grain `CostLedger` bucket lets operators see where budget goes.

**Negative / tradeoffs:**
- Adding a fourth grain later requires touching: `ConsolidationGrain` enum, `ConsolidationScope` tagged union, the validator's compatibility table, the JSON schema, the producer module layout (`producer/{new_grain}.rs`), the orchestrator's `ProducerSelection::for_trigger`, the cost-estimate table, the CLI subcommand, the routing surface (`family_for` + collection), and the dashboard's grain filter chip. The blast radius is non-trivial — adding a grain is a deliberate phase, not an incremental change.
- DecisionTrace's auto-promotion to Opus 4.7 makes the cost model jump 50× per consolidation. Operators with a $1 000/month cap can produce ~25 DecisionTraces/month before the budget gate trips. Documented in `docs/cortex/consolidation-tuning.md`.
- HDBSCAN clustering for Topic lives in the orchestrator (§2.7), not the producer (§2.5), so the algorithm is swappable. If HDBSCAN's parameters drift the corpus into many tiny clusters, the orchestrator can swap to a different clusterer without touching the producer pipeline. Untested in production; first real run will surface tuning issues.

## Live read path (added 2026-05-05 by phase11p)

The producer side now has a live envelope source. `LiveSessionSource`,
`LiveTopicSource`, and `LiveDecisionTraceSource` (under
`crates/cortex-workers/src/consolidator/source/`) feed
`Orchestrator::run_session` / `run_topic` / `run_decision_trace`
from the parquet archive via two new `cortex-storage` helpers
(`scan_envelopes_by_session`, `scan_envelope_by_event_id`). The
nightly cadence is owned by the new cron seed
`retention.consolidator_nightly` (`0 2 * * *`, runs one hour
before `retention.consolidation_prune` so the pruner sweeps over
fresh rows). HDBSCAN runs with `min_samples = 1` after the unit
tests caught the default core-distance gate over-rejecting tight
clusters at the spec's `min_cluster_size = 3` floor — captured as
a learning under
`.rulebook/learnings/2026-05-05T00-00-00-hdbscan-min-samples-1-needed-for-tight-clusters-at-min-cluster-size-floor.md`.

The cron timeline (02→03) is now load-bearing for the consolidation
tier; if any other subsystem grows a dependency on the same slot
ordering it should be lifted into a fresh ADR.
