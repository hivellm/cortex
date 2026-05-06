# Proposal: phase14a_consolidator-daemon-trait-impls

Source: `docs/analysis/rework/01-consolidation.md` Phases 1-3; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase B.1.

## Why

The consolidator exists as a CLI but never runs as a daemon in production. Triggers (`SessionEnd`, `NightlyTopic`, `DecisionLanded`) are defined but unused. Output silently dropped via the consolidation publisher gap (closed by phase12a). Cost telemetry decentralised across 3 grain producers. The 4-doc consolidation audit names this as the single largest "everything is wired but nothing happens" defect.

Phase A's `Sweep` + `EnvelopeProducer` traits (phase13a, phase13b) make the daemon structurally trivial.

## What Changes

- New `Consolidator` trait + 3 grain impls (`SessionGrain`, `TopicGrain`, `DecisionTraceGrain`) replacing the current 3-file producer arrangement.
- New daemon binary `crates/cortex-workers/src/bin/cortex-consolidator.rs` that subscribes to triggers via the producer-checkpoint table from phase13b and runs the matching grain on each trigger.
- Centralised cost telemetry in `consolidator/cost_telemetry.rs` consumed by all 3 grains.
- Output via `EnvelopeProducer` — re-uses the resilient publisher from phase12a.
- Health endpoint `cortex-api /v1/health/consolidator` reports last-trigger timestamps per grain.

## Impact

- Affected specs: `docs/specs/15-consolidation.md` § Daemon contract.
- Affected code: `crates/cortex-workers/src/consolidator/{trait.rs,session.rs,topic.rs,decision_trace.rs,cost_telemetry.rs}`, `crates/cortex-workers/src/bin/cortex-consolidator.rs` (new binary), `crates/cortex-api/src/dashboard/consolidations.rs`, `docker-compose.yml` (new service).
- Breaking change: NO. Additive.
- User benefit: `cortex_consolidations` Meili index grows nightly; user sees actual consolidations instead of "consolidação não funciona".
